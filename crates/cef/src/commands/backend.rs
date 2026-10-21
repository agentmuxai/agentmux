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
pub fn get_mux_init_opts(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
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
    let pending_migrations = *state.pending_migrations.lock();

    serde_json::json!({
        "pid": pid,
        "started_at": started_at,
        "web_endpoint": endpoints.web_endpoint,
        "version": current_version,
        "pending_migrations": pending_migrations,
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

/// `fe_log_batch`: the page's console lines, batched by `frontend/log/log-pipe.ts`
/// so logging never floods the IPC connections; each entry is written exactly
/// as `fe_log_structured` writes one.
pub fn fe_log_batch(args: &serde_json::Value) -> serde_json::Value {
    if let Some(entries) = args.get("entries").and_then(|v| v.as_array()) {
        for entry in entries {
            fe_log_structured(entry);
        }
    }
    serde_json::Value::Null
}

/// Restart the agentmux-srv backend sidecar.
///
/// Phase B.1: in launcher-managed runs (`AGENTMUX_BACKEND_PID` env
/// is set by the launcher), the host does NOT own the srv child
/// handle — `state.sidecar_child` stays None. Naively running the
/// kill-then-spawn flow here would skip killing the launcher's srv
/// (no handle to kill) and spawn a SECOND srv touching the same
/// data dir, corrupting state. Refuse with a clear message until
/// Phase B.2 wires a Quit command from host to launcher to do the
/// restart cleanly (#571). The launcher-managed adoption path is
/// sidecar.rs's `use_launcher_endpoints`.
pub async fn restart_backend(state: Arc<AppState>) -> Result<serde_json::Value, String> {
    tracing::info!("[restart_backend] user-initiated restart");

    if std::env::var("AGENTMUX_BACKEND_PID").is_ok() {
        let msg = "backend lifecycle is owned by the launcher in this run \
                   (AGENTMUX_BACKEND_PID set); host-initiated restart is \
                   disabled until Phase B.2 wires the launcher RPC. \
                   Restart the entire app to get a fresh srv.";
        tracing::warn!("[restart_backend] refused: {}", msg);
        return Err(msg.to_string());
    }

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

    // Re-register this host's UI-automation credentials with the FRESH srv
    // instance — its AppState::host_ipc starts as None, and the only other
    // registration call happens once, at initial host startup (lib.rs), so
    // without this, UIScreenshot/UIClick/UIQuery would 503 for the rest of
    // the host process's lifetime after any backend restart (reagent P2,
    // PR #2662, 2026-08-19).
    {
        let web_endpoint = result.web_endpoint.clone();
        let auth_key = state.auth_key.lock().clone();
        let ipc_port = *state.ipc_port.lock();
        let ipc_token = state.ipc_token.clone();
        let host_reg_secret = state.host_reg_secret.lock().clone();
        tokio::task::spawn_blocking(move || {
            crate::client::register_ipc_with_backend(&web_endpoint, &auth_key, ipc_port, &ipc_token, &host_reg_secret);
        });
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
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .to_string();
    tracing::debug!("set_window_init_status status={} label={}", status, label);
    // A promoted pool window's content revealed (its splash started fading).
    // Not an init status: it only releases the deferred pane-pool refill.
    if status == "revealed" {
        crate::commands::pane_pool::on_window_revealed(state, &label);
        return serde_json::Value::Null;
    }
    // A promoted floater put its tear-off snapshot up; the host shows it.
    if status == "snapshot-shown" {
        crate::commands::pane_pool::on_snapshot_shown(&label);
        return serde_json::Value::Null;
    }
    if status == "wave-ready" && label == "main" {
        crate::commands::startup_pools::on_main_window_ready(state);
    }
    *state.window_init_status.lock() = status.to_string();
    // Capture HWND once the window is fully shown (CEF Views returns NULL at
    // on_after_created time; the renderer-ready callback is the earliest safe moment).
    #[cfg(target_os = "windows")]
    if status == "ready" {
        crate::commands::window::capture_hwnd_for_label(state, &label);
    }
    serde_json::Value::Null
}

/// First-paint signal from the frontend (startup white-flash fix, see
/// docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md and
/// docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md).
/// Sent via a double-`requestAnimationFrame` at the very top of
/// `bootstrap.ts` — the earliest reliable proxy for "the compositor actually
/// presented a frame", as opposed to CEF's `on_load_end` which only means
/// "main-frame HTML finished loading" and can fire before anything has
/// visually painted.
///
/// On Linux and Windows this unblocks the window `on_load_end` deferred (see
/// `client::navigation::reveal_gated_window`) — Windows ported 2026-09-03,
/// same mechanism, not a separate implementation. On macOS it's currently
/// just logged for telemetry — not gated on it; see that report's §6 for why
/// macOS is deliberately not included in this pass.
pub fn report_first_paint(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .to_string();
    tracing::info!(
        target: "startup-paint",
        label = %label,
        "[startup-paint] frontend reported first paint"
    );
    #[cfg(target_os = "windows")]
    crate::floating_pane::reveal::on_first_paint(&label);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    crate::client::navigation::on_frontend_first_paint(state.clone(), label);
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = state;
    serde_json::Value::Null
}
