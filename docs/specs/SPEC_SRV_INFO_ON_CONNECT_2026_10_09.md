# SPEC: srv describes itself on every connect (`srvinfo`)

**Date:** 2026-10-09
**Status:** active — slice 1 shipped in #4517 (srv sends `srvinfo`; the reload notice). Slice 2 remains (the UI reads srv's machine from it; six host commands deleted).
**Author:** Agent4
**Related:** `SPEC_HOST_API_SEAM_2026_09_26.md` (the frontend reaches its host only through `AppApi`), `SPEC_SRV_HEADLESS_MODE_2026_09_26.md` (srv without a desktop host).

## 1. Problem

1. **The UI learns about srv's machine from the desktop host.** At startup `initCefApi()` asks the host for the user name, host name, platform and the AgentMux home directory, among others. These describe the machine srv runs on: the UI shows the names, and builds paths from the home directory that it sends back to srv (an agent's working directory, `GH_CONFIG_DIR`, the widgets file). They are right only because the desktop host and srv run on the same machine, as the same user. A UI attached to a headless srv has no host to ask, and its host's machine wouldn't be srv's anyway.
2. **Nothing notices when the UI and srv are different versions.** They ship together, but an update can replace one under the other (srv restarted after an update while a window stayed open). The UI then talks to a srv it wasn't built for, with no sign of it.

## 2. Design

**srv sends one `srvinfo` event on every WebSocket connect**, right after the existing `config` event, on the same `eventrecv` path:

```json
{ "version": "0.59.16", "platform": "linux", "userName": "alex",
  "hostName": "workstation", "homeDir": "/home/alex/.agentmux" }
```

| Field | Source in srv | Same as the desktop host's |
|---|---|---|
| `version` | `state.version` (`CARGO_PKG_VERSION`) | the app version both are built at |
| `platform` | `agentmux_common::platform_name::platform_name()`: Node's names (`darwin`, `win32`, `linux`) | the host's `get_platform` now uses the same function (it had two copies of the mapping) |
| `userName` | `whoami::username()` | the same call, same OS user |
| `hostName` | `state.hostname` (`whoami::fallible::hostname()`, `"unknown"` on error) | the same expression |
| `homeDir` | `agentmux_root()`, captured first thing in `main` (`srv_info::capture_home_dir`) | the host's `DataPaths::from_env().home_dir`, which is `agentmux_root()` too |

**Why `homeDir` is captured at the start.** Startup sets `AGENTMUX_DATA_HOME` to the data directory (`bootstrap/stores.rs`), and `agentmux_root()` reads that variable, so after startup the same resolver answers with the data directory instead of the root.

**Why an event and not an endpoint.** The UI already opens the WebSocket at startup, and reconnects after srv restarts. Riding on that connect costs no request, and a reconnect to a different srv (an update) refreshes the facts and the version check together.

**The UI side** (`frontend/app/store/srv-info.ts`):
- **Caught as it arrives, not through a subscription.** srv sends `srvinfo` the moment the socket opens, which can be before the UI has subscribed to any event (`initGlobalEventSubs` runs after an HTTP call), and the event bus drops an event nobody subscribes to. So `initWshrpc`'s message handler passes every message to `noteSrvInfoMessage` before routing it.
- `srvInfo()` holds the last report.
- `UI_VERSION` is the UI's own version, stamped at build time from `package.json` (`vite.config.ts` `define: __AGENTMUX_VERSION__`).
- `versionSkew()` is srv's version when it differs from `UI_VERSION`. The status bar then shows "Reload for <version>" (`VersionSkewStatus`); clicking calls `AppApi.reloadWindow()`. The desktop host reloads through the URL that carries its IPC credentials (`reloadKeepingHostCredentials`, the same path the startup recovery uses), because a plain reload drops them and can strand the window.

**Why compare app versions, not a protocol number.** A protocol number only helps if every breaking change remembers to bump it. The UI and srv are always built from the same tree, so equal versions are the real compatibility contract, and a reload is the fix for any difference.

## 3. Slices

| Slice | Content |
|---|---|
| **1** | srv sends `srvinfo`; the shared `platform_name()`; the UI's `srv-info` store; the reload notice |
| **2** | The UI reads srv's machine from `srvInfo()`: user and host names (status bar, empty tab, the "Local" connection), the home directory (`agentmuxHome()`), and srv's platform where the code means srv's OS rather than the UI's (the ConPTY resize workaround, toolchain commands, file-name rules, the macOS protected-folder gate, keychain notices). The UI's own OS keeps coming from the host (key bindings, window chrome). The host commands this replaces (`get_user_name`, `get_host_name`, `get_user_home_dir`) are deleted, with `get_data_dir`, `get_config_dir` and `get_docsite_url`, which nothing reads. Startup then makes six fewer host requests |

## 4. Performance

Slice 1 adds one small WebSocket frame per connect and no request. Slice 2 removes six host requests from startup's parallel batch; the facts arrive with the connect the UI makes anyway. Slice 2 measures startup with the existing `benchMark` marks (`invoke-batch-start`, `invoke-batch-done`) before and after.

## 5. Testing

- srv: `srv_info` names the machine and version; `srv_info_event` carries it on the `eventrecv` path. `platform_name` maps Rust's OS names.
- UI: the store records a report, ignores one without a version, and reports skew only for a different version; `VersionSkewStatus` shows only on skew.
