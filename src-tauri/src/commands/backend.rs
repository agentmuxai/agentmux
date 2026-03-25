use crate::state::AppState;
use tauri::{Emitter, Manager};

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
    let pid = state.backend_pid.lock().unwrap().clone();
    let started_at = state.backend_started_at.lock().unwrap().clone();

    serde_json::json!({
        "pid": pid,
        "started_at": started_at,
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

/// Restart the agentmuxsrv-rs backend sidecar.
///
/// Kills any existing sidecar, waits 500 ms for the OS to release the port,
/// spawns a fresh one using the same binary/env logic as the initial launch,
/// updates the stored endpoints in `AppState`, and broadcasts `backend-ready`
/// to every open window so all frontends reconnect.
///
/// Returns the new endpoints on success; the frontend ignores the return value
/// and relies on the `backend-ready` Tauri event to trigger reconnect.
#[tauri::command]
pub async fn restart_backend(app: tauri::AppHandle) -> Result<(), String> {
    tracing::info!("[restart_backend] user-initiated restart");

    // Kill existing sidecar if still alive
    {
        let state = app.state::<AppState>();
        let mut sidecar = state.sidecar_child.lock().unwrap();
        if let Some(child) = sidecar.take() {
            let _ = child.kill();
            tracing::info!("[restart_backend] killed stale sidecar");
        }
    }

    // Small delay — lets the OS release the port before the new process binds it
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Spawn fresh (runs binary resolution, wsh deploy, env vars, ESTART wait)
    let result = crate::sidecar::spawn_backend(&app).await?;

    // Update stored endpoints
    {
        let state = app.state::<AppState>();
        let mut endpoints = state.backend_endpoints.lock().unwrap();
        endpoints.ws_endpoint = result.ws_endpoint.clone();
        endpoints.web_endpoint = result.web_endpoint.clone();
    }

    // Broadcast to ALL windows — secondary windows also need to reconnect
    let payload = serde_json::json!({
        "ws":  result.ws_endpoint,
        "web": result.web_endpoint,
    });
    for window in app.webview_windows().values() {
        let _ = window.emit("backend-ready", &payload);
    }

    tracing::info!(
        "[restart_backend] backend restarted: ws={} web={}",
        result.ws_endpoint, result.web_endpoint
    );

    Ok(())
}

/// Structured log from the frontend with level, module, message, and optional data.
/// Persisted to the host log file for post-mortem debugging.
#[tauri::command]
pub fn fe_log_structured(level: String, module: String, message: String, data: Option<serde_json::Value>) {
    match level.as_str() {
        "error" => tracing::error!(module = %module, data = ?data, "[fe] {}", message),
        "warn"  => tracing::warn!(module = %module, data = ?data, "[fe] {}", message),
        "debug" => tracing::debug!(module = %module, data = ?data, "[fe] {}", message),
        _       => tracing::info!(module = %module, data = ?data, "[fe] {}", message),
    }
}
