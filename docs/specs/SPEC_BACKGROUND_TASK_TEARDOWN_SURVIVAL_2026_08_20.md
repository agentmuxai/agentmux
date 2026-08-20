# Spec: Background Task Teardown Survival (Phase B)

**Date:** 2026-08-20 (revised same day — see §0)
**Author:** AgentA
**Status:** Proposed
**Depends on:** `SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md` (Phase A)
**Addresses:** Issue #2492, rung 4 of `docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md`

## 0. Revision note

The first version of this spec proposed a Windows Job Object breakaway mechanism (a new `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` flag + a per-task "Background Task Container"). While starting implementation, `process_tracker/windows.rs`'s existing per-block job was found to already carry a deliberate, documented decision **against** allowing breakaway:

```rust
// - BREAKAWAY_OK is NOT set: descendants can't opt out of the
//   job. (Some CLIs try CREATE_BREAKAWAY_FROM_JOB; we want
//   those attempts to fail so the child stays tracked.)
```

Breakaway permission is a per-job flag, not a per-spawn one — there is no way to let only bashwrap's own controlled breakaway succeed while keeping everything else's attempts blocked. Enabling it would have weakened that existing, intentional containment guarantee for every process in a block's tree, not just the one task this spec cares about. That version is abandoned; this revision uses a fundamentally simpler mechanism found by re-reading `AgentProcessRegistry` more carefully (credit: user's own observation, "the job object stays open as long as the app is open," prompted re-checking this).

## 1. Problem, precisely

Issue #2492's observed symptom: a declared long-running background task (`task dev` via `run_in_background: true`) dies, with its whole process tree, when "the agent's session restarts" — even though the OS-level idle-timeout (#2491, already fixed) was never the cause.

Traced to an exact call chain, verified against current code:

1. A session restart (reconnect with a different connection, an explicit force-restart, or a controller-type change) reaches `resync_controller` (`agentmux-srv/src/backend/blockcontroller/mod.rs`), the documented "main entry point for starting/restarting blocks."
2. When `needs_replace` is true, `resync_controller` calls `delete_controller(block_id)` (`mod.rs:413-415`) — the **exact same function** real pane/tab/workspace deletion sagas call (`delete_tab.rs:148`, `delete_block.rs:137`, `wcore/tab.rs:103`, `websocket.rs:944`). There is currently no distinction in code between "this block's session is restarting" and "this block is being permanently deleted."
3. `delete_controller` (`mod.rs:239-257`) stops the old controller, then unconditionally does `process_tracker::registry::global().remove(block_id)`, which drops the block's tracker — and by design ("tracked ⇒ dies with the pane") this kills the whole descendant tree (Windows `KILL_ON_JOB_CLOSE`, Linux `cgroup.kill`, macOS `killpg`) — including bashwrap and anything it spawned, since Windows job membership / Linux cgroup membership are both inherited transitively through the whole `claude → agentmux-bashwrap exec → bash -c → task.exe/node/cargo` chain regardless of process-group/session boundaries.

## 2. The actual fix: the registry already supports this — the bug is calling the wrong cleanup function

`AgentProcessRegistry::ensure_tracker` (`process_tracker/registry.rs:80-102`) is **already idempotent**, with its own doc comment stating the intended design directly:

> Idempotent — calling twice for the same block returns the existing tracker so **the job survives controller re-creation (e.g. on `/clear`)**.

The module doc comment says the same thing at a higher level: *"the lifetime of the tracker matches the lifetime of the pane — multiple turns on the same block share the same job, so descendants from turn N are still visible on turn N+1."*

So the registry was **already designed** for "tracker lifetime == pane lifetime, not controller-generation lifetime." The bug is narrower than the original version of this spec assumed: `resync_controller`'s replace path simply calls `delete_controller` — the function meant for *permanent* pane closure — instead of a lighter path that only swaps the `Controller` implementation and leaves the process tracker alone. No new OS-level container, no breakaway, no Job Object flag changes are needed at all.

### 2.1 The remaining wrinkle: the old CLI process still needs to actually die

Simply skipping `registry.remove()` on replace isn't sufficient on its own — the *old* `claude` process legitimately needs to terminate (it's being replaced by a new spawn), just without taking the rest of the job/cgroup down with it. Two existing mechanisms matter here, and neither is currently scoped correctly for this case:

- **`ShellController::stop()`** (`shell/lifecycle.rs:820-852`) is `#[cfg(unix)]`-gated for its actual kill logic: `libc::kill(-(pid as libc::pid_t), SIGTERM)` — a **negative** pid, targeting the whole process group. This is intentional for a genuine user-initiated stop (the comment: "so that child processes spawned by the shell... are also signalled" — you want a build process the agent kicked off to die when you stop the agent). But for a *replace*, this would still kill bashwrap (a plain, non-`setsid`'d child of `claude`, inheriting the same process group) even if the job/cgroup itself is left alone. **On Windows, `stop()`'s kill branch doesn't exist at all** — today, the old `claude` process on Windows is only ever actually terminated as a side effect of `delete_controller`'s whole-job close. If replace stops calling that, Windows would leak the old `claude` process with nothing to kill it.
- **`AgentProcessRegistry::kill_pid(block_id, pid)`** (`registry.rs:151-157`) already exists and does exactly what's needed instead: kill one specific tracked PID without touching the container. This works on all three platforms via the existing `TrackerHandle::kill_pid` implementations.

### 2.2 Design

Add a new `Controller` trait method for the replace case, distinct from the existing `stop()` (which stays exactly as-is for real user-initiated stops and real deletion):

```rust
/// Stop this controller because it's being REPLACED by a new one for the
/// same block (session restart / resync), not because the block is being
/// closed. Must terminate this controller's own CLI process so it doesn't
/// linger, but must NOT touch the block's shared process tracker/job —
/// any declared-background descendant (bashwrap, a `task dev` instance)
/// must survive. Default implementation delegates to `stop()` for
/// controller types with no subprocess tree of their own to be careful
/// about (nothing to preserve, so the distinction doesn't matter).
fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
    self.stop(true, new_status)
}
```

`ShellController` overrides it:

```rust
fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
    let pid_to_kill = { /* same lock-and-extract as stop() */ };
    if let Some(pid) = pid_to_kill {
        // Single tracked PID, not the group and not the job — this is the
        // whole point: kill exactly the old CLI process, leave everything
        // else in the block's tracker (declared-background descendants)
        // alone. Works on all three platforms via the existing
        // AgentProcessRegistry::kill_pid.
        if let Some(registry) = crate::backend::process_tracker::registry::global() {
            if !registry.kill_pid(&self.block_id, pid) {
                // Not tracked (e.g. registry global unset in tests, or a
                // race before track_spawned landed) — fall back to a
                // direct single-process kill so the old process still
                // dies even without tracker involvement.
                #[cfg(unix)]
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
                #[cfg(windows)]
                { /* OpenProcess + TerminateProcess on just this pid */ }
            }
        }
    }
    Ok(())
}
```

`resync_controller`'s replace branch (`mod.rs:413-415`) changes from:

```rust
if needs_replace {
    let _ = ctrl.stop(true, STATUS_DONE);
    delete_controller(block_id);
}
```

to:

```rust
if needs_replace {
    let _ = ctrl.stop_for_replace(STATUS_DONE);
    // Remove from CONTROLLER_REGISTRY only — NOT the process tracker.
    // The new controller created below reuses the same block's tracker
    // via its own track_spawned call (ensure_tracker is idempotent).
    remove_controller_entry_only(block_id);
}
```

(`remove_controller_entry_only` is a new, small function alongside `delete_controller` — just the `CONTROLLER_REGISTRY.write().remove(block_id)` step, none of `delete_controller`'s process-tracker/broker cleanup.)

**Real deletion paths are unaffected** — `delete_tab.rs`, `delete_block.rs`, `wcore/tab.rs`, `websocket.rs:944` all keep calling the existing, unmodified `delete_controller`, which still tears down the whole tracker (including any declared-background descendant) on genuine pane/tab/workspace close. This matches the scope implied by #2492's own title ("session restarted," not "pane closed") and by the registry's own pre-existing design intent — a background task surviving *indefinitely*, even past its owning pane being closed, was never actually promised by anything in the current architecture, and isn't required to close this issue.

## 3. Why this supersedes the original breakaway design entirely

- **No isolation-invariant risk.** No Job Object flags change, no cgroup migration, no new OS-level container. The per-block tracker's containment guarantee is identical to today's — it's just not invoked as *often* (only on real deletion, which is its documented, intended trigger already).
- **No platform-specific spike needed.** `kill_pid` already exists and is already implemented for Windows/Linux/macOS (`process_tracker/{windows,cgroup_linux,macos}.rs` — whatever the concrete `TrackerHandle` impls are named). No PTY-across-breakaway risk, no cgroup-migration-ordering risk.
- **Smaller diff, smaller review surface.** One new trait method + one override + one small registry-adjacent helper, versus a new container type, spawn-path rewrite, and three divergent platform mechanisms.
- **Directly explains the observed bug**, rather than requiring a new mechanism to route around it: the registry already intended for this to work: `resync_controller` just wasn't using the right cleanup call.

## 4. Phasing

Given the reduced scope, this no longer needs the original B.1/B.2/B.3 split — it's a single, coherent, low-risk change:

1. Add `Controller::stop_for_replace` (default delegates to `stop()`) and `ShellController`'s override.
2. Add `remove_controller_entry_only` alongside `delete_controller` in `blockcontroller/mod.rs`.
3. Change `resync_controller`'s replace branch to use both.
4. (Optional, can follow as a fast-follow rather than blocking this PR): apply the same `stop_for_replace` override to any other `Controller` impl that manages its own subprocess tree the same way `ShellController` does, if one exists (`SubprocessController`/`PersistentSubprocessController` — check whether either has an independently-spawned descendant class worth preserving the same way; if their subprocess model doesn't support declared-background tasks at all, the default delegating implementation is already correct and no override is needed).

## 5. Testing

- Unit: `resync_controller`'s replace path now calls `stop_for_replace` + `remove_controller_entry_only`, not `stop` + `delete_controller` — assert the process tracker entry for a block survives a simulated replace (mock/stub controller + a `AgentProcessRegistry` with a real or fake tracker, confirm `ensure_tracker` returns the SAME tracker instance before and after a replace cycle).
- Unit: `remove_controller_entry_only` removes exactly the `CONTROLLER_REGISTRY` entry and nothing else (no process-tracker call, no broker `forget` call) — contrast with a `delete_controller` test confirming it still does all three.
- Live-verify (see the dashboard spec's end-to-end pass): background `task dev` in a real portable build, force a session restart on that same block (e.g. via whatever UI action drives `resync_controller`'s `force: true` path — whatever recreates the controller, such as switching agent/model or an explicit "restart" action), confirm via `tasklist`/`ps` that the `task dev` process tree is still alive and still shows up in `AgentProcessRegistry::list_block` for that block id afterward. Then close the pane entirely and confirm it now *does* die — proving both halves (survives restart, dies on real close) actually hold.

## 6. Non-goals

- No UI for explicitly stopping a background task independent of its owning pane (unaffected by this design either way — `kill_pid`/`kill_tree` already exist and could back such a UI later; not required to close #2492).
- No change to bashwrap's idle-timeout (#2491, already shipped) or to PID capture (Phase A, already shipped).
- No attempt to survive the declared-background task's own pane being permanently closed, or `agentmux-srv` itself crashing/being force-killed — both remain out of scope, matching the registry's existing "tracked ⇒ dies with the pane" contract for anything short of a session restart specifically.
