use crate::state::AppState;

/// Get the backend WebSocket and HTTP endpoints.
///
/// The frontend uses these to establish its WebSocket RPC connection
/// and make HTTP requests to the Go backend.
///
/// This is a new command (no Electron equivalent) -- in Electron,
/// the frontend received these via the wave-init event pushed
/// from the main process.
#[tauri::command]
pub fn get_backend_endpoints(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let endpoints = state.backend_endpoints.lock().unwrap();

    if endpoints.ws_endpoint.is_empty() {
        return Err("Backend not ready yet".to_string());
    }

    Ok(serde_json::json!({
        "ws": endpoints.ws_endpoint,
        "web": endpoints.web_endpoint,
    }))
}

/// Get the window initialization options (client/window/tab IDs).
/// Used by the frontend to initialize the Wave application.
#[tauri::command]
pub fn get_wave_init_opts(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let client_id = state.client_id.lock().unwrap();
    let window_id = state.window_id.lock().unwrap();
    let tab_id = state.active_tab_id.lock().unwrap();

    if client_id.is_none() || window_id.is_none() || tab_id.is_none() {
        return Err("Window state not initialized yet".to_string());
    }

    Ok(serde_json::json!({
        "clientId": client_id.as_ref().unwrap(),
        "windowId": window_id.as_ref().unwrap(),
        "tabId": tab_id.as_ref().unwrap(),
        "activate": true,
        "primaryTabStartup": true,
    }))
}

/// Get backend process info for the status bar popover.
#[tauri::command]
pub fn get_backend_info(state: tauri::State<'_, AppState>) -> serde_json::Value {
    let current_version = env!("CARGO_PKG_VERSION");
    let endpoints = state.backend_endpoints.lock().unwrap();

    serde_json::json!({
        "web_endpoint": endpoints.web_endpoint,
        "version": current_version,
    })
}

/// Log a message from the frontend.
/// Replaces: ipcMain.on("fe-log") in emain/emain.ts
#[tauri::command]
pub fn fe_log(msg: String) {
    tracing::info!("[frontend] {}", msg);
}
