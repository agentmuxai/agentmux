# The terminal stats badge on SSH and WSL panes: the remote shell's figures, not the local client's

**Date:** 2026-10-12
**Author:** Loap.
**Status:** implemented. Phase 0 shipped with the badge tooltip (#4776, `docs/reports/REPORT_TERMINAL_STATS_BADGE_MEMORY_2026_10_12.md` §5); phases 1-3 are built as described below, with the owner's choice of §4 (a).
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
- For a remote pane it names the connection and its machine: for SSH, the host `ssh -G` resolves the destination to (an alias's `HostName`) and its IP address from a DNS lookup; for WSL, the distribution and its address on the interface its default route uses, read from `/proc/net/route` and `/proc/net/fib_trie` inside it (BusyBox's `hostname` has no `-I`). Both are cached for 5 minutes (`backend/remote/address.rs`).
- It says the figures are this computer's `ssh` (or `wsl.exe`), not the remote shell's, until phases 1-3 replace them.

### Phase 1: the helper measures a pane's processes (built)

1. **Helper** (`crates/remote`): a request `PaneProcs { block_id, session }` (file protocol 3). `procs::Sampler::pane` finds the pane's processes from the same measurement `Procs` uses, two ways:
   - **By session:** when `session` names a durable session whose shell is running, the tree under that shell (exact).
   - **By marker:** otherwise every process whose environment holds `AGENTMUX_BLOCKID=<block_id>` (Linux: `/proc/<pid>/environ`, the user's own processes; read once per process and remembered while it runs), and what those started.

   It answers with the process count, names (the earliest-started root first), that root's start, CPU since the previous request on this connection, resident and private memory, at most 64 processes like the local badge, and the host's own totals (cores, CPU, private memory, process count). Neither way finding anything is `found: none` with the totals only.
2. **WSL:** the pane's `wsl.exe` already forwards `AGENTMUX_BLOCKID` into the distro (`remote/wsl.rs`), so its shell and everything it starts carry it. srv asks the helper `tower_remote` runs inside the distribution.

Matching by environment, not by tree from a PID, because the distro's PIDs aren't visible from Windows. A process that clears its environment (`env -i`, some daemons) drops out of the count.

### Phase 2: durable SSH panes (built)

1. **Daemon:** a control command `PID <session>`, answered with the session's shell PID or `none`. A daemon older than this answers it as a bad request, and the pane reads as `found: none`.
2. **`serve --stdio`** asks the daemon over its socket (`attach::session_pid`, 2 s limit) when a `PaneProcs` names a session, on the connection the file browser and Tower already use: no new `ssh` per sample.
3. **srv:** `blockstats.detail` sends the pane's `remote:session_id` with its id, so a durable pane is measured by its session and gets the badge it lacked.

### Phase 3: plain SSH panes (built)

- `ssh_pane::command` adds `AGENTMUX_BLOCKID=<pane>` to the `SetEnv` it already sends (`ssh::set_pane_env`; only an id of letters, digits and `-`). Where the host's sshd accepts `AGENTMUX_*` (`AcceptEnv`), phase 1's marker finds the pane's shell. Most hosts drop it.
- Otherwise the badge shows **the host's** figures, prefixed "host" and in italics, and the tooltip says the host can't tell this pane's processes from the rest and that a durable pane, or a server accepting AgentMux's variables, shows the pane's.

A rewrite of the plain pane's remote command (`exec env AGENTMUX_BLOCKID=… $SHELL -l`) would mark the shell everywhere, but `ssh::launch` deliberately sends no remote command for a plain interactive pane (a host may not run a POSIX shell, and a login shell started that way differs from sshd's), so it is out of scope.

### How srv reaches the machine

- **WSL:** `tower_remote::wsl_helper`, the Linux helper this package ships, run inside the distribution.
- **SSH:** `files::connect_quiet`: the open helper connection, or a new one only on a host the helper is known to be on (`helper_hosts`), with no install, no consent question and no ssh prompt. The badge asks every 2 s on its own, so it must never prompt; a pane's shared ssh connection usually makes a prompt unneeded anyway.
- A machine that can't be asked, or doesn't answer within 5 s, is `found: unavailable` with the reason; the badge shows "–" and the tooltip says why.
- Each answer's peak (the highest resident memory for that pane's shell, keyed by its start) is kept in srv and forgotten 10 minutes after the pane was last asked about.

## 4. What the badge shows on a remote pane (decided: (a))

The badge updates every second from a local snapshot that costs nothing extra; a remote sample costs a round trip through the helper and a process snapshot on the host (about 20 ms of the host's CPU).

- **(a) Chosen by the owner, 2026-10-13.** The badge shows the remote figures, asked every 2 s and only while the pane is visible (not in a hidden tab or a minimized window), as Tower samples. Built with one change: a plain SSH pane whose processes can't be told apart shows the host's figures, marked "host", rather than hiding the badge, so the tooltip (and the connection's name and address) stays reachable.
- **(b)** The badge stays local (the client process), and only the tooltip, while open, shows the remote figures. Cheapest, but the badge keeps showing a number that isn't the pane's work.

## 5. Not covered

- Remote panes on hosts where the helper isn't installed or the user declined it (`helper_consent`): the badge reads "–" and the tooltip says the helper isn't there.
- macOS hosts: the helper's macOS reader measures the user's own processes; the marker needs `sysctl KERN_PROCARGS2` there instead of `/proc`, so a WSL-style or plain SSH pane on a Mac shows the host's figures. A durable pane (exact PID) works.
- GPU, disk and network per pane.

## 6. Tests

- Helper: the protocol round-trips both `PaneFrame` shapes; `pane_frame` sums roots and their trees, names the shell first, caps the tree, and with no roots gives the host's totals only; the marker is read from an environment; on Linux, a child started with the marker is found with its children and an unmarked process isn't; a session's shell is found by its PID; end to end (Linux), `serve --stdio` asks a real daemon for a durable session's shell and sums it and what it started.
- srv: a helper answer becomes the badge's figures and peak (a new shell starts over); an SSH pane's machine answers through an in-process helper; a host without the helper is `unavailable`, not an error; `set_pane_env` adds the id and refuses odd ones.
- Frontend: a remote badge asks without being hovered and shows the remote shell's figures; the host-totals variant is marked; an unavailable machine says why; local panes are unchanged.
- By hand (dev build, Windows): a WSL pane's badge and tooltip show the distro's shell.
