use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_shell::process::{CommandEvent, TerminatedPayload};

/// Parsed fields from the `WAVESRV-ESTART` line written by the backend on ready.
pub struct EStartPayload {
    pub ws: String,
    pub web: String,
    pub version: String,
    pub instance_id: String,
}

/// Drive the `CommandEvent` stream from a spawned backend process.
///
/// - Relays stderr lines to the host log as `[agentmuxsrv-rs] {line}`
/// - Parses `WAVESRV-ESTART` and sends the parsed payload on `endpoint_tx` once
/// - Parses `WAVESRV-EVENT:` and forwards to the frontend via Tauri event
/// - On `Terminated`: logs startup vs runtime crash, emits `backend-terminated`
///   to **all** open windows, then returns
///
/// This function is called inside a `tokio::spawn` and runs for the lifetime
/// of the backend process.
pub async fn run(
    mut rx: tokio::sync::mpsc::Receiver<CommandEvent>,
    app_handle: tauri::AppHandle,
    endpoint_tx: tokio::sync::mpsc::Sender<EStartPayload>,
) {
    let mut estart_received = false;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stderr(line) => {
                let line = String::from_utf8_lossy(&line);
                for l in line.lines() {
                    if l.starts_with("WAVESRV-ESTART") {
                        let payload = parse_estart(l);
                        tracing::info!(
                            "Backend started: ws={} web={} version={} instance={}",
                            payload.ws, payload.web, payload.version, payload.instance_id
                        );
                        estart_received = true;
                        let _ = endpoint_tx.send(payload).await;
                    } else if let Some(event_data) = l.strip_prefix("WAVESRV-EVENT:") {
                        super::handle_backend_event(&app_handle, event_data);
                    } else {
                        tracing::info!("[agentmuxsrv-rs] {}", l);
                    }
                }
            }
            CommandEvent::Stdout(line) => {
                let line = String::from_utf8_lossy(&line);
                tracing::info!("[agentmuxsrv-rs stdout] {}", line.trim());
            }
            CommandEvent::Error(err) => {
                tracing::error!("[agentmuxsrv-rs error] {}", err);
            }
            CommandEvent::Terminated(status) => {
                emit_terminated(&app_handle, &status, estart_received);
                break;
            }
            _ => {}
        }
    }
}

/// Parse the key=value fields out of a `WAVESRV-ESTART` line.
fn parse_estart(line: &str) -> EStartPayload {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let get = |prefix: &str| -> String {
        parts
            .iter()
            .find_map(|p| p.strip_prefix(prefix))
            .unwrap_or_default()
            .to_string()
    };
    EStartPayload {
        ws: get("ws:"),
        web: get("web:"),
        version: get("version:"),
        instance_id: get("instance:"),
    }
}

/// Log the termination event and broadcast `backend-terminated` to all windows.
///
/// The `estart_received` flag distinguishes two crash kinds for diagnostics:
/// - **STARTUP CRASH** — backend exited before writing WAVESRV-ESTART (DB open
///   failure, port bind failure, etc.).  Exit code 1 is typical here.
/// - **RUNTIME CRASH** — backend ran successfully but terminated later.
///   Exit code `-1073740791` (0xC0000409, Windows fast-fail/abort) is typical
///   after long sessions (OOM, deadlock, unhandled panic).
fn emit_terminated(
    app: &tauri::AppHandle,
    status: &TerminatedPayload,
    estart_received: bool,
) {
    let state = app.state::<crate::state::AppState>();
    let pid = state.backend_pid.lock().unwrap().unwrap_or(0);
    let started_at = state.backend_started_at.lock().unwrap().clone();
    let uptime_secs = uptime_from_started_at(started_at.as_deref());

    if estart_received {
        tracing::error!(
            "[agentmuxsrv-rs] RUNTIME CRASH — pid={} exit_code={:?} signal={:?} uptime_secs={:?}",
            pid, status.code, status.signal, uptime_secs
        );
    } else {
        tracing::error!(
            "[agentmuxsrv-rs] STARTUP CRASH — terminated before ready (pid={} exit_code={:?} uptime_secs={:?})",
            pid, status.code, uptime_secs
        );
    }

    let payload = serde_json::json!({
        "code":       status.code,
        "signal":     status.signal,
        "pid":        pid,
        "uptime_secs": uptime_secs,
    });

    // Broadcast to all open windows so secondary windows also transition to Offline.
    for window in app.webview_windows().values() {
        let _ = window.emit("backend-terminated", &payload);
    }
}

/// Compute elapsed seconds since the ISO 8601 `started_at` timestamp.
/// Returns `None` if the timestamp is missing or unparseable.
fn uptime_from_started_at(started_at: Option<&str>) -> Option<i64> {
    started_at.and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| {
            (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds()
        })
    })
}
