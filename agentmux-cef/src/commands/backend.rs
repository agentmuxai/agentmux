// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Backend/sidecar management commands for the CEF host.
// Ported from src-tauri/src/commands/backend.rs.

use std::sync::Arc;

use tokio::io::AsyncBufReadExt as _;

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

/// Restart the agentmux-srv backend sidecar.
///
/// Phase B.1: in launcher-managed runs (`AGENTMUX_BACKEND_PID` env
/// is set by the launcher), the host does NOT own the srv child
/// handle — `state.sidecar_child` stays None. Naively running the
/// kill-then-spawn flow here would skip killing the launcher's srv
/// (no handle to kill) and spawn a SECOND srv touching the same
/// data dir, corrupting state. Refuse with a clear message until
/// Phase B.2 wires a Quit command from host to launcher to do the
/// restart cleanly. (codex P2 @ sidecar.rs:58, PR #571 round-3.)
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

/// Trigger an on-demand migration run via `agentmux-srv … migrate`.
///
/// Returns `{"started": true}` immediately; progress events are pushed to the
/// frontend as `upgrade:migration-event` / `upgrade:migrations-complete` /
/// `upgrade:migrations-failed` CEF events so the Maintenance panel can render
/// a live stage list without polling.
pub async fn run_migrations(state: Arc<AppState>) -> Result<serde_json::Value, String> {
    // Guard: only one run at a time.
    {
        let mut running = state.migration_running.lock();
        if *running {
            return Err("Migration already in progress".to_string());
        }
        *running = true;
    }

    let data_dir = state.version_data_dir.lock().clone().ok_or_else(|| {
        *state.migration_running.lock() = false;
        "Data directory not initialized".to_string()
    })?;

    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    let srv_path = crate::sidecar::resolve_backend_binary_pub("agentmux-srv", exe_suffix)
        .map_err(|e| {
            *state.migration_running.lock() = false;
            e
        })?;

    tokio::spawn(async move {
        if let Err(e) = run_migrations_inner(state.clone(), srv_path, data_dir).await {
            tracing::error!("[run_migrations] subprocess error: {}", e);
            crate::events::emit_event_from_state(
                &state,
                "upgrade:migrations-failed",
                &serde_json::json!({ "error": e, "failedId": null }),
            );
        }
        *state.migration_running.lock() = false;
    });

    Ok(serde_json::json!({ "started": true }))
}

async fn run_migrations_inner(
    state: Arc<AppState>,
    srv_path: std::path::PathBuf,
    data_dir: String,
) -> Result<(), String> {
    use tokio::process::Command;
    use tokio::io::BufReader;

    let mut child = Command::new(&srv_path)
        .arg("--wavedata")
        .arg(&data_dir)
        .arg("migrate")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn agentmux-srv migrate: {}", e))?;

    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut applied = 0usize;
    let mut last_error: Option<String> = None;
    let mut failed_id: Option<String> = None;

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let val: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let event_kind = val.get("event").and_then(|v| v.as_str()).unwrap_or("");
        match event_kind {
            "migration_start" => {
                crate::events::emit_event_from_state(
                    &state,
                    "upgrade:migration-event",
                    &serde_json::json!({
                        "kind": "start",
                        "id": val.get("id"),
                        "label": val.get("description"),
                    }),
                );
            }
            "migration_done" => {
                crate::events::emit_event_from_state(
                    &state,
                    "upgrade:migration-event",
                    &serde_json::json!({
                        "kind": "done",
                        "id": val.get("id"),
                        "duration_ms": val.get("duration_ms"),
                    }),
                );
                applied += 1;
            }
            "complete" => {
                let a = val.get("applied").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                applied = a;
                crate::events::emit_event_from_state(
                    &state,
                    "upgrade:migration-event",
                    &serde_json::json!({
                        "kind": "complete",
                        "applied": a,
                        "skipped": val.get("skipped"),
                    }),
                );
            }
            _ => {}
        }
    }

    let status = child.wait().await.map_err(|e| format!("wait failed: {}", e))?;

    if status.success() && last_error.is_none() {
        *state.pending_migrations.lock() = 0;
        crate::events::emit_event_from_state(
            &state,
            "upgrade:migrations-complete",
            &serde_json::json!({ "applied": applied }),
        );
        Ok(())
    } else {
        let err = last_error.unwrap_or_else(|| {
            format!("agentmux-srv migrate exited with code {:?}", status.code())
        });
        crate::events::emit_event_from_state(
            &state,
            "upgrade:migrations-failed",
            &serde_json::json!({ "error": err, "failedId": failed_id }),
        );
        Err(err)
    }
}

/// Trigger an on-demand saga log vacuum.
///
/// Stubbed: the `agentmux-srv saga-vacuum` subcommand is not yet implemented.
/// Returns `{"rows_deleted": 0}` and emits `upgrade:saga-vacuum-done` so the
/// Maintenance panel's vacuum state machine completes correctly.
pub async fn run_saga_vacuum(state: Arc<AppState>) -> Result<serde_json::Value, String> {
    // TODO: spawn `agentmux-srv --wavedata <path> saga-vacuum` once the subcommand exists.
    tracing::info!("[run_saga_vacuum] stub — returning rows_deleted=0");
    crate::events::emit_event_from_state(
        &state,
        "upgrade:saga-vacuum-done",
        &serde_json::json!({ "rows_deleted": 0 }),
    );
    Ok(serde_json::json!({ "rows_deleted": 0 }))
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
    *state.window_init_status.lock() = status.to_string();
    // Capture HWND once the window is fully shown (CEF Views returns NULL at
    // on_after_created time; the renderer-ready callback is the earliest safe moment).
    #[cfg(target_os = "windows")]
    if status == "ready" {
        crate::commands::window::capture_hwnd_for_label(state, &label);
    }
    serde_json::Value::Null
}
