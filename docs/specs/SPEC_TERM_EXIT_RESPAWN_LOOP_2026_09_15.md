# SPEC: Typing `exit` in a terminal pane doesn't close it — instead the shell respawns and appears to loop

**Date:** 2026-09-15
**Status:** implemented (§4b's respawn loop — see §7; §4a, close-on-exit
being dead code, is a separate, pre-existing gap deliberately left open —
also see §7)
**Related:** `docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` (PtyShell attach/reuse semantics),
`docs/reports/REPORT_RENDERER_CPU_UNBATCHED_PTY_OUTPUT_2026_09_11.md` (PTY output coalescing — ruled out, see §3)

## 1. Symptom

In a terminal pane, typing `exit` and pressing Enter is expected to end the
shell and close the pane. Instead, the pane stays open and the terminal
repeatedly emits the literal text `exit` followed by what looks like three
page-feed/form-feed control characters, in a loop, rather than settling into
a closed or "done" state.

## 2. Repro context

Found immediately after fast-forwarding `agentmux` from `cdbdc61ab` to
`origin/main` (`e03678a39`, 14 commits). Two of those commits touch exactly
this code path:

- `76ea0bb14` — "perf(srv): coalesce PTY output before broadcasting" —
  rewrote `agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs`
  (818-line diff), `shell/pty.rs`, and `server/websocket.rs`.
- `15ed08943` — "feat(mcp): PtyShell attaches to the pane's real visible
  shell, with a lease-based lock" — added `agentmux-srv/src/server/mod.rs`'s
  `/api/v1/ptyshell/*` endpoints and changed `PtyShellStop`'s semantics.

## 3. What was ruled out

The new PTY output coalescing/flush logic in `lifecycle.rs` (commit
`76ea0bb14`) is **not** the source of a repeating broadcast:

- `lifecycle.rs:865-917` (blocking PTY read loop): `Ok(0) => break` on EOF,
  `Err(e) => break` on read error — unchanged in behavior from before the
  coalescing commit. Breaking drops `pty_tx` (moved into the closure, never
  cloned), closing the output channel exactly once.
- `lifecycle.rs:97-208` (`run_pty_output_flusher`): terminates deterministically
  on channel close (`Ok(None) => break true`, `if closed_mid_batch { break; }`).
  No branch retries or re-sends an already-flushed batch, and no branch
  busy-loops.
- `lifecycle.rs:1010-1136` (wait/cleanup task): reaps the child unconditionally
  first, then bounds the flusher join with `FLUSHER_DRAIN_TIMEOUT` (10s,
  `pty.rs:79`). Per the report's design intent, a timeout here deliberately
  does **not** `abort()` the flusher — it detaches it, so a flusher stuck
  behind a background descendant that still holds the PTY slave fd open
  keeps running silently rather than looping or re-broadcasting old output.
  This is a real, documented edge case, but it produces *silence*, not a
  repeating "exit" broadcast — it doesn't match the symptom.

## 4. Root cause: no close-on-exit wiring, and `resync_controller` silently respawns a dead shell

Two separate, compounding facts:

### 4a. Close-on-exit is dead code

`agentmux-srv/src/backend/blockcontroller/shell/controller.rs:218-245` fully
implements the meta-driven close-on-exit knobs:

```rust
#[allow(dead_code)]
pub(super) fn should_close_on_exit(meta: &MetaMapType) -> bool { ... }
#[allow(dead_code)]
pub(super) fn should_close_on_exit_force(meta: &MetaMapType) -> bool { ... }
#[allow(dead_code)]
pub(super) fn close_on_exit_delay_ms(meta: &MetaMapType) -> u64 { ... }
```

All three carry `#[allow(dead_code)]` and are referenced only from
`shell/tests.rs`. Grepping the whole tree confirms none of them is called
from the wait/cleanup task (`lifecycle.rs:1083-1136`, exactly where
`STATUS_DONE` is set and published), from `blockcontroller/mod.rs`, from
`server/`, or from any frontend file. **There is no code path today that
ever closes a `shell`-controller pane in response to its child process
exiting.** This is pre-existing — not introduced by the 14 pulled commits —
and by itself explains "the pane doesn't close."

### 4b. `resync_controller` respawns any shell whose status is `STATUS_DONE`

`agentmux-srv/src/backend/blockcontroller/mod.rs:568-585`:

```rust
} else {
    let status = ctrl.get_runtime_status();
    ...
    if status.shellprocstatus == STATUS_INIT || status.shellprocstatus == STATUS_DONE {
        return ctrl.start(block_meta.clone(), rt_opts, force);
    }
    return Ok(());
}
```

Any resync against a `shell`-controller block whose process already exited
(`STATUS_DONE` — exactly the state left behind after the user's `exit`) spawns
a **brand-new shell process in the same pane**, rather than leaving it closed.
This logic itself predates the 14 pulled commits, but commit `15ed08943`
added a new, much more easily triggered caller of it:

`agentmux-srv/src/server/mod.rs:1170-1219` (`try_attach_to_existing_shell`,
called from `handle_pty_shell_create`, the `/api/v1/ptyshell/create` HTTP
handler backing the MCP `PtyShellCreate` tool) calls `resync_controller(...)`
**unconditionally** on every call, with no check of whether the shell is
still alive:

```rust
async fn try_attach_to_existing_shell(...) -> Option<Response> {
    let block = state.wstore.get::<Block>(id).ok().flatten()?;
    ...
    if let Err(e) = blockcontroller::resync_controller(&block, "ptyshell", None, false, ...) {
        ...
    }
    answer_conpty_handshake_if_seen(state, id, baseline_len).await;
    ...
}
```

Compounding this, the same commit changed `PtyShellStop`'s semantics from
*destructive* (kill the process, delete the block) to *non-destructive*
(`agentmux-srv/src/server/mod.rs:1812-1834`, `handle_pty_shell_stop` —
clears `term:agentlockuntil` only, never touches the process or the block).
The design intent, per the handler's own doc comment
(`server/mod.rs:1150-1168`), is that a pane's shell is now shared and
pane-scoped: `PtyShellCreate` reuses "this pane's one shell"
(`META_KEY_SHELL_SUBBLOCK_ID`) in either direction — a human's
composer-drawer shell (`AgentShellSubblock.tsx`) and an agent's PtyShell tool
calls are meant to be the *same* PTY.

Put together: once the human's `exit` leaves the pane's shell block at
`STATUS_DONE`, **any subsequent `PtyShellCreate` call against that same
pane** (e.g., an agent in the same agent-pane still holding/re-acquiring the
lease-based lock, retrying a shell command, or simply attaching again)
silently resurrects the shell via `ctrl.start()` instead of failing or
creating a genuinely new pane. Each respawn opens a fresh PTY
(`lifecycle.rs:380-392`), re-deploys shell integration, and prints a fresh
prompt/banner into the pane's `term` file. That file is **append-only for
the block's entire lifetime** — confirmed via `termwrap.ts`'s
`doTerminalWrite`/`handleNewFileSubjectData` handling, which only appends on
a plain resync and never clears the scrollback — so each respawn's startup
output (and, if the respawn cycle repeats, each subsequent `exit`) is
appended after the last, visually indistinguishable from "the same output
repeating."

`server/mod.rs:1150-1157`'s own doc comment states this reuse is deliberate
design (`docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md`), but
neither that spec nor the resync path it depends on account for the case
where the shared shell has already exited — `resync_controller` was written
for the "PTY died unexpectedly, revive it transparently" case (crash
recovery, backend restart — see `term.tsx`'s `TermResyncHandler` comment),
not for "the human intentionally typed `exit`," and it cannot currently tell
the two apart.

## 5. Not yet confirmed

- **The exact "3 page-feed" bytes.** No `\x0c`/form-feed constant or emitted
  sequence was found in `websocket.rs`, `lifecycle.rs`, `pty.rs`, or the
  deployed shell-integration scripts
  (`agentmux-srv/src/backend/shellintegration/{bash.sh,zsh.sh,pwsh.ps1,fish.fish}`).
  The closest candidate is `answer_conpty_handshake_if_seen`
  (`server/mod.rs:~1605-1633`, new in `15ed08943`, Windows/ConPTY-only): it
  writes an `ESC[1;1R` cursor-position-report reply into the PTY when it
  spots a buffered `ESC[6n` query past a baseline offset recorded just
  before each resync. That's a different escape sequence than a form feed,
  but it is exactly the kind of new, resync-triggered injection into a live
  PTY that's worth checking against a real capture — especially since the
  reporter's machine is Windows, where ConPTY is in play and a fast
  respawn loop could plausibly cause the baseline/offset logic to
  misidentify which `ESC[6n` it's answering.
- **What actually drives the *repeated* respawns in a pure human-only
  repro** (no agent visibly touching the pane). `TermResyncHandler`
  (`frontend/app/view/term/term.tsx:36-79`) only calls `resyncController` on
  a `connStatus` transition or a backend-restart detection — for a plain
  local terminal, `connStatus` doesn't change on process exit (the
  handler's own comment says as much), so it alone doesn't explain a loop.
  The more likely context is a pane using `AgentShellSubblock.tsx` (an
  agent pane's composer-drawer shell) where an agent process is also
  attached via `PtyShellCreate`/`PtyShellInput` to the same underlying
  block — each of the agent's own tool calls against that block re-triggers
  §4b. This needs a live repro with logging (`muxlog srv grep ptyshell`,
  `muxlog srv grep resync`) to confirm whether the reporter's pane was
  agent-shared or a plain standalone terminal, and to capture the raw bytes
  of the "3 page feeds."

## 6. Recommended fix direction (not yet implemented)

1. **Wire up close-on-exit** (§4a): call `should_close_on_exit`/
   `should_close_on_exit_force`/`close_on_exit_delay_ms` from the wait/cleanup
   task in `lifecycle.rs`, right where `STATUS_DONE` is published, and emit
   whatever "close this pane" signal the frontend already has for other
   pane-close flows (needs a matching frontend listener — audit whether one
   already exists and is simply never fed by the backend, or needs adding).
2. **Make `resync_controller`'s `STATUS_DONE` branch distinguish "died
   unexpectedly" from "exited intentionally."** At minimum,
   `try_attach_to_existing_shell` (§4b) should not silently resurrect a
   `STATUS_DONE` shell on every `PtyShellCreate` call — either surface a
   clear error to the calling agent ("this pane's shell has exited") or
   require an explicit `force`/`allow_respawn` flag, rather than defaulting
   to transparent resurrection the way the crash-recovery path
   (`TermResyncHandler`) intentionally does.
3. Once (1) and (2) land, re-verify the "3 page feed" bytes and the
   ConPTY handshake-reply path (§5) are actually clean under a real Windows
   repro — that investigation was blocked on not having a live capture, not
   on ruling it out.

## 7. What was actually fixed

Implemented recommendation (2) above, scoped narrowly: `try_attach_to_existing_shell`
(`agentmux-srv/src/server/mod.rs`) now checks the target block's live
controller status via `blockcontroller::get_controller(id)` *before* calling
`resync_controller`. If a controller is registered and its
`get_runtime_status().shellprocstatus == STATUS_DONE`, the function returns
`None` instead of resyncing — the same signal already used for "stale
pointer" (block deleted out from under the pointer). Both existing callers
(`handle_pty_shell_create`'s top-level reuse check and its "lost the atomic
claim" fallback) already treat a `None` return as "fall through to creating
a fresh shell block and repoint `term:shellsubblockid` at it," so no new
response shape or caller-side branching was needed — an exited shell is now
healed the same way a vanished one already was.

This does not touch `resync_controller` itself or its other callers
(`TermResyncHandler`'s WS `ControllerResyncCommand` path, `agent_open.rs`) —
those still respawn a `STATUS_DONE` controller unconditionally, which is
correct for their actual purpose (reviving a shell that died in a crash or
backend restart, not one a human intentionally exited). Only
`PtyShellCreate`'s "attach to whatever's already there" semantics needed to
stop conflating the two.

Regression test:
`server::tests::ptyshell_create_does_not_respawn_a_shell_that_already_exited`
(`agentmux-srv/src/server/tests.rs`) — spawns a real shell via
`PtyShellCreate`, sends `exit\r\n`, polls `/api/v1/ptyshell/status` until
`running: false`, then calls `PtyShellCreate` again and asserts the returned
`shell_id` differs from the first and the agent block's
`term:shellsubblockid` pointer now follows it. `#[ignore]`d like this file's
other real-PTY tests (`ptyshell_create_input_read_stop_round_trips_through_a_real_pty`
et al.) — they hang on GitHub-hosted `windows-latest` runners (tracked,
unrelated cause) but pass locally; run manually with `cargo test -p
agentmux-srv --bin agentmux-srv
ptyshell_create_does_not_respawn_a_shell_that_already_exited -- --ignored
--nocapture`. Verified passing locally alongside the full existing
`ptyshell_*` suite and the full non-ignored `agentmux-srv` test suite
(3350 passed, 0 failed).

**§4a (dead close-on-exit code) and the §5 open questions (exact "3 page
feed" bytes, ConPTY handshake interaction) are deliberately NOT addressed by
this fix.** §4a is a separate, pre-existing, opt-in feature gap (no caller
sets `cmd:closeonexit` today, so it doesn't affect default terminal
behavior) — worth its own follow-up, not bundled here. §5 was investigation
blocked on a live repro/capture, not something this change could resolve;
if the "3 page feeds" still reproduce after this fix ships, that confirms
they were a symptom of the respawn loop's repeated banner injection (and
should disappear), rather than an independent bug — worth a quick manual
recheck once this lands.

## 8. Review follow-ups (PR #3225)

Automated review on the PR surfaced three real gaps in the original §7 fix,
all addressed in the same PR:

- **ReAgent P2:** the "winner's block vanished... (exceedingly unlikely)"
  comment on the atomic-claim transaction's `Ok(Some(winner_id))` branch
  (`server/mod.rs`, near the fallback insert) became misleading once §7's
  fix made an exited shell take the exact same `None`-return path as a
  genuinely vanished block — exited shells are common (that's the whole
  point of this fix), not rare. Resolved as a side effect of the codex
  fix below: since `try_attach_to_existing_shell` now clears the stale
  pointer itself the moment it detects `RESYNC_ERR_ALREADY_EXITED`, the
  common "retry after exit" case is intercepted one level up (the claim
  transaction sees an already-cleared pointer and takes the plain
  `Ok(None)` path), so this branch is reached for an exited shell only
  under the same rare concurrent-race conditions as the original
  vanished-block case. Comment updated to explain why, not just reworded.
- **Codex P2 (line 1196, TOCTOU):** the original §7 fix checked
  `get_controller(id).get_runtime_status()` in `try_attach_to_existing_shell`
  itself, then called `resync_controller` separately — a shell that
  exited in the gap between those two calls would be caught by
  `resync_controller`'s OWN unconditional-respawn STATUS_DONE branch,
  respawning it anyway. Fixed by giving `resync_controller` a new
  `respawn_if_done: bool` parameter (threaded through all 9 call sites;
  `true` everywhere except this one, preserving existing behavior for
  crash/backend-restart recovery on the WS `ControllerResyncCommand`
  path). When `false` and the controller is `STATUS_DONE`,
  `resync_controller` returns `Err(RESYNC_ERR_ALREADY_EXITED)` (a new
  sentinel constant) instead of respawning — a single status read inside
  the function makes the decision, not two independent reads racing each
  other.
- **Codex P2 (line 1202, concurrent double-create):** two `PtyShellCreate`
  calls racing after the same shell exited would both see `None` from
  `try_attach_to_existing_shell`, both hit the atomic-claim transaction's
  `Ok(Some(winner_id))` branch (both reading the same still-set dead
  pointer), and both independently run the non-transactional "vanished,
  create it for real" fallback — leaving one orphaned live controller and
  two callers holding different `shell_id`s for what's supposed to be one
  pane's one shell. Mitigated by having `try_attach_to_existing_shell`
  clear the parent's stale pointer (`META_KEY_SHELL_SUBBLOCK_ID` → null)
  as soon as it detects `RESYNC_ERR_ALREADY_EXITED`, before returning
  `None`. (Round 2, §9, tightened this from an unconditional clear to a
  guarded compare-before-clear — see there.) This makes the
  already-correct, already-serialized (`with_tx`'s single-connection-lock)
  atomic claim transaction see "unclaimed" instead of "claimed by a dead
  id," so it resolves concurrent callers to one winner the same way it
  already does for a genuinely-never-claimed pointer. Best-effort (a
  failed clear costs an extra round trip through the same fallback chain,
  not correctness in the common case) — not a from-scratch fix of the
  pre-existing "two concurrent callers both reach the non-transactional
  vanished-block fallback" limitation for a truly adversarial interleaving
  (same residual rarity class as before this PR, not made worse).

Verified: full non-ignored `agentmux-srv` suite (3350 passed) and the
`ptyshell_*` suite including real-PTY `#[ignore]`d tests, both re-run after
these changes with no regressions.

## 9. Round 2: ReAgent found a real bug in §8's own fix

ReAgent re-reviewed after §8 landed and requested changes (`CHANGES_REQUESTED`,
not just `COMMENTED`) — one real P1, one real P2:

- **P1: the §8 pointer-clear was itself unconditional, with no compare
  before clearing.** `resync_controller`'s `STATUS_DONE` read
  (`respawn_if_done=false` path) happens outside any lock. A concurrent
  resync against the SAME block with `respawn_if_done=true` — the WS
  `ControllerResyncCommand`/`TermResyncHandler` path, which always revives
  a `STATUS_DONE` controller unconditionally — can respawn this exact
  shell in the gap between that status read and `try_attach_to_existing_shell`'s
  pointer-clear. The clear then unconditionally nulled `term:shellsubblockid`
  regardless of whether it still pointed at the (now dead-but-possibly-
  revived) `id` — wiping a pointer that had become valid again, orphaning a
  live, correctly-pointed shell. Exactly the one-shell-per-pane invariant
  this whole PR exists to protect, broken by its own mitigation for a
  different race. Fixed: `must_get` the parent fresh immediately before
  clearing and only clear if `parent.meta.get(META_KEY_SHELL_SUBBLOCK_ID)`
  still equals `id` — the same compare-before-clear pattern already used a
  few hundred lines down in this file's spawn-failure rollback path
  (`parent.meta.get(META_KEY_SHELL_SUBBLOCK_ID) == Some(child_id.as_str())`
  before removing). Not fully transactional (neither is the precedent it
  mirrors) — closes the specific "unconditional null" defect ReAgent
  flagged, not a claim of eliminating every possible race at this layer.
- **P2: a second, earlier occurrence of the same stale comment.** The P2
  from round 1 (§8) fixed the "exceedingly unlikely" comment on the
  atomic-claim transaction's `Ok(Some(winner_id))` branch; this round
  caught an *earlier* comment in the same function — the top-level reuse
  check's fallthrough ("Stale pointer (the block was deleted some other
  way) — fall through...") — that has the identical problem (now also
  covers the common exited-shell case) and was missed in round 1. Updated
  to say so explicitly.

Re-verified after these fixes: `cargo check -p agentmux-srv` clean, full
non-ignored suite (3350 passed), and the `ptyshell_*` suite including
real-PTY `#[ignore]`d tests — all with no regressions.
