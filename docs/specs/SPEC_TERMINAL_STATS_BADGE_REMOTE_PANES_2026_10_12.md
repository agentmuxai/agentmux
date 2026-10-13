# The terminal stats badge on SSH and WSL panes: the remote shell's figures, not the local client's

**Date:** 2026-10-12
**Author:** Loap.
**Status:** proposed. Phase 0 is built with the badge tooltip (`docs/reports/REPORT_TERMINAL_STATS_BADGE_MEMORY_2026_10_12.md` §5); phases 1-3 are not.
**Area:** `crates/srv/src/backend/sysinfo.rs` (the badge), `blockstats_detail.rs` and `server/app_api/blockstats.rs` (the tooltip), `crates/remote` (the helper: `procs.rs`, `daemon.rs`, `fsproto.rs`), `backend/tower_remote.rs` (how srv reaches a helper).

## 1. The problem

The badge sums the pane's shell and its descendants **on this computer** (`pidregistry` holds the PID the pane's PTY spawned). For a remote pane that process is not the shell:

| Pane | What runs locally (`shell/lifecycle.rs`) | What the badge measures today |
|---|---|---|
| Local terminal | the shell | the shell and what it started: right |
| WSL (`wsl://<distro>`) | `wsl.exe -d <distro> …` | `wsl.exe`: a few MB. The distro's processes run in the WSL VM and aren't in Windows' process list |
| SSH, plain | the system `ssh` | the `ssh` client: a few MB |
| SSH, durable (`term:durable`, the default for new panes on a host) | no PTY: `ssh -T host -- agentmux-remote attach …` | **nothing**: `durable_ssh.rs` never registers a PID, so there is no badge |

So a remote pane's badge is either misleading (WSL, plain SSH) or missing (durable SSH). The user wants the badge and its tooltip to describe the remote side: the shell's processes on the host, and the host itself.

## 2. What exists to build on

- **The helper** (`crates/remote`, `agentmux-remote`) already samples a host's processes with the same reader srv uses (`agentmux_procstats`) and answers `fsproto::Request::Procs` with a small `ProcFrame`: totals for the host (CPU, private memory, process count, cores) and the top rows by CPU and memory. Tower uses it through `tower_remote::sample` over the connection the file browser opens (`serve --stdio`; for WSL, the Linux helper run inside the distro).
- **WSL:** the pane's `wsl.exe` forwards `AGENTMUX_BLOCKID` into the distro (`remote/wsl.rs`, `FORWARDED_ENV`), so the pane's shell inside it carries the pane's id in its environment.
- **Durable SSH:** the helper's daemon on the host spawns each session's shell itself (`daemon.rs`, `pty::spawn_login_shell`) and keeps its PID. Its control socket already answers `LIST` and `END <session>`; the pane knows its session id (`remote:session_id`).
- **Plain SSH:** nothing on the host ties a shell to a pane. `ssh::launch` sends `SetEnv` with AgentMux's build info, but sshd passes those on only where its `AcceptEnv` lists `AGENTMUX_*`, which stock configs don't.

## 3. Proposal

### Phase 0: say what is measured, and where (built)

- The tooltip shows immediately, with no fade.
- For a remote pane it names the connection and its machine: for SSH, the host `ssh -G` resolves the destination to (an alias's `HostName`) and its IP address from a DNS lookup; for WSL, the distribution and its first address (`hostname -I` inside it). Both are cached for 5 minutes (`backend/remote/address.rs`).
- It says the figures are this computer's `ssh` (or `wsl.exe`), not the remote shell's, until phases 1-3 replace them.

### Phase 1: WSL panes measure the distro's shell

1. **Helper:** a new request `PaneProcs { block_id }`. It finds the processes on the host whose environment holds `AGENTMUX_BLOCKID=<block_id>` (Linux: `/proc/<pid>/environ`, readable for the user's own processes; the shell and everything it started inherit it), and answers like the badge's tree: process count, names, the oldest's start time, CPU (rate since the previous request on this connection, as `Procs` does), resident and private memory, and the host's own totals. Bump `fsproto`'s version (`Hello`); an older helper answers "unknown request", which srv shows as "not measured".
2. **srv:** for a `wsl://` pane, `blockstats.detail` asks the distro's helper (the one `tower_remote` already starts and idles out after 2 minutes) instead of taking a local snapshot.
3. **Badge:** see §4: whether the 1 Hz badge itself switches to the remote figures.

Matching by environment, not by tree from a PID, because the distro's PIDs aren't visible from Windows and `wsl.exe`'s child inside the distro has no stable relation to it that srv can see. A process that clears its environment (rare: `env -i`, some daemons) drops out of the count; that is accepted and noted in the tooltip ("processes that kept the pane's id").

### Phase 2: durable SSH panes get a badge, measured on the host

1. **Daemon:** a control command `STATS <session>`. The daemon knows the session's shell PID, so it walks that process's tree on the host with `agentmux_procstats` (exact, no environment matching) and answers the same figures as phase 1.
2. **Helper CLI:** `agentmux-remote stats --session ID`, as `list` and `end` are reached today (`sessions.rs` runs them over the pane's host connection), or, cheaper per request, a `SessionStats { session }` request on the `serve --stdio` connection the file browser and Tower already hold open, which asks the daemon over its socket. The second is preferred: no new `ssh` per sample.
3. **srv:** `sysinfo.rs` publishes `blockstats` for durable panes too, from these samples (§4), so durable panes get the badge they lack today.

### Phase 3: plain SSH panes

No reliable way exists to find the pane's shell on the host:
- `AcceptEnv`: add `AGENTMUX_BLOCKID` to the `SetEnv` `ssh::launch` sends. Where the host's sshd accepts `AGENTMUX_*`, phase 1's environment match works unchanged. Most hosts won't.
- Otherwise the tooltip shows **the host's** figures (`Procs` totals: CPU across its cores, memory in use, process count), labelled as the host's, not the pane's, and suggests making the pane durable.

A rewrite of the plain pane's remote command (`exec env AGENTMUX_BLOCKID=… $SHELL -l`) would mark the shell everywhere, but `ssh::launch` deliberately sends no remote command for a plain interactive pane (a host may not run a POSIX shell, and a login shell started that way differs from sshd's), so it is out of scope.

## 4. The decision this needs: what the badge shows on a remote pane

The badge updates every second from a local snapshot that costs nothing extra; a remote sample costs a round trip through the helper and a process snapshot on the host (about 20 ms of the host's CPU).

- **(a) Recommended.** The badge shows the remote figures for panes phases 1-2 can measure, sampled every 2 s and only while the pane is visible (as Tower samples), with the connection's name or a small marker so it reads as remote. Plain SSH panes without `AcceptEnv` hide the badge rather than show the `ssh` client's few MB; their tooltip still shows the host's figures.
- **(b)** The badge stays local (the client process), and only the tooltip, while open, shows the remote figures. Cheapest, but the badge keeps showing a number that isn't the pane's work.

## 5. Not covered

- Remote panes on hosts where the helper isn't installed or the user declined it (`helper_consent`): the tooltip shows the connection and says the host can't be measured without the helper, linking to the Remotes pane.
- macOS hosts: the helper's macOS reader measures the user's own processes; phase 1's environment match needs `sysctl KERN_PROCARGS2` there instead of `/proc`. Phase 2 (exact PID) works unchanged.
- GPU, disk and network per pane.

## 6. Tests

- Helper: `PaneProcs` finds a child process started with a marker environment and its descendants, and not a sibling without it; `STATS` on a session the daemon spawned sums the shell's tree; an unknown session or block id is an error, not zeros.
- srv: `blockstats.detail` for a `wsl://` and a durable pane asks the helper (a fake helper connection), and falls back to the connection line on an older helper.
- Frontend: the tooltip's remote layout (host, address, "on <host>" figures, the host-totals variant for plain SSH).
- By hand: a WSL pane running a build shows the distro's processes; a durable pane on a Linux host shows its shell's tree; a plain pane to a stock host shows the host's totals.
