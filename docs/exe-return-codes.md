# AgentMux Executable Return Codes

## agentmux.exe (Desktop Application)

| Exit Code | Description |
|-----------|-------------|
| **0** | Clean exit — application closed normally (e.g., user quit via tray icon or window close) |

## agentmuxsrv-rs.exe (Backend Server)

| Exit Code | Description |
|-----------|-------------|
| **0** | Clean shutdown — server exited normally. This includes: version/help flag requested, signal-based shutdown (SIGTERM/SIGINT), lock file indicates another instance is running, or graceful stop via internal command |
| **1** | Fatal startup error — server failed to start. Causes include: lock file creation failure, database migration failure, HTTP/WebSocket server bind failure, or other unrecoverable initialization error |

## wsh.exe (Shell Integration)

| Exit Code | Description |
|-----------|-------------|
| **0** | Command completed successfully |
| **1** | Command failed or invalid arguments |

## NSIS Installer (AgentMux_*_x64-setup.exe)

| Exit Code | Description |
|-----------|-------------|
| **0** | Installation completed successfully |
| **1** | Installation cancelled by user (clicked Cancel or closed installer) |
| **2** | Application already exists on the device (silent/passive mode only) |
| **3** | Another installation is already in progress |
| **4** | Insufficient disk space to complete installation |
| **5** | A restart is required to complete the install |
| **6** | Network failure (e.g., WebView2 download failed) |
| **7** | Package rejected during installation (binary missing after extraction) |

These codes are required for Microsoft Store compliance. The standard NSIS `/S` (silent) and `/P` (passive) flags are supported.

## Notes

- The desktop application (`agentmux.exe`) auto-spawns `agentmuxsrv-rs.exe` as a sidecar process. If the backend exits with code 1, the application will not function correctly.
- Child processes running inside terminal panes (shells, commands) have their own exit codes which are tracked internally but do not affect the application's exit code.
- On Windows, the NSIS installer (`AgentMux_*_x64-setup.exe`) supports the `/S` flag for silent installation.
