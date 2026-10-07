# Agent-Spawned Processes: Tracking and Cleanup — Analysis

**Date:** 2026-10-07
**Status:** analysis — findings plus a proposed design. Built so far: the Linux cgroup tracker (§6.2, a per-agent cgroup without sub-scopes), the best-effort scan tracker for macOS and Linux without delegation (§6.3), and the escape report (§6.4)
**Scope:** every process an agent causes to exist (its CLI, the CLI's MCP
servers, Bash tool commands, `run_in_background` tasks, whatever those
daemonize, `Shell()` / `!cmd` / PtyShell sessions), on Linux, macOS and
Windows: how AgentMux finds them, and what ends them when the agent ends.
**Related:** `docs/specs/SPEC_AGENT_TEARDOWN_SINGLE_PATH_2026_10_01.md`
(Phases 3–4 are the unbuilt part this analysis is about),
`docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md`,
tracking issue #3338, PR #4428 (graceful window close / app quit).

---

## 1. Summary

- **Linux and macOS have no process tracker.** `process_tracker::new_tracker`
  returns `StubTracker` on every non-Windows OS
  (`crates/srv/src/backend/process_tracker/mod.rs:197-201`): it lists nothing,
  kills nothing, and reports confidence `none`. The teardown, the close dialog,
  the Processes panel and the `agent.kill-tree` RPC all sit on top of it.
- **So on Linux and macOS, ending an agent ends only its CLI.** Close, `/quit`
  and app exit use policies with `stop_background: false`, relying on "the
  tracker takes them" (`crates/srv/src/sagas/agent_teardown.rs:69-111`). The
  tracker is the stub, so declared background tasks (`task dev`, dev servers),
  still-running Bash commands and anything they daemonized keep running,
  reparented to `systemd --user` or init. Nothing in AgentMux sees them again.
- **The process tree defeats every grouping AgentMux uses today.** Live on
  the dev host (§3): Claude Code starts every Bash tool command in a new
  session, bashwrap starts the real command in another one, and a `setsid`
  daemon inside that leaves both. Process groups and sessions can't hold an
  agent's processes together on Unix.
- **cgroup v2 can, and works on a stock desktop.** A per-agent cgroup under a
  delegated systemd user scope captured a `setsid` daemon, a double-fork and a
  plain child, listed all three, and `cgroup.kill` ended all three, with
  nothing left behind (§3.3).
- **Windows is the one platform with real tracking** (a Job Object per agent),
  with fixable holes: a spawn-then-assign race, a silent fallback to the stub,
  a 256-PID listing cap, and no graceful phase.
- **Recommendation (§6):** one `ProcessScope` abstraction, backed by cgroup v2
  on Linux, Job Objects on Windows and a best-effort scan on macOS. Every
  agent-side spawn goes into its agent's scope at creation, and the one
  teardown path (`agent_teardown`) uses it alone. Declared background tasks
  get their own sub-scope so restart and Stop can keep them, and close and
  quit can end them.

## 2. What exists today

### 2.1 The tracker

| | Windows | Linux | macOS |
|---|---|---|---|
| Implementation | `JobObjectTracker` (`process_tracker/windows.rs`) | `StubTracker` | `StubTracker` |
| Membership | Job Object, `KILL_ON_JOB_CLOSE`, no `BREAKAWAY_OK`, nested in the launcher's J0 | none | none |
| Assignment | after spawn (`OpenProcess` + `AssignProcessToJobObject`, `windows.rs:120-152`); the race is acknowledged at `windows.rs:20-25` | no-op | no-op |
| Listing | `JobObjectBasicProcessIdList`, capped at 256 (`windows.rs:158`) | always `[]` | always `[]` |
| Kill | `TerminateJobObject` (hard) | no-op | no-op |
| Confidence | High | None | None |

The registry (`process_tracker/registry.rs`) keys trackers by block id.
`track_spawned` / `track_spawned_agent` assign a root, `track_adopted` adds a
non-root PID (`Shell()`, `!cmd`), `release_block_processes` drops the tracker
(Windows: closing the job kills the tree). A 2 s poller emits
`agent:process-added` / `-exited`.

### 2.2 Spawn sites and how each is grouped (Unix)

| Spawn | Process group / session | Tracker | How it ends today |
|---|---|---|---|
| Persistent CLI (`common/src/cli.rs` `make_cli_cmd`, `persistent/spawn.rs:221`) | srv's group, srv's session | `track_spawned_agent` (stub) | interrupt, stdin EOF, `child.kill()` of the one PID |
| Per-turn CLI, `/btw` (`subprocess/host_spawn.rs`) | srv's group | stub | SIGTERM, 400 ms, kill, one PID |
| ACP CLI (`acp.rs:203-245`) | srv's group | stub | one PID |
| Codex App Server (`app_server.rs:834-841`) | srv's group | **never tracked, any OS** | `kill_pid` |
| PTY agent / terminal (`shell/lifecycle.rs:956`) | own session (portable-pty `setsid`) | stub | SIGTERM/SIGKILL to the session's group; on Unix also every background task's group |
| PtyShell drawer | own session | stub, on the drawer's own block | as above |
| `Shell()` (`shell_node.rs:402-427`) | `process_group(0)` | `track_adopted` (stub) | group kill (`common::process::kill_process_group`) |
| `!cmd` (`server/shell_handlers.rs:305-330`) | `process_group(0)` | `track_adopted` (stub) | group SIGKILL **only on timeout** |
| Bash tool command (Claude only, via bashwrap) | Claude starts it in a new session; bashwrap runs the command in a PTY, i.e. a second new session | none | SIGHUP when the PTY closes |
| `run_in_background` task (Claude only) | as above, plus bashwrap calls `setsid()` on itself (`bash_wrap.rs:373-390`) | its bashwrap PID only, in `db_background_tasks.pid` | only Stop with `stop_background`, or a PTY controller's `stop()` |
| MCP servers the CLI starts | the CLI's group | none | exit on their own when the CLI's pipes close, if they do |
| Docker containers the agent starts | children of dockerd | none, any OS | never |

### 2.3 Where a background task's PID comes from

bashwrap publishes `{op:"pid", tool_id, pid}` as a `tool_chunk` on the message
bus. srv doesn't consume it: the **frontend** relays it back as the
`backgroundtaskpid` RPC (`useToolChunkStream.ts:65-77` →
`websocket.rs:1519-1545`). If the pane isn't mounted, the PID is never
recorded. A command backgrounded mid-run (the feed's `Backgrounded` event)
never went through `--declared-background`, so it gets no PID at all.

### 2.4 What a close actually does on Linux

`close_cli` (`agent_teardown.rs:435-511`) waits for the CLI, reads
`list_block` (empty), calls `release_block_processes` (a no-op), narrates
`before.processes` (empty) and verifies them (nothing to verify). The report
says no survivors. Only `Shell()` sessions, PtyShell drawers and the agent
container are actually stopped.

## 3. Live evidence (Ubuntu desktop, GNOME Wayland, kernel 7.0, systemd 259)

### 3.1 An agent's real process tree

Captured from inside a running Claude agent on AgentMux 0.59.11:

```
agentmux-launcher            pgid=L    sid=S
└─ agentmux-srv              pgid=SRV  sid=S
   ├─ claude                 pgid=SRV  sid=S        ← the CLI shares srv's group
   │  ├─ agentmux-mcp        pgid=SRV  sid=S
   │  └─ bash -c …           pgid=B    sid=B        ← Claude: new session per Bash call
   │     └─ agentmux-bashwrap pgid=W   sid=B
   │        └─ bash -c <cmd> pgid=C    sid=C        ← bashwrap's PTY: another new session
```

Every process above, the whole desktop session included, is in one cgroup:
`…/session.slice/org.gnome.Shell@ubuntu.service`. Nothing separates one agent
from another or from GNOME.

### 3.2 What escapes

From inside a Bash tool call:

| Started as | Where it ended up | Survived the call? |
|---|---|---|
| `(sleep N &)` (double fork, same group) | reparented to `systemd --user` at once, still in the command's group `C` | no: the group died with the command |
| `setsid nohup sleep N &` | own session, reparented to `systemd --user` once its shell exited | **yes**: invisible to AgentMux, no tie to the agent |

The second row is how dev servers, `npm run dev &`, `nohup`, `docker run -d`
and most "start it in the background" commands look.

### 3.3 cgroup v2 under a delegated user scope

```
systemd-run --user --scope -p Delegate=yes …
  mkdir <scope>/agent-a
  child: echo $BASHPID > agent-a/cgroup.procs; exec setsid bash -c \
         "setsid sleep A & (sleep B &); sleep C"
```

- `agent-a/cgroup.procs` listed **all three** sleeps (setsid, double-fork, plain).
- `cgroup.events` said `populated 1`.
- `echo 1 > agent-a/cgroup.kill` ended all three. `populated 0`, and a process-table
  check found no leftovers.
- `cgroup.kill`, `cgroup.procs` and `cgroup.events` need no controller.
  Enabling controllers (`+pids +memory`) in `subtree_control` failed while the
  scope root still held a process: cgroup v2's no-internal-process rule. The
  design must keep processes only in leaf cgroups (§6.2).

Every agent process inherits `AGENTMUX_BLOCKID` and `AGENTMUX_AGENT_ID`
(checked on the CLI's `/proc/<pid>/environ`). That tag survives `setsid`,
double-forks and reparenting, so it can find escapees after the fact (§6.4).

## 4. Gaps, by severity

**P1: processes outlive their agent**

1. **No tracker on Linux or macOS** (`process_tracker/mod.rs:197-201`).
   Close, `/quit` and app exit leave every descendant of the CLI running.
2. **Declared background tasks survive close and quit.**
   `Policy::close/quit/app_exit` have `stop_background: false` on the stated
   assumption that the tracker takes them (`agent_teardown.rs:69-111`).
3. **Daemonized processes escape every grouping AgentMux uses** (§3.2): only
   membership that doesn't follow sessions or parentage (cgroups, jobs) holds them.

**P2: tracking is incomplete or depends on the UI**

4. The background-task PID reaches srv only through a mounted frontend
   (§2.3).
5. Mid-run backgrounded commands get no PID (§2.3).
6. Killing a background task's group reaches bashwrap's own session, not the
   command's PTY session. The command dies only from the PTY hangup, so
   `nohup` or a SIGHUP-ignoring server survives. bashwrap's Unix
   `kill_process_tree` is a no-op (`bash_wrap.rs:448-451`), and portable-pty's
   `kill` sends SIGHUP to one PID, not the group (`portable-pty` `lib.rs:340-347`;
   the comment at `bash_wrap.rs:407-416` assumes otherwise).
7. `! foo &` leaves `foo` running: the `!cmd` group is killed only on timeout
   (`shell_handlers.rs:394-401`).
8. The Codex App Server is never assigned to a tracker on any OS.
9. bashwrap is Claude-only (a PreToolUse hook). Codex, Gemini and ACP agents'
   commands get no background-task capture at all.
10. Containers an agent starts with `docker run` belong to dockerd and escape
    every mechanism on every OS.

**P3: Windows holes**

11. Spawn-then-assign race: agents aren't created `CREATE_SUSPENDED`.
12. Job creation failure silently falls back to the stub (`mod.rs:187-194`).
13. 256-PID listing cap (`windows.rs:158`).
14. `kill_tree` closes the job but leaves the registry entry, so later
    assignments on that block fail and go untracked.
15. Close is a hard `TerminateJobObject`, with no graceful signal first.
16. WMI, Task Scheduler, services and COM servers start processes outside the job.

**Consistency**

17. At least eight kill implementations with different graces (300 ms, 400 ms,
    5 s, none): `common::process`, the shell controller, the subprocess
    controller, `shell_handlers`, `agent_resources::force_kill_tree`,
    bashwrap, the launcher and the Job Object tracker.
18. The PTY controller's `stop()` kills background tasks on Unix, while
    `Policy::stop` keeps them by default (`agent:stopkeepsbackground`).
19. Comments and docs claim behaviour that doesn't exist: `cgroup.kill` /
    `killpg` in `registry.rs:189-191`, `blockcontroller/mod.rs:545-548`,
    `rpc_types/commands.rs:446-447`; a Swarm confidence badge in
    `process_tracker/mod.rs:24-25`; and the missing
    `agentmux-ai/AGENT_SPAWNED_PROCESSES_SPEC.md` they cite. The survival spec
    says `cgroup_linux.rs` and `macos.rs` exist; they don't.
20. The two specs disagree on the Linux plan: the tracker's doc says cgroup v2
    (High), the teardown spec's Phase 4 says process groups with cgroups as an
    optional add-on. §3.2 shows process groups can't be the baseline.

## 5. What "solid" has to mean

| # | Requirement |
|---|---|
| S1 | **Complete membership.** Every process descended from an agent's spawns belongs to that agent, whatever it does: `setsid`, double fork, `nohup`, reparenting. |
| S2 | **No race.** A process is in its scope before it runs a single instruction of its own (so before it can fork). |
| S3 | **Selective survival.** Declared background tasks are a separate sub-scope: restart and Stop (by default) keep them; close, quit, `/quit` and app exit end them. |
| S4 | **Graceful, then certain.** Signal everything (SIGTERM / CTRL_BREAK / WM_CLOSE), wait within the existing teardown deadline, then a kill that can't miss (`cgroup.kill`, `TerminateJobObject`). |
| S5 | **Observable.** List members with command line and start time; know when a scope is empty without polling where the OS allows it. |
| S6 | **Honest.** Whatever can't be held (macOS, no delegation, dockerd) is detected after the fact and reported as a survivor, never silently claimed as handled. |
| S7 | **One implementation.** Teardown, Stop, the Processes panel, `agent.kill-tree` and bashwrap use the same scope API, not eight kill paths. |
| S8 | **Independent of the UI.** Nothing depends on a pane being mounted. |

## 6. Proposed design

### 6.1 One abstraction: `ProcessScope`

Replace `TrackerHandle` with a per-agent scope that owns sub-scopes:

```
scope(block)
├── cli/          the CLI, its MCP servers, foreground Bash commands
├── bg/<tool_id>/ one per declared background task
└── shells/       Shell() and !cmd sessions
```

Operations: `spawn_into(sub, Command)` (S2), `members(sub|all)`,
`signal(sub|all, Graceful)`, `kill(sub|all)`, `wait_empty(sub|all, deadline)`,
`confidence()`. Teardown policies map onto sub-scopes:

| Policy | Ends |
|---|---|
| close, `/quit`, app exit | `all` |
| Stop (keep background) | `cli/` |
| Stop (stop background) | `cli/`, `bg/*` |
| restart / controller replace | `cli/` |

`agent_teardown::close_cli` then becomes: graceful CLI shutdown (unchanged),
`signal(all, Graceful)`, `wait_empty(all, deadline)`, `kill(all)`, and the
verify step reads `members(all)` instead of a snapshot. `stop_background_tasks`,
`force_kill_tree`, the shell controller's background kill and the per-module
kill helpers go away (gap 17).

### 6.2 Linux: cgroup v2 (confidence High)

- **Delegation.** The launcher starts srv in its own transient scope with
  `Delegate=yes`: `StartTransientUnit` on the user manager over D-Bus (what
  `systemd-run --user --scope` does). srv then owns that subtree and creates
  `agents/<block>/{cli,bg/<tool_id>,shells}` itself. Processes live only in
  leaves (srv itself goes in `srv/`), which satisfies the no-internal-process
  rule and allows `+pids +memory` per agent later.
- **Placement without a race (S2).** `clone3(CLONE_INTO_CGROUP)` (kernel 5.7+),
  or a `pre_exec` hook that writes `0` into the target `cgroup.procs` before
  `exec`. Both put the child in the cgroup before it runs its own code.
- **Background tasks.** bashwrap with `--declared-background` moves itself
  into `bg/<tool_id>` as its first act (srv passes the agent's cgroup path in
  an env var, e.g. `AGENTMUX_CGROUP`). Membership doesn't follow parentage, so
  the task survives a `cli/` kill on restart while still belonging to the
  agent. This also retires the frontend PID relay (gap 4): srv reads the
  members itself.
- **Mid-run backgrounding (gap 5).** The task stays in `cli/`. Restart would
  end it; the scope still holds it, so close never leaks it.
- **Ending.** SIGTERM each member, `inotify` on `cgroup.events` for
  `populated 0`, then `cgroup.kill` at the deadline, then `rmdir`.
- **Fallback when there's no delegation** (no systemd user manager: some
  containers, WSL without systemd, minimal distros): process groups plus
  `PR_SET_CHILD_SUBREAPER` on srv, so orphans reparent to srv instead of init
  and stay visible, plus the env-tag scan (§6.4). Confidence: BestEffort.

### 6.3 macOS: best effort (confidence BestEffort)

macOS has no cgroups, no subreaper and no public API that follows `setsid`
escapes (Endpoint Security needs an entitlement).

- Spawn the CLI with its own process group and record pgids and sessions
  bashwrap creates (it can publish them to srv directly, not via the UI).
- Every 2 s, list processes (`sysctl KERN_PROC_ALL`) and claim:
  descendants of known roots (ppid walk), members of recorded
  groups/sessions, and processes whose environment (`KERN_PROCARGS2`, readable
  for the same user) carries the agent's `AGENTMUX_BLOCKID`. Watch each claimed
  PID with `kqueue EVFILT_PROC NOTE_EXIT`.
- Ending: SIGTERM by group/session and claimed PID, wait, SIGKILL leaves-first
  with the start-time check `force_kill_tree` already does.
- Optional and private: the "responsible PID"
  (`responsibility_get_pid_responsible_for_pid`) is inherited across `setsid`.
  It would close most escapes but is undocumented; keep it behind a feature
  check, if at all.

### 6.4 Escape detection on every OS (S6)

After every teardown, and in the 2 s poll, scan for live processes whose
environment carries a closed or unknown `AGENTMUX_BLOCKID`. Report them as
survivors in `TeardownReport` and in the Processes panel ("1 process started by
this agent escaped tracking"), with a kill action. This catches `systemd-run`
and launchd-started processes, and processes in the macOS and fallback modes.
It can't catch processes started with a cleared environment, or containers.

### 6.5 Windows: fix the holes (confidence High)

- Spawn agents `CREATE_SUSPENDED`, assign, then resume, as the launcher
  already does for srv and the host (gap 11).
- Treat job-creation failure as an error with a visible notice, not a silent
  stub (gap 12).
- Page the PID list past 256 (gap 13); mark a killed tracker closed and
  recreate it on the next assignment (gap 14).
- Graceful phase before `TerminateJobObject`: `GenerateConsoleCtrlEvent` /
  `WM_CLOSE` to members (teardown spec Phase 3) (gap 15).
- Sub-scopes as nested jobs: `cli` and `bg/<tool_id>` jobs inside the agent's
  job, so restart can end `cli` alone.
- Assign the Codex App Server (gap 8).

### 6.6 Containers the agent starts (gap 10)

Out of reach of process tracking. Options, cheapest first:
1. Report them: list containers whose creation the agent's Bash commands
   caused (match `docker` CLI invocations in its scope) and show them as
   leftovers.
2. Label them: put a `DOCKER_*` / wrapper setting in the agent's environment so
   `docker run` adds `--label agentmux.block=<id>`, then stop labelled
   containers on close. Needs a shim; fragile.
Recommend (1) first.

### 6.7 Non-Claude agents (gap 9)

With cgroups and jobs, membership no longer needs bashwrap: every process a
Codex, Gemini or ACP CLI starts lands in its `cli/` scope and ends with it.
Only the restart-survival of *declared* background tasks needs the provider to
say which commands are background; that stays Claude-only until other CLIs
expose it.

## 7. Plan

| Phase | Work | Outcome |
|---|---|---|
| 1 | `ProcessScope` API and the Windows backend behind it (the existing Job Object code, plus CREATE_SUSPENDED, error on job failure, paging, closed-state fix). Teardown, Stop and `agent.kill-tree` use only it. Delete the duplicate kill paths. | One implementation (S7), no behaviour change elsewhere |
| 2 | Linux cgroup backend: delegated scope from the launcher, per-agent leaves, `CLONE_INTO_CGROUP`/`pre_exec` placement, `cgroup.events` watch, `cgroup.kill`. bashwrap joins `bg/<tool_id>`. | Linux High: close/quit/app exit end everything (gaps 1–7) |
| 3 | Fallback (no delegation) and macOS: process groups, subreaper (Linux), process-table scan with env tag, kqueue exit watch. | BestEffort, honestly labelled |
| 4 | Escape detection and UI: survivors in `TeardownReport`, the Processes panel shows members on every OS with confidence, close dialog lists real processes. | S5, S6 |
| 5 | Containers: report agent-started containers. | Gap 10 visible |

**Tests (per OS in CI):** an agent that starts (a) a plain child, (b) a
double fork, (c) `setsid nohup`, (d) a SIGHUP-ignoring server, (e) a declared
background task, (f) a mid-run backgrounded task. Then: restart keeps (e) and
ends the CLI; Stop keeps (e) by default; close, `/quit` and app exit leave
nothing in the scope and nothing tagged in the process table; a forced
`kill -9` of srv followed by relaunch finds and reports the leftovers.

## 8. Fix now, independent of the above

- Correct the false comments in gap 19 and the survival spec's claim about
  `cgroup_linux.rs` / `macos.rs`.
- Pick one Linux plan in the teardown spec (cgroup v2 as the baseline, process
  groups as fallback) so the two specs stop disagreeing (gap 20).
- Until Phase 2 lands, make close/quit/app exit on Unix also stop declared
  background tasks explicitly (`stop_background: true` for those policies when
  the tracker's confidence is `None`), so at least `task dev` doesn't outlive
  its pane. This is one line per policy and removes the most visible leak.
  It reaches only bashwrap's session (gap 6): the command then dies from the
  PTY hangup, so a `nohup` or SIGHUP-ignoring server still survives. Pair it
  with a Unix `kill_process_tree` in bashwrap that signals its PTY child's
  session group.
