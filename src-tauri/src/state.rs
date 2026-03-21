use std::collections::HashMap;
use std::sync::Mutex;

/// Tracks a stable sequential instance number for each open window.
/// Main window is always 1. Additional windows get 2, 3, … in creation order.
/// Numbers are never reused within a session.
pub struct WindowInstanceRegistry {
    instances: HashMap<String, u32>,
    next_num: u32,
}

impl WindowInstanceRegistry {
    pub fn new() -> Self {
        let mut instances = HashMap::new();
        instances.insert("main".to_string(), 1);
        Self { instances, next_num: 2 }
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

/// Type of drag item being moved across windows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragType {
    Pane,
    Tab,
}

/// Payload carried by a cross-window drag session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
}

/// Active cross-window drag session state.
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

/// Shared application state managed by Tauri.
/// Replaces the scattered state across emain/*.ts files.
pub struct AppState {
    /// Auth key for backend communication (replaces emain/authkey.ts)
    /// Can be updated after querying database for existing client
    pub auth_key: Mutex<String>,

    /// Backend (agentmuxsrv-rs) connection endpoints
    pub backend_endpoints: Mutex<BackendEndpoints>,

    /// Handle to the sidecar child process for graceful shutdown
    pub sidecar_child: Mutex<Option<tauri_plugin_shell::process::CommandChild>>,

    /// Backend process PID (set after spawn)
    pub backend_pid: Mutex<Option<u32>>,

    /// Backend process start time as ISO 8601 string
    pub backend_started_at: Mutex<Option<String>>,

    /// Current zoom factor (replaces Electron's webContents zoom)
    pub zoom_factor: Mutex<f64>,

    /// Client ID (replaces Electron's clientData tracking)
    /// Set after querying backend on startup
    pub client_id: Mutex<Option<String>>,

    /// Window ID (replaces Electron's window tracking)
    /// Set after querying/creating window via backend
    pub window_id: Mutex<Option<String>>,

    /// Active tab ID (replaces Electron's tab tracking)
    /// Set after querying/creating default tab via backend
    pub active_tab_id: Mutex<Option<String>>,

    /// Window initialization status ("ready" or "wave-ready")
    pub window_init_status: Mutex<String>,

    /// Sequential instance numbers for each open window.
    pub window_instance_registry: Mutex<WindowInstanceRegistry>,

    /// Active cross-window drag session.
    /// Set when a drag leaves the source window and enters cross-window mode.
    pub active_drag: Mutex<Option<DragSession>>,

    /// Cancellation channel for an in-progress CLI login process.
    /// Sending on this channel signals the background task to kill the child.
    pub cli_login_cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,

    /// Windows Job Object handle — keeps backend alive until frontend exits.
    /// When this handle is closed (including on crash), Windows kills all assigned processes.
    #[cfg(target_os = "windows")]
    pub job_handle: Mutex<Option<JobHandle>>,
}

#[derive(Default, Clone, serde::Serialize)]
pub struct BackendEndpoints {
    pub ws_endpoint: String,
    pub web_endpoint: String,
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
            active_drag: Mutex::new(None),
            cli_login_cancel: Mutex::new(None),
            #[cfg(target_os = "windows")]
            job_handle: Mutex::new(None),
        }
    }
}
