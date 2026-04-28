// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Named-pipe IPC server — accept loop + per-connection handler.
//
// Tokio's `NamedPipeServer` lets us treat each accepted instance as
// an independent bidirectional stream. We run one accept-loop task
// that creates a fresh `NamedPipeServer` after each connection (so
// the next client has something to connect to) and spawns a
// per-connection task to drive it.
//
// Wire framing: newline-delimited JSON. Reads use `BufReader::lines()`,
// writes use `write_all` + `\n`. Robust to partial reads since `lines`
// already buffers until `\n`. Strict policy: `Register` MUST be the
// first message; subsequent `Register` is ignored with an error.
//
// What this commit does NOT do:
//   * Forward Commands to a reducer (no reducer yet — B.3).
//   * Emit Events spontaneously (only Registered/Pong/Error replies).
//   * Persist client_id assignment across reconnect.
//
// Connection lifecycle is intentionally short-lived for B.2: each
// pipe instance handles one client connection, end-to-end. When the
// client drops, the per-connection task ends and the accept loop
// continues with a fresh instance.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use agentmux_common::ipc::{ClientKind, Command, ErrorCode, Event};

/// Monotonic event-version counter. Each Event sent over any pipe
/// gets a fresh version. Phase D uses this as the GetSnapshot resync
/// anchor (`event.version > snapshot.version_at_snapshot` semantics).
static EVENT_VERSION: AtomicU64 = AtomicU64::new(0);

/// Monotonic client_id assigned per Register. Stable per
/// launcher-run; not persisted.
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

fn next_event_version() -> u64 {
    EVENT_VERSION.fetch_add(1, Ordering::Relaxed)
}

/// State the IPC server shares across connections. Today it's just
/// the launcher's PID + version (for Registered echoes); the reducer
/// (B.3) will land here.
#[derive(Debug, Clone)]
pub struct ServerCtx {
    pub launcher_pid: u32,
    pub launcher_version: String,
}

/// Run the named-pipe IPC server until cancelled (or task panics).
///
/// Returns a JoinHandle the caller (main.rs) holds for the life of
/// the launcher. The server keeps accepting until the launcher's
/// Tokio runtime shuts down.
///
/// Each accepted connection becomes a new tokio task running
/// `handle_connection`. The accept loop creates a fresh
/// `NamedPipeServer` instance for the next client BEFORE spawning
/// the handler — without this, a slow handler could starve the next
/// connect. Standard Win32 named-pipe pattern.
#[cfg(target_os = "windows")]
pub fn run_ipc_server(
    pipe_name: String,
    ctx: ServerCtx,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let ctx = Arc::new(ctx);
        crate::log(&format!("[ipc] server starting on {}", pipe_name));

        // First server instance — `first_pipe_instance(true)` rejects
        // a second launcher trying to bind the same pipe (Phase B.6's
        // single-instance check rides on this). For B.2 we use it
        // defensively to surface name-collision bugs early.
        let first = match ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name)
        {
            Ok(s) => s,
            Err(e) => {
                crate::log(&format!(
                    "[ipc] FATAL: bind failed for {}: {} (another launcher running for this data dir?)",
                    pipe_name, e
                ));
                return;
            }
        };
        let mut current = first;

        loop {
            // Wait for a client to connect to the existing instance.
            if let Err(e) = current.connect().await {
                crate::log(&format!("[ipc] connect failed: {}", e));
                // Keep looping — recreating the server below.
            }

            // Hand the accepted instance to a handler task, then
            // create the NEXT server instance so the next client
            // doesn't have to wait for the handler to finish.
            let accepted = current;
            current = match ServerOptions::new().create(&pipe_name) {
                Ok(s) => s,
                Err(e) => {
                    crate::log(&format!(
                        "[ipc] FATAL: failed to create next pipe instance: {} (server stopping)",
                        e
                    ));
                    // Drain the accepted client, then bail.
                    tokio::spawn(handle_connection(accepted, Arc::clone(&ctx)));
                    return;
                }
            };

            tokio::spawn(handle_connection(accepted, Arc::clone(&ctx)));
        }
    })
}

#[cfg(not(target_os = "windows"))]
pub fn run_ipc_server(
    _pipe_name: String,
    _ctx: ServerCtx,
) -> tokio::task::JoinHandle<()> {
    // Non-Windows: pipe IPC isn't built yet. Phase 7 of the broader
    // tear-off cross-platform work (separate spec) will add Unix
    // domain socket support. For now return an immediately-finished
    // task so the caller can hold a handle uniformly.
    tokio::spawn(async {})
}

/// Drive one connection: read newline-delimited JSON Commands,
/// write back JSON Events. First message MUST be `Register`.
#[cfg(target_os = "windows")]
async fn handle_connection(stream: NamedPipeServer, ctx: Arc<ServerCtx>) {
    // Split into read + write halves so we can call them
    // independently. tokio's NamedPipeServer is bidi by default.
    let (read_half, write_half) = tokio::io::split(stream);
    let writer = Arc::new(Mutex::new(write_half));
    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();

    let mut registered_kind: Option<ClientKind> = None;
    let mut client_id: Option<u64> = None;

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => {
                crate::log(&format!(
                    "[ipc] connection closed (client_id={:?}, kind={:?})",
                    client_id, registered_kind
                ));
                return;
            }
            Err(e) => {
                crate::log(&format!(
                    "[ipc] read error (client_id={:?}): {}",
                    client_id, e
                ));
                return;
            }
        };

        // Skip blank lines so a stray newline doesn't churn errors.
        if line.trim().is_empty() {
            continue;
        }

        let cmd = match serde_json::from_str::<Command>(&line) {
            Ok(c) => c,
            Err(e) => {
                let _ = send_event(
                    &writer,
                    Event::Error {
                        code: ErrorCode::InvalidCommand,
                        message: format!("parse failed: {}", e),
                        fatal: false,
                        version: next_event_version(),
                    },
                )
                .await;
                continue;
            }
        };

        match cmd {
            Command::Register { kind, pid, version } => {
                if registered_kind.is_some() {
                    let _ = send_event(
                        &writer,
                        Event::Error {
                            code: ErrorCode::AlreadyRegistered,
                            message: "Register sent twice on the same connection".into(),
                            fatal: false,
                            version: next_event_version(),
                        },
                    )
                    .await;
                    continue;
                }
                let id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
                registered_kind = Some(kind);
                client_id = Some(id);
                crate::log(&format!(
                    "[ipc] registered client_id={} kind={:?} pid={} version={}",
                    id, kind, pid, version
                ));
                let _ = send_event(
                    &writer,
                    Event::Registered {
                        client_id: id,
                        launcher_pid: ctx.launcher_pid,
                        launcher_version: ctx.launcher_version.clone(),
                        version: next_event_version(),
                    },
                )
                .await;
            }
            Command::Ping { nonce } => {
                if registered_kind.is_none() {
                    let _ = send_event(
                        &writer,
                        Event::Error {
                            code: ErrorCode::NotRegistered,
                            message: "Ping before Register".into(),
                            fatal: false,
                            version: next_event_version(),
                        },
                    )
                    .await;
                    continue;
                }
                let _ = send_event(
                    &writer,
                    Event::Pong {
                        nonce,
                        version: next_event_version(),
                    },
                )
                .await;
            }
            Command::Goodbye => {
                crate::log(&format!(
                    "[ipc] goodbye from client_id={:?} kind={:?}",
                    client_id, registered_kind
                ));
                return;
            }
        }
    }
}

/// Serialize an Event as one JSON line + `\n` and write atomically
/// (under the per-connection writer mutex). Returns Err if the
/// connection died mid-write.
#[cfg(target_os = "windows")]
async fn send_event(
    writer: &Arc<Mutex<tokio::io::WriteHalf<NamedPipeServer>>>,
    event: Event,
) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(&event).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    })?;
    buf.push(b'\n');
    let mut w = writer.lock().await;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}
