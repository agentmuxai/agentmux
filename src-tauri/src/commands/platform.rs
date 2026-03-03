use tauri::Manager;

use crate::state::AppState;

/// Get the current OS platform name.
/// Replaces: ipcMain.on("get-platform") in emain/platform.ts
#[tauri::command]
pub fn get_platform() -> String {
    match std::env::consts::OS {
        "macos" => "darwin".to_string(),
        other => other.to_string(),
    }
}

/// Get the current user's username.
/// Replaces: ipcMain.on("get-user-name") in emain/platform.ts
#[tauri::command]
pub fn get_user_name() -> String {
    whoami::username()
}

/// Get the system hostname.
/// Replaces: ipcMain.on("get-host-name") in emain/platform.ts
#[tauri::command]
pub fn get_host_name() -> String {
    whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string())
}

/// Check if running in development mode.
/// Replaces: ipcMain.on("get-is-dev") in emain/platform.ts
#[tauri::command]
pub fn get_is_dev() -> bool {
    cfg!(debug_assertions)
}

/// Get the app data directory path.
/// Replaces: ipcMain.on("get-data-dir") in emain/platform.ts
#[tauri::command]
pub fn get_data_dir(app: tauri::AppHandle) -> Result<String, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to get data dir: {}", e))
}

/// Get the app config directory path.
/// Replaces: ipcMain.on("get-config-dir") in emain/platform.ts
#[tauri::command]
pub fn get_config_dir(app: tauri::AppHandle) -> Result<String, String> {
    app.path()
        .app_config_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to get config dir: {}", e))
}

/// Get an environment variable value.
/// Replaces: ipcMain.on("get-env") in emain/emain.ts
#[tauri::command]
pub fn get_env(key: String) -> Option<String> {
    std::env::var(&key).ok()
}

/// Get details for the About modal.
/// Replaces: ipcMain.on("get-about-modal-details") in emain/emain.ts
#[tauri::command]
pub fn get_about_modal_details(app: tauri::AppHandle) -> serde_json::Value {
    let version = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    // Get backend version from state if available
    let state = app.state::<AppState>();
    let endpoints = state.backend_endpoints.lock().unwrap();

    serde_json::json!({
        "version": version,
        "buildTime": env!("CARGO_PKG_VERSION"),
        "platform": get_platform(),
        "arch": std::env::consts::ARCH,
        "backendEndpoints": {
            "ws": endpoints.ws_endpoint,
            "web": endpoints.web_endpoint,
        }
    })
}

/// Get the documentation site URL.
/// Replaces: ipcMain.on("get-docsite-url") in emain/docsite.ts
#[tauri::command]
pub fn get_docsite_url(app: tauri::AppHandle) -> String {
    let state = app.state::<AppState>();
    let endpoints = state.backend_endpoints.lock().unwrap();

    if !endpoints.web_endpoint.is_empty() {
        // Try embedded docsite first
        format!("http://{}/docsite/", endpoints.web_endpoint)
    } else {
        // Fallback to public docs
        "https://docs.agentmux.ai".to_string()
    }
}
