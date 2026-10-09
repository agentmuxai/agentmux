# SPEC: Tower, a read-only task manager pane (CPU and memory per task)

**Status:** active. The Tasks and Host views (§9 phases 1-3) are
implemented (PR #4497); remote hosts (§8) are next. Name chosen by the
operator: **Tower**. Defaults taken as recommended (2026-10-08): CPU as a share
of the whole machine with a per-core toggle; one AgentMux row that expands to
its processes; one-time pairing for LAN viewing; internet hosts in a separate
spec.
**Date:** 2026-10-08
**Verified against:** `main` @ `c9fa25f` (#4495).
**Related:**
- `per-pane-cpu-memory.md` and `per-pane-process-tree-metrics.md`: the
  `blockstats` badge this builds on.
- `docs/analysis/agent-spawned-process-tracking-2026-10-07.md`: the per-agent
  process trackers (Job Object / cgroup / scan). Its §1/§2.1 predate the
  Linux cgroup and scan trackers.
- `SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md`: lists "a general task
  manager" as a non-goal of the drawer panel. This spec is that task manager,
  as its own pane.
- `SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`: how the pane registers.

## 0. The ask

From the operator: explore a task manager inside AgentMux. Read-only to start,
focused on CPU and memory per task, and with no UAC or other privilege
escalation on any of the three platforms. Follow-ups: it should also be able
to show every process on the host, and processes on other hosts, efficiently.

Tower has three views, built in this order:

| View | Shows | Section |
|---|---|---|
| **Tasks** | Each pane and everything it started, plus AgentMux itself | §1-§6 |
| **Host** | Every process on this machine that the OS lets us read without elevation | §7 |
| **Remote hosts** | Tasks/Host for another machine: SSH and WSL first, LAN peers next, same-account WAN hosts last | §8 |

## 1. What a "task" is

A **task** is one pane (block) and every process it started: the agent CLI or
shell, its MCP servers, and whatever those spawn (builds, test runners,
language servers, background jobs). AgentMux itself (launcher, srv, the CEF
host, and its renderer/GPU/utility helpers) is one more row, **AgentMux**, so
the pane also answers "is it the app or the agents?".

The Tasks view shows one row per task with its total CPU and memory,
expandable to the processes under it. The rest of the machine is the Host
view (§7); the System Info pane keeps the system-wide graphs.

## 2. What exists today

Most of the plumbing is there; the gaps are coverage and metric quality.

| Piece | Where | Gap for a task manager |
|---|---|---|
| `blockstats` event (cpu, mem, pids per block, every `telemetry:interval`) | `backend/sysinfo.rs:1189-1291` | Only blocks in `pidregistry`, which only the PTY shell controller registers (`blockcontroller/shell/lifecycle.rs:987`). Subprocess, persistent, ACP and app-server agents get nothing. |
| Process-tree walk | `blockcontroller/process_tree.rs` | Parent links only, so `setsid`/daemonized children drop out. Capped at `MAX_PIDS_PER_BLOCK = 64`. Pass 1 refreshes **every** process on the machine each tick. |
| Per-agent containment | `backend/process_tracker/` (`windows.rs` Job Object per block, `cgroup_linux.rs` cgroup v2 per agent, `scan.rs` env-tag scan on macOS) | Used for teardown and the drawer list, not metrics. `TrackedProcess` has no CPU field; Windows `started_at_ms` is always 0. |
| Process list RPCs | `server/app_api/agent_io.rs` (`agent.process-list`, `agent.tracked-blocks`) | Snapshot only, memory only. |
| Event lanes | `backend/eventbus.rs:308`, `websocket.rs:347-360`, `frontend/app/store/mps.ts:181` | Ready to reuse: the background lane keeps telemetry from delaying terminal input. |

Metric quality today: `blockstats.mem` is sysinfo's `memory()`, which is
working set on Windows and RSS on macOS/Linux. Neither matches what the OS's
own task manager shows, and both count shared library pages, so per-task sums
overstate. `blockstats.cpu` is sysinfo's per-core percentage (can exceed 100%).

Privilege posture today, which this spec keeps: the launcher and host
manifests are `asInvoker`, the Inno installer is `PrivilegesRequired=lowest`,
nothing enables `SeDebugPrivilege`; macOS uses hardened runtime without the
App Sandbox and without `task_for_pid`; Linux packaging is AppImage/deb/rpm/
tarball (no Flatpak/Snap sandbox). Every process the pane reads is a
same-user child, which all three OSes let us read without elevation.

## 3. No-escalation rules

These hold for every phase, including later ones that add actions.

**Use:**
- Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` only, never
  `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ`. The limited right is all
  `GetProcessTimes` and `GetProcessMemoryInfo` need, and it is the right
  Microsoft added for protected/elevated targets. Job Object queries need no
  privilege.
- macOS: `proc_pid_rusage` and `proc_pidinfo` on our own PIDs; `sysctl
  KERN_PROC_ALL` for parent links.
- Linux: `/proc/<pid>/stat` and `/proc/<pid>/status`; the agent's own cgroup
  files.

**Never:**
- Windows: `SeDebugPrivilege`, a `requireAdministrator` manifest, `runas`, a
  service or elevated helper.
- macOS: `task_for_pid` (needs root or the debugger entitlement plus an
  authorization prompt), a privileged helper, the `footprint` CLI.
- Linux: `CAP_SYS_PTRACE`, setuid helpers, system-level `systemd-run` without
  `--user` (polkit prompt), drop-ins that need root.

**When access is denied**, the row shows `—`, never `0`, with a tooltip saying
why. On all three OSes a denied read fails quietly (`ERROR_ACCESS_DENIED`,
`EPERM`); none of the "use" calls above can raise a prompt. The case that can
happen: an agent runs an elevated child (e.g. `Start-Process -Verb RunAs`).
Windows starts that through the AppInfo service, so it is outside our Job
Object and not in the task at all; the pane cannot see it, by design.

## 4. Metrics

### 4.1 Memory

One headline column, named **Memory**, meaning the process's *private*
memory: the number that grows when that process leaks, that doesn't count
shared DLLs twice, and that matches the OS's own tool where one exists.

| OS | Headline | Matches | Fallback / tooltip |
|---|---|---|---|
| Windows | `WorkingSetPrivateSize` from `SystemProcessInformation` (§7) | Task Manager "Memory (active private working set)" | Every Windows since Vista, every process, no handle needed (so not `PROCESS_MEMORY_COUNTERS_EX2`, which needs 22H2 and an open handle). Tooltip: working set and commit. |
| macOS | `rusage_info_v2::ri_phys_footprint` | Activity Monitor "Memory" | Tooltip: resident size. |
| Linux | `RssAnon + VmSwap` from `/proc/<pid>/status` | No single standard; this is Chrome's "private footprint" definition | Tooltip: `VmRSS`. PSS/USS (`smaps_rollup`) only on demand when a row is expanded: it walks page tables and is too costly per tick. |

A task's total is the sum of its processes' headline values.

### 4.2 CPU

CPU% = Δ(user + kernel CPU time) ÷ Δ(monotonic wall time), sampled each tick.
The first sample after a process appears has no value.

- **Default: percent of the whole machine** (Windows Task Manager convention;
  0-100% however many cores). Divide by the logical CPU count
  (`GetActiveProcessorCount(ALL_PROCESSOR_GROUPS)` on Windows).
- **Toggle: percent of one core** (`top`/Activity Monitor convention; can
  exceed 100%). The current `blockstats` badge uses this, so the toggle also
  lets the pane and the badge agree.

Per-task CPU comes from the containment primitive where it keeps exited
children's time, so a burst of short-lived build processes isn't lost between
samples:
- Windows: `JobObjectBasicAccountingInformation` (`TotalUserTime +
  TotalKernelTime`, covers terminated members) on the per-block job.
- Linux: `cpu.stat` `usage_usec` in the agent's cgroup. It exists whether or
  not the cpu controller is enabled, so it works with today's
  controller-less cgroups.
- macOS, and anywhere without a tracker: sum of per-process deltas.

**macOS unit trap:** `pti_total_user`/`ri_user_time` are Mach absolute-time
ticks, not nanoseconds. On Apple Silicon multiply by `mach_timebase_info`
(125/3). htop and osquery both shipped this bug. `agentmux-procstats`
converts them, and its `cpu_time_grows_while_this_process_spins` test bounds
the result on every OS (a units error misses by orders of magnitude).

### 4.3 Identity and cadence

- A process is keyed by **(pid, start)**, never pid alone, so a reused PID
  is a new row. Start: the creation time (Windows), `/proc/<pid>/stat` field
  22 (Linux), the kernel's unique id from `PROC_PIDUNIQIDENTIFIERINFO`
  (macOS, which answers it for any user's process). On the wire it is
  `TowerProcess.id`, `"pid:start"`.
- Cadence: the pane asks every 2 s while visible (after 1 s when the first
  answer has no rates yet). A gap of more than 10 s starts the rates over, so
  a pane hidden for minutes doesn't show minutes-long averages.
- Cost: one snapshot of the process table per sample (one syscall on
  Windows; two small `/proc` reads per process on Linux; a few `proc_pidinfo`
  calls per process on macOS), plus each tracked block's PID list and CPU
  account. Two panes asking within 900 ms share one sample.

## 5. Membership: which processes belong to a task

Prefer the existing tracker, fall back to the tree walk, and make membership
sticky.

1. **Tracker first.** `AgentProcessRegistry` already holds a tracker per agent
   block (Windows Job Object, Linux cgroup, macOS/Linux-without-systemd
   scan). Its member list is the task's process set. Job Objects and cgroups
   catch `setsid`/daemonized children, which the parent walk loses.
2. **Tree walk for the rest.** PTY terminals have a Job Object on Windows but
   only the stub tracker on Unix; those take the descendants of the
   registered root PID (`pidregistry`) from the same snapshot.
3. **Sticky.** Once a (pid, start time) is seen in a task, it stays in that
   task until it exits, even if it is reparented to init/launchd. On Windows,
   trust a parent PID only if the parent's creation time precedes the
   child's.
4. **No 64-PID cap.** The walk stops only at 2,048 processes per task (a fork
   bomb shouldn't stall a sample).
5. **AgentMux.** From srv, up through parents whose executable is AgentMux's
   (launcher, window host), then everything under that no task claimed.

The `agent_started` filter (which hides the CLI itself, conhost and MCP
servers from the drawer) hides nothing here: those processes use CPU and
memory. It sets each row's `role` instead: `main` (the CLI or shell),
`support` (MCP servers, conhost; shown dimmer) or `started`.

## 6. Shape of the change

### 6.1 Backend

- **`crates/procstats`** (`agentmux-procstats`): the per-OS readers
  (`windows-sys`; `libc` on Linux and macOS), `snapshot()`,
  `command_line(pid)`, `cpu_count()` and `RateMeter` (cumulative times to
  rates). Dependency-light so the static remote helper (`crates/remote`) can
  link the same code (§8.2): a remote Linux host and a local one report the
  same numbers. sysinfo is not used: its memory numbers match no OS task
  manager and it knows nothing about Job Objects or cgroups.
- **`TrackerHandle`** gains `member_pids()` (the PIDs alone, no per-process
  enrichment) and `cpu_time_ns()` (Windows: `JobObjectBasicAccountingInformation`;
  Linux: `cpu.stat` `usage_usec`). `AgentProcessRegistry::members_by_block()`
  reports both per block, querying the trackers outside its lock.
- **`backend/tower_sampler.rs`**: the grouping (§5) as a pure function, the
  rates, sticky membership and the 900 ms shared sample.
- **RPCs** (`server/app_api/tower_pane.rs`): `tower.sample { host? }` returns
  a `TowerSnapshot`; `tower.command-line { id }` returns one process's command
  line, only if that exact (pid, start) is still running. Windows reads it
  with `NtQueryInformationProcess(ProcessCommandLineInformation)`, which needs
  only `PROCESS_QUERY_LIMITED_INFORMATION`. **Both refuse an agent's
  connection** (`RpcContext.agent_id` set): a process list shows what every
  other agent runs, and command lines can hold secrets.
- **Request-driven, not an event.** An earlier draft published a `taskstats`
  event on the background lane. Having the pane ask instead means nothing is
  sampled while no Tower is open, without any subscription bookkeeping, and
  the Host view's larger answer goes only to the pane that asked for it.
- `blockstats` (the pane badge) keeps its own numbers for now; moving it onto
  this sampler is a follow-up (§9).

### 6.2 Frontend

- `frontend/app/view/tower/`: a native pane tab (`tower.tsx`), its model
  (`tower-model.ts`: polls only while the tab is visible, keeps the view and
  CPU convention in block meta as `tower:view` / `tower:cpu`), the view and
  pure helpers (`tower-util.ts`). Controls are `element/ui`'s (`Tabs`,
  `SegmentedControl`, `Button`, `IconButton`, `FilterInput`).
- Tasks view: one row per task (name, kind badge, CPU, Memory, process
  count), sortable by any column, expandable to its processes; a button
  reveals the pane. Each process row fetches its command line on demand.
- Host view: every process with its task named, a filter (name, PID or task),
  the first 400 rows of the sorted, filtered list, and the count of processes
  the OS wouldn't measure.
- Reachable from the command palette ("Open Tower (Task Manager)"), the
  widgets list (`defwidget@tower`, not pinned), `pane.open` with
  `view: "tower"`, and saved layout files.

### 6.3 Out of scope

- Any action: kill, suspend, priority, limits. The kill RPCs exist
  (`agent_io.rs`), but Tower is read-only until a separate spec says
  otherwise.
- Container agents.
- GPU, disk and network per task.
- An MCP tool that lets agents read process lists. Possible later; it would
  need its own decision about which agents may see which hosts.

## 7. The Host view: every process on this machine

Same table, same metrics (§4), but every process the OS will report to an
unelevated caller, with AgentMux's own tasks highlighted and grouped at the
top. No elevation, so coverage differs by OS:

| OS | Coverage without elevation | How |
|---|---|---|
| Windows | **All processes**, including SYSTEM and elevated ones, with CPU and memory | One `NtQuerySystemInformation(SystemProcessInformation)` call returns every process's CPU times, private working set, working set, commit and parent PID without opening any process handle. This is how an unelevated Task Manager shows everything. The struct's layout (mostly `Reserved` in winternl.h) is pinned by compile-time offset assertions and by a test comparing it with `GetProcessTimes` / `GetProcessMemoryInfo` for a child process. |
| Linux | **All processes**, unless `/proc` is mounted with `hidepid`, in which case only the user's own | `/proc/<pid>/stat` and `status` are world-readable by default. |
| macOS | **The user's own processes in full; other users' (root daemons, `WindowServer`, `kernel_task`) as name and PID only**, CPU and memory shown as `—` | `proc_pid_rusage`/`PROC_PIDTBSDINFO` return `EPERM` for other users' processes; `proc_listallpids`, `PROC_PIDT_SHORTBSDINFO` and `proc_pidpath` still list and name them. Activity Monitor and `ps` only see more through privileged helpers, which §3 rules out. A footer says "N processes owned by other users can't be measured without administrator rights", rather than hiding them. |

Cost and cadence: this is a machine-wide read each tick (hundreds of
processes), so the Host view samples **only while it is the visible tab of a
visible Tower pane**, at 2 s. On Windows it is one syscall per tick; on
Linux and macOS a few small reads per process. Command lines are fetched only
when asked for, one row at a time, never every tick.

## 8. Remote hosts

### 8.1 Principles

Efficiency comes from doing the work where the processes are and sending
little:

1. **Sample on the remote host**, with the same reader code (§6.1). Never
   ship raw `/proc` text or run `ps` per tick over the wire.
2. **Only while someone is looking.** The viewer holds a lease (renewed every
   ~10 s by an open Tower pane on that host's tab); the remote sampler stops
   when the lease lapses. Zero cost for hosts nobody is viewing.
3. **Latest value, never a queue.** A slow link skips frames instead of
   buffering them (the `fleet_feed` `watch`-channel pattern); a reconnect
   gets one full snapshot.
4. **Deltas and caps.** After the first snapshot, each frame carries only
   rows whose CPU or memory moved past a threshold, plus added and exited
   (pid, start time) keys. The Host view sends the top 50 by the current sort
   key and one "other processes" total row. Names are interned per
   connection. Target: well under 2 KB per frame for the Tasks view and
   under 8 KB for the Host view, at 2 s (5 s when the pane isn't focused).
5. **No command lines by default.** Process command lines routinely hold
   tokens and paths. Remote frames carry the executable name only; full
   command lines are an opt-in per remote host, fetched on demand for one
   row, never streamed.

### 8.2 SSH and WSL remotes (first)

These already have a channel and a consent model, and need no new network
surface.

- **SSH (Linux, macOS hosts):** add a `procs --stdio` subcommand to the
  `agentmux-remote` helper (`crates/remote/src/main.rs`, beside `serve
  --stdio` and `attach`). srv keeps one `ssh -T` connection per viewed host
  open, as `backend/remote/files.rs` does for file operations, and closes it
  when the lease lapses. The helper's existing install consent (`conn:helper`,
  `helper_consent.rs`) and per-agent host access (`remote/agent_access.rs`)
  apply unchanged.
- srv answers `tower.sample` for that connection from the latest frame (the
  pane names the connection; the lease is renewed by the pane's own polling).
  The same feed could later populate the System Info pane's `connection`
  scope, which the frontend already supports but nothing publishes.
- **WSL:** run the Linux helper inside the distribution with `wsl.exe -d
  <distro> -- … procs --stdio`. No network and no SSH.
- **Windows SSH hosts:** no helper exists for them. Out of scope until one
  does; Tower shows the host as "not supported".
- What a remote "task" is: on an SSH host Tower has no panes to group by, so
  it shows the Host view only. (An SSH pane's local `blockstats` measures the
  local `ssh` client, which is correct for the Tasks view on this machine.)

### 8.3 Other AgentMux instances on the LAN (second)

The LAN links that exist today aren't suitable for process data. The routes
that accept the LAN key are deliberately limited to agent names, kinds and
status (`server/reactive.rs`, `handle_reactive_agent_names`), and
`SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md` §6 already explains why
anything beyond that needs a stronger credential. Process lists are well past
that line.

The model to reuse is the mobile viewer's (`backend/viewer/`): a one-time
pairing code shown on the target machine, a revocable per-device token, TLS
with a pinned certificate, and per-subscriber byte limits that reset a slow
reader (`viewer/feed.rs`). Tower adds:

- a viewer route on the target, e.g. `GET /agentmux/viewer/procs` (SSE,
  frames per §8.1), gated by that token and by a per-machine "allow paired
  devices to see processes" setting, off by default;
- a desktop-side viewer client in srv, which doesn't exist yet: today only
  mobile pairs as a viewer;
- the target's own user sees which devices are watching and can revoke them.

Remote AgentMux instances can show both views, since they have panes: the
Tasks view is the most useful one for a fleet of agent machines.

### 8.4 Same-account hosts over the internet (third, separate spec)

AgentMux's cloud relay carries messages and low-rate presence, not streams,
so there is no efficient path for 2 s process frames today. Two options, to
be decided in a separate spec (cloud-side design belongs in the private cloud
repository, not here):

1. **Summary only:** a small signed per-host total (CPU, memory, busiest few
   tasks, no command lines) at presence cadence, a minute or so. Cheap, and
   enough for "which machine is on fire".
2. **Direct when reachable:** use the account's verified instance identity
   (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md`) to find and authenticate the
   host, then open the §8.3 viewer connection directly when the network
   allows it.

A true live stream over the internet would need new cloud infrastructure and
is not proposed here.

## 9. Phases

1. **Done.** Tasks: membership from the trackers for every controller type
   that has one, (pid, start) identity, the per-OS memory headline, Job/cgroup
   CPU accounting, the shared `procstats` crate.
2. **Done.** Tasks: `tower.sample` and the Tower pane.
3. **Done.** Host view (§7).
   - **Follow-up:** point the `blockstats` badge at the same sampler, so badge
     and pane agree (today the badge shows working set / RSS, per core).
4. **SSH and WSL remotes** (§8.2).
5. **LAN peers** (§8.3).
6. **Later, separately specced:** WAN hosts (§8.4), actions (kill tree,
   already safe with Job Objects/`cgroup.kill`), per-task memory via an
   enabled cgroup memory controller on Linux (needs srv moved out of the
   scope root into a leaf cgroup).

## 10. Testing

- Unit: CPU delta math, both CPU conventions, (pid, start time) reuse,
  sticky membership after reparenting, truncation reporting, `—` on denied.
- Per OS, in CI where available: spawn a child tree (including a `setsid`/
  detached grandchild and a short-lived burst), assert it is attributed to the
  right task, that Job/cgroup CPU includes the exited children, and that the
  memory headline is within a few percent of the OS tool's value.
- macOS on Apple Silicon: a busy-loop child reads ~100% of one core, not
  ~2.4% or ~4100% (the timebase bug).
- Manual, in an isolated `task dev` build, never the operator's instance:
  an elevated child on Windows does not prompt and is not shown.
- Host view: on Windows, SYSTEM and elevated processes have CPU and memory;
  on macOS, other users' processes show `—` and are counted in the footer;
  on Linux with `hidepid=2`, only the user's processes appear, without
  errors.
- Remote: a frame-size test for 50 and 500 processes; the remote sampler
  exits within one lease period after the last viewer closes; a link
  throttled to a few KB/s shows stale-but-current data, not a growing
  backlog; no command line appears in any remote frame unless the host's
  opt-in is on.

## 11. Open questions

1. Default CPU convention: whole machine (Task Manager) or per core (badge,
   `top`)? This spec picks whole machine with a toggle.
2. Should the AgentMux row split out CEF renderer/GPU helpers, or show one
   total? They are in launcher's job J0 on Windows, but on macOS/Linux they
   need their own walk.
3. LAN viewing (§8.3): is a one-time pairing per machine pair acceptable, or
   should machines on the same account pair automatically using their
   verified instance identity?
4. Which of §8.4's two options for internet hosts, if either.

## 12. Name

**Tower**, chosen by the operator: air-traffic control next to Hangar, and
"watching, not touching" fits a read-only pane. Alternatives considered:
Muxtop, Gauges, Vitals, Hive. "Task Manager" was avoided because it collides
with the Work queue's tasks.

## 13. Sources

- Microsoft: process access rights
  (https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights),
  `GetProcessMemoryInfo` and `PROCESS_MEMORY_COUNTERS_EX2`
  (https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex2),
  Job Objects and nested jobs
  (https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects,
  https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs),
  `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`
  (https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information).
- Task Manager's memory and CPU columns:
  https://scorpiosoftware.net/2023/04/12/memory-information-in-task-manager/,
  https://aaron-margosis.medium.com/task-managers-cpu-numbers-are-all-but-meaningless-2d165b421e43.
- macOS: Activity Monitor's sources
  (https://www.bazhenov.me/posts/activity-monitor-anatomy/), the Mach-tick
  CPU bug (https://github.com/htop-dev/htop/pull/752,
  https://github.com/osquery/osquery/pull/7473), `task_for_pid` restrictions
  (https://developer.apple.com/forums/thread/655349), `phys_footprint`
  without root (https://github.com/aristocratos/btop/issues/1871).
- Linux: proc(5) (https://man7.org/linux/man-pages/man5/proc.5.html),
  `smaps_rollup`
  (https://www.kernel.org/doc/Documentation/ABI/testing/procfs-smaps_rollup),
  Yama (https://www.kernel.org/doc/html/latest/admin-guide/LSM/Yama.html),
  cgroup v2 (https://docs.kernel.org/admin-guide/cgroup-v2.html), systemd
  delegation (https://systemd.io/CGROUP_DELEGATION/).
- Prior art: VS Code process explorer
  (https://github.com/microsoft/vscode/blob/main/src/vs/base/node/ps.ts,
  https://github.com/microsoft/vscode-windows-process-tree), Electron
  `ProcessMetric` (https://www.electronjs.org/docs/latest/api/structures/process-metric),
  Chrome's private footprint
  (https://groups.google.com/a/chromium.org/g/chromium-dev/c/ELSYMXnvbBc),
  sysinfo (https://github.com/GuillaumeGomez/sysinfo), bottom
  (https://github.com/ClementTsang/bottom).
