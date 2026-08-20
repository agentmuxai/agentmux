# Spec: Background Task Teardown Survival (Phase B)

**Date:** 2026-08-20
**Author:** AgentA
**Status:** Proposed
**Depends on:** `SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md` (Phase A)
**Addresses:** Issue #2492, rung 4 of `docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md`
**Isolation-invariant review required:** yes — this spec adds a new OS-level process-lifetime container and a Windows Job Object breakaway flag. Must be checked against `CLAUDE.md`'s I1–I6 before merge (see §7).

## 1. Problem, precisely

Issue #2492's observed symptom: a declared long-running background task (`task dev` via `run_in_background: true`) dies, with its whole process tree, when "the agent's session restarts" — even though the OS-level idle-timeout (#2491, already fixed) was never the cause.

Traced to an exact call chain, verified against current code:

1. A session restart (reconnect with a different connection, an explicit force-restart, or a controller-type change — e.g. resuming a conversation under a new spawn generation) reaches `resync_controller` (`agentmux-srv/src/backend/blockcontroller/mod.rs`), the documented "main entry point for starting/restarting blocks."
2. When `needs_replace` is true, `resync_controller` calls `delete_controller(block_id)` (`mod.rs:413-415`) — the exact same function real pane/tab/workspace deletion sagas call (`delete_tab.rs:148`, `delete_block.rs:137`, `wcore/tab.rs:103`, `websocket.rs:944`). **There is currently no distinction in code between "this block's session is restarting" and "this block is being permanently deleted."**
3. `delete_controller` (`mod.rs:239-257`) stops the old controller, then unconditionally does:
   ```rust
   if let Some(registry) = crate::backend::process_tracker::registry::global() {
       registry.remove(block_id);
   }
   ```
4. `AgentProcessRegistry::remove` (`process_tracker/registry.rs:104-112`) drops the block's tracker, and by design ("tracked ⇒ dies with the pane") this kills the **whole descendant tree**:
   - Windows (`JobObjectTracker`): `TerminateJobObject`/`Drop` closes the per-block Job Object. `KILL_ON_JOB_CLOSE` is set with no `BREAKAWAY_OK`/`SILENT_BREAKAWAY_OK` (confirmed absent from `process_tracker/windows.rs` and `job_object.rs`), and Windows job membership is inherited transitively through every `CreateProcess` in the tree (`claude` → `agentmux-bashwrap exec` → `bash -c` via PTY → `task.exe`/`node`/`cargo`) with no opt-out available after the fact.
   - Linux (`Cgroupv2Tracker`): `cgroup.kill` on the scope. Cgroup membership is inherited by fork/exec regardless of process-group/session boundaries — a `setsid()`'d descendant is **still** in the same cgroup and **still** dies.
   - macOS (`ProcessGroupTracker`): `killpg` on the tracked pgid — this one *is* process-group scoped, so a descendant that already escaped into its own session (e.g. via a PTY spawn's implicit `setsid`) may already be structurally exempt today, inconsistently with the other two platforms. Not something to rely on; see §3.3.
5. Only `AgentProcessRegistry::track_spawned(block_id, pid)` is ever called explicitly (`shell/lifecycle.rs:466`, for the `claude` CLI's own PTY spawn) — bashwrap and everything under it are never deliberately tracked; they are swept in purely by OS-level inheritance from being a descendant of a tracked process. **No code anywhere treats a declared-background task as special during teardown.** `db_background_tasks` (Phase A) is a bystander — durable bookkeeping with no enforcement power over what actually happens to the process.

## 2. Why "just don't call `registry.remove()` on restart" is not sufficient

The naive fix — skip the `process_tracker::registry::global().remove(block_id)` call specifically on the `resync_controller` replace path — is necessary but not sufficient, because:

- The per-block registry entry (Job Object / cgroup scope) is keyed by `block_id`, one entry per block, not per-process. If the old controller's `claude` PID is about to be replaced by a *new* `claude` PID under the same `block_id`, and the tracker isn't cleared, the new controller's own `track_spawned` call would need to either reuse the same OS container (fine for the new `claude` process, but the old `claude` process — now genuinely dead, stopped intentionally — would sit in the same job/cgroup as a zombie until whenever the container is eventually closed) or create a second one (Windows: `AssignProcessToJobObject` semantics around re-assignment need checking; simplest is to always create a fresh tracker per controller generation and only defer the *old* one's teardown).
- More fundamentally: even if the per-block container survives the restart unclosed, it **will** eventually be closed by a *later*, genuine pane-close — at which point the background task dies anyway, just later than #2492 originally observed. That only turns "dies on every restart" into "dies on the next pane close," which is not what "survive session teardown" means (a user closing an unrelated tab shouldn't kill their dev server either).

**The declared-background task's process must never be a member of the per-block container in the first place.** Trying to surgically extract it after the fact isn't supported by any of the three platforms' primitives (Job Objects have no "remove one member" operation short of breakaway-at-spawn; cgroups likewise; only pgrp-based killpg is removable after the fact, and only macOS uses that).

## 3. Design

### 3.1 A separate, srv-owned container per background task

Introduce a **Background Task Container** — one new OS-level process-lifetime container (Job Object / cgroup scope / process group, using the exact same three-platform abstraction `AgentProcessRegistry` already implements) created the moment a task is confirmed `declared_background` and its PID is known (Phase A's `background_task_set_pid` call landing). Key differences from the existing per-block registry:

- Keyed by the background task's own `id` (== `tool_use_id`, matching `db_background_tasks.id`), not `block_id`. One container per task, not shared across a block's other activity — this keeps blast-radius bounded to exactly one declared task (consistent with I3's "bounded blast radius" spirit even though this container is a new concept, not the launcher's own J0).
- Owned by **srv's own process lifetime**, not any block's controller. It is only closed by:
  1. The task's own natural completion (bashwrap's process exits on its own — the container becomes empty, nothing to clean up beyond dropping the now-inert handle).
  2. An explicit stop request (a future `muxspect`/UI "stop this background task" action — out of scope for this phase to build UI for, but the container should support being closed for exactly this action).
  3. `db_background_tasks`-driven cleanup on *real* block/tab/workspace deletion (§3.4) — never on restart.

### 3.2 Getting the process into its own container: breakaway, not adoption

Because none of the three platforms support removing a live process from an existing container, the process must be spawned so it **never enters** the per-block container to begin with.

**Windows:**
1. `process_tracker/windows.rs`'s `JobObjectTracker::new` adds `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` to the per-block job's limit flags (alongside the existing `KILL_ON_JOB_CLOSE`) — this is what *permits* a descendant to request breakaway; it does not, by itself, weaken any existing containment guarantee (nothing breaks away unless it explicitly asks to, and only AgentMux's own bashwrap binary will ever ask).
2. `bash_wrap.rs`, when `args.declared_background` is true, does **not** spawn the actual work via `pair.slave.spawn_command(cmd)` (portable-pty's ConPTY spawn, which does not expose Win32 creation flags) as it does for ordinary invocations. Instead it re-spawns itself as a detached child using `std::os::windows::process::CommandExt::creation_flags(CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW)`, passing all the same arguments plus an internal `--already-detached` marker, and the detached child performs the actual PTY-hosted bash spawn + streaming as today. The original (still-in-job) bashwrap process becomes a thin, short-lived shim: it launches the detached child, publishes the child's PID (not its own — supersedes Phase A's "bashwrap's own PID" default for this specific case, since the detached child is now the real root) via the same WPS PID-publish path, and exits once the child has confirmed it's running (avoiding the original invocation hanging around inside the doomed per-block job any longer than necessary). **Open implementation risk, flagged rather than hand-waved:** whether `portable-pty`'s PTY allocation works correctly when opened from the detached (breakaway) child rather than the original process needs a spike before this is assumed to work — if PTY handle inheritance across the breakaway boundary is a problem, the fallback is to have the *original* process open the PTY pair first and pass the slave handle to the detached child, or accept the `run_via_pipes` fallback path unconditionally for declared-background tasks (losing live PTY streaming for backgrounded tasks specifically is an acceptable degradation — dev-server output is already captured via WPS chunk streaming either way, and a backgrounded task's whole point is that nobody's watching it live in real time).

**Linux:** the detached-respawn shim does the same job at the process level, but the *container* side must move the PID to a **new** `systemd-run --user --scope` cgroup (Background Task Container, not the per-block one) at spawn time — since cgroup migration via `cgroup.procs` write is itself supported for a live process, an alternative simpler-than-Windows path exists: skip the re-spawn shim entirely and instead have `bash_wrap.rs` call `setsid()` (already necessary — see below) then, once its own PID is known, write it into a freshly-created scope's `cgroup.procs`. This migrates it OUT of the per-block cgroup without needing a breakaway re-exec. Confirm this migration is unaffected by later cgroup-freezer/kill operations on the *source* cgroup, i.e. moving a PID out of cgroup A into cgroup B before A is killed genuinely exempts it — this is standard cgroup v2 behavior (a killed cgroup only affects processes still resident in it at kill time) but should be verified against the actual `Cgroupv2Tracker` implementation during implementation, not assumed.

**macOS:** `setsid()` (new session + process group) is suffient on its own, since `ProcessGroupTracker`'s `killpg` is pgrp-scoped — no additional container-migration step needed, consistent with §2's observation that macOS may already be closer to correct than the other two platforms.

**Shared requirement, all platforms:** `bash_wrap.rs` calls `setsid()`/creates a new process group for a `declared_background` invocation's actual work, in all cases — even on Windows, where it has no direct bearing on Job Object membership, for two reasons: (1) defense in depth — don't rely on a single platform-specific mechanism per platform when a cheap second one is available, (2) consistency — the Unix migration design above depends on it, and having Windows/Linux/macOS all establish a fresh session at the same point in the code keeps the three platform branches structurally parallel rather than diverging in surprising ways.

### 3.3 `resync_controller`'s replace path

With §3.2 in place, the naive fix from §2 becomes correct rather than merely necessary: `resync_controller`'s `needs_replace` branch (`mod.rs:413-415`) can safely call the **existing, unmodified** `delete_controller(block_id)` — because by the time a background task exists, its process is no longer a member of the per-block container at all. No new "preserving" variant of `delete_controller` is needed; the fix lives entirely in §3.2's spawn-time behavior, not in the teardown call site. This is a meaningfully smaller, lower-risk change to the teardown path itself than originally scoped — the complexity moved to "spawn it correctly the first time," which is the more tractable half.

### 3.4 Real deletion must still clean up

Because a declared-background task's process is now **outside** the per-block container by design, genuine pane/tab/workspace deletion (`delete_tab.rs`, `delete_block.rs`, `wcore/tab.rs`, `websocket.rs:944`) no longer kills it as a side effect — this must become an explicit step, or deleting a pane silently leaks the background task forever (a regression in the opposite direction from #2492).

Add one step to `delete_controller` itself (so every existing call site gets this for free, no per-caller changes needed): before returning, query `db_background_tasks::list_for_block(block_id)` for `Running` entries with a known `pid`, and for each, close its Background Task Container (§3.1's container-close path — same mechanism the future explicit-stop UI action will use). This makes `delete_controller`'s contract exactly what every existing caller already assumes ("this block's processes are gone after this call returns"), just now covering two containers (the per-block one, always; each background task's own, only if any exist) instead of one.

### 3.5 Reconnection / adoption bookkeeping

Once a background task's process genuinely survives a restart, the **new** controller (created after `resync_controller`'s replace) has no in-memory knowledge that a task from the *previous* generation is still running attached to this block — that knowledge lives only in `db_background_tasks` (durable) and the OS (the process itself). No process-adoption code is needed here (nothing was ever un-adopted — the process kept running the whole time, untouched) — this is purely a bookkeeping/UI concern: the new controller (or whatever assembles the block's initial state on (re)start) should query `db_background_tasks::list_for_block(block_id)` for `Running` entries and feed them into the frontend's `attachedTask` axis on load, so the UI reflects "yes, this is still running" immediately rather than waiting for a stray transcript event. This is Phase C's concern (the registry-reader design) — noted here only so Phase B's design doesn't accidentally block it.

## 4. Phasing (independently shippable, per this repo's own norm)

1. **B.1 — plumbing only, no behavior change.** Add `delete_controller`'s new cleanup step (§3.4) as dead code today (queries `db_background_tasks`, finds nothing because Phase B.2 hasn't shipped, no-ops). Land the `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` flag addition to the per-block job (§3.2) — inert on its own (nothing requests breakaway yet), but isolated and easy to review/revert independently of the riskier spawn-path change. **Isolation-invariant check applies to this PR** (touches Job Object creation flags — CLAUDE.md's explicit gate list).
2. **B.2 — the actual breakaway/migration mechanism.** `bash_wrap.rs`'s detached-respawn (Windows) / cgroup-migration (Linux) / setsid (macOS) changes, per §3.2. Highest-risk, most platform-specific phase — needs the Windows PTY-across-breakaway spike resolved or the pipe-fallback decision made explicitly (§3.2's flagged open question) before this is considered done, not after.
3. **B.3 — reconnection bookkeeping.** The new-controller-queries-registry-on-start piece from §3.5, feeding Phase C.

## 5. Testing

- B.1: unit tests for `delete_controller`'s new cleanup branch (mock `db_background_tasks` with a `Running` entry with a PID, confirm the container-close call fires; confirm it's a no-op when no such entry exists — the common case must stay a true no-op, not add latency to every pane close).
- B.2: this is the phase that most needs the live-verify pass from `SPEC_..._DASHBOARD...`'s §9 / the original ladder doc's proposed smoke test — unit tests alone cannot prove Windows Job Object breakaway actually works end-to-end (Job Object behavior is only observable against the real kernel, not mockable meaningfully). Plan: a `task dev`-style long-running command, backgrounded, followed by a deliberate session restart (force-resync a block via the same code path `resync_controller` uses), confirmed via `tasklist`/`Get-Process` (Windows) or `ps`/`/proc` (Unix) that the process is still alive and no longer a member of the per-block container.
- B.3: reducer/UI-adjacent — covered by Phase C's own test plan.

## 6. Non-goals

- No UI for explicitly stopping a background task from outside its owning pane (Phase C may want this eventually — not required to close #2492).
- No change to bashwrap's idle-timeout (#2491, already shipped).
- No attempt to survive `agentmux-srv` itself crashing or being force-killed (`SIGKILL`/Task Manager "End Process") — only graceful teardown paths (`delete_controller`'s two flavors) are in scope. A hard srv crash orphaning a Background Task Container is an accepted, pre-existing risk class (same as today's `PR_SET_PDEATHSIG`-guarded srv-under-launcher relationship) — out of scope here.

## 7. Isolation-invariant review checklist (CLAUDE.md I1–I6)

This spec's changes are localized to per-block/per-task Job Objects — never the launcher's own J0, never a named/shared OS object:

- **I1 (pipe uniqueness):** unaffected — no pipes involved.
- **I2 (no global lifecycle handles):** the new Background Task Container is created and owned exclusively by srv, for exactly the processes srv itself spawns (transitively) — same ownership discipline as the existing per-block registry, just a second container per task instead of one per block. No handle to any process/job this code didn't create is ever opened.
- **I3 (bounded blast radius):** improves on the status quo — a background task's container is now scoped to exactly one task, not shared with the rest of its block's activity, and killing it (§3.4/explicit stop) cannot reach anything outside that one task's own descendant tree.
- **I4/I5 (cross-instance contact, keyed shared objects):** the new Job Objects/cgroup scopes are unnamed/uniquely-scoped (Windows: `CreateJobObjectW(null, null)`, same pattern as J0 and the existing per-block job; Linux: `systemd-run --user --scope` with a unique scope name derived from the task id) — no new named/shared OS object is introduced.
- **I6 (data isolation):** unaffected — no data/logs/cef-cache directory changes.

The one genuinely new primitive is `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` on the per-block job (§3.2/B.1) — this *loosens* a containment guarantee (a member can now leave the job under its own request), which is exactly the class of change CLAUDE.md's gate calls out explicitly. The mitigation is that breakaway is opt-in per-process (nothing breaks away unless it calls `CreateProcess` with `CREATE_BREAKAWAY_FROM_JOB` itself) and only AgentMux's own `agentmux-bashwrap` binary, and only for `declared_background` invocations specifically, will ever do so — this must be re-confirmed by a human/reagent isolation-invariant review before B.1 merges, not assumed from this writeup alone.
