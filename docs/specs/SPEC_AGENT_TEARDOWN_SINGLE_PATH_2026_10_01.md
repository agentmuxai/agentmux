# SPEC: One teardown path for everything an agent owns

**Status:** active (Phase 1: PR #4160; Phase 2: PR #4164 and the Phase 2 follow-up; see §12)
**Date:** 2026-10-01
**Author:** Lark
**Builds on:** `SPEC_AGENT_SELF_QUIT_2026_09_24.md` (`/quit`, `QuitSelf`, the
override window), `SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md`
(per-agent shutdown order, §4.2; sub-block teardown, §4.8),
`SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md` (restart keeps
background tasks), `SPEC_PANE_CLOSE_CONFIRM_NAMES_PROCESSES_2026_09_23.md`
(the close dialog names processes).

## 0. The ask

"`/quit` needs to handle everything the agent may be doing, including
multiple `task dev` instances or other processes. Make sure there is a
single DRY path that multiple consumers can use."

## 1. Summary

An agent can start processes and hold resources in eight different ways.
Today each way of ending an agent (`/quit`, `QuitSelf`, pane ×, `ClosePane`,
Stop, `FleetBulkStop`, restart, app exit) runs its own subset of cleanup, and
each subset misses something:

- On Windows, the CLI's process tree dies with its Job Object, but with no
  graceful step.
- On Linux and macOS there is no tracker at all: only `claude` is killed, and
  `run_in_background` tasks (e.g. `task dev`) are orphaned.
- `Shell()` sessions survive every path except `/quit`.
- PtyShell drawers survive whenever the pane isn't mounted.
- Container agents' containers and dev-proxy routes are never stopped.

This spec replaces those subsets with **one inventory and one teardown**:

1. **`AgentResources`** lists everything a given agent owns, from every
   source, in one place.
2. **`agent_teardown::run(block_id, Policy)`** is one saga that stops those
   resources in a fixed order:
   - graceful first, then forced;
   - awaited, so it doesn't return until the stops have landed;
   - verified: it checks afterwards what is still alive;
   - reported: it returns what it stopped, what it had to force, what it kept
     and what survived.
3. **Every consumer calls it**, differing only in the `Policy` it passes
   (§5). Nothing else kills an agent's processes.

## 2. What an agent can own today (verified in code, 2026-10-01)

| # | Resource | How it's started | Tracked where | Killed by `/quit` today? |
|---|---|---|---|---|
| R1 | The CLI process (`claude`, …) | `persistent/spawn.rs` | controller | yes: interrupt → EOF → 5 s → `child.kill()` (single PID) |
| R2 | CLI descendants: Bash tool commands via `agentmux-bashwrap`, incl. `run_in_background` (e.g. several `task dev`) | child of the CLI; `--declared-background` → `setsid()` on Unix (`bash_wrap.rs:371-388`), PID in `db_background_tasks` (`websocket.rs:1387-1457`) | Windows: per-block Job Object (`process_tracker/windows.rs`); **Unix: none** (`StubTracker`, `process_tracker/mod.rs:199-203`) | Windows: yes, but hard kill only (`KILL_ON_JOB_CLOSE`), and not if it escaped the job (assign race `windows.rs:20-25`; stub fallback on job-creation failure). **Unix: no.** |
| R3 | `Shell()` sessions | srv-spawned `cmd /C` / `sh -c` (`shell_node.rs:324-332`) | srv-global `ShellSessionRegistry`, `block_id` in status | `/quit`/`QuitSelf` only, fire-and-forget (`self_quit.rs:243-250`); pane ×, `ClosePane`, Stop: **no** |
| R4 | `PtyShell` drawers | headless sub-block, own controller and job (`http_pty_shell.rs:142-313`) | sub-block of the agent's block | only via the frontend's `deletesubblock` on unmount (`agent-view.tsx:299-301`); **no srv cascade** |
| R5 | `!cmd` / `shellexec` | srv-owned `sh -c` (`shell_handlers.rs:271-320`) | none | no (a `! foo &` grandchild survives) |
| R6 | Container + dev-proxy routes (`RegisterDevServer`) | `ContainerManager`; routes in `DevProxyRegistry` (`dev_proxy.rs`) | container manager, proxy registry | **no** (`cleanup_dev_proxy_routes` is called only from tests) |
| R7 | Work-queue claims | `WorkClaim` | work queue | `/quit`/`QuitSelf` only |
| R8 | Crons targeting the agent; Loops | `CronCreate`; `Loop` (in the MCP process) | cron store; MCP | crons kept and reported; Loops die with MCP |

Other paths are worse than `/quit`:
- **Stop** (`stop_one_agent_block`, `agent_io.rs:564-581`) force-kills only R1
  and leaves the job alive.
- **Restart** deliberately keeps R2.
- **App exit** stops R3 (`main.rs:278-288`) and relies on the launcher's
  backstop for the rest (Windows J0; Unix SIGTERM to srv's group).

## 3. Goals

1. **G1. One path.** A single saga, `agent_teardown::run`, is the only code that
   stops an agent's resources. `/quit`, `QuitSelf`, `ClosePane` (all forms),
   pane and tab ×, Stop, `FleetBulkStop`, restart/replace, app exit and the
   orphan reaper all call it.
2. **G2. One inventory.** `AgentResources` is the only code that answers "what
   does this agent own?". The teardown, the close-confirmation dialog, the
   `/quit` summary, Swarm and a new `AgentResources` RPC/MCP read all use it.
3. **G3. Everything, on every OS.** After a quit, no process the agent
   started is left running on Windows, Linux or macOS, unless the policy kept
   it on purpose (§5). That includes several concurrent `task dev` trees.
4. **G4. Graceful first.** Processes get a chance to exit cleanly (dev servers
   flush and release ports) before a forced kill, within a bounded total time.
5. **G5. Verified and reported.** The saga checks afterwards what is still
   alive and returns a report. The user sees "stopped 3 · forced 1 · kept 0 ·
   survived 0", with names. A survivor is never silent.
6. **G6. Safe.** Only this agent's processes are touched: never by image name,
   never another agent's, never the installed AgentMux that hosts the agent.

## 4. Non-goals

- Changing who may quit an agent, the `QuitSelf` gate, or the 15 s override
  window. Those stay as `SPEC_AGENT_SELF_QUIT` defines them.
- Deleting crons that target the agent (still kept and reported).
- Changing restart's guarantee that background tasks survive a controller
  replace (§5 encodes it as a policy instead).

## 5. Policies: one path, different scopes

Each consumer passes a `Policy` that says which resource kinds to stop. The
order, grace, verification and report are identical for all of them.

| Consumer | CLI (R1) | Foreground descendants (R2) | Background tasks (R2 `run_in_background`) | `Shell()` (R3) | PtyShell (R4) | `!cmd` (R5) | Container/routes (R6) | Claims (R7) | Block record |
|---|---|---|---|---|---|---|---|---|---|
| `/quit`, `QuitSelf`, `ClosePane`, pane ×, tab × | stop | stop | **stop** | stop | stop | stop | stop | release | delete |
| Stop / `FleetBulkStop` `Stop` | stop | stop | **ask** (§7.4), default keep | keep | keep | stop | keep | release | keep |
| Restart / controller replace | stop | stop | **keep** (`SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL`) | keep | keep | keep | keep | keep | keep |
| App exit | stop | stop | stop | stop | stop | stop | stop | — | keep |
| Orphan reaper (no live controller) | — | stop | stop | stop | stop | stop | stop | release | as today |

`Policy` is a value, not a branch in each caller. A consumer that needs a new
scope adds a row here and a constructor (`Policy::quit()`, `Policy::stop()`,
`Policy::replace()`, …). It never adds its own kill code.

## 6. Design

### 6.1 `AgentResources`: the inventory

A new module, `backend/agent_resources.rs`, provides:

```rust
pub struct AgentResources {
    pub block_id: String,
    pub cli: Option<ProcInfo>,                 // R1
    pub tree: Vec<ProcInfo>,                   // R2: every live descendant, with parent pid
    pub background_tasks: Vec<BackgroundTask>, // R2 subset declared `run_in_background`
    pub shell_sessions: Vec<ShellSessionInfo>, // R3
    pub pty_drawers: Vec<SubBlockInfo>,        // R4 (and their own trees)
    pub shell_exec: Vec<ProcInfo>,             // R5
    pub container: Option<ContainerInfo>,      // R6, with its dev-proxy routes
    pub claims: Vec<ClaimInfo>,                // R7
    pub crons: Vec<CronInfo>,                  // R8, report only
}
pub struct ProcInfo { pid: u32, ppid: u32, started_at: SystemTime, name: String, cmdline: String, kind: ProcKind }
pub fn snapshot(state: &AppState, block_id: &str) -> AgentResources;
```

- **One collection function.** `snapshot` reads every source in §2: the
  process tracker (job / process groups), `db_background_tasks`,
  `ShellSessionRegistry`, sub-blocks (`subblockids`, `term:shellsubblockid`),
  the new shellexec registry (§6.5), the container manager and proxy
  registry, the work queue, and crons.
- **Descendants by parent pid.** `tree` is built by walking descendants of
  every root PID the sources name (the CLI, background task PIDs, shell PIDs,
  drawer PTYs). It uses one OS snapshot (Windows Toolhelp32, `/proc` on Linux,
  `sysctl`/`libproc` on macOS), not one query per process.
- **PID reuse.** Every `ProcInfo` carries `started_at`, so a reused PID is never
  mistaken for the agent's process (G6).
- **Reuse, not a fourth list.** The close dialog
  (`SPEC_PANE_CLOSE_CONFIRM_NAMES_PROCESSES`) renders this inventory (it needs
  exactly `pid` + `ppid` + name). The `/quit` summary and Swarm's per-agent
  process list read it too. An `agent.resources` RPC and a read-only
  `AgentResources` MCP tool (own agent only) expose it, so an agent can check
  what it's leaving behind before it asks to quit.

### 6.2 `agent_teardown::run`: the saga

A new saga, `sagas/agent_teardown.rs`:

```rust
pub async fn run(state: &AppState, block_id: &str, policy: Policy, origin: TeardownOrigin) -> TeardownReport;
```

It absorbs today's `close_pane::shutdown_one`, `self_quit::run_with`'s
cleanup steps, `stop_one_agent_block`, and the controller-replace teardown.
Those call sites become one-line calls to `run` with their policy.

Steps, in order (extends `SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN` §4.2):

1. **Gate input.** `mark_closing`; refuse new `AgentInput`; unregister from jekt
   delivery. The never-built "stop new input" of `SPEC_AGENT_SELF_QUIT` is
   built here.
2. **Snapshot.** `let before = agent_resources::snapshot(state, block_id)`.
3. **Release non-process resources** the policy covers: claims (R7) and
   container dev-proxy routes (R6), so nothing new is routed to a dying agent.
4. **Stop the CLI gracefully (R1).** Interrupt the turn, EOF on stdin, wait.
   This is today's `PersistentSubprocessController::shutdown`, unchanged.
5. **Ask every covered process to exit (graceful phase, §6.3)**, in
   dependency order:
   - PtyShell drawers (R4), by recursing into `run` for each sub-block with
     the same policy, so a drawer is torn down exactly like an agent;
   - `Shell()` sessions (R3) and shellexec (R5);
   - background tasks and the rest of the tree (R2), leaves first;
   - the container (R6), with `docker stop`'s own grace.
6. **Wait for all of them, under one shared deadline:** `TEARDOWN_GRACE`
   (default 8 s, configurable). The CLI's 5 s is part of it.
7. **Force what's left** (§6.4): Windows `TerminateJobObject` / per-PID
   terminate; Unix `SIGKILL` to recorded process groups, then per-PID.
8. **Verify.** Take `snapshot` again and match it against `before` by
   `(pid, started_at)`. Anything still alive is a **survivor**. Try one more
   forced kill per survivor; whatever remains goes in the report.
9. **Save final state**, then (if the policy says so) **delete the block** and
   prune the layout. This is today's `save_final_state` / `delete_block`.
10. **Report and audit.** Return `TeardownReport { stopped, forced, kept,
    survivors, released_claims, crons_kept, duration }`, write it to the audit
    log (`agent.quit` / `agent.stop` / …), and publish it to the pane.

`run` is idempotent per block. A second call while one is in flight joins it
and returns the same report: `already_quitting` today, generalized.

### 6.3 Graceful phase per platform

All of these are best-effort and bounded by `TEARDOWN_GRACE`.

**Windows:**
- A process with a visible top-level window (e.g. a `task dev` AgentMux window)
  gets `WM_CLOSE`. That runs AgentMux's own window-close teardown, which reaps
  its srv and CEF through its launcher's J0.
- Console processes started in their own process group (§6.5) get
  `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pgid)`.
- A process with neither has no graceful signal on Windows. It goes straight
  to the forced phase.

**Unix:** `SIGTERM` to each recorded process group (§6.4), then to each
remaining descendant PID.

**Known-application hooks:** an optional, small table keyed by command-line
pattern (e.g. AgentMux dev builds, `vite`), each with a better graceful step
than the generic one. These stay additive and optional; the generic path must
work without them.

### 6.4 One tracker per agent, on every OS

`process_tracker` gets a real Unix implementation, closing the gap noted at
`shell/lifecycle.rs:1587-1592` (the module doc's table at
`process_tracker/mod.rs:16-21` describes it but it was never built).

**Linux / macOS:**
- Spawn the CLI in its own process group (`process_group(0)`) and record the
  pgid in the block's tracker.
- bashwrap already publishes each declared-background task's PID and calls
  `setsid()`. The tracker also records that new session's pgid (via the same
  `backgroundtaskpid` message), so `run_in_background` tasks such as `task dev`
  are known and killable as groups.
- Linux, optional: a cgroup v2 leaf per agent, when the user's systemd slice
  allows it, for containment that `setsid` can't escape. Feature-detected;
  process groups are the baseline.

**Windows:**
- Close the assignment race (`windows.rs:20-25`): spawn the CLI
  `CREATE_SUSPENDED`, assign it to the job, then resume it. Nothing can start
  before it's in the job.
- On job-creation failure, log loudly and fall back to the descendant walk in
  §6.1 for teardown, instead of a silent no-op `StubTracker`.

**Verification beyond the tracker:** the before/after snapshot (§6.2 step 8)
walks descendants by parent PID, independent of the tracker. So a process that
escaped the job or group (the Windows assignment race, a job-creation failure,
a Unix `setsid` the tracker didn't record) is still found, stopped and
reported. Escaping the tracker should no longer mean escaping teardown.
`Start-Process` is not such an escape on today's Windows build (verified,
§11 O4).

### 6.5 srv-spawned processes join the agent's tracker

`Shell()` (R3) and `!cmd`/shellexec (R5) are spawned by srv on the agent's
behalf, but sit outside the agent's job, so only bespoke code can stop them.

**Change:** when the request carries the agent's `block_id`, srv assigns the
child to that block's tracker (`track_spawned`). On Windows it also uses
`CREATE_NEW_PROCESS_GROUP`, so CTRL_BREAK reaches the whole group (§6.3). The
`ShellSessionRegistry` stays the source of `Shell()` status and output, but no
longer has its own kill path for teardown.

Effects:
- The `taskkill /T` gap (`cmd /C` wrapper already exited, grandchildren
  orphaned) is closed, because the job holds the grandchildren.
- `list_active`'s blind spot (pipes closed, process tree alive) stops
  mattering, because the inventory walks the tree rather than the session list.

`ShellStop` gets the same-agent ownership check it lacks today
(`http_shell.rs:132-139`).

### 6.6 PtyShell drawers cascade server-side

Implements `SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN` §4.8, which was never
built. `agent_teardown::run` recurses into each sub-block (`subblockids`,
`term:shellsubblockid`) before the parent's own tree is forced. The frontend's
`deletesubblock` on unmount stays as a fast path, but teardown no longer
depends on the pane being mounted.

### 6.7 Containers and dev servers

For a container agent, `Policy::quit()` stops the container
(`ContainerManager::stop`, which already calls `cleanup_dev_proxy_routes`) and
removes it if it was created for this agent. A container the user attached
the agent to is stopped but not removed. `RegisterDevServer` routes are
dropped in step 3, so the proxy stops sending to it before it dies.

### 6.8 Consumers after this spec

| Consumer | Today | After |
|---|---|---|
| `/quit` (`quit.ts` → `ObjectService.QuitAgent` → `self_quit::run`) | `self_quit` cleanup + `delete_block` → `close_pane::shutdown_agents` | `self_quit::run` keeps its gate and audit, then calls `agent_teardown::run(_, Policy::quit(), UserSlash)` |
| `QuitSelf`, no-arg `ClosePane` (`pane.rs:472-519`) | same as `/quit`, after gate or override window | same: `pending_shutdown` and gate untouched; the action calls `agent_teardown::run` |
| `ClosePane block_id=…`, pane ×, tab × (`close_pane::run`) | `shutdown_one` per stack member | `agent_teardown::run(_, Policy::quit(), …)` per member, layout pruned once (unchanged) |
| Stop, `FleetBulkStop Stop` (`stop_one_agent_block`) | `ctrl.stop()`: force-kills R1 only | `agent_teardown::run(_, Policy::stop(keep_background), …)` |
| Restart / replace (`resync_controller` → `stop_for_replace`) | bespoke | `agent_teardown::run(_, Policy::replace(), …)` |
| App exit (`main.rs` `stop_all` + launcher backstop) | `shell_sessions.stop_all()` + backstop | `agent_teardown::run(_, Policy::app_exit(), …)` for every live agent, concurrently, under one deadline; the launcher backstop stays as the safety net |
| Orphan reaper | bespoke | `agent_teardown::run(_, Policy::orphan(), …)` |
| Close-confirmation dialog | its own process RPC | renders `AgentResources` |

After this, nothing else in srv calls `child.kill()`, `kill_tree`,
`kill_process_group`, `release_block_processes` or `ShellSessionRegistry::stop`
for teardown. A lint test pins it (§9).

## 7. User experience

1. **Before** (close dialog): "Closing will stop: claude, 2 × task dev (AgentMux
   dev, vite :5179), 1 shell (`npm run watch`), PTY drawer". That's the
   inventory, grouped by root process, as
   `SPEC_PANE_CLOSE_CONFIRM_NAMES_PROCESSES` designs.
2. **During:** the shutdown log (`SPEC_AGENT_SELF_QUIT` Phase 1b) gets one line
   per resource: "asked to exit", "exited", "forced", "kept".
3. **After:** a one-line summary notice: "Quit lark: stopped 6 · forced 1 ·
   survived 0 · released 2 claims · 1 cron still targets lark". On a survivor,
   a warning notice names it, with its PID and command line, and offers "Kill
   now". That runs the same forced step for that PID, after re-checking it's
   the same process by `started_at`.
4. **Stop with background tasks:** Stop on an agent with running
   `run_in_background` tasks asks "Also stop 2 background tasks (task dev ×2)?
   [Keep running] [Stop them]". The default is keep, today's behaviour. A
   remembered choice is per user. Quit never asks; it stops everything, which
   is the ask.
   - **Amended 2026-10-01 (implementation):** no pane surface ends an agent
     with Stop today. The pane's Esc / stop button interrupts the turn
     (`ControllerInput` SIGINT); Stop is reached only through `agent.stop`,
     `FleetBulkStop` and a deferred `Stop` action, all programmatic. So the
     question is a parameter, `agent.stop`'s `stop_background`. When it is
     absent, the per-user setting `agent:stopkeepsbackground` (O2) answers
     (default keep). A dialog comes with the first UI that offers Stop.

## 8. Safety rules

- **Scope:** everything goes through the agent's own tracker, recorded PIDs and
  the descendant walk from its own roots. No image-name kills, no "every
  `agentmux-srv`". The installed AgentMux hosting the agent is never a
  descendant of the agent, so the walk can't reach it.
- **PID reuse:** every kill re-checks `(pid, started_at)` against the snapshot
  immediately before acting.
- **Trust:** an agent's `AgentResources` read and the `ShellStop` ownership
  check are limited to the calling agent's own block (the signed `auth` already
  used by `QuitSelf`).
- **Kept is final:** a resource kept by policy is never killed by a later step
  of the same run.

## 9. Tests

1. **One path (structural).** A source-scan test fails if any srv module other
   than `agent_teardown` / `process_tracker` calls `child.kill()`,
   `kill_tree`, `kill_process_group`, `release_block_processes` or
   `ShellSessionRegistry::stop` outside an allow-list (the Shell tool's own
   explicit `ShellStop`, and process-tracker internals).
2. **Every consumer calls it.** For each consumer in §6.8, an integration test
   spies on `agent_teardown::run` and asserts it was called once, with the
   expected policy.
3. **Multiple `task dev` (the ask).**
   - An agent starts two long-running background commands (a fake dev server
     that spawns a child and listens on a port) and one foreground command,
     then quits.
   - Assert: no process from the snapshot survives, both ports are free, the
     report lists all of them.
   - Run on Windows, Linux and macOS CI.
4. **Graceful first.**
   - A fake dev server that writes a marker on SIGTERM / CTRL_BREAK / WM_CLOSE
     writes it, and isn't in `forced`.
   - One that ignores the signal is in `forced`.
5. **Escapes.**
   - A process started via `Start-Process` (Windows) or `setsid nohup … &`
     (Unix) from the agent's Bash is found by the descendant walk and stopped.
     If it can't be, it's reported as a survivor, never silent.
6. **`Shell()` and `!cmd` grandchildren.** `cmd /C start …` / `sh -c 'x &'`
   grandchildren die with the agent on every close path, not just `/quit`.
7. **PtyShell headless.** Quit an agent whose pane isn't mounted: its drawer's
   PTY and children are gone.
8. **Container agent.** Quit stops the container and drops its dev-proxy routes.
9. **Policies.**
   - Restart keeps background tasks (the existing
     `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL` test, now through `run`).
   - Stop keeps them by default and stops them when asked.
10. **Safety.**
    - Another agent's `task dev` and the host AgentMux survive one agent's quit.
    - A PID reused between snapshot and kill (simulated) isn't killed.
11. **Idempotency.** Two quits in flight produce one teardown and the same
    report.

## 10. Phases

- **Phase 1: one inventory, one path, no behaviour change.**
  - Add `AgentResources` and `agent_teardown::run` with today's behaviour for
    each consumer, encoded as policies.
  - Route every user-facing consumer through it (§6.8): `/quit`, `QuitSelf`,
    `ClosePane` (all forms), pane, tab, window-tab and window ×, Stop /
    `agent.stop` / `FleetBulkStop`, and the orphan reaper (which closes via
    `close_pane`).
  - Controller replace (`stop_for_replace`, below the saga layer in
    `blockcontroller`) and app exit (`main.rs`) move in Phase 2, together with
    their behaviour changes. The structural test allow-lists exactly those two
    until then.
  - Add the structural tests (§9.1–2).
  - The close dialog and `/quit` summary read the inventory.
- **Phase 2: close the gaps on Windows; the last two consumers move.**
  - Controller replace (`resync_controller` → `stop_for_replace`) calls
    `agent_teardown` with `Policy::replace()`. That needs a backend-level
    entry, since `blockcontroller` sits below the saga layer.
  - App exit (`main.rs`) runs `agent_teardown` with `Policy::app_exit()` for
    every live agent, under the 10 s cap (§11 O3).
  - Both entries leave the structural test's allow-list, which then lists only
    definitions and the `ShellStop` tool.
  - `Shell()` and `!cmd` join the agent's job (§6.5).
  - PtyShell cascade (§6.6), containers and routes (§6.7).
  - `Stop` releases claims, with the background-task question (§7.4).
  - Verify step and survivor report (§6.2 steps 8–10).
- **Phase 3: graceful phase** (§6.3): WM_CLOSE / CTRL_BREAK / SIGTERM before
  force, under one `TEARDOWN_GRACE`.
- **Phase 4: Unix tracker** (§6.4): process groups for the CLI and background
  sessions, optional cgroup v2. Windows `CREATE_SUSPENDED` assignment. CI runs
  §9.3 on all three OSes.

Phase 1 is behaviour-neutral, which makes it safe to land first. Each later
phase changes one resource kind behind the same entry point.

## 11. Decisions (resolved 2026-10-01)

- **O1. Grace: one cap, `TEARDOWN_GRACE` = 8 s.** It's shared by every graceful
  step of one teardown, and the CLI's existing 5 s deadline sits inside it.
  Per-kind budgets were rejected: they'd add tuning knobs without changing the
  worst case. The cap is a setting (`agent:teardowngraceseconds`) for anyone
  who needs longer.
- **O2. Stop's background-task choice is remembered per user.**
  - The setting `agent:stopkeepsbackground` defaults to `true` (today's
    behaviour).
  - The Stop dialog's "Remember my choice" writes it.
  - **Why per user, not per agent:** the choice is about how the user works
    (dev servers they want to outlive a Stop or not). A per-agent setting would
    mean answering again for every new agent.
  - **Doesn't affect quit:** Quit never asks; it always stops everything.
- **O3. App exit: 10 s overall.**
  - Every live agent is torn down concurrently under one 10 s deadline, then
    the launcher backstop (Windows J0 / Unix group kill) takes over as today.
  - 10 s is the per-agent 8 s grace plus margin for the verify step. Agents run
    in parallel, so the total doesn't grow with the agent count.
  - **Amended 2026-10-01 (implementation, PR #4164):** the cap is 8 s, sweep
    included (`agentmux_common::process::SRV_APP_EXIT_CAP`). The launcher's
    upgrade quiesce (10 s) is pinned above it. On a normal quit the launchers
    keep their timing (Windows drops J0 when the host exits; Unix waits 1.5 s
    after SIGTERM), so there the backstop usually ends srv first. A longer
    wait holds the single-instance pipe while agents close, so a quick
    relaunch would forward to a dying instance and open nothing. It needs a
    single-instance handoff first (§12).
  - **Amended 2026-10-07 (normal-quit wait):** both launchers now stop srv
    through `upgrade::quiesce_srv` on every quit: close its stdin (Unix:
    also SIGTERM), wait up to `SRV_EXIT_WAIT` (10 s, pinned above the cap),
    and only then force-kill (Unix: srv's process group too; Windows: J0).
    Teardowns run as tracked tasks (`agent_teardown::detached`), so a caller
    that stops waiting (the host gives `CloseWindow` 2 s) can't cut one
    short, and `app_exit` waits for every close in flight. The handoff: a
    relaunch whose `open_new_window` forward fails (the host is gone, and a
    clean exit deletes its port file; or it is still starting) keeps at it
    for `SRV_EXIT_WAIT` + 5 s (`second_instance::await_running_instance`):
    it starts fresh as soon as the old launcher lets go of the socket/pipe,
    or forwards as soon as the host answers. Past that, the old outcome:
    silent exit, or the "not responding" dialog on Windows.
- **O4. Verified: `Start-Process` does not escape the agent's job on today's
  Windows build.**
  - **Test:** 2026-10-01, from inside an agent's Bash on AgentMux 0.59. A
    `Start-Process` child and a `cmd /c start /b` child both report
    `IsProcessInJob == true`. Children inherit every job of their creator, and
    the agent job doesn't set `BREAKAWAY_OK` (`process_tracker/windows.rs:86-90`),
    so they're in the agent's job.
  - **Why the retro differs:** its escape report
    (`RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md`) predates the
    `PROCESS_SET_QUOTA` assignment fix, when no process was in any job.
  - **What changes:** §6.4's descendant walk stays as the verify step's safety
    net (for the assignment race and job-creation failures), not as a known
    escape route.

## 12. Implementation status (2026-10-01)

- **Phase 1** (PR #4160): `AgentResources`, `agent_teardown`, every user-facing
  consumer routed, structural test.
- **Phase 2**
  - PR #4164:
    - Controller replace (`Policy::replace`) and the watchdog's stops go
      through the teardown.
    - App exit (`Policy::app_exit`, 8 s cap) goes through it too.
    - `Shell()` and `!cmd` join the agent's tracker (`track_adopted`).
    - PtyShell drawers cascade, and pane close stops `Shell()` sessions.
    - Every `delete_controller` caller goes through `agent_teardown::discard`.
    - The structural test now lists file by file and pins
      `stop_for_replace` and `delete_controller`.
  - Follow-up PR:
    - The verify step and survivor report (§6.2 steps 8–10): `/quit`'s
      summary carries `survivors`.
    - Stop releases claims and takes `stop_background` (§7.4 as amended).
    - Containers stop on close, quit and app exit (§6.7). They are not
      removed: `ensure_running` restarts them and the volume keeps state.
      A container another live pane uses is kept.
- **Not yet:**
  - Stopping foreground descendants on Stop (§5). They need telling apart
    from background tasks in the tracker.
  - The close dialog reading `AgentResources`.
  - The `AgentResources` MCP tool.
  - Phases 3 and 4.
