# SPEC: Typing `exit` in the agent pane's shell drawer should close the shell and collapse the drawer

**Date:** 2026-09-15
**Status:** proposed — design only, no code changed by this document.
**Related:**
`docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` (close-on-exit for
top-level panes, and §11's deliberate exclusion of sub-blocks — read that
before implementing this),
`docs/specs/SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md` (the drawer shell
itself),
`docs/specs/SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md` (where it renders),
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

1. **Crash vs. intentional exit.** A shell that dies because the backend
   restarted, or because the process crashed, produces the same
   `STATUS_DONE`. Collapsing the drawer on a crash may hide the error output
   the human needs. `TermResyncHandler`'s crash-recovery path deliberately
   revives such a shell for top-level term panes; the drawer has no
   equivalent. Cheapest resolution is to collapse only on exit code 0 and
   leave the drawer open (with its scrollback intact) otherwise — but
   `BlockControllerRuntimeStatus` carries `shellprocexitcode`, so this is a
   decision to make, not a capability to build. **Recommend: collapse on
   exit code 0 only.**
2. **Drawer height.** `term:shellheight` persists across open/close. Nothing
   here changes it; confirm on implementation that a collapse-then-reopen
   still restores the human's chosen height rather than the default.
