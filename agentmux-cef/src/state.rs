// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared application state for the CEF host.

use std::collections::{HashMap, VecDeque};
use parking_lot::Mutex;

use cef::Browser;

// ── Cross-window drag types (ported from src-tauri/src/state.rs) ─────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragType {
    Pane,
    Tab,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragSession {
    pub drag_id: String,
    pub drag_type: DragType,
    pub source_window: String,
    pub source_workspace_id: String,
    pub source_tab_id: String,
    pub payload: DragPayload,
    pub started_at: u64,
}

/// Tracks a stable sequential instance number for each open window.
/// Main window is always 1. Additional windows get 2, 3, ... in creation order.
/// Numbers are never reused within a session.
pub struct WindowInstanceRegistry {
    instances: HashMap<String, u32>,
    next_num: u32,
}

impl WindowInstanceRegistry {
    pub fn new() -> Self {
        let mut instances = HashMap::new();
        instances.insert("main".to_string(), 1);
        Self {
            instances,
            next_num: 2,
        }
    }

    /// Assign the next instance number to a new window label.
    pub fn register(&mut self, label: &str) -> u32 {
        let num = self.next_num;
        self.instances.insert(label.to_string(), num);
        self.next_num += 1;
        num
    }

    /// Remove a window from the registry when it closes.
    pub fn unregister(&mut self, label: &str) {
        self.instances.remove(label);
    }

    /// Look up the instance number for a window label.
    pub fn get(&self, label: &str) -> Option<u32> {
        self.instances.get(label).copied()
    }

    /// Total number of currently open windows.
    pub fn count(&self) -> usize {
        self.instances.len()
    }
}

// Phase B.1: removed `JobHandle` wrapper. Host no longer owns a Job
// Object on srv; the launcher's J0 covers srv directly via
// AssignProcessToJobObject. The same RAII pattern lives in
// agentmux-launcher/src/main.rs::JobHandle.

/// Backend (agentmux-srv) connection endpoints.
#[derive(Default, Clone, serde::Serialize)]
pub struct BackendEndpoints {
    pub ws_endpoint: String,
    pub web_endpoint: String,
}

/// Window role in the AgentMux multi-window model.
///
/// Two distinct types with different taskbar treatment:
/// - `FullInstance`: independent AgentMux window (like Chrome/VS Code new window).
///   Appears in the Windows taskbar. All user-facing "new window" paths (status-bar
///   version click, second `agentmux.exe` launch, `Ctrl+Shift+N`) create one.
/// - `Subwindow`: hidden from the taskbar via `ITaskbarList::DeleteTab`. Only
///   reachable through the backend `open_subwindow` API — reserved for agent /
///   internal use cases (transient auxiliary views, tool-spawned panels). Closes
///   when its parent full instance closes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FullInstance,
    Subwindow,
}

/// Per-window metadata held alongside the CEF `Browser`. See `WindowKind` for
/// the semantics of `kind` and `parent_instance_id`.
#[derive(Clone, Debug)]
pub struct WindowMeta {
    pub label: String,
    pub kind: WindowKind,
    /// For `Subwindow` only: label of the `FullInstance` that owns this window.
    /// `None` for `FullInstance`.
    pub parent_instance_id: Option<String>,
}

/// Shared application state for the CEF host.
///
/// Unlike the Tauri version, this uses `Arc<AppState>` directly instead of
/// `tauri::State<AppState>`. The sidecar child is `std::process::Child` instead
/// of `tauri_plugin_shell::process::CommandChild`.
pub struct AppState {
    /// Maps window label (e.g. "main", "window-{uuid}") to the backend window ID.
    /// Populated when the frontend calls `register_backend_window` during init.
    /// Used by `on_before_close` to notify the backend to clean up.
    pub window_id_map: Mutex<HashMap<String, String>>,

    /// Auth key for backend communication
    pub auth_key: Mutex<String>,

    /// Backend (agentmux-srv) connection endpoints
    pub backend_endpoints: Mutex<BackendEndpoints>,

    /// Handle to the sidecar child process for graceful shutdown
    pub sidecar_child: Mutex<Option<std::process::Child>>,

    /// Backend process PID (set after spawn)
    pub backend_pid: Mutex<Option<u32>>,

    /// Backend process start time as ISO 8601 string
    pub backend_started_at: Mutex<Option<String>>,

    /// Current zoom factor
    pub zoom_factor: Mutex<f64>,

    /// Client ID (set after querying backend on startup)
    pub client_id: Mutex<Option<String>>,

    /// Window ID (set after querying/creating window via backend)
    pub window_id: Mutex<Option<String>>,

    /// Active tab ID (set after querying/creating default tab via backend)
    pub active_tab_id: Mutex<Option<String>>,

    /// Window initialization status ("ready" or "wave-ready")
    pub window_init_status: Mutex<String>,

    /// Sequential instance numbers for each open window
    pub window_instance_registry: Mutex<WindowInstanceRegistry>,

    /// Phase B.5b — shadow registry fed by the launcher's
    /// `WindowInstanceAssigned` / `WindowInstanceReleased` events.
    /// Maintained in parallel to `window_instance_registry` (which
    /// remains authoritative for now); on every launcher event the
    /// reader task compares the two and logs drift. B.5c will swap
    /// the roles — launcher becomes authoritative, this map becomes
    /// the read path, and `window_instance_registry` is retired.
    pub shadow_instance_registry: Mutex<HashMap<String, u32>>,

    /// Cancellation channel for an in-progress CLI login process
    pub cli_login_cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,

    /// Stdin handle for the running CLI login child process.
    /// Written to by `set_provider_auth` to deliver the OAuth device code.
    pub cli_login_stdin: Mutex<Option<tokio::process::ChildStdin>>,

    /// IPC HTTP server port
    pub ipc_port: Mutex<u16>,

    /// IPC bearer token — injected into the page alongside the port.
    /// Verified on every IPC request to prevent unauthorized local access.
    pub ipc_token: String,

    /// CEF Browser handles keyed by window label (multi-window support).
    /// "main" is the primary window; tear-off windows get "window-{UUID}" labels.
    pub browsers: Mutex<HashMap<String, Browser>>,

    /// Per-window metadata (kind, parent linkage). Maintained in parallel with
    /// `browsers` — any `browsers.insert()` for a real window must also record
    /// a `WindowMeta` here so on_after_created knows whether to hide the HWND
    /// from the taskbar and so on_before_close can cascade sub-window closure.
    pub window_meta: Mutex<HashMap<String, WindowMeta>>,

    /// FIFO queue of labels for windows that are about to be created.
    /// Pushed in `open_new_window` / `open_window_at_position` before
    /// `post_create_window`; popped in `on_after_created` so the browser gets
    /// the correct label rather than a freshly-generated UUID.
    pub pending_window_labels: Mutex<VecDeque<String>>,

    /// Tear-off Phase 6 — pre-warmed pool of hidden CEF windows ready for
    /// instant promotion on tear-off. Each entry is a label of a window
    /// that's already painted, has its renderer connected, and is sitting
    /// in pool-mode (`?pool=1` URL flag) waiting to be assigned a workspace.
    /// On tear-off: pop a label, reposition + show + emit `pool:promote`.
    /// Cold-path (open_window_at_position) remains as defence-in-depth.
    /// See SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.5.
    pub window_pool: Mutex<VecDeque<String>>,
    /// Labels of pool windows that have been spawned but NOT yet
    /// promoted. Populated synchronously in `spawn_pool_window` so
    /// callers (list_windows, app-exit decision) can identify pool
    /// windows BEFORE the renderer-ready handshake fires and
    /// `window_pool` gets populated. Removed on promote, on
    /// destroy-before-promote, and during pool teardown.
    /// Without this, list_windows reports unpromoted pool windows
    /// as user-visible instances during the ~100ms gap between
    /// spawn and renderer-ready.
    pub unpromoted_pool_labels: Mutex<std::collections::HashSet<String>>,
    /// Single in-flight respawn semaphore — prevents pool refill from
    /// stacking spawns when the user does back-to-back tear-offs faster
    /// than CEF can create windows.
    pub window_pool_respawn_in_flight: std::sync::atomic::AtomicBool,

    /// Version-specific data directory (e.g. ai.agentmux.cef.v0-32-111/)
    pub version_data_dir: Mutex<Option<String>>,

    /// Version-specific config directory
    pub version_config_dir: Mutex<Option<String>>,

    /// User data home used by the frontend's `agentmuxHome()` helper to
    /// construct per-agent paths (working dir, `GH_CONFIG_DIR`, etc.).
    ///
    /// - Portable: `<portable>/data/` — keeps agent state inside the portable
    ///   folder so two coexisting portables don't clobber each other on the
    ///   same `~/.agentmux/agents/<slug>/` path (slugs are unique per Forge
    ///   DB, not globally; see `docs/specs/portable-agent-working-dirs.md`).
    /// - Installed: `~/.agentmux/` — preserves existing behavior.
    ///
    /// `AGENTMUX_DATA_HOME` env var, if set, overrides both.
    pub user_home_dir: Mutex<Option<String>>,

    /// Active cross-window drag session (at most one at a time).
    pub active_drag: Mutex<Option<DragSession>>,

    /// Embedded browser panes (native CefBrowserView per pane).
    pub browser_panes: crate::browser_panes::BrowserPaneManager,

    /// Browser DOM API state — CDP target cache + future connection
    /// pool. See `crate::browser_api`.
    pub browser_api: crate::browser_api::BrowserApiState,

    /// CEF remote debugging port (9223 dev / 9222 release). Populated
    /// by `main.rs` from the same `is_dev` branch that sets
    /// `Settings.remote_debugging_port`. Used by the browser DOM API
    /// (`/agentmux/browser/*`) to open CDP WebSocket connections to
    /// pane targets. See `docs/specs/SPEC_BROWSER_DOM_API.md` §6.
    pub debug_port: Mutex<u16>,

    // Phase B.1 removed `job_handle` (was Windows-only). Launcher
    // owns J0 wrapping srv now; host no longer needs its own job.
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            window_id_map: Mutex::new(HashMap::new()),
            auth_key: Mutex::new(uuid::Uuid::new_v4().to_string()),
            backend_endpoints: Mutex::new(BackendEndpoints::default()),
            sidecar_child: Mutex::new(None),
            backend_pid: Mutex::new(None),
            backend_started_at: Mutex::new(None),
            zoom_factor: Mutex::new(1.0),
            client_id: Mutex::new(None),
            window_id: Mutex::new(None),
            active_tab_id: Mutex::new(None),
            window_init_status: Mutex::new(String::new()),
            window_instance_registry: Mutex::new(WindowInstanceRegistry::new()),
            shadow_instance_registry: Mutex::new({
                // Pre-seed with main=1 to mirror the launcher's
                // pre-seeded `instance_registry` (B.5a). Without
                // this, the very first drift compare would log a
                // spurious mismatch for "main" before any event
                // arrives.
                let mut m = HashMap::new();
                m.insert("main".to_string(), 1);
                m
            }),
            cli_login_cancel: Mutex::new(None),
            cli_login_stdin: Mutex::new(None),
            ipc_port: Mutex::new(0),
            ipc_token: uuid::Uuid::new_v4().to_string(),
            browsers: Mutex::new(HashMap::new()),
            window_meta: Mutex::new(HashMap::new()),
            pending_window_labels: Mutex::new(VecDeque::new()),
            window_pool: Mutex::new(VecDeque::new()),
            unpromoted_pool_labels: Mutex::new(std::collections::HashSet::new()),
            window_pool_respawn_in_flight: std::sync::atomic::AtomicBool::new(false),
            version_data_dir: Mutex::new(None),
            version_config_dir: Mutex::new(None),
            user_home_dir: Mutex::new(None),
            active_drag: Mutex::new(None),
            browser_panes: crate::browser_panes::BrowserPaneManager::new(),
            browser_api: crate::browser_api::BrowserApiState::new(),
            debug_port: Mutex::new(0),
        }
    }
}
