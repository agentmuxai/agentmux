# SPEC: Host API seam — the frontend reaches its host only through `AppApi`, and asks what the host can do

**Date:** 2026-09-26
**Status:** active — Slice 1 (capabilities, test host, boundary ratchet) ships in PR #3878 and slice 2 (Settings) in PR #3879. Slices 3–5 remain (§5).
**Author:** Maricon

---

## 1. Problem

The frontend runs inside the CEF desktop host, and it reaches that host in two ways:

1. **Through the seam:** `getApi()` returns `window.api`, an `AppApi` (`frontend/types/custom.d.ts`) built by `buildCefApi()` (`frontend/util/cef-api.ts`). That is 109 methods and 218 call sites in 76 files.
2. **Around it:**
   - 95 direct `invokeCommand` calls in 39 files, 27 of them `browser_pane_*`, plus dynamic `import("@/app/platform/ipc")` in 7 more files;
   - 21 direct `listenEvent` calls;
   - reads of the `__AGENTMUX_IPC_*` globals.

Two consequences:

- **The frontend cannot run on any host but CEF.** A test harness, or a plain browser pointed at srv, has to fake the IPC transport itself (`app/platform/ipc.ts`), not just an object with the right methods. `detectHost()` already has a `"browser"` branch, and today it throws.
- **There is no way to ask what the host can do.** UI for a desktop-only feature (tear-off, native browser panes, the updater, start at login) calls the host and fails, instead of hiding itself.

About 80–85% of the UI already depends only on `agentmux-srv` over WebSocket and HTTP. The host coupling is small but scattered, and this spec gathers it into one place.

## 2. Goals

1. `AppApi` is the **only** way UI code reaches the host. Only the CEF host implementation (`util/cef-api.ts`, `cef-init.ts`, `app/platform/ipc.ts`, `app/init/host-detect.ts`) talks IPC.
2. The host **reports its capabilities** (`HostCaps`). UI for a missing capability hides itself.
3. Tests can run UI against a **stand-in host** with any capability set, without hand-building 110 methods.
4. The boundary is **enforced by a test**, not by convention.

**Non-goals:** no behavior change in the desktop app, where CEF reports every capability. No new host is added by this spec.

## 3. Design

### 3.1 `HostCaps`

Declared next to `AppApi` in `types/custom.d.ts`, and returned by `AppApi.getHostCaps()`:

| Capability | Covers |
|---|---|
| `multiWindow` | Open, focus and list native windows |
| `tearOff` | Tab/pane tear-off; cross-window drag |
| `nativeBrowserPane` | Browser panes (CEF child browsers) |
| `nativeDialogs` | Native open/save dialogs |
| `updater` | Update check and install |
| `autostart` | Start at login |
| `tray` | Tray icon and menu |
| `localCliInstall` | Detect and install provider CLIs on this machine |
| `windowTransparency` | Transparency and opacity |
| `nativeWindowChrome` | Custom title bar and window controls, window drag |

- **CEF** reports all of them (`CEF_HOST_CAPS`, `frontend/app/host/host-caps.ts`).
- `NO_HOST_CAPS` is the empty set.
- `hostHas(cap)` is the one-line check UI code uses.

New capabilities are added when a slice needs one, not speculatively.

### 3.2 Test host

`makeTestHostApi(overrides, caps)` (`frontend/app/host/test-host.ts`) returns an `AppApi`. Methods the test doesn't override are no-ops returning `undefined`, and the capabilities default to `NO_HOST_CAPS`. It is for tests only.

### 3.3 Boundary ratchet

`frontend/app/host/host-boundary.test.ts` scans `frontend/` for files that import `app/platform/ipc`, statically or with a dynamic `import()`, or read `__AGENTMUX_IPC_*`. The result must equal the **seam** (the CEF implementation files) plus a **PENDING** list of files that still bypass it.

- A **new** bypass fails the test.
- A listed file that **no longer** bypasses also fails the test, until it's removed from the list. The list can only shrink.

It runs in the existing vitest CI job; no new CI step is needed.

### 3.4 Moving a call behind the seam

For each direct call:
1. Add an `AppApi` method named for what it does, not for the IPC command.
2. Implement it in `buildCefApi()`.
3. Call it through `getApi()`.
4. If the UI around it only makes sense with a capability, guard it with `hostHas(...)`.
5. Remove the file from PENDING when its last direct call is gone.

## 4. Why not a larger restructure

Moving the CEF implementation into `frontend/app/host/cef/`, or splitting the frontend into packages, would touch hundreds of files and every open PR. The ratchet gives the same guarantee, one file at a time. The files can move once PENDING is empty.

## 5. Slices

| Slice | Content | PENDING after |
|---|---|---|
| **1** | `HostCaps` + `getHostCaps()`, test host, boundary ratchet, this spec | 47 |
| **2** | Settings: `getAutostartStatus()` and `openSettingsFileInEditor()` on `AppApi` (also used by the command palette); the System tray rows guarded by `tray` and `autostart` | 44 |
| 3 | Browser panes: the 27 `browser_pane_*` calls behind a browser-pane group on `AppApi`; pane type guarded by `nativeBrowserPane` | ↓ |
| 4 | Window drag, position and focus, tear-off, floating panes (`nativeWindowChrome`, `tearOff`, `multiWindow`) | ↓ |
| 5 | The remainder: logging, clipboard, drag-and-drop, approvals, startup (`bootstrap.ts`, `app-init.ts`) | 0 |

## 6. Testing

- Slice 1: `host-caps.test.ts` covers the capability sets, `hostHas` and the test host. `host-boundary.test.ts` is the ratchet. A mutation check (adding a new file that imports `platform/ipc`) makes the ratchet fail.
- Each later slice: the ratchet shrinks, plus a component test rendering the guarded UI against `makeTestHostApi` with the capability off.
