# AgentMux Tauri v2 Migration — Implementation Plan

> **Status:** SPEC
> **Date:** 2026-02-07
> **Author:** Agent3
> **Target:** Tauri v2.10+ (stable)
> **Estimated Scope:** ~30% rewrite (emain/ → Rust), ~10% adaptation (frontend IPC layer), ~60% unchanged (Go backend + React frontend)

---

## Executive Summary

AgentMux is a three-tier desktop application: Electron main process (TypeScript) → Go backend server (`agentmuxsrv`) → React frontend. The Tauri migration replaces only the Electron layer with Rust, while the Go backend and React frontend remain largely intact.

**Key benefits:**
- Installer size: ~120-150MB → ~10-15MB (10x reduction)
- Idle memory: ~150-300MB → ~30-50MB (5x reduction)
- Startup time: ~1-2s → <0.5s (3x faster)
- No bundled Chromium — uses OS-native webview (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux)

**Key risks:**
- xterm.js WebGL2 unavailable on macOS (WKWebView) — requires canvas fallback
- No multi-WebContentsView equivalent — tab system must become frontend-managed
- Cross-platform rendering inconsistencies (different webview engines per OS)

---

## Current Architecture

```
┌──────────────────────────────────────────────────────┐
│                    Electron Main Process              │
│  emain/emain.ts          — Entry point, IPC handlers │
│  emain/emain-window.ts   — Window management, tabs   │
│  emain/emain-tabview.ts  — WebContentsView per tab   │
│  emain/preload.ts        — contextBridge (40+ APIs)  │
│  emain/menu.ts           — App menu, context menu    │
│  emain/platform.ts       — Path resolution, platform │
│  emain/updater.ts        — Auto-updater (disabled)   │
│  emain/authkey.ts        — Auth key + header inject  │
│  emain/emain-agentmuxsrv.ts — Go backend spawning     │
│  emain/emain-wsh.ts      — WSH RPC client            │
│  + 10 more files (crash, log, heartbeat, utils)      │
└──────────────────┬───────────────────────────────────┘
                   │ child_process.spawn()
                   │ stdio (WAVESRV-ESTART/WAVESRV-EVENT)
                   ▼
┌──────────────────────────────────────────────────────┐
│              Go Backend (agentmuxsrv)                  │
│  cmd/server/     — Main server entry                  │
│  pkg/web/        — HTTP + WebSocket server            │
│  pkg/wshrpc/     — RPC type definitions               │
│  pkg/wstore/     — SQLite data store                  │
│  pkg/blockcontroller/ — Terminal/block lifecycle       │
│  pkg/shellexec/  — PTY + shell execution              │
│  pkg/remote/     — SSH connections                     │
│  pkg/waveai/     — AI integration                     │
│  + 15 more packages                                   │
└──────────────────┬───────────────────────────────────┘
                   │ WebSocket + HTTP (localhost)
                   ▼
┌──────────────────────────────────────────────────────┐
│              React Frontend (Vite)                     │
│  frontend/wave.ts         — WS init, app bootstrap    │
│  frontend/app/store/      — State, services, WOS      │
│  frontend/app/view/       — React components           │
│  frontend/types/custom.d.ts — ElectronApi type (40+)  │
└──────────────────────────────────────────────────────┘
```

### Current IPC Patterns

| Pattern | Direction | Mechanism | Count |
|---------|-----------|-----------|-------|
| `ipcMain.on` (sync) | Renderer → Main | `sendSync`/`returnValue` | ~15 channels |
| `ipcMain.on` (fire-and-forget) | Renderer → Main | `send` | ~20 channels |
| `ipcMain.handle` (async) | Renderer → Main | `invoke`/`Promise` | ~2 channels |
| `webContents.send` | Main → Renderer | Push event | ~10 channels |
| WebSocket | Frontend ↔ Go Backend | Bidirectional RPC | Primary data channel |
| HTTP | Frontend → Go Backend | REST-like | File streaming |
| stdio | Main ↔ Go Backend | WAVESRV-ESTART/EVENT | Process management |

---

## Target Architecture

```
┌──────────────────────────────────────────────────────┐
│                 Tauri Rust Backend                     │
│  src-tauri/src/main.rs    — Entry point, builder      │
│  src-tauri/src/commands/  — #[tauri::command] fns     │
│  src-tauri/src/window.rs  — Window management         │
│  src-tauri/src/menu.rs    — App + context menus       │
│  src-tauri/src/sidecar.rs — Go backend lifecycle      │
│  src-tauri/src/auth.rs    — Auth key management       │
│  src-tauri/src/platform.rs— Path resolution           │
│  src-tauri/src/state.rs   — App state (Mutex<T>)      │
│  src-tauri/src/tray.rs    — System tray               │
└──────────────────┬───────────────────────────────────┘
                   │ Tauri sidecar (Command::new_sidecar)
                   │ stdio (WAVESRV-ESTART/WAVESRV-EVENT)
                   ▼
┌──────────────────────────────────────────────────────┐
│          Go Backend (agentmuxsrv) — UNCHANGED          │
│  (Identical to current — zero changes needed)         │
└──────────────────┬───────────────────────────────────┘
                   │ WebSocket + HTTP (localhost)
                   ▼
┌──────────────────────────────────────────────────────┐
│      React Frontend — ADAPTED (IPC layer only)        │
│  frontend/util/tauri-api.ts — Tauri invoke() shim     │
│  frontend/types/custom.d.ts — Updated ElectronApi     │
│  frontend/app/tab/          — Frontend tab management │
│  (All other components unchanged)                     │
└──────────────────────────────────────────────────────┘
```

---

## Phase 0: Project Scaffolding

### 0.1 Initialize Tauri in AgentMux repo

```bash
# From agentmux root
cargo install create-tauri-app
npm install @tauri-apps/cli @tauri-apps/api
npx tauri init
```

This creates `src-tauri/` with:
- `Cargo.toml` — Rust dependencies
- `tauri.conf.json` — App configuration
- `src/main.rs` — Entry point
- `capabilities/default.json` — Permission manifest
- `icons/` — App icons

### 0.2 Configure `tauri.conf.json`

```json
{
  "productName": "AgentMux",
  "version": "0.1.0",
  "identifier": "com.agentmuxhq.agentmux",
  "build": {
    "devUrl": "http://localhost:8190",
    "frontendDist": "../frontend/dist"
  },
  "app": {
    "windows": [
      {
        "title": "AgentMux",
        "width": 1200,
        "height": 800,
        "minWidth": 400,
        "minHeight": 300,
        "decorations": false,
        "transparent": true
      }
    ],
    "security": {
      "csp": "default-src 'self'; connect-src 'self' ws://localhost:* http://localhost:*; script-src 'self' 'unsafe-eval'"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["nsis", "dmg", "deb", "appimage"],
    "externalBin": ["binaries/agentmuxsrv", "binaries/wsh"],
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```

### 0.3 Configure Rust dependencies (`Cargo.toml`)

```toml
[dependencies]
tauri = { version = "2", features = ["tray-icon", "devtools"] }
tauri-plugin-shell = "2"
tauri-plugin-dialog = "2"
tauri-plugin-notification = "2"
tauri-plugin-clipboard-manager = "2"
tauri-plugin-global-shortcut = "2"
tauri-plugin-fs = "2"
tauri-plugin-opener = "2"
tauri-plugin-process = "2"
tauri-plugin-store = "2"
tauri-plugin-window-state = "2"
tauri-plugin-websocket = "2"
tauri-plugin-single-instance = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
tracing = "0.1"
tracing-subscriber = "0.3"
tracing-appender = "0.2"
```

### 0.4 Set up sidecar binaries directory

```
src-tauri/binaries/
├── agentmuxsrv-x86_64-unknown-linux-gnu
├── agentmuxsrv-x86_64-pc-windows-msvc.exe
├── agentmuxsrv-aarch64-apple-darwin
├── agentmuxsrv-x86_64-apple-darwin
├── wsh-x86_64-unknown-linux-gnu
├── wsh-x86_64-pc-windows-msvc.exe
├── wsh-aarch64-apple-darwin
└── wsh-x86_64-apple-darwin
```

Tauri requires binary names to include the target triple suffix. The existing `Taskfile.yml` build system compiles Go binaries per-platform — extend it to copy outputs with the correct naming convention.

### 0.5 Capabilities manifest

```json
// src-tauri/capabilities/default.json
{
  "identifier": "default",
  "description": "Default capabilities for AgentMux",
  "windows": ["*"],
  "permissions": [
    "core:default",
    "shell:default",
    {
      "identifier": "shell:allow-spawn",
      "allow": [
        { "name": "binaries/agentmuxsrv", "sidecar": true },
        { "name": "binaries/wsh", "sidecar": true }
      ]
    },
    "dialog:default",
    "notification:default",
    "clipboard-manager:default",
    "global-shortcut:default",
    "fs:default",
    "opener:default",
    "process:default",
    "store:default",
    "window-state:default",
    "websocket:default"
  ]
}
```

---

## Phase 1: Go Backend Sidecar (Replace `emain/emain-agentmuxsrv.ts`)

The Go backend is **completely independent** of Electron — it communicates via HTTP/WebSocket/Unix sockets. This is the easiest and highest-value phase.

### 1.1 Implement `src-tauri/src/sidecar.rs`

Port the logic from `emain/emain-agentmuxsrv.ts` (~5KB):

```rust
use tauri::Manager;
use tauri_plugin_shell::ShellExt;
use tokio::sync::watch;

pub struct BackendState {
    pub ws_endpoint: String,
    pub web_endpoint: String,
    pub child: Option<tauri_plugin_shell::process::CommandChild>,
}

pub async fn spawn_backend(app: &tauri::AppHandle) -> Result<BackendState, String> {
    let shell = app.shell();
    let (mut rx, child) = shell
        .sidecar("binaries/agentmuxsrv")
        .map_err(|e| format!("Failed to find sidecar: {}", e))?
        .args(&["--wavedata", &data_dir(app)])
        .spawn()
        .map_err(|e| format!("Failed to spawn: {}", e))?;

    // Parse WAVESRV-ESTART from stderr to get endpoints
    // Format: WAVESRV-ESTART ws:<addr> web:<addr> version:<ver> buildtime:<time>
    let (tx, endpoint_rx) = watch::channel(None);

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                tauri_plugin_shell::process::CommandEvent::Stderr(line) => {
                    let line = String::from_utf8_lossy(&line);
                    if line.starts_with("WAVESRV-ESTART") {
                        // Parse ws and web endpoints
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        let ws = parts.iter().find(|p| p.starts_with("ws:"))
                            .map(|p| p[3..].to_string());
                        let web = parts.iter().find(|p| p.starts_with("web:"))
                            .map(|p| p[4..].to_string());
                        if let (Some(ws), Some(web)) = (ws, web) {
                            tx.send(Some((ws, web))).ok();
                        }
                    }
                    // Handle WAVESRV-EVENT: messages
                    if line.starts_with("WAVESRV-EVENT:") {
                        handle_backend_event(&line[14..]);
                    }
                }
                _ => {}
            }
        }
    });

    // Wait for backend to be ready (with timeout)
    // ...
}
```

### 1.2 Integrate into Tauri setup

```rust
// src-tauri/src/main.rs
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match sidecar::spawn_backend(&handle).await {
                    Ok(state) => {
                        handle.manage(state);
                    }
                    Err(e) => {
                        eprintln!("Failed to start backend: {}", e);
                        std::process::exit(1);
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 1.3 Graceful shutdown

```rust
// On app close, send SIGTERM to agentmuxsrv
app.on_window_event(|window, event| {
    if let tauri::WindowEvent::CloseRequested { .. } = event {
        if let Some(state) = window.app_handle().try_state::<BackendState>() {
            if let Some(child) = &state.child {
                child.kill().ok();
            }
        }
    }
});
```

**Deliverable:** AgentMux launches with Go backend as Tauri sidecar, frontend connects via WebSocket.

---

## Phase 2: IPC Bridge (Replace `emain/preload.ts`)

The frontend currently uses `window.api.*` (40+ methods) injected by Electron's `contextBridge`. Replace with a Tauri-compatible shim.

### 2.1 Catalog all `window.api` methods

From `emain/preload.ts` and `frontend/types/custom.d.ts`:

**Synchronous getters (ipcRenderer.sendSync):**
| Method | Returns | Tauri Equivalent |
|--------|---------|-----------------|
| `getAuthKey()` | string | `invoke("get_auth_key")` |
| `getIsDev()` | boolean | `invoke("get_is_dev")` |
| `getPlatform()` | string | `invoke("get_platform")` |
| `getCursorPoint()` | {x,y} | `invoke("get_cursor_point")` |
| `getUserName()` | string | `invoke("get_user_name")` |
| `getHostName()` | string | `invoke("get_host_name")` |
| `getDataDir()` | string | `invoke("get_data_dir")` |
| `getConfigDir()` | string | `invoke("get_config_dir")` |
| `getAboutModalDetails()` | object | `invoke("get_about_modal_details")` |
| `getDocsiteUrl()` | string | `invoke("get_docsite_url")` |
| `getWebviewPreload()` | string | N/A (no webview tags in Tauri) |
| `getZoomFactor()` | number | `invoke("get_zoom_factor")` |
| `getEnv(key)` | string | `invoke("get_env", {key})` |
| `getAppUpdateStatus()` | object | `invoke("get_update_status")` |
| `getUpdaterChannel()` | string | `invoke("get_updater_channel")` |

**Fire-and-forget actions (ipcRenderer.send):**
| Method | Tauri Equivalent |
|--------|-----------------|
| `openNewWindow()` | `invoke("open_new_window")` |
| `showContextMenu(menu)` | `invoke("show_context_menu", {menu})` |
| `downloadFile(opts)` | `invoke("download_file", {opts})` |
| `openExternal(url)` | `@tauri-apps/plugin-opener` |
| `updateWindowControlsOverlay(rect)` | `invoke("update_wco", {rect})` |
| `setActiveTab(tabId)` | Frontend-managed (no IPC needed) |
| `createTab()` | Frontend-managed |
| `closeTab(tabId)` | Frontend-managed |
| `createWorkspace(name)` | `invoke("create_workspace", {name})` |
| `switchWorkspace(id)` | `invoke("switch_workspace", {id})` |
| `deleteWorkspace(id)` | `invoke("delete_workspace", {id})` |
| `setWindowInitStatus(status)` | `invoke("set_window_init_status")` |
| `feLog(msg)` | `invoke("fe_log", {msg})` |
| `quicklook(path)` | `invoke("quicklook", {path})` |
| `openNativePath(path)` | `@tauri-apps/plugin-opener` |
| `setKeyboardChordMode(mode)` | `invoke("set_keyboard_chord_mode")` |
| `installAppUpdate()` | `invoke("install_update")` |

**Async with return (ipcRenderer.invoke):**
| Method | Tauri Equivalent |
|--------|-----------------|
| `captureScreenshot(rect)` | Custom Rust implementation |
| `clearWebviewStorage()` | N/A or custom |

**Event listeners (ipcRenderer.on):**
| Event | Tauri Equivalent |
|-------|-----------------|
| `contextmenu-click` | `listen("contextmenu-click")` |
| `fullscreen-change` | `listen("fullscreen-change")` |
| `zoom-factor-change` | `listen("zoom-factor-change")` |
| `app-update-status` | `listen("app-update-status")` |
| `menu-item-about` | `listen("menu-item-about")` |
| `reinject-key` | `listen("reinject-key")` |
| `wave-init` | `listen("wave-init")` |

### 2.2 Create Tauri API shim

```typescript
// frontend/util/tauri-api.ts
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { open as openUrl } from "@tauri-apps/plugin-opener";

// Cache synchronous values at startup
let cachedAuthKey: string;
let cachedPlatform: string;
let cachedIsDev: boolean;
// ... etc

export async function initTauriApi(): Promise<void> {
  // Pre-fetch all "synchronous" values
  [cachedAuthKey, cachedPlatform, cachedIsDev] = await Promise.all([
    invoke<string>("get_auth_key"),
    invoke<string>("get_platform"),
    invoke<boolean>("get_is_dev"),
  ]);
}

// Expose same interface as ElectronApi
export const tauriApi: ElectronApi = {
  getAuthKey: () => cachedAuthKey,
  getIsDev: () => cachedIsDev,
  getPlatform: () => cachedPlatform,
  // ... synchronous getters return cached values

  openExternal: (url: string) => openUrl(url),
  openNewWindow: () => invoke("open_new_window"),
  // ... async methods call invoke()

  onFullscreenChange: (cb) => {
    listen("fullscreen-change", (e) => cb(e.payload));
  },
  // ... event listeners use listen()
};
```

### 2.3 Swap the API provider

```typescript
// frontend/util/global.ts
import { tauriApi, initTauriApi } from "./tauri-api";

// Replace:
// export function getApi(): ElectronApi { return window.api; }
// With:
let api: ElectronApi;
export function getApi(): ElectronApi {
  if (!api) api = tauriApi;
  return api;
}

// Call initTauriApi() during app bootstrap (wave.ts)
```

### 2.4 Implement Rust command handlers

```rust
// src-tauri/src/commands/mod.rs
mod platform;
mod auth;
mod window;

// src-tauri/src/commands/platform.rs
#[tauri::command]
fn get_platform() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
fn get_user_name() -> String {
    whoami::username()
}

#[tauri::command]
fn get_host_name() -> String {
    whoami::hostname()
}

#[tauri::command]
fn get_is_dev() -> bool {
    cfg!(debug_assertions)
}

#[tauri::command]
fn get_data_dir(app: tauri::AppHandle) -> String {
    app.path().app_data_dir().unwrap().to_string_lossy().to_string()
}

#[tauri::command]
fn get_config_dir(app: tauri::AppHandle) -> String {
    app.path().app_config_dir().unwrap().to_string_lossy().to_string()
}

#[tauri::command]
fn get_env(key: String) -> Option<String> {
    std::env::var(&key).ok()
}
```

```rust
// src-tauri/src/commands/auth.rs
use std::sync::Mutex;
use uuid::Uuid;

pub struct AuthState {
    pub key: String,
}

impl Default for AuthState {
    fn default() -> Self {
        Self { key: Uuid::new_v4().to_string() }
    }
}

#[tauri::command]
fn get_auth_key(state: tauri::State<'_, Mutex<AuthState>>) -> String {
    state.lock().unwrap().key.clone()
}
```

**Deliverable:** Frontend communicates with Rust backend via `invoke()` instead of Electron IPC. Same `window.api` interface preserved.

---

## Phase 3: Tab System Redesign (Replace `emain/emain-tabview.ts` + `emain/emain-window.ts`)

This is the **most complex** phase. Electron's model uses one `WebContentsView` per tab (separate Chromium renderers). Tauri has one webview per window — tabs must become frontend-managed.

### 3.1 Current Electron tab model

```
BaseWindow
├── WebContentsView (Tab 1) — active, positioned on-screen
├── WebContentsView (Tab 2) — inactive, positioned at (-15000, -15000)
├── WebContentsView (Tab 3) — inactive, positioned at (-15000, -15000)
└── Hot spare WebContentsView — pre-created blank tab
```

Each tab is an independent renderer process with full DOM isolation. Tab switching moves the active view on-screen and the previous one off-screen. An LRU cache evicts old tabs.

### 3.2 New Tauri tab model

```
Tauri Window (single webview)
└── React App
    ├── TabBar component (renders tab headers)
    ├── TabContent component (renders active tab's content)
    └── Tab state managed in React/Zustand store
```

**Key insight:** AgentMux's tabs already render their content via WebSocket data from the Go backend. The frontend receives block data (terminal output, file content, AI responses) through the existing `wos` (Wave Object Store) system. The current per-tab `WebContentsView` is largely unnecessary — the React frontend already has all the state it needs to render any tab.

### 3.3 Implementation strategy

**A) Move tab state to React:**

```typescript
// frontend/app/tab/tab-store.ts
interface TabState {
  tabs: Tab[];
  activeTabId: string;
  workspaceId: string;
}

// Tab switching = update activeTabId in store
// No IPC needed — purely frontend state
function switchTab(tabId: string) {
  tabStore.setState({ activeTabId: tabId });
}
```

**B) Remove IPC for tab operations:**

Current flow: Frontend → `ipcRenderer.send("set-active-tab")` → Electron main → moves WebContentsView.

New flow: Frontend → `tabStore.switchTab(tabId)` → React re-renders active tab content.

**C) Handle tab isolation:**

Electron provided DOM isolation between tabs (separate renderers). In the single-webview model, all tabs share one DOM. This is acceptable because:
- Terminal blocks already use xterm.js instances that manage their own state
- Block content is driven by WebSocket data from the Go backend
- Memory cleanup when closing tabs is handled by React component lifecycle (unmount)

**D) Remove hot spare and LRU cache:**

These were optimizations for expensive `WebContentsView` creation/destruction. In the React model, tab "creation" is just adding an entry to the store (instant). No spare pool needed.

### 3.4 Files to create/modify

| File | Action | Description |
|------|--------|-------------|
| `frontend/app/tab/tab-store.ts` | CREATE | Zustand store for tab/workspace state |
| `frontend/app/tab/TabBar.tsx` | CREATE | Tab bar UI component |
| `frontend/app/tab/TabContent.tsx` | MODIFY | Render active tab's blocks |
| `frontend/app/view/WorkspaceView.tsx` | MODIFY | Use tab store instead of Electron IPC |
| `src-tauri/src/window.rs` | CREATE | Tauri window creation/management |
| `src-tauri/src/commands/window.rs` | CREATE | Window-related commands |

**Deliverable:** Tabs work entirely in React. No WebContentsView, no off-screen positioning, no hot spares.

---

## Phase 4: Window Management (Replace `emain/emain-window.ts`)

### 4.1 Window creation

```rust
// src-tauri/src/window.rs
use tauri::{WebviewWindowBuilder, WebviewUrl, Manager};

#[tauri::command]
async fn open_new_window(app: tauri::AppHandle) -> Result<(), String> {
    let label = format!("window-{}", uuid::Uuid::new_v4());
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("AgentMux")
        .inner_size(1200.0, 800.0)
        .min_inner_size(400.0, 300.0)
        .decorations(false)  // Custom titlebar
        .transparent(true)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
```

### 4.2 Custom titlebar

Since `decorations: false`, the frontend renders its own titlebar (AgentMux already does this). Tauri provides `data-tauri-drag-region` attribute for drag areas:

```html
<div data-tauri-drag-region class="titlebar">
  <!-- Window controls -->
</div>
```

### 4.3 Window events

```rust
// Handle window events in Rust
app.on_window_event(|window, event| {
    match event {
        WindowEvent::Resized(size) => {
            window.emit("window-resized", size).ok();
        }
        WindowEvent::Moved(pos) => {
            window.emit("window-moved", pos).ok();
        }
        WindowEvent::Focused(focused) => {
            window.emit("window-focused", focused).ok();
        }
        WindowEvent::CloseRequested { api, .. } => {
            // Check if we should confirm close
            // api.prevent_close() if needed
        }
        _ => {}
    }
});
```

### 4.4 Window state persistence

Use `tauri-plugin-window-state` to automatically save/restore window position and size:

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_window_state::Builder::default().build())
```

**Deliverable:** Multi-window support with custom titlebar, state persistence, and event handling.

---

## Phase 5: Native Menus (Replace `emain/menu.ts`)

### 5.1 Application menu

```rust
// src-tauri/src/menu.rs
use tauri::menu::{Menu, Submenu, MenuItem, PredefinedMenuItem};

pub fn build_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let file_menu = Submenu::with_items(app, "File", true, &[
        &MenuItem::with_id(app, "new-window", "New Window", true, Some("CmdOrCtrl+Shift+N"))?,
        &MenuItem::with_id(app, "new-tab", "New Tab", true, Some("CmdOrCtrl+T"))?,
        &PredefinedMenuItem::separator(app)?,
        &MenuItem::with_id(app, "close-tab", "Close Tab", true, Some("CmdOrCtrl+W"))?,
        &PredefinedMenuItem::close_window(app, None)?,
    ])?;

    let edit_menu = Submenu::with_items(app, "Edit", true, &[
        &PredefinedMenuItem::undo(app, None)?,
        &PredefinedMenuItem::redo(app, None)?,
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::cut(app, None)?,
        &PredefinedMenuItem::copy(app, None)?,
        &PredefinedMenuItem::paste(app, None)?,
        &PredefinedMenuItem::select_all(app, None)?,
    ])?;

    let view_menu = Submenu::with_items(app, "View", true, &[
        &MenuItem::with_id(app, "reload", "Reload", true, Some("CmdOrCtrl+Shift+R"))?,
        &MenuItem::with_id(app, "devtools", "Toggle DevTools", true, Some("CmdOrCtrl+Shift+I"))?,
        &PredefinedMenuItem::separator(app)?,
        &MenuItem::with_id(app, "zoom-in", "Zoom In", true, Some("CmdOrCtrl+="))?,
        &MenuItem::with_id(app, "zoom-out", "Zoom Out", true, Some("CmdOrCtrl+-"))?,
        &MenuItem::with_id(app, "zoom-reset", "Reset Zoom", true, Some("CmdOrCtrl+0"))?,
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::fullscreen(app, None)?,
    ])?;

    Menu::with_items(app, &[&file_menu, &edit_menu, &view_menu])
}
```

### 5.2 Context menus

Replace `electron.Menu.popup()` with a frontend-rendered context menu or use a community plugin:

```rust
#[tauri::command]
fn show_context_menu(window: tauri::Window, items: Vec<ContextMenuItem>) {
    // Option A: Emit to frontend, render in React
    window.emit("show-context-menu", items).ok();

    // Option B: Use native menu popup (limited styling)
    // tauri::menu::Menu::popup()
}
```

**Deliverable:** Native application menu with keyboard shortcuts, context menu support.

---

## Phase 6: xterm.js Terminal Compatibility

### 6.1 WebGL2 fallback strategy

xterm.js WebGL addon is unavailable on macOS (WKWebView). Implement automatic fallback:

```typescript
// frontend/app/view/term/term-renderer.ts
import { Terminal } from "xterm";
import { WebglAddon } from "@xterm/addon-webgl";
import { CanvasAddon } from "@xterm/addon-canvas";

export function attachRenderer(terminal: Terminal): void {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => {
      webgl.dispose();
      terminal.loadAddon(new CanvasAddon());
    });
    terminal.loadAddon(webgl);
  } catch (e) {
    console.warn("WebGL2 unavailable, using canvas renderer");
    terminal.loadAddon(new CanvasAddon());
  }
}
```

### 6.2 Known xterm.js issues in Tauri webviews

| Issue | Platform | Workaround |
|-------|----------|------------|
| WebGL2 unavailable | macOS (WKWebView) | Canvas addon fallback |
| `fitAddon.fit()` text disappearance | All (reported with Svelte, may affect React) | Debounce resize, call fit() after layout stabilizes |
| Font rendering differences | Cross-platform | Test with monospace fonts on all targets; pin font-family |
| Missing letter glyphs (WebGL) | Safari/WebKit | Canvas fallback eliminates this |
| Cursor blinking rate differences | Cross-platform | Set explicit `cursorBlink` and `cursorBlinkInterval` |

### 6.3 PTY integration

The PTY is already managed by the Go backend (`pkg/shellexec/`). xterm.js connects via WebSocket to the backend, not to a local PTY. **No changes needed** — the terminal data flow (Go PTY → WebSocket → xterm.js) is unaffected by the Electron→Tauri migration.

**Deliverable:** Terminal works across all platforms with automatic WebGL→Canvas fallback.

---

## Phase 7: Auth System (Replace `emain/authkey.ts`)

### 7.1 Current mechanism

Electron injects `X-AuthKey` header into all HTTP requests via `session.webRequest.onBeforeSendHeaders()`. This is transparent to the frontend.

### 7.2 Tauri replacement

Tauri has no session-level request interception. Two options:

**Option A (Recommended): Frontend-injected auth header**

The frontend already knows the auth key (via `getApi().getAuthKey()`). Modify the HTTP/WebSocket client to include it:

```typescript
// frontend/app/store/wshrpcutil.ts
const authKey = getApi().getAuthKey();

// For WebSocket connections:
const ws = new WebSocket(`${wsEndpoint}?authkey=${authKey}`);

// For HTTP requests:
fetch(url, {
  headers: { "X-AuthKey": authKey }
});
```

**Option B: Rust HTTP proxy**

Route all backend requests through a Rust-side proxy that injects the header. More complex, less transparent.

**Recommendation:** Option A. It requires minor changes to the frontend HTTP client but is simpler and more maintainable.

### 7.3 Go backend changes

If using Option A with WebSocket query param, update the Go backend to also accept auth key from query string (currently only checks `X-AuthKey` header). This is a one-line change in the auth middleware.

**Deliverable:** Auth key system works without Electron session interception.

---

## Phase 8: Platform Utilities (Replace `emain/platform.ts`)

### 8.1 Path resolution

```rust
// src-tauri/src/platform.rs
use tauri::Manager;

#[tauri::command]
fn get_data_dir(app: tauri::AppHandle) -> Result<String, String> {
    app.path().app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_config_dir(app: tauri::AppHandle) -> Result<String, String> {
    app.path().app_config_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}
```

### 8.2 Screen/display APIs

Electron's `screen.getAllDisplays()` and `screen.getPrimaryDisplay()` are used for:
- Default window positioning
- Multi-monitor awareness
- Cursor position detection

Tauri doesn't expose screen APIs directly. Options:
- Use the `tao` crate (Tauri's window management library) directly for monitor info
- Use platform-specific APIs via `#[cfg(target_os = "...")]`

```rust
#[tauri::command]
fn get_cursor_point(window: tauri::Window) -> Result<(f64, f64), String> {
    // Use window.cursor_position() if available,
    // or platform-specific API
    let pos = window.cursor_position()
        .map_err(|e| e.to_string())?;
    Ok((pos.x, pos.y))
}
```

**Deliverable:** All platform utility functions ported to Rust.

---

## Phase 9: Crash Handling & Logging (Replace crash-*.ts + log.ts + heartbeat.ts)

### 9.1 Logging

Replace `winston` with Rust `tracing`:

```rust
// src-tauri/src/logging.rs
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};
use tracing_appender::rolling;

pub fn init_logging(log_dir: &std::path::Path) {
    let file_appender = rolling::daily(log_dir, "agentmux.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive("agentmux=info".parse().unwrap()))
        .with(fmt::layer().with_writer(non_blocking))
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();
}
```

### 9.2 Crash handling

```rust
// src-tauri/src/crash.rs
use std::panic;

pub fn init_crash_handler() {
    panic::set_hook(Box::new(|panic_info| {
        let msg = format!("PANIC: {}", panic_info);
        tracing::error!("{}", msg);
        // Write crash report to file
        // Show native dialog if possible
    }));
}
```

### 9.3 Heartbeat

```rust
// src-tauri/src/heartbeat.rs
use tokio::time::{interval, Duration};

pub async fn heartbeat_loop(data_dir: std::path::PathBuf) {
    let heartbeat_file = data_dir.join("agentmux.heartbeat");
    let mut ticker = interval(Duration::from_secs(5));
    loop {
        ticker.tick().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        std::fs::write(&heartbeat_file, now.to_string()).ok();
    }
}
```

**Deliverable:** Structured logging, crash reports, and heartbeat monitoring in Rust.

---

## Phase 10: Build System & CI/CD

### 10.1 Build pipeline

Extend `Taskfile.yml` to include Tauri builds:

```yaml
tasks:
  tauri:build:
    desc: Build Tauri application for current platform
    cmds:
      - task: go:build         # Build agentmuxsrv + wsh
      - task: frontend:build   # Build React frontend
      - task: tauri:copy-sidecars
      - npx tauri build

  tauri:copy-sidecars:
    desc: Copy Go binaries to src-tauri/binaries with target triples
    cmds:
      - |
        TRIPLE=$(rustc -vV | grep host | awk '{print $2}')
        cp build/agentmuxsrv src-tauri/binaries/agentmuxsrv-$TRIPLE
        cp build/wsh src-tauri/binaries/wsh-$TRIPLE

  tauri:dev:
    desc: Run Tauri in development mode
    cmds:
      - task: go:build
      - task: tauri:copy-sidecars
      - npx tauri dev
```

### 10.2 GitHub Actions CI

```yaml
# .github/workflows/tauri-build.yml
name: Build Tauri
on: [push, pull_request]

jobs:
  build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: macos-latest
            target: aarch64-apple-darwin
          - os: macos-13
            target: x86_64-apple-darwin
          - os: windows-latest
            target: x86_64-pc-windows-msvc
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@v5
        with: { go-version: '1.22' }
      - uses: actions/setup-node@v4
        with: { node-version: '20' }
      - uses: dtolnay/rust-toolchain@stable
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tagName: v__VERSION__
          releaseName: "AgentMux v__VERSION__"
          releaseBody: "See the assets for download links."
```

### 10.3 Expected bundle sizes

| Platform | Electron (current) | Tauri (projected) |
|----------|-------------------|-------------------|
| Windows (.exe NSIS) | ~120 MB | ~15-20 MB |
| macOS (.dmg) | ~140 MB | ~12-18 MB |
| Linux (.AppImage) | ~130 MB | ~15-20 MB |

Note: The Go sidecar binaries (`agentmuxsrv` ~25MB + `wsh` ~5MB) are the dominant size contributor in Tauri builds. The Tauri shell itself adds only ~2-5MB.

**Deliverable:** Cross-platform builds via Taskfile + GitHub Actions.

---

## Phase 11: Auto-Updater (Replace `emain/updater.ts`)

Currently disabled in the AgentMux fork. When enabled:

```json
// tauri.conf.json
{
  "plugins": {
    "updater": {
      "endpoints": [
        "https://releases.agentmux.ai/{{target}}/{{arch}}/{{current_version}}"
      ],
      "pubkey": "YOUR_PUBLIC_KEY_HERE"
    }
  }
}
```

```rust
// src-tauri/src/updater.rs
use tauri_plugin_updater::UpdaterExt;

#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(update.version)),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
```

**Deliverable:** In-app update system with signature verification.

---

## Migration Order & Dependencies

```
Phase 0: Scaffolding ──────────────────────────────┐
                                                     │
Phase 1: Go Sidecar ──────────────────────────────┐ │
                                                    │ │
Phase 2: IPC Bridge ──────────────────────────────┐│ │
                                                   ││ │
Phase 6: xterm.js compat ────────────────────────┐│││
                                                  ││││
Phase 7: Auth system ────────────────────────────┐│││├─ Can run in parallel
                                                 │││││
Phase 8: Platform utils ─────────────────────────┤│││
                                                 │││││
Phase 9: Crash/logging ──────────────────────────┤│││
                                                 │││││
Phase 3: Tab redesign ─────────── (depends on 2) ┤│││
                                                  ││││
Phase 4: Window management ──── (depends on 2,3) ─┘│││
                                                    │││
Phase 5: Menus ─────────────── (depends on 4) ─────┘││
                                                     ││
Phase 10: Build/CI ─────────── (depends on all) ─────┘│
                                                       │
Phase 11: Auto-updater ─────── (last, currently off) ──┘
```

**Critical path:** Phase 0 → Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 10

---

## Risk Matrix

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| xterm.js WebGL2 on macOS | High | Certain | Canvas fallback (Phase 6) |
| Tab isolation regression | High | Medium | Thorough testing; React key-based cleanup |
| Cross-platform rendering bugs | Medium | High | Per-platform CI testing; CSS normalization |
| Go sidecar startup race | Medium | Low | Retry logic; health check endpoint |
| Auth header injection | Medium | Low | Frontend-side injection (Phase 7) |
| `<webview>` tag removal | High | Certain | Replace with iframes + custom protocol |
| Linux WebKitGTK version | Medium | Medium | Pin minimum GTK version; test on Ubuntu LTS |
| Screen API gaps | Low | Medium | Use `tao` crate directly |
| Build time (Rust compilation) | Low | Certain | Accept; incremental builds help after first |

---

## What Does NOT Change

- **Go backend** (`cmd/`, `pkg/`) — Zero modifications. Same binary, same protocols
- **React components** — All UI components stay as-is
- **WebSocket communication** — Frontend ↔ Go backend unchanged
- **Wave Object Store** — `wos.ts`, `wps.ts`, `services.ts` unchanged
- **Block system** — Terminal, file, AI blocks unchanged
- **Shell integration** — `wsh` binary and shell startup files unchanged
- **Data storage** — SQLite store in Go backend unchanged
- **SSH/Remote** — Handled by Go backend, unchanged
- **AI integration** — Handled by Go backend, unchanged

---

## Acceptance Criteria

- [ ] AgentMux launches and displays the React frontend via Tauri
- [ ] Go backend spawns as sidecar and frontend connects via WebSocket
- [ ] Terminal (xterm.js) works on Windows, macOS, and Linux
- [ ] Tab creation, switching, and closing works (frontend-managed)
- [ ] Multi-window support works
- [ ] Workspace creation/switching works
- [ ] Native menus with keyboard shortcuts work
- [ ] System tray icon works
- [ ] File dialogs work (open, save)
- [ ] External URL opening works
- [ ] Auth key system works (frontend-injected)
- [ ] Window state persists across restarts
- [ ] Installer size < 25MB per platform
- [ ] No WebGL2 errors on macOS (canvas fallback active)
- [ ] CI builds for all 4 targets (linux-x64, macos-arm64, macos-x64, windows-x64)

---

## References

- [Tauri v2 Documentation](https://v2.tauri.app/)
- [Tauri v2 Stable Release Blog](https://v2.tauri.app/blog/tauri-20/)
- [Official Tauri v2 Plugins](https://github.com/tauri-apps/plugins-workspace)
- [Tauri Sidecar Guide](https://v2.tauri.app/develop/sidecar/)
- [Tauri IPC / Calling Rust](https://v2.tauri.app/develop/calling-rust/)
- [Tauri Multi-Window](https://v2.tauri.app/reference/javascript/api/namespacewindow/)
- [tauri-plugin-pty (Community)](https://github.com/Tnze/tauri-plugin-pty)
- [Terminon — Tauri v2 Terminal Emulator](https://github.com/Shabari-K-S/terminon)
- [xterm.js WebGL2 Issue in Tauri](https://github.com/tauri-apps/tauri/issues/2866)
- [tauri-action GitHub Action](https://github.com/tauri-apps/tauri-action)
- [Electron vs Tauri Comparison (DoltHub)](https://www.dolthub.com/blog/2025-11-13-electron-vs-tauri/)
- [Tauri vs Electron Performance (Hopp)](https://www.gethopp.app/blog/tauri-vs-electron)
- [WebView2 on Windows](https://v2.tauri.app/reference/webview-versions/)
