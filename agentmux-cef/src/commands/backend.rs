// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Backend/sidecar management commands for the CEF host.
// Ported from src-tauri/src/commands/backend.rs.

use std::sync::Arc;

use crate::state::AppState;

/// Get the backend WebSocket and HTTP endpoints.
pub fn get_backend_endpoints(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let endpoints = state.backend_endpoints.lock();

    if endpoints.ws_endpoint.is_empty() {
        return Err("Backend not ready yet".to_string());
    }

    Ok(serde_json::json!({
        "ws": endpoints.ws_endpoint,
        "web": endpoints.web_endpoint,
    }))
}

/// Get the window initialization options (client/window/tab IDs).
pub fn get_wave_init_opts(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let client_id = state.client_id.lock();
    let window_id = state.window_id.lock();
    let tab_id = state.active_tab_id.lock();

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
pub fn get_backend_info(state: &Arc<AppState>) -> serde_json::Value {
    let current_version = env!("CARGO_PKG_VERSION");
    let endpoints = state.backend_endpoints.lock();
    let pid = *state.backend_pid.lock();
    let started_at = state.backend_started_at.lock().clone();

    serde_json::json!({
        "pid": pid,
        "started_at": started_at,
        "web_endpoint": endpoints.web_endpoint,
        "version": current_version,
    })
}

/// Log a message from the frontend.
pub fn fe_log(args: &serde_json::Value) -> serde_json::Value {
    let msg = args
        .get("msg")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    tracing::info!("[frontend] {}", msg);
    serde_json::Value::Null
}

/// Structured log from the frontend.
pub fn fe_log_structured(args: &serde_json::Value) -> serde_json::Value {
    let level = args.get("level").and_then(|v| v.as_str()).unwrap_or("info");
    let module = args.get("module").and_then(|v| v.as_str()).unwrap_or("unknown");
    let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let data = args.get("data");

    match level {
        "error" => tracing::error!(module = %module, data = ?data, "[fe] {}", message),
        "warn" => tracing::warn!(module = %module, data = ?data, "[fe] {}", message),
        "debug" => tracing::debug!(module = %module, data = ?data, "[fe] {}", message),
        _ => tracing::info!(module = %module, data = ?data, "[fe] {}", message),
    }
    serde_json::Value::Null
}

/// Restart the agentmuxsrv-rs backend sidecar.
pub async fn restart_backend(state: Arc<AppState>) -> Result<serde_json::Value, String> {
    tracing::info!("[restart_backend] user-initiated restart");

    // Kill existing sidecar if still alive
    {
        let mut sidecar = state.sidecar_child.lock();
        if let Some(ref mut child) = *sidecar {
            let _ = child.kill();
            tracing::info!("[restart_backend] killed stale sidecar");
        }
        *sidecar = None;
    }

    // Small delay to let the OS release the port
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Spawn fresh backend
    let result = crate::sidecar::spawn_backend(&state).await?;

    // Update stored endpoints
    {
        let mut endpoints = state.backend_endpoints.lock();
        endpoints.ws_endpoint = result.ws_endpoint.clone();
        endpoints.web_endpoint = result.web_endpoint.clone();
    }

    // Emit backend-ready event
    let payload = serde_json::json!({
        "ws": result.ws_endpoint,
        "web": result.web_endpoint,
    });
    crate::events::emit_event_from_state(&state, "backend-ready", &payload);

    tracing::info!(
        "[restart_backend] backend restarted: ws={} web={}",
        result.ws_endpoint,
        result.web_endpoint
    );

    Ok(serde_json::Value::Null)
}

/// Set the window initialization status.
pub fn set_window_init_status(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    tracing::debug!("set_window_init_status status={}", status);
    *state.window_init_status.lock() = status.to_string();
    serde_json::Value::Null
}
