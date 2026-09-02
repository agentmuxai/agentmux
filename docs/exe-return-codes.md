# AgentMux Executable Return Codes

## agentmux.exe / agentmux-launcher (Windows entry point)

On Windows, the executable users actually launch is `agentmux.exe` — `scripts/package-portable.sh` builds it by copying `agentmux-launcher.exe`, and `packaging/windows/agentmux.iss` installs it under that name. The launcher owns `agentmux-srv`'s lifecycle directly and spawns `agentmux-cef` itself (see the Architecture section of `CLAUDE.md`).

This table is not a small fixed enum — on Windows, `supervisor/windows.rs` can also **pass through the CEF host's or `agentmux-srv`'s own raw OS exit code** once a restart/recovery budget is exhausted (`break code`, e.g. after repeated abnormal host exits or repeated system-OOM restarts) or a relaunch attempt itself fails to spawn. Codes below are the launcher's own, not inherited ones.

| Exit Code | Description |
|-----------|-------------|
| **0** | Clean exit — the launcher supervisor finished normally (host exited, a `--diag` subcommand succeeded, or on macOS/Linux, `second_instance.rs` detected a running peer and exited cleanly as the second instance) |
| **1** | Fatal startup/supervision error — causes include: the supervisor thread panicking, the runtime being misconfigured (host binary resolves to the launcher itself), the CEF host binary not found in `runtime/`, or a `--diag sagas` failure |
| **2** | Fatal IPC setup failure unrelated to `--diag`: on Windows, `supervisor/windows.rs` failing to bind/forward the named pipe (also used for the `--diag` CLI usage-error case: missing or unknown topic, known topics `wrr`/`srv`/`sagas`); on macOS/Linux, `second_instance.rs::bind_socket_with_recovery` failing to bind/recover the IPC Unix socket (initial bind failure, lockfile open failure, `flock` failure, post-lock or post-recovery rebind failure) |
| **86** | Windows-only: `TEARDOWN_BACKSTOP_EXIT_CODE` (`supervisor/windows.rs`) — the CEF host wedged with zero user windows open and stopped responding to UI-thread liveness probes past the grace period; the launcher force-terminates the job and exits with this distinct code so the failure is unambiguous in log/exit-code triage |
| *(other)* | Windows-only: the host's or `agentmux-srv`'s own raw exit code, passed through after `supervisor/windows.rs` gives up retrying that failure class (host/srv restart budget exhausted, OOM-recovery budget exhausted, or a relaunch attempt itself failed to spawn) |

## agentmux-cef (Desktop Host)

| Exit Code | Description |
|-----------|-------------|
| **0** | Clean exit — application closed normally (e.g., user quit via tray icon or window close) |

`agentmux-cef` is not spawned directly by the user on Windows — `agentmux-launcher` launches it, per the table above. On macOS/Linux, `agentmux-cef` is invoked via the launcher too (see `CLAUDE.md`'s Architecture section) rather than run standalone.

## agentmux-srv (Backend Server)

| Exit Code | Description |
|-----------|-------------|
| **0** | Clean shutdown — server exited normally. This includes: version/help flag requested, signal-based shutdown (SIGTERM/SIGINT), lock file indicates another instance is running, or graceful stop via internal command |
| **1** | Fatal startup error — server failed to start. Causes include: lock file creation failure, database migration failure, HTTP/WebSocket server bind failure, or other unrecoverable initialization error |

## Windows Installer (Inno Setup)

The Windows installer moved from NSIS to **Inno Setup** (`packaging/windows/agentmux.iss`, driven by `scripts/package-installer.ps1`). It uses Inno Setup's own standard `Setup.exe` exit codes (not the NSIS table this doc previously listed) — see the [Inno Setup documentation](https://jrsoftware.org/ishelp/index.php?topic=setupexitcodes) for the authoritative list, since no custom exit-code handling is defined in `agentmux.iss`. Standard silent-install flags apply: `/SILENT`, `/VERYSILENT`.

## Notes

- On Windows, `agentmux-launcher` auto-spawns `agentmux-srv` as a sidecar process (owning its lifecycle directly, rather than `agentmux-cef` spawning it) and also spawns `agentmux-cef` itself. If the backend exits with code 1, the application will not function correctly.
- Child processes running inside terminal panes (shells, commands) have their own exit codes which are tracked internally but do not affect the application's exit code.
- `wsh` (the old shell-integration binary) has been retired — see `docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md`. There is no binary to deploy or document return codes for anymore.
