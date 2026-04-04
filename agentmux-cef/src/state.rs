// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared application state for the CEF host.

use std::collections::HashMap;
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

/// Wrapper for a Windows HANDLE that is Send + Sync.
/// Windows HANDLEs are safe to use from any thread.
#[cfg(target_os = "windows")]
pub struct JobHandle(*mut std::ffi::c_void);

#[cfg(target_os = "windows")]
unsafe impl Send for JobHandle {}
#[cfg(target_os = "windows")]
unsafe impl Sync for JobHandle {}

#[cfg(target_os = "windows")]
impl JobHandle {
    pub fn new(handle: *mut std::ffi::c_void) -> Self {
        Self(handle)
    }
}

#[cfg(target_os = "windows")]
impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

/// Backend (agentmux-srv) connection endpoints.
#[derive(Default, Clone, serde::Serialize)]
pub struct BackendEndpoints {
    pub ws_endpoint: String,
    pub web_endpoint: String,
}

/// Shared application state for the CEF host.
///
/// Unlike the Tauri version, this uses `Arc<AppState>` directly instead of
/// `tauri::State<AppState>`. The sidecar child is `std::process::Child` instead
/// of `tauri_plugin_shell::process::CommandChild`.
pub struct AppState {
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

    /// Cancellation channel for an in-progress CLI login process
    pub cli_login_cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,

    /// IPC HTTP server port
    pub ipc_port: Mutex<u16>,

    /// IPC bearer token — injected into the page alongside the port.
    /// Verified on every IPC request to prevent unauthorized local access.
    pub ipc_token: String,

    /// CEF Browser handles keyed by window label (multi-window support).
    /// "main" is the primary window; tear-off windows get "window-{UUID}" labels.
    pub browsers: Mutex<HashMap<String, Browser>>,

    /// Version-specific data directory (e.g. ai.agentmux.cef.v0-32-111/)
    pub version_data_dir: Mutex<Option<String>>,

    /// Version-specific config directory
    pub version_config_dir: Mutex<Option<String>>,

    /// Active cross-window drag session (at most one at a time).
    pub active_drag: Mutex<Option<DragSession>>,

    /// Windows Job Object handle -- keeps backend alive until frontend exits
    #[cfg(target_os = "windows")]
    pub job_handle: Mutex<Option<JobHandle>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
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
            cli_login_cancel: Mutex::new(None),
            ipc_port: Mutex::new(0),
            ipc_token: uuid::Uuid::new_v4().to_string(),
            browsers: Mutex::new(HashMap::new()),
            version_data_dir: Mutex::new(None),
            version_config_dir: Mutex::new(None),
            active_drag: Mutex::new(None),
            #[cfg(target_os = "windows")]
            job_handle: Mutex::new(None),
        }
    }
}
