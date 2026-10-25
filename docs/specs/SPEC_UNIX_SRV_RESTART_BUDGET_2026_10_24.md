# Unix srv restart budget: recycle srv instead of ending the app

**Status:** implemented (#5519) — approved by the operator 2026-10-25 (via AgentY); macOS check by Clare pending
**Owner:** Maricon (launcher, Linux); macOS verification: Clare
**Task:** Kanban #58
**Related:** `SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11` (#942 phase 2, the Windows recycle), `SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16` §4.4 (the restart notice), PR #5407 (agent CLIs in their own process group), PR #5404 (the signal is logged)

## 1. Problem

On Linux and macOS, when srv exits unexpectedly, the launcher's Unix supervisor (`supervisor/unix.rs`) logs "srv exited UNEXPECTEDLY … terminating launcher" and ends the whole app: the host, every window and every agent. Windows instead respawns srv and recycles the host, at most `SRV_RESTART_BUDGET` (3) times per `SRV_RESTART_WINDOW` (120 s). An srv killed by one stray signal takes every window down on Unix, and the user has to start AgentMux again.

## 2. What Windows does (`run_windows`, the `srv_child.wait()` arm)

1. It classifies the exit: a system OOM waits for memory (`OOM_RESTART_BUDGET`). Anything else counts against `SRV_RESTART_BUDGET` and is given up on with `incidents::crash_give_up(GaveUpOn::Service, …)` once that budget is spent.
2. It builds an `incidents::srv_restarted(SrvStop::…)` notice and calls `spawn_srv(…, Some(&restart_incident.id))`. The new srv reads `RESTART_ID_ENV`, and `session_recovery::note_interrupted` writes how many agents it found interrupted against that notice.
3. It rebinds `srv_result` and `srv_child`, takes the new stdin (`srv_stdin_keepalive`), points the notification presenter at the new endpoint (`notify::update_endpoint`), and records the incident.
4. It sets `srv_recycle_kill = true` and kills the host. The host arm's restart path then relaunches the host against the new `srv_result`, with a splash (`respawn_splash_for_restart`), and skips the host crash budget and the `--disable-gpu` ladder for this one kill. The new host reprojects the session from srv's durable state, and agent panes come back through the normal controller start, including eager resume.

If srv dies while the host is in an OOM wait, Windows ends the launcher. It doesn't recycle there.

## 3. Change (Unix supervisor)

### 3.1 Main loop, srv arm
- Keep a clean exit (`s.success()`) and a group-shutdown signal (SIGINT/SIGTERM/SIGHUP) as a shutdown, as today.
- Anything else is unexpected. Log it with the #5404 wording (the signal is named), then:
  - **End what the dead srv left.** `agent_roots::kill_orphans(data_dir, old_pid)` (from #5407), then `kill_group_forcefully(old_pgid)` for anything else left in its group. The new srv eager-resumes the same sessions, so the old CLIs must be gone first, or there would be two CLIs on one session.
  - **System OOM.** Classify with `mem_supervisor::classify_host_exit(code, commit_free)`. On Linux a kernel OOM kill arrives as SIGKILL, so the low-commit reading decides. If it's an OOM, wait with `await_commit_recovery`, raced against the host exiting and against SIGINT/SIGTERM, under `OOM_RESTART_BUDGET`, as Windows does.
  - **Otherwise,** use `budget_exhausted(&mut srv_restarts, now, SRV_RESTART_WINDOW, SRV_RESTART_BUDGET)`. When the budget is spent, record `crash_give_up(GaveUpOn::Service, …)` and end the launcher as today, with exit code 1.
  - **Respawn.** Call `spawn_srv(…, Some(&restart_incident.id))`, rebind `srv_result`/`srv_child`, re-take its stdin, call `notify::update_endpoint` (made non-Windows-only), and record the incident.
  - **Recycle the host.** Set `srv_recycle_kill = true` and SIGTERM the host's process group, then SIGKILL it after the same grace the teardown backstop uses.
  - If the respawn fails, log FATAL and end the launcher, as Windows does.

### 3.2 Host arm
- Read and clear `srv_recycle_kill` before the exit is classified.
- On a recycle kill, relaunch the host against the new `srv_result` without touching `host_restarts`, `last_abnormal_code` or `host_degraded`. The recycle is srv's fault and is counted in srv's budget. Keep the GPU flag as it is.
- The teardown backstop is already disarmed on every host exit.
- **Splash.** Unix has no restart splash today; a host crash relaunch shows none either. I keep that: the reprojected windows are the feedback. If AgentY wants one, it is a follow-up.

### 3.3 Host-OOM wait branch
Today, if srv dies while the host is waiting for memory, the launcher ends. The task asks for this branch too. There, srv is respawned the same way, under the same budget, but no host is recycled, because there is none running. The wait goes on, and the host relaunched when it ends gets the new `srv_result`. A respawn that fails, or a spent budget, ends the launcher as today.

To keep this to one copy, the respawn becomes a helper:

`async fn respawn_srv(ctx, old: &mut Child, cause) -> Result<(SrvSpawnResult, Child, Option<ChildStdin>), Exit>`

It runs the orphan kill, the budget, the incident and the spawn. Both branches call it.

### 3.4 Cleanup epilogue
- `srv_pgid` is read from the current `srv_child` at shutdown (it already is).
- `_srv_stdin_keepalive` becomes a `mut` that is rebound on each respawn, and `quiesce_srv` gets the current one.

### 3.5 Liveness prober
Unix has no srv liveness prober (`srv_liveness::RecyclePolicy`) yet; Windows does. Out of scope: this change only handles srv actually exiting.

## 4. What the user sees

- **Windows come back.** srv dies, the windows close, then come back after srv is up (about 2–5 s on charlie).
- **Agents come back.** Each pane's controller starts again. A persistent agent with a session on file eager-resumes; its interrupted turn is marked by `session:was_interrupted` (the "interrupted" banner).
- **A notice.** The recovery notice says AgentMux restarted its background service and how many agents were interrupted (`incidents::srv_restarted` + `note_interrupted`), the same text as on Windows.
- **The 4th unexpected exit within 120 s** ends the app as today, with the "gave up" incident.

## 5. Tests

- **Unit (launcher).** Extract the srv-exit decision into a pure function, `classify_srv_exit(status, commit_free, &mut budgets) -> SrvExitAction {Shutdown, WaitForMemory, Respawn, GiveUp}`, and test it:
  - clean exit and SIGTERM/SIGINT/SIGHUP shut down;
  - SIGKILL and SIGSEGV respawn;
  - the 4th exit within the window gives up;
  - the budget refills after the window;
  - low commit waits for memory.
- **The host arm's recycle path,** as a pure helper too: a recycle kill doesn't consume the host budget or step the GPU ladder.
- **E2E on charlie,** in an isolated instance on Xvfb, with each PID recorded at launch:
  1. Open two windows and a Claude agent.
  2. SIGKILL srv by the PID in its `srv.lock`.
  3. Check that the windows reproject, the agent pane comes back and resumes its session, the notice appears, the old claude and agentmux-mcp are gone, and exactly one new CLI runs.
  4. Repeat 4 times within 120 s and check that the 4th ends the app with the give-up incident.
- **macOS:** Clare runs the same check on starpower.

## 6. Risks

- **A crash loop costs at most 3 host recycles in 120 s,** the same bound as Windows.
- **Duplicate CLIs on one session if the orphan kill misses one.** #5407's cgroup.kill / pid+start-time fallback covers it; the E2E checks for exactly one CLI.
- **Ports.** srv binds new ports on each start; the host gets them through its spawn env, as on Windows.
- **Data-dir lock.** The dead srv's `flock` is released by the kernel when it dies; `stale_srv` isn't needed.

## 7. Built as specified, with these details

- The decision is `classify_srv_exit` (`supervisor/srv_exit.rs`): clean exit and SIGINT/SIGTERM/SIGHUP shut down; low commit waits (own budget, `OOM_RESTART_BUDGET`); everything else is charged to `SRV_RESTART_BUDGET`. `srv_exit_action` wraps it with the logging and the give-up incidents, and the three places that see srv exit (the srv arm, and the host-OOM wait) call it.
- `respawn_srv` ends what the dead srv left (`agent_roots::kill_orphans`, then a SIGKILL to srv's own group while it still has members), builds the `srv_restarted` incident, starts srv with its id, points the notification presenter at it (`notify::update_endpoint` now builds on every platform) and records the incident.
- The recycle ends the host with SIGTERM to its process group (CEF flushes its profile) and SIGKILL 1.5 s later if it hasn't stopped (`host_kill_at`). The host arm reads `srv_recycle_kill` before anything else, so the SIGTERM is never taken for a group shutdown, and relaunches against the new srv without touching the host budget, the GPU ladder or `host_degraded`.
- srv's exit by a signal is named in the notice (`SrvStop::Signal`): "Its process was ended by signal 9 (SIGKILL)."
- A srv that dies while the host waits for memory is noted; the wait goes on, and srv starts again once memory is back, before the host.

## 8. Verified (isolated instance on charlie, packaged build, fake HOME, own display)

- SIGKILL of srv: a new srv within 1 s, the same launcher, a new host, the same panes back, and the notice "AgentMux's background service restarted: Its process was ended by signal 9 (SIGKILL)" on screen.
- Planted a root for a process in its own group in `agent-roots.json`, then SIGKILL of srv: the launcher logged "killed 0 agent cgroup(s) and 1 CLI process group(s)"; the process was gone and the file deleted.
- Three kills in 12 s were restarted (1/3 .. 3/3); the fourth ended the launcher with "the srv restart budget is spent (3 in 120s)", exit code 1, and the `srv_give_up` notice on the next start.
- Not run: an agent with a live session resuming after the restart. The spawn gate refuses to start an agent without a linked account, and the isolated home has none; the agent half is the Windows path's, unchanged. Clare repeats the checks on macOS.

## 9. Order

This builds on #5407 (`agent_roots::kill_orphans`), so it is branched after #5407 merges.
