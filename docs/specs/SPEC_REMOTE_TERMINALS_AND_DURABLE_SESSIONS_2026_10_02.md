# SPEC: Remote terminals (SSH, WSL) and durable remote sessions — implementation plan

**Date:** 2026-10-02
**Status:** proposed — plan only; nothing implemented. Section 10 asks for decisions.
**Author:** AgentX (narko), at the owner's request ("lets get both remote terminals and durable sessions in, lets work first on that ... this would also need support across the 3 platforms")
**Affects:** `crates/srv` (blockcontroller/shell, a new `remote/` module, fs_ops, wconfig, server/service), a new crate `crates/remote` (the remote helper), `crates/cef` (none expected), `frontend/app` (term view, block frame, conntypeahead, Hangar, settings), `Taskfile.yml` and `.github/workflows` (new build targets)
**Builds on:** `docs/reports/REPORT_WAVETERM_FEATURE_GAP_2026_10_02.md` §2.10 (what Wave has), `docs/specs/SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md` (the `term` blockfile and offset replay a durable session plugs into), `docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md` (why the last remote helper was removed, and what not to repeat)

## 1. Goal

A terminal pane can belong to a **connection**: a WSL distribution or an SSH host. It looks and behaves like a local terminal (same pane, same scrollback, same themes), its files can be browsed and edited in Hangar and the editor, and with **durable sessions** its shell keeps running on the remote machine when the network drops, the laptop sleeps or AgentMux restarts, then reattaches with everything it missed.

On all three platforms AgentMux ships on. Specifically:

| | Windows client | macOS client | Linux client |
|---|---|---|---|
| WSL terminals | yes | n/a (no WSL) | n/a |
| SSH terminals | yes | yes | yes |
| Remote files (Hangar, editor) | yes | yes | yes |
| Durable sessions | yes | yes | yes |
| **Remote hosts** reached over SSH | Linux x86_64 and arm64, macOS arm64 and x86_64 from the first release of each phase; Windows (OpenSSH Server) as a later phase (§8, P6) | same | same |

"Across the 3 platforms" applies first to where AgentMux runs. Remote hosts are mostly Linux and macOS servers; a Windows server is supported later because its terminal and process model needs its own helper work (§6.5).

## 2. What exists today (verified on `main` at `9d206f9d7`)

- **Terminals are local only.** `ShellController` (`blockcontroller/shell/controller.rs`) opens a PTY with `portable-pty` (ConPTY on Windows) in `shell/lifecycle.rs`, streams output into the `term` blockfile with a `blockfile` event per append (`shell/file_ops.rs`), and takes input and resize over `controllerinput`. The shell dies with srv; scrollback survives through the `cache:term:full` snapshot plus the `term` delta from `ptyoffset` (`termwrap.ts`). Nothing reattaches to a running process.
- **Wave-era connection leftovers, all inert:** block meta `connection` (`META_KEY_CONNECTION`, default `"local"`; a change already forces a controller replace in `resync_controller`); `CONN_TYPE_SSH`/`CONN_TYPE_WSL` and a `ConnInterface` trait with no real implementation (`shellexec.rs`); `ConnKeywords` with `ssh:*`, `conn:*` and `cmd:initscript.*` fields and a `connections` map in `wconfig/types.rs`, read by nothing; `GetAllConnStatus` returning empty; and a frontend connection typeahead, `ConnStatusOverlay` and `ConnEnsure` calls (`conntypeahead.tsx`, `blockframe.tsx`) that call RPCs with **no backend handler**.
- **Files are local only.** `fs.*` RPCs and the editor read and write through `std::fs`; requests carry paths, no connection.
- **No SSH code, no ssh-config parser, no WSL detection** in the dependency tree or the source.
- **Packaging:** extra binaries (`agentmux-bashwrap`, `agentmux-mcp`) ship in `tools/bin`. CI builds Windows x86_64, Linux x86_64 (glibc) and macOS arm64. No musl/static build, no Linux arm64 binary, no macOS x86_64.
- **Patterns worth reusing:** the pure `update(state, event) -> (state, effects)` state machine in `persistent_resume.rs` (for reconnect), session-recovery meta flags (`session_recovery.rs`), and the `term` offset replay (for catching up after a reattach).

## 3. Decisions this plan rests on (recommended; see §10)

### 3.1 Use the system's OpenSSH, not an SSH library

Every client platform has an OpenSSH client: Windows 10 1809+ ships `C:\Windows\System32\OpenSSH\ssh.exe`, macOS and Linux have `ssh`. Spawning it (in a PTY, as the terminal's process) gives, for free and exactly as the user's own `ssh` behaves: `~/.ssh/config` including `Match` (which Wave's own parser cannot handle), `ProxyJump`/`ProxyCommand`, `Include`, the user's agent (OpenSSH agent pipe on Windows, `SSH_AUTH_SOCK` elsewhere), FIDO/hardware keys, certificates, Kerberos/GSSAPI, `known_hosts`, and every fix OpenSSH ships. An embedded library (`russh`) would mean reimplementing all of that in security-sensitive code AgentMux would then own.

Costs, and how they are handled:
- **No multiplexing on Windows.** Win32-OpenSSH does not support `ControlMaster`. Each terminal and the file channel are separate `ssh` processes. That is acceptable (one TCP connection each); on macOS and Linux a per-connection `ControlMaster` socket under the data dir cuts repeat logins to one.
- **Prompts.** Passwords, passphrases and host-key confirmations are delivered through `SSH_ASKPASS` with `SSH_ASKPASS_REQUIRE=force` pointing at a small AgentMux askpass shim (§5.3), so they appear as AgentMux dialogs (or come from the secret store) instead of text in the pane. Supported by OpenSSH 8.4+ on all three platforms; Windows' bundled OpenSSH is 8.x or 9.x on supported Windows versions. Verify the minimum at P2 and fall back to in-pane prompts below it.
- **Which `ssh`.** On Windows, prefer System32 OpenSSH over a Git-for-Windows `ssh` on PATH (their agents differ). Setting `ssh:binarypath` overrides it everywhere.

### 3.2 A remote helper, installed per host, is required for files and durability; not for a plain remote shell

A plain SSH terminal is `ssh -tt host` in a PTY and needs nothing installed remotely. Remote file browsing and durable sessions need a program on the remote side: `agentmux-remote` (§6). It is uploaded over SSH on first use (with the user's consent per host, §10.2), verified by hash, and versioned so a client and helper of different versions never talk past each other.

The retired `wsh` (`SPEC_RETIRE_WSH_2026_04_12.md`) was removed because nothing used it, not because the idea failed. Its lessons apply directly: one binary with a narrow job, a version handshake, explicit install and reinstall, a size budget, and tests that exercise the deploy path.

### 3.3 WSL needs no helper

WSL is a local VM. A WSL terminal is `wsl.exe -d <distro>` in a ConPTY. Its files are reachable from Windows at `\\wsl.localhost\<distro>\...`, so Hangar and the editor work through the existing local fs layer with a path mapping, and there is no network to drop, so durability does not apply (the distro's shell lives as long as the WSL VM; §7.6 covers a VM shutdown).

## 4. The connection model

- **Name.** A block's `connection` meta: `local` (default), `wsl://<distro>`, or an SSH target written as the user writes it for `ssh`: `host`, `user@host`, `user@host:port`, or a `Host` alias from `~/.ssh/config`.
- **Settings per connection** in `settings.json` under `connections.<name>`, reusing the existing `ConnKeywords` type, trimmed to what the system-ssh design needs. SSH options are passed as `-o Key=Value` only for keys the user set; everything else comes from their ssh config.
  - `term:theme`, `term:fontsize`, `term:fontfamily`: per-connection look (a red theme for production is the motivating case).
  - `cmd:env`, `cmd:initscript.{bash,zsh,fish,pwsh}`: environment and startup for shells on that connection.
  - `ssh:user`, `ssh:port`, `ssh:identityfile`, `ssh:proxyjump`, `ssh:binarypath`, `ssh:passwordsecretname` (with the Secrets work; until then a prompt).
  - `conn:helper` (`ask` default, `always`, `never`), `term:durable` (§7.1).
- **Listing.** The typeahead shows `local`, WSL distros, recent connections, connections configured in settings, and `Host` entries from `~/.ssh/config` (patterns without wildcards; `Include` followed one level). Listing is the only thing AgentMux parses ssh config for; connecting is always the system `ssh`.
- **Status.** One `ConnStatus` per connection, `connecting | connected | disconnected | error`, with the last error, published as an event the existing `ConnStatusOverlay` already renders. `GetAllConnStatus`, `ConnList`, `WslList`, `ConnEnsure`, `ConnConnect` and `ConnDisconnect` get real handlers matching the frontend's existing calls.

## 5. SSH and WSL terminals

### 5.1 Spawning

`ShellController` gains a launch-plan step that, for a non-local connection, builds the argv instead of a local shell:

- **WSL:** `wsl.exe -d <distro> --cd ~` (or `--cd <cmd:cwd>`); `cmd`/`cmd:args` become `wsl.exe -d <distro> -- <cmd>`. List distros with `wsl.exe --list --quiet` (UTF-16 output; decode it).
- **SSH, plain:** `ssh -tt [opts] <target> -- <remote login command>`, where the remote command starts the user's login shell with AgentMux's shell integration if it was installed (§5.4), or just the login shell. Options always set: `ServerAliveInterval=15`, `ServerAliveCountMax=3` (§7.5), `SetEnv TERM_PROGRAM=agentmux` where allowed, and on macOS/Linux the `ControlMaster`/`ControlPath`/`ControlPersist` trio.
- **SSH, durable:** `ssh -tt [opts] <target> -- ~/.agentmux-remote/bin/<ver>/agentmux-remote attach --session <id> --cols C --rows R --offset N` (§7).

Everything downstream is unchanged: the PTY's output goes to the `term` blockfile, input and resize flow in, the frontend does not know the difference. Resize reaches the remote side through SSH's window-change; for durable sessions the helper also gets it explicitly.

### 5.2 The pane

The block frame shows the connection (the existing connection chip), and its status: a remote pane that lost its connection says so in the pane rather than going quiet. Per-connection theme and font apply. A terminal can be opened on a connection from the typeahead, from a Hangar folder on that connection ("open terminal here"), and from `muxsh` (`muxsh term --conn host`).

### 5.3 Authentication

- Keys and agents work through the user's ssh setup with no AgentMux involvement.
- **Askpass bridge:** `SSH_ASKPASS` points at `agentmux-askpass` (a mode of `agentmux-bashwrap`, already shipped on all platforms). It calls srv over the authenticated local channel with the prompt text and the block id. srv answers from the secret store when `ssh:passwordsecretname` is set, otherwise shows a dialog in that pane (password field, or yes/no for a new host key, showing the fingerprint). A host-key mismatch is never auto-answered.
- Nothing typed into an askpass dialog is logged or stored unless the user ticks "remember" (which writes a named secret, once Secrets lands).

### 5.4 Shell integration on remote shells

Local shells get AgentMux's integration scripts (cwd via OSC 7, prompt marks, titles). On a remote host the scripts are installed with the helper (§6.2) and sourced by the remote login command. Without the helper, a remote shell is a plain shell: still fully usable, no cwd tracking. `muxsh` itself does not work remotely in this plan (it calls a localhost URL); reaching it through an SSH reverse tunnel is a later item (§9).

## 6. The remote helper, `agentmux-remote`

### 6.1 What it is

One small Rust binary (new crate `crates/remote`, target size under 4 MB stripped), with three jobs and nothing else:

1. **`serve --stdio`**: a JSON-lines RPC over stdin/stdout for file operations: stat, list (paged), read (ranged), write (atomic replace), mkdir, rename, delete, and a watch for the open folder. Run as `ssh host -- agentmux-remote serve --stdio`, one per connection, kept open.
2. **`daemon` and `attach`**: the durable session manager (§7).
3. **`version`**: prints its protocol version and build hash for the handshake.

It holds no credentials, opens no network port, and talks only to its own SSH channel and a per-user Unix socket.

### 6.2 Install and versioning

- Location: `~/.agentmux-remote/bin/<version>/agentmux-remote`, plus `~/.agentmux-remote/shell/` for integration scripts. A versioned path means two AgentMux versions can use one host without fighting, and an upgrade never replaces a binary a running daemon is executing.
- Detect platform with `uname -sm`, pick the matching bundled build (§8.6), upload over the existing SSH connection (`ssh host -- 'umask 077; mkdir -p ... && cat > ....tmp' < binary`, then hash check, `chmod 700`, rename into place). No `scp`/`sftp` dependency on the remote.
- Handshake on every `serve`/`attach`: protocol version must match, or the client installs its own version alongside.
- **Consent:** first use on a host asks, in the pane, "Install AgentMux's helper on <host> for file browsing and durable sessions? (about 3 MB in ~/.agentmux-remote)", with "always for this host" and "never for this host" (`conn:helper`).
- **Removal:** a "Remove helper from <host>" action and `muxsh conn uninstall <host>` delete `~/.agentmux-remote` after stopping its daemon.
- Old versions are pruned on the host when no running session uses them.

### 6.3 Remote files

`fs_ops` gets a backend trait, `FsBackend`, with `Local` (today's code), `Wsl` (path mapping to `\\wsl.localhost\<distro>`, then `Local`) and `Remote` (the helper over its stdio channel). Every `fs.*` request and the editor's read and write gain an optional `connection`. Hangar shows connections under Places and opens a folder on a connection; the editor saves back through the same backend. Copy and move between a remote host and this machine use the helper's ranged read and atomic write, with the existing job and progress machinery.

### 6.4 Supported remote platforms

Linux x86_64 and aarch64 (static musl builds, so any distro and glibc version works) and macOS arm64 and x86_64. Anything else gets a plain SSH terminal and a clear "no helper for this platform" note in place of files and durability.

### 6.5 Windows as a remote host (P6)

A Windows machine running OpenSSH Server can host plain SSH terminals from P2 (its default shell is cmd or PowerShell; ssh gives it a ConPTY). Files and durable sessions there need a Windows build of the helper: ConPTY for the daemon's terminals, a named pipe in place of the Unix socket, and `%USERPROFILE%\.agentmux-remote` for install. Same protocol, platform-specific internals.

## 7. Durable sessions

### 7.1 Turning it on

`term:durable` on the block, then on the connection, then global; default **on** for SSH connections whose host has the helper (recommended, §10.4). Not applicable to local and WSL panes. The pane menu has "Make durable" / "Stop keeping alive".

### 7.2 How it works

- **The daemon.** `agentmux-remote daemon` runs per remote user, started on demand by `attach` if not running (detached: new session, stdio closed, survives the SSH channel). It listens on `~/.agentmux-remote/run/daemon.sock` in a `0700` directory; only the same user can connect, as with tmux.
- **Sessions.** Each session is a PTY running the user's login shell (with integration), keyed by an id AgentMux generates and stores in the block meta (`remote:session_id`). The daemon keeps each session's output in an on-disk ring log (default 8 MB, configurable) with a monotonic byte offset.
- **Attach.** `agentmux-remote attach --session <id> --offset N` connects to the daemon (starting it, and creating the session, if needed), replays everything after byte `N` that is still in the ring, then streams live output and forwards input and resize. If `N` has fallen out of the ring, it replays from the oldest byte and says how much was lost, so the pane can mark the gap.
- **Offsets on the AgentMux side.** srv records the last remote offset it appended to the `term` blockfile in block meta (`remote:offset`), alongside the existing `ptyoffset`. Reattach asks for exactly what is missing, and the frontend's existing offset replay shows it. Nothing new is needed in the terminal view.

### 7.3 What survives

| Event | Result |
|---|---|
| Network drop, VPN change, Wi-Fi switch | Pane shows "Disconnected — reconnecting". Shell keeps running remotely. Reconnects with backoff and catches up. |
| Laptop sleep | Same; reconnects on wake. |
| AgentMux restart, or srv crash | On start, every block with `remote:session_id` reattaches (lazily when its tab is shown, eagerly for the visible tab), using the same pattern as the persistent controller's eager resume. |
| Remote host reboot | Session is gone. Pane shows "Session ended on <host> (host restarted)" and offers a new session. |
| User closes the pane | srv tells the daemon to end the session (kill its process group), unless the user chose "Detach" from the pane menu, which leaves it running and listed. |

### 7.4 Reconnect state machine

A pure `update(state, event) -> (state, effects)` reducer in the style of `persistent_resume.rs`, so every transition is unit-tested without a network:

`Starting → Attached ⇄ Detached(reconnecting, attempt n) → Attached` and `→ Ended(reason)`; `AwaitingAuth` when a reconnect needs a prompt. Backoff 1 s, 2 s, 5 s, 10 s, then every 30 s, never giving up while the pane is open (the mDNS lesson in `SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` §8.3 applies: a cap on retries is how a recoverable pane becomes a dead one). Status drives a small indicator on the pane (attached, reconnecting, ended) and the Swarm row if an agent is involved.

### 7.5 Keepalive and stalled detection

`ServerAliveInterval=15`, `ServerAliveCountMax=3` make ssh exit within about 45 s of a dead link instead of hanging for the TCP timeout; the exit triggers reconnect. Additionally, srv sends a helper-level ping every 15 s over durable sessions and treats 30 s without any byte as "stalled": it shows the state immediately and kills and restarts the `ssh` process rather than waiting for ssh's own timer.

### 7.6 Housekeeping on the remote host

The daemon ends sessions whose shell exited, keeps detached-but-running sessions until an idle limit (default 7 days without an attach, configurable, never while a process in the session is using CPU), and exits when it has no sessions. `muxsh conn sessions <host>` lists them; the pane menu "Sessions on <host>" lets the user reattach to or end an orphan.

## 8. Delivery plan

Each phase is one or more PRs, each with its own tests and docs. Client-side work lands for Windows, macOS and Linux in the same PR, never one platform at a time.

| Phase | Scope | Verified on |
|---|---|---|
| **P0: groundwork** | Remove the dead `ConnInterface`/`ShellProc`/`MockConn` scaffolding and unused `REMOTE_*` constants; keep `META_KEY_CONNECTION`. Add the connection model (§4), the `ConnStatus` events and real handlers for `ConnList`, `GetAllConnStatus`, `ConnEnsure`, `ConnConnect`, `ConnDisconnect` (local-only results at first), and the launch-plan seam in `ShellController`. No user-visible change. | unit tests, all three CI platforms |
| **P1: WSL terminals** | `WslList`, `wsl.exe` launch plans, the typeahead section, per-connection settings, Hangar and editor through `\\wsl.localhost` mapping. | narko (Windows 11, WSL Ubuntu) |
| **P2: SSH terminals** | ssh launch plans, `ssh:binarypath` selection, `ControlMaster` on macOS/Linux, the askpass bridge and dialogs (password, passphrase, host key), connection status in the pane, ssh-config host listing, per-connection theme and env. | CI: an OpenSSH server container on the Linux runner, connected from Linux (and from Windows and macOS runners where they can reach it); manual: narko → a Linux box, starpower (macOS) → the same box |
| **P3: the helper and remote files** | `crates/remote` with `serve --stdio` and `version`; static builds (§8.6); install, consent, hash check, versioning, removal; `FsBackend` and `connection` on fs and editor requests; Hangar connections. | CI container with deploy, list, read, write, rename, delete, upgrade and downgrade; manual on a real Linux and a macOS host |
| **P4: durable sessions** | `daemon`/`attach`, the ring log, offsets, the reconnect reducer, keepalive and stall detection, eager reattach on restart, pane states, housekeeping, `muxsh conn sessions`. | CI: kill and restore the ssh process mid-output and assert no byte is lost or duplicated; restart srv and assert reattach; manual: Wi-Fi off and on, laptop sleep, app restart, on all three client platforms |
| **P5: polish** | Remote shell integration (cwd, prompt marks), "open terminal here" from Hangar, connection icons in the tab bar, per-connection colour accent | — |
| **P6: Windows as a remote host** | Windows build of the helper (ConPTY, named pipe), install paths, files and durable sessions on Windows OpenSSH Server | a Windows VM with OpenSSH Server (the gamerlove VM) |

### 8.6 Builds and packaging

- New targets for the helper only: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (built with `cargo-zigbuild` or `cross` on the Linux runner), `aarch64-apple-darwin` and `x86_64-apple-darwin` (on the macOS runner); `x86_64-pc-windows-msvc` from P6.
- Every desktop package (Windows zip and installer, macOS app, Linux AppImage, deb, rpm) carries all helper builds under `tools/remote/<target>/agentmux-remote`, since any client may connect to any host. Budget: about 4 × 3 MB compressed; measured at P3 and recorded in the spec.
- A CI check that each packaged helper runs `version` (in QEMU for arm64) so a broken cross-build cannot ship.

## 9. Not in this plan

- Running **agent panes** on a remote host (the agent CLI on the server, the pane here). It builds on P2 to P4 but has its own questions (credentials on the remote, the provider CLI install there); its own spec once P4 lands.
- `muxsh` and the MCP tools reaching back from a remote shell (would need an SSH reverse tunnel to srv's local port).
- AWS SSM and other non-SSH transports (the typeahead's `ConnListAWS` call is removed in P0 unless the owner wants it).
- Sharing one remote session between two AgentMux windows at once.

## 10. Decisions for the owner

1. **System OpenSSH vs an embedded SSH library.** Recommended: system OpenSSH (§3.1).
2. **Installing the helper on remote hosts.** Recommended: ask once per host, with "always" and "never" remembered (§6.2).
3. **Remote platforms.** Recommended: Linux x86_64/arm64 and macOS from P3; Windows hosts in P6.
4. **Durable by default.** Recommended: on for SSH connections that have the helper, off otherwise; Wave makes it opt-in. On by default is the point of the feature for long agent and build jobs, and the housekeeping in §7.6 bounds its cost on servers.
5. **Where connection settings live.** Recommended: `settings.json` under `connections` (the type exists), not a separate `connections.json`.

## 11. Risks

- **OpenSSH version spread on Windows.** Older Windows builds ship OpenSSH without `SSH_ASKPASS_REQUIRE`; in-pane prompts are the fallback, and the minimum is checked at P2.
- **Deploying a binary to servers.** Some environments forbid it (read-only home, `noexec` home). The helper falls back to `$XDG_RUNTIME_DIR` or refuses clearly; plain SSH terminals are unaffected.
- **Package size.** Four helper builds in every package; measured and budgeted at P3.
- **Ring log size vs scrollback.** A session that prints more than the ring while detached loses the overflow; the pane says how much, never silently.
- **Two clients, one session.** Explicitly unsupported (§9); the daemon refuses a second attach to a session and offers "take over".
