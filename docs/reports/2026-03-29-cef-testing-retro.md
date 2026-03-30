# CEF Integration Testing Retro — 2026-03-29

## Session Summary

First end-to-end test of agentx's CEF integration branch (`agentx/cef-integration`,
PR #253). Built the CEF host crate, bundled the runtime, and launched against the
Vite dev server. Found and fixed four issues before reaching a working state.

---

## GPU Errors — Red Herring

```
ERROR:components\viz\service\main\viz_main_impl.cc:189
  Exiting GPU process due to errors during initialization

ERROR:gpu\ipc\service\gpu_channel_manager.cc:919
  ContextResult::kFatalFailure: Failed to create shared context for virtualization.
```

**These errors appear on EVERY launch** (with and without `--disable-gpu`) and are
**non-fatal**. CEF's Chromium retries GPU initialization 2-3 times, logs errors
for each failed attempt, then falls back to software compositing or a simpler GPU
path. The app renders correctly in both cases.

We initially assumed the GPU errors were causing the blank grey window. This led
to passing `--disable-gpu` which was unnecessary. The actual cause was the
`set_zoom_factor` IPC deadlock (see below).

**Lesson:** CEF GPU errors on Windows are common noise — check the frontend init
logs before blaming the GPU. The `viz_main_impl` and `gpu_channel_manager` errors
are Chromium internal retries, not application failures.

---

## Bug 1: `set_zoom_factor` Deadlocks CEF Message Loop

**Symptom:** Blank grey window. Frontend logs stopped at "Init Bare - Host app
mode: true". No further JS execution — `setTimeout`, `Promise.race`, and all
subsequent IPC calls frozen.

**Root cause:** `agentmux-cef/src/commands/window.rs` `set_zoom_factor()` calls
`host.set_zoom_level()` directly from the axum IPC handler thread. CEF browser
host methods must be called on the CEF UI thread. Calling from another thread
deadlocks the CEF message loop, which freezes the JS event loop (no timers, no
promises, no microtasks).

**Why it was hard to find:** The `setZoomFactor` call in `wave.ts` is
fire-and-forget (`invokeCommand(...).catch(console.error)`). The JS side returns
immediately. But the IPC HTTP request blocks on the Rust side, and subsequent
`sendLog` IPC calls also go to the same axum server — they queue behind the
deadlocked handler. So ALL frontend logging stops, making it look like JS itself
crashed.

**Debugging approach:**
1. Added granular `sendLog` calls between each line in `initBare()`
2. Saw "setZoomFactor" as the last log before silence
3. Deferred to `setTimeout(..., 0)` — still deadlocked (the setTimeout fires,
   calls setZoomFactor, deadlocks the event loop)
4. Skipped setZoomFactor in CEF mode — everything worked

**Fix applied:** Guard `setZoomFactor(1.0)` with `isTauriHost()` in `wave.ts`.

**Proper fix (TODO):** CEF `set_zoom_factor` handler must post to the UI thread
via `CefPostTask(TID_UI, ...)` instead of calling `host.set_zoom_level()` directly.
This pattern applies to ALL CEF browser/host method calls from IPC handlers.

**Files:** `frontend/wave.ts`

---

## Bug 2: Tauri API Crashes in CEF Mode

**Symptom:** Opening a terminal pane throws:
```
TypeError: Cannot read properties of undefined (reading 'metadata')
    at getCurrentWindow → getCurrentWebview → term.tsx:211
```

**Root cause:** `term.tsx` imports `getCurrentWebview` from `@tauri-apps/api/webview`
and calls it unconditionally in `onMount`. In CEF mode, `__TAURI_INTERNALS__` is
undefined, so `getCurrentWindow()` returns undefined, and `.metadata` throws.

Also: `action-widgets.tsx` imports `invoke` directly from `@tauri-apps/api/core`
instead of using the platform-agnostic `invokeCommand` from `ipc.ts`.

**Fix applied:**
- `term.tsx`: Guard drag-drop with `detectHost() !== "tauri"`, use dynamic
  `import("@tauri-apps/api/webview")` only when needed
- `action-widgets.tsx`: Replace `invoke` with `invokeCommand` from `ipc.ts`

**Lesson:** Any file that imports from `@tauri-apps/api/*` at the top level will
crash in CEF. All Tauri-specific APIs should either be dynamically imported behind
a host check, or replaced with the platform-agnostic `invokeCommand`/`listenEvent`
from `ipc.ts`.

**Remaining direct Tauri imports:**
- `frontend/app/drag/CrossWindowDragMonitor.linux.tsx` — Linux only, not active

**Files:** `frontend/app/view/term/term.tsx`, `frontend/app/window/action-widgets.tsx`

---

## Bug 3: Context Menu Not Showing

**Symptom:** Right-click anywhere shows nothing.

**Root cause:** `show_context_menu` IPC command was stubbed in
`agentmux-cef/src/ipc.rs` (returns null, logs "stubbed"). CEF has no built-in
native context menu API equivalent to Tauri's.

**Fix applied:** Implemented a pure JS/HTML context menu overlay in `cef-api.ts`.
Renders `NativeContextMenuItem[]` as a positioned popup with support for:
- Separators
- Radio and checkbox items (with indicators)
- Nested submenus (hover to expand)
- Viewport edge clamping
- Click-outside to dismiss

Fires the `onContextMenuClick` callback directly with the item ID — no IPC
roundtrip needed.

**Files:** `frontend/util/cef-api.ts`

---

## Bug 4: Submenu Clipping

**Symptom:** Hovering "Opacity" in the context menu doesn't expand the submenu.

**Root cause:** The menu container had `maxHeight: 80vh; overflowY: auto` which
creates a CSS clipping context. Absolutely-positioned submenus (with
`left: 100%`) were clipped by the parent's overflow boundary.

**Fix applied:** Removed overflow constraints from the menu container. Added
z-index to submenu panels.

**Files:** `frontend/util/cef-api.ts`

---

## Bug 5: Vite Config Missing for Standalone Dev Server

**Symptom:** `npx vite` fails with:
```
Failed to resolve import "@/util/startup-bench" from "frontend/tauri-bootstrap.ts"
```

**Root cause:** Running `npx vite` without `--config vite.config.tauri.ts` doesn't
load `tsconfigPaths` plugin, so `@/` path aliases don't resolve.

**Fix:** Not a code fix — operator error. Must run
`npx vite --config vite.config.tauri.ts` or use `task dev` which passes the config
automatically. The `cef:dev` task in Taskfile should be updated to include the
config flag.

---

## Build Toolchain Notes

**CMake + Ninja required for CEF SDK build.** Both are bundled with VS 2025
Community but not in PATH by default. Need:
```bash
export PATH="/c/Program Files/Microsoft Visual Studio/18/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja:/c/Program Files/Microsoft Visual Studio/18/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin:$PATH"
```

**First build downloads CEF SDK (~350MB)** via `download-cef` crate. Takes ~2
minutes on fast connection. Cached in `target/debug/build/cef-dll-sys-*/out/`.

**Bundle size:** 350MB (`libcef.dll` alone is 262MB). This is the debug build —
release should be similar since CEF is a prebuilt binary.

**Taskfile `cef:bundle:windows` is broken.** The PowerShell script has a variable
interpolation issue (Task runner eats `$cefDir`). Bundled manually with bash.

---

## What Works

- CEF host starts, spawns backend sidecar with Job Object cleanup
- IPC bridge (HTTP POST to localhost) handles commands and events
- Frontend detects CEF host and uses API shim
- Terminal panes open and work (shell integration, I/O)
- Sysinfo panes render charts
- Context menus (JS overlay) with submenus
- GPU rendering works (errors are non-fatal retries)

## What Doesn't Work Yet

- `set_zoom_factor` deadlocks (skipped, needs CefPostTask fix)
- Window transparency (CEF window compositor setup needed)
- Taskfile `cef:bundle:windows` PowerShell script broken
- No `cef:dev` Vite config flag (must use standalone Vite + manual CEF launch)
- File drag-and-drop disabled in CEF (Tauri-only feature)

---

## Commits Added to Branch

1. `fix(cef): skip set_zoom_factor in CEF to prevent message loop deadlock`
2. `fix(cef): guard Tauri-only APIs in terminal and action-widgets`
3. `feat(cef): implement JS context menu overlay for CEF host`
4. `fix(cef): fix submenu clipping in JS context menu`
