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

/// Run pending data migrations by spawning `agentmux-srv migrate`.
///
/// Called from the upgrade panel when `pending_migrations > 0`. Blocks until
/// the migration subprocess exits. On success, updates `state.pending_migrations`
/// to 0. The host process inherits all AGENTMUX_* env vars from the launcher,
/// so no extra env configuration is needed when forwarding them to the subprocess.
pub async fn run_migrations(state: Arc<AppState>) -> Result<serde_json::Value, String> {
    tracing::info!("[run_migrations] user-initiated migration run");

    let pending = *state.pending_migrations.lock();
    if pending == 0 {
        tracing::info!("[run_migrations] no pending migrations — nothing to do");
        return Ok(serde_json::json!({ "applied": 0, "already_current": true }));
    }

    let data_dir = state.version_data_dir.lock().clone()
        .ok_or_else(|| "run_migrations: data dir not yet resolved".to_string())?;

    let backend_path = crate::sidecar::resolve_srv_binary()?;

    let mut cmd = tokio::process::Command::new(&backend_path);
    cmd.args([
        "--wavedata",
        &data_dir,
        "migrate",
    ])
    // Inherit the current process env — the launcher already populated
    // every AGENTMUX_* var we need (AGENTMUX_DATA_DIR, AGENTMUX_SHARED_DIR, etc.).
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("run_migrations: spawn failed: {}", e))?;

    // Drain stderr to log.
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                tracing::info!("[migrate stderr] {}", line);
            }
        });
    }

    // Read stdout for the {"event":"complete"} line.
    let mut applied = 0u32;
    let mut complete = false;
    if let Some(stdout) = child.stdout.take() {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::info!("[migrate] {}", line);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if v.get("event").and_then(|e| e.as_str()) == Some("complete") {
                    applied = v.get("applied").and_then(|a| a.as_u64()).unwrap_or(0) as u32;
                    complete = true;
                    break;
                }
            }
        }
    }

    let _ = child.start_kill();
    let status = child.wait().await
        .map_err(|e| format!("run_migrations: wait failed: {}", e))?;

    if complete || status.success() {
        *state.pending_migrations.lock() = 0;
        tracing::info!("[run_migrations] complete: applied={}", applied);
        Ok(serde_json::json!({ "applied": applied, "already_current": false }))
    } else {
        Err(format!("run_migrations: srv migrate exited with status {}", status))
    }
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
