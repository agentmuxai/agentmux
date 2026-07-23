# AgentMux Executable Return Codes

## agentmux-cef (Desktop Host)

| Exit Code | Description |
|-----------|-------------|
| **0** | Clean exit — application closed normally (e.g., user quit via tray icon or window close) |

On Windows, `agentmux-launcher` is what actually launches `agentmux-cef` (and, in turn, owns `agentmux-srv`'s lifecycle) — see the Architecture section of `CLAUDE.md`. `agentmux-cef` is not spawned directly by the user on that platform.

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
- `wsh` (the old shell-integration binary) has been retired — see `specs/SPEC_RETIRE_WSH_2026_04_12.md`. There is no binary to deploy or document return codes for anymore.
