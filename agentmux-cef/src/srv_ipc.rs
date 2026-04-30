// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2c.5a — host-side client for the srv reducer's named-pipe
// IPC server. Connects on host startup, sends `Register`, and runs
// a read loop that forwards every event to the renderer via
// `srv_event_bridge::dispatch_to_renderers`.
//
// This is the host-bridge half of E.2c.5; the renderer dispatcher
// half (the JS handler `window.__agentmux_srv_event`) lands as a
// separate frontend PR (E.2c.5b).
//
// Activated only when `AGENTMUX_SRV_PIPE_PATH` is set — the env var
// the launcher provides post-E.1b. Absent → host runs without the
// srv bridge; renderer falls back to the bespoke
// `waveobj:update` HTTP/WS path (still wired during the migration).
//
// Mirrors `launcher_ipc::connect_to_launcher` but slimmer:
//   * No outbound command channel (host doesn't issue srv commands
//     today; saga coordinator E.5+ adds the producer).
//   * No shadow state tracking (host's existing state shadow the
//     launcher reducer; srv events go straight through to the
//     renderer).
//
// Reconnect / resync semantics: B.3 launcher pattern leaves them
// for a follow-up; same here. If the srv pipe drops, we log and
// stop forwarding. Renderer falls back to the legacy HTTP/WS path
// until the host restarts. E.2c.5b/E.5 will tighten this once it
// matters (saga consumers can't tolerate dropped events).

use std::sync::Arc;

use agentmux_common::ipc::{ClientKind, Command, Event};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::ClientOptions;

/// Handle held by main.rs for the host's lifetime so the srv pipe
/// connection stays open. Dropping it closes the pipe.
#[cfg(target_os = "windows")]
pub struct SrvIpcHandle {
    #[allow(dead_code)]
    reader_task: tokio::task::JoinHandle<()>,
}

#[cfg(not(target_os = "windows"))]
pub struct SrvIpcHandle;

/// If `AGENTMUX_SRV_PIPE_PATH` is set, connect, Register as Host,
/// spawn a read loop that forwards srv events to all top-level
/// renderers. Returns a handle to keep the connection alive.
///
/// Errors are logged but non-fatal: a srv-IPC failure should NOT
/// prevent the host from running. The renderer continues using the
/// legacy `waveobj:update` HTTP/WS path until the connection comes
/// back at the next host start.
#[cfg(target_os = "windows")]
pub async fn connect_to_srv(
    state: std::sync::Arc<crate::state::AppState>,
) -> Option<SrvIpcHandle> {
    let pipe_path = match std::env::var("AGENTMUX_SRV_PIPE_PATH") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            tracing::info!(
                "AGENTMUX_SRV_PIPE_PATH unset — running without srv IPC bridge (dev mode)"
            );
            return None;
        }
    };

    let client = match ClientOptions::new().open(&pipe_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                "[srv-ipc] failed to open {}: {} — continuing without srv bridge",
                pipe_path,
                e
            );
            return None;
        }
    };
    tracing::info!("[srv-ipc] connected to {}", pipe_path);

    let (read_half, mut write_half) = tokio::io::split(client);

    // Send Register FIRST (server enforces register-first; violation
    // is a fatal close).
    let register = Command::Register {
        kind: ClientKind::Host,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let mut buf = match serde_json::to_vec(&register) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("[srv-ipc] failed to serialize Register: {}", e);
            return None;
        }
    };
    buf.push(b'\n');
    if let Err(e) = write_half.write_all(&buf).await {
        tracing::error!("[srv-ipc] failed to send Register: {} — bailing", e);
        return None;
    }
    if let Err(e) = write_half.flush().await {
        tracing::error!("[srv-ipc] failed to flush Register: {} — bailing", e);
        return None;
    }

    // Read loop: parse newline-delimited Events and forward each to
    // every top-level renderer.
    let state_for_reader = Arc::clone(&state);
    let reader_task = tokio::spawn(async move {
        let reader = BufReader::new(read_half);
        let mut lines = reader.lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if line.trim().is_empty() => continue,
                Ok(Some(line)) => match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        crate::srv_event_bridge::dispatch_to_renderers(&state_for_reader, &event);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[srv-ipc] could not parse event line ({}): {}",
                            e,
                            line
                        );
                    }
                },
                Ok(None) => {
                    tracing::info!("[srv-ipc] srv pipe EOF — connection closed");
                    return;
                }
                Err(e) => {
                    tracing::warn!("[srv-ipc] read error: {}", e);
                    return;
                }
            }
        }
    });

    // Keep the writer alive (Host doesn't send Commands today; saga
    // coordinator E.5+ adds the producer). For now the writer is
    // moved into a background task that just holds it open; dropping
    // would close the pipe and trigger Goodbye on the server.
    let _writer_keepalive = tokio::spawn(async move {
        // No-op: just owns write_half so it isn't dropped.
        let _ = write_half;
        std::future::pending::<()>().await;
    });

    Some(SrvIpcHandle { reader_task })
}

#[cfg(not(target_os = "windows"))]
pub async fn connect_to_srv(
    _state: std::sync::Arc<crate::state::AppState>,
) -> Option<SrvIpcHandle> {
    None
}
