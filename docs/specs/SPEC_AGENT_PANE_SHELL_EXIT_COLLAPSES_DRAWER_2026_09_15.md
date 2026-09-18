# SPEC: Typing `exit` in the agent pane's shell drawer should close the shell and collapse the drawer

**Date:** 2026-09-15
**Status:** implemented — §3's design shipped in the same PR as this document.
§5's first open question is resolved (clean exits only; see §6). — #3253
**Related:**
`docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` (close-on-exit for
top-level panes, and §11's deliberate exclusion of sub-blocks — read that
before implementing this),
`docs/specs/SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md` (where the drawer
shell renders),
`docs/specs/SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md` (its mount
sequence — the zoom seed the exit subscription now sits alongside),
`docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` (PtyShell
attach/reuse and the agent lease that shares this same shell).

---

## 1. Symptom

Open the shell drawer in an agent pane, type `exit`, press Enter. The shell
process dies, and the drawer stays open showing a dead terminal: it renders
its final scrollback, accepts keystrokes that go nowhere, and gives no
indication that anything ended. The only way out is to toggle the drawer
closed by hand.

Expected: the shell exits *and* the drawer collapses, the same way typing
`exit` in a standalone Terminal pane now closes that pane
(`SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §10).

## 2. Why it behaves this way today

Three facts compose into it. None is a bug on its own.

**2.1 The drawer's shell is a sub-block, and close-on-exit deliberately
excludes sub-blocks.** `shell/lifecycle.rs:437`:

```rust
let close_on_exit = Self::effective_close_on_exit(&block_meta, &self.controller_type) && !is_sub_block;
```

`effective_close_on_exit` already defaults to `true` for
`BLOCK_CONTROLLER_SHELL`, so the drawer's shell *would* qualify — the
`!is_sub_block` term is what excludes it, and it is there for a good reason
(§11 of that spec, found by live testing): the close action is
`sagas::delete_block::run`, which deletes the block and prunes it from the
owning Tab's layout tree. A `PtyShellCreate`/drawer shell has no layout entry
to prune, and deleting it leaves the parent agent block's
`term:shellsubblockid` pointer dangling — whereupon the drawer's own
attach-or-create logic silently spawns a replacement moments later,
reproducing the original respawn loop with extra latency. **Do not implement
this spec by removing that exclusion.**

**2.2 The drawer's open/closed state is frontend reducer state, not
anything the backend can see.** `paneModel.state.detailsOpen`
(`agent-view.tsx:2925` gates the whole `agent-composer-details` region;
actions `DetailsExpand`/`DetailsCollapse`, `reducer.ts:1508`). It is
persisted through the pane snapshot (`useSnapshotPersistence.ts`), so it
survives reload. There is no meta key and no backend concept of "the drawer
is open" — which is why the close must be driven from the frontend.

**2.3 Nothing tells the drawer its shell died.** `AgentShellSubblock.tsx`
has no subscription to controller status at all: it creates or reattaches the
sub-block, constructs a `TermWrap`, and from then on only reacts to zoom meta
and the agent lease. The backend *does* publish the exit —
`publish_controller_status` fires a `controllerstatus` WPS event carrying
`shellprocstatus: "done"` — and the frontend already has machinery to consume
those (`wps-events.ts`'s `ControllerStatus`, `useControllerStatusEvents`,
which `agent-view.tsx:1607` uses for turn tracking). Nobody has wired the
drawer's own sub-block to it.

## 3. Design

Frontend-driven, using the event the backend already publishes. Nothing in
`lifecycle.rs` changes; §2.1's exclusion stays exactly as it is.

### 3.1 The drawer learns its shell exited

`AgentShellSubblock` subscribes to `controllerstatus` for **its own
sub-block id** (not the parent agent block's) and treats
`shellprocstatus === "done"` as the exit signal. Two notes:

- Subscribe only while a shell id exists, and unsubscribe on cleanup — the
  component already tracks `subBlockId()` and has an `onCleanup` path.
- `STATUS_DONE` is published once per process exit, but the component may
  mount against an *already*-done controller (reopening a drawer whose shell
  died while collapsed). Handle both: subscribe, and also treat a `done`
  status observed at attach time as an immediate exit. The existing attach
  path already calls `ControllerResyncCommand` and can be told not to
  resurrect (`RESYNC_ERR_ALREADY_EXITED` exists for exactly this, added by
  `SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §8).

### 3.2 What happens on exit

In order:

1. **Dispose the terminal.** Existing `onTermDispose`
   (`AgentShellSubblock.tsx:419` → `agent-view.tsx:1164`) already clears the
   parent's `termWrite` handle; reuse it rather than adding a second teardown.
2. **Tell the parent**, via a new `onShellExited?: () => void` prop — a
   distinct callback from `onTermDispose`, which also fires on ordinary
   unmount (drawer collapse, pane dispose) and therefore cannot carry "the
   process ended" without becoming ambiguous.
3. **The parent clears the pointer.** `agent-view.tsx` sets
   `term:shellsubblockid` to `null` via `SetMetaCommand` and deletes the
   sub-block (`DeleteSubBlockCommand`), the same call its `onCleanup` already
   makes. This is what prevents §2.1's respawn: the next drawer open finds no
   pointer and creates a genuinely fresh shell instead of resyncing a
   `STATUS_DONE` controller.
4. **The parent collapses the drawer**: `dispatchPane({ type: "DetailsCollapse" }, "system")`.
   `"system"` rather than `"user"` — the human did cause it, but indirectly,
   and the dispatch-source field is used for provenance in the reducer's own
   diagnostics.

`DetailsCollapse` is already a no-op when `detailsOpen` is false
(`reducer.ts:1509`), so a late or duplicated exit signal is harmless.

### 3.3 Interaction with the agent lease

The same shell can be driven by an agent through `PtyShell*`
(`SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` §10). On exit the
backend already releases the lease — both the in-memory registry and the
`term:agentlockuntil` meta the drawer gates on (PR #3249). That ordering
matters here: if the meta copy were left set, the human's keystrokes would
still be suppressed client-side while the drawer collapsed around them.

**Decision: exit collapses the drawer even if an agent is mid-work.** The
shell is gone either way; leaving a dead terminal open to represent an
agent's interest in it shows nothing useful. The agent's next
`PtyShellCreate` creates a fresh shell (already true — §8 of the exit-loop
spec), and that call does **not** re-open the drawer: the drawer is the
human's view, and an agent acquiring a shell has never opened it.

### 3.4 What is deliberately NOT done

- **No backend pane-close saga for sub-blocks.** §2.1.
- **No auto-reopen**, on either an agent's shell creation or the next
  message. Collapsing is a terminal state until the human opens the drawer.
- **No "shell exited" banner in the collapsed state.** The exit is the
  human's own action, taken one keystroke earlier; announcing it back to
  them is noise. (Revisit only if §5's first open question resolves toward
  exits the human didn't cause.)

## 4. Test plan

Frontend (`AgentShellSubblock.test.tsx`, `agent-view` suite):

1. A `controllerstatus` event with `shellprocstatus: "done"` for the
   drawer's sub-block id → `onShellExited` fires exactly once.
2. The same event for a *different* block id → nothing fires. (The pane
   subscribes to controller status for its own agent block already; the two
   subscriptions must not cross.)
3. On `onShellExited`: `term:shellsubblockid` is cleared, the sub-block is
   deleted, and `DetailsCollapse` is dispatched — asserted as three distinct
   effects, since any one of them silently not happening leaves a different
   broken state (dangling pointer → respawn; live PTY → leak; open drawer →
   the original bug).
4. Reopening the drawer after an exit creates a NEW sub-block id rather than
   reattaching the dead one.
5. An exit signal arriving when the drawer is already collapsed is a no-op
   and does not throw.

Each of these must be falsified — break the corresponding wiring, confirm
that specific test fails — before the PR is opened. The drawer's existing
tests pass against a component that has never once observed a process exit,
which is precisely how this gap survived.

## 5. Open questions

1. ~~**Crash vs. intentional exit.**~~ **RESOLVED — clean exits only.** A
   shell that dies from a crash or a backend restart produces the same
   `STATUS_DONE`, and collapsing the drawer there would hide the very output
   the human needs to read. Implemented as: collapse on `shellprocexitcode`
   0 (or absent — the field is `#[serde(default)]`, so a clean exit can
   arrive omitted), leave the drawer open with its scrollback on anything
   else. Both branches are tested.
2. **Drawer height.** `term:shellheight` persists across open/close and is
   read from the PARENT agent block's meta (`agent-view.tsx`'s
   `ResizableDetailsDrawer persistedHeight=`), not the sub-block's — so
   deleting the exited sub-block cannot disturb it, and a
   collapse-then-reopen restores the human's chosen height. Confirmed by
   reading the call site; not separately regression-tested.

## 6. What shipped

Implemented exactly as designed in §3 — `lifecycle.rs` untouched, §2.1's
sub-block exclusion intact.

- **`AgentShellSubblock.tsx`** subscribes to `controllerstatus` for its own
  sub-block id and fires a new `onShellExited` prop on a clean exit.
  Subscribed from a `createEffect`, not `onMount`: a freshly created shell
  has no id at mount (the async IIFE assigns it), so a one-shot mount
  subscription would bind an empty scope and never fire — which is the
  *common* case of opening the drawer for the first time. Guarded to fire at
  most once per mount, since `STATUS_DONE` can be republished and the
  parent's handler tears down real state.
- **`shell-exit-collapse.ts`** (new) holds the parent-side response, split
  out of `agent-view.tsx` purely so it can be tested — that file is ~3000
  lines with no render harness, and these are exactly the effects that
  silently stop happening.
- **`agent-view.tsx`** reads-and-clears its `shellSubBlockIdRef` (so the
  pane-level cleanup can't later delete a block this already removed) and
  delegates.

**Tests, each falsified** (break the source, confirm the specific test
fails, restore): thirteen in `AgentShellSubblock.test.tsx`, six in
`shell-exit-collapse.test.ts`.

**Not verified on a running build.** The whole path is unit-tested but has
never been exercised against a real PTY in a real drawer.

## 7. Three things review caught that §3's design did not

All three were live in the first pushed revision. Recording them because
each is a property of *this subsystem*, not a slip peculiar to this PR.

**7.1 (P0) A replayed exit is not an exit.** `controllerstatus` is published
with `persist: 1` specifically so a new subscriber is handed the CURRENT
status synchronously as part of subscribing (`blockcontroller/mod.rs`,
`wps.rs`'s `replay_to_route`). §3.1 hand-waved this as "handle both" and the
implementation did not. The failure it produced was worse than the bug this
spec exists to fix: an agent's shell exits while the drawer is closed (§3.3's
supported case), the human opens the drawer *to read what the agent did*, the
broker replays that old `done` at subscribe time, and the drawer collapses
and deletes the sub-block — destroying its persisted `term` file. Not hidden
behind a collapsed drawer; gone.

Fixed by requiring the shell to have been observed ALIVE first: a live exit
is always preceded by the `running` status the shell published while running
(itself replayed, for a shell that is running). No `running` seen ⇒ the shell
was already over before this mount ⇒ not ours to collapse; the attach path
handles it (7.2).

**None of the original twelve tests could have caught this**, because the
`waveEventSubscribe` mock only delivered events a test emitted after mount —
the replay slot didn't exist in the harness, so the entire code path was
structurally invisible. The mock now delivers a queued status synchronously
inside `subscribe`, like the broker.

**7.2 (P2) Reattaching to an exited shell used to silently revive it.** The
drawer's attach path calls `ControllerResyncCommand`, whose handler passed
`respawn_if_done: true` — correct for its original caller (`term.tsx`'s
crash-recovery `TermResyncHandler`), wrong for a caller that is only asking
whether the shell is still alive. Reopening after an exit resurrected the
dead controller in place and appended a fresh banner to the block's
append-only `term` file: the respawn-loop symptom
`SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §7 fixed on the `PtyShellCreate`
side, still open on this one. Added a `norespawn` flag to the resync command
(default `false` — every existing caller unchanged) and the drawer sets it,
so an exited shell surfaces as `RESYNC_ERR_ALREADY_EXITED` and takes the same
fall-through as a vanished one: a genuinely fresh shell, clean scrollback,
dead block deleted rather than orphaned.

**7.2a (P0, round 2) The attach path deleted crash diagnostics.** 7.2's
`norespawn` fix made an exited shell fail its resync — and the catch block
deleted the block and created a fresh one on *any* failure, exit code
unexamined. For a shell that crashed while the drawer was closed, that
persisted `term` file is the only record of why, and the live listener
already refuses to collapse on a non-zero exit (§5) precisely to preserve it.
So the same "not hidden — gone" data loss 7.1 had just closed was still open
through the attach path.

The delete is now gated on a positively-known **clean** exit, read from the
same replayed status 7.1 taught the component to receive. `undefined` — a
genuinely vanished block, or a status we were never told — is NOT treated as
clean: skipping a delete costs at worst a dead block lingering until the pane
closes, while getting it wrong costs the user their diagnostics. A vanished
block needs no delete anyway.

**Residual limitation, deliberately not fixed here:** the crashed block is
*preserved* but not *shown*. The drawer still creates a fresh shell and
repoints `term:shellsubblockid` at it, so the crash output survives on disk
with no UI route back to it. Fixing that means attaching the drawer read-only
to the dead block — which trades the data-loss bug for "the human has a
terminal they cannot type into", the very state this feature exists to
remove. That trade-off deserves its own decision, not a silent one made
inside a review round.

**7.3 (P2) The teardown RPCs had to be ordered.** `DeleteSubBlockCommand`
read-modify-writes the parent to drop the child from `subblockids`. Fired
concurrently with the pointer clear, it can read a pre-clear snapshot and
write it back afterwards, RESTORING the dead pointer — leaving the pane
pointing at a deleted block. They are now awaited in order. The UI half
(collapse) still runs first and synchronously: it needs no round trip, and
the drawer should go the instant the shell does.
