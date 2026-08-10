# SPEC — Process Broker Phase B: register `ShellController`/`AcpController` spawns with `process_tracker::registry`

**Date:** 2026-07-31
**Type:** Design/scoping spec (no code changes yet)
**Scope:** `agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs`, `agentmux-srv/src/backend/blockcontroller/acp.rs`, `agentmux-srv/src/backend/process_tracker/registry.rs`
**Status:** implemented — PR #2376 (track_spawned in shell + acp); verified in code 2026-08-10.
**Tracking:** GitHub Discussion #2375, item "Process Broker Phase B"

---

## 1. What Phase A left undone

`agentmux-srv/src/broker/process.rs` (Phase A, shipped) computes `ProcessStatus.processes`
by querying `process_tracker::registry::global()` — but that registry is only ever
*written to* from two of the four `Controller` implementors:

| Controller | Registers with `process_tracker::registry`? | Call site |
|---|---|---|
| `SubprocessController` | Yes | `blockcontroller/subprocess/host_spawn.rs:154` (spawn) → `:188-189` (`ensure_tracker`/`assign_process`) |
| `PersistentSubprocessController` | Yes | `blockcontroller/persistent.rs:1941` (spawn) → `:1961-1962` (identical pattern, comment cites the Subprocess path explicitly) |
| `ShellController` (handles both `"shell"` and `"cmd"` types) | **No** | Spawns at `shell/lifecycle.rs:446` (`pair.slave.spawn_command(cmd)`), no registry call anywhere in the file |
| `AcpController` | **No** | Spawns at `acp.rs:225` (`cmd.spawn()`), no registry call anywhere in the file |

Net effect: a `ProcessStatus` for a shell or ACP-agent block always has `processes: []`,
even when a real child OS process is running — so the broker's "opportunistic OS-process
enrichment" (its own doc comment's phrase) silently doesn't apply to those two controller
types. This is the literal gap the Phase A module doc calls "deferred to later phases."

## 2. Why this isn't purely mechanical (the one real risk)

The actual write path is `AgentProcessRegistry::ensure_tracker` → `TrackerHandle::assign_process(pid)`,
which on Windows calls `JobObjectTracker::assign_process` (`process_tracker/windows.rs:187-189`,
`AssignProcessToJobObject`). This is a **second, distinct Job Object** from the one
`agentmux-launcher` owns for multi-instance isolation (I1-I6 in this repo's `CLAUDE.md`) — it's
purely an internal resource-tracking job, not a lifecycle-control job. But two things need
verifying before this is safe to wire into two more spawn sites, not just asserted:

1. **Nested-job assignment.** `AssignProcessToJobObject` fails if the target process is already
   assigned to a job it wasn't created in and the OS/job doesn't have nesting enabled — Windows
   only allows nested jobs from Windows 8 / Server 2012 onward, and only if the *creator* opted
   in (`JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` or nested-job support). `SubprocessController`/
   `PersistentSubprocessController` clearly already do this successfully today for their own
   children — so the mechanism itself works — but a PTY-spawned child (via `portable_pty`'s
   `openpty`, not `std`/`tokio::process::Command` directly) may already sit inside a different
   ambient job (the shell's own conhost/job wrapping) by the time `ShellController` gets a PID
   back. Whether `assign_process` silently fails (registry.rs likely already logs+ignores
   failures for exactly this reason — needs confirming) or panics/errors matters a lot here.
2. **PTY child PID identity.** `ShellController` doesn't have a raw `child.id()` the way
   `Command::spawn()` gives — `portable_pty`'s `Child` trait exposes `process_id()` instead, and
   depending on the PTY backend (ConPTY on Windows) the "child" it returns may be a proxy/console
   host process rather than the actual shell PID. Needs a quick check of what `process_id()`
   actually returns for this repo's PTY backend before assuming it's the real, trackable PID.

Neither of these is a large problem, and both are answerable — but they're exactly the kind of
"looks like copy-paste, turns out there's one Windows-specific gotcha" issue that's cheap to get
wrong and only shows up by actually running a shell pane and a Job-Object-tracking sysinfo/Swarm
view side by side, not from reading the code. This is why this pass stops at a design doc rather
than shipping the change directly.

## 3. Proposed implementation (once §2's two risks are checked out)

1. **Factor the duplicated block into one helper**, since it will otherwise exist 4 times:
   ```rust
   // process_tracker/registry.rs, new fn
   pub fn track_spawned(block_id: &str, pid: u32) {
       let Some(registry) = global() else { return };
       let tracker = registry.ensure_tracker(block_id);
       if let Err(e) = tracker.assign_process(pid) {
           tracing::warn!(block_id, pid, error = %e, "failed to assign spawned process to tracking job");
       }
   }
   ```
   (Exact error-handling shape should match whatever `host_spawn.rs:187-199` already does today —
   this spec assumes it already warns-and-continues rather than propagating, consistent with
   "opportunistic enrichment, not a liveness signal on its own" from the broker's own doc comment;
   confirm against the existing call site rather than inventing new behavior.)
2. Replace the two existing duplicated blocks (`host_spawn.rs:187-199`, `persistent.rs:1960-1972`)
   with calls to `track_spawned`, as a small drive-by cleanup (behavior-preserving).
3. Add `track_spawned(block_id, pid)` at `shell/lifecycle.rs` right after the PTY spawn
   (~line 446), using `portable_pty`'s child `process_id()`.
4. Add `track_spawned(block_id, pid)` at `acp.rs` right after `cmd.spawn()` (~line 225-230,
   `pid` already bound there per the research pass).
5. Tests: a controller-level test per type (shell, acp) asserting `process_tracker::registry::
   global().unwrap().list_block(block_id)` is non-empty after spawn — mirroring whatever
   equivalent test (if any) exists for `SubprocessController` today.

## 4. Non-goals for this pass

- Not migrating `blockcontroller`'s registration model itself (Phase A's `CONTROLLER_REGISTRY`
  stays the "which blocks exist" source of truth — this only affects the OS-process-detail
  enrichment layer).
- Not implementing a `TsunamiController` (doesn't exist yet — `mod.rs:490-493` returns
  `Err("tsunami controller not yet implemented")`; out of scope here).
- Not touching the broker's own caching/single-flight logic (§Phase A, unaffected).

## 5. Go/no-go — what needs a live check before implementing

This is a backend change that's easy to write and easy to get subtly wrong on Windows
specifically (job nesting, PTY PID semantics) — and this repo's own `CLAUDE.md` calls out that
any Job-Object-touching change should be reviewed against the I1-I6 isolation invariants. Before
writing the actual diff, I'd want to:

1. Confirm `assign_process`'s failure behavior (warn-and-continue vs. propagate) by reading
   `host_spawn.rs:187-199` directly (quick, no live testing needed — will do this regardless).
2. Confirm what `portable_pty`'s `process_id()` actually returns on this repo's Windows PTY
   backend, and whether `AssignProcessToJobObject` on that PID succeeds without already being in
   a job — this is the part that benefits from actually spawning a shell pane under `task dev`
   and checking `process_tracker::registry` picks it up (e.g. via a debug log or the Swarm pane's
   process badge) rather than reasoning about it from source alone.

**Given the "stop and ask before anything needing live verification" instruction: do you want me
to (a) go ahead and write the implementation + unit tests now and only pause for the live
task-dev check in step 2 above, or (b) hold off entirely on code until you're free to test
interactively?**

---

*Design/scoping only. No files changed except this spec.*
