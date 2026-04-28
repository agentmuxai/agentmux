// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Named-pipe IPC server — accept loop + per-connection handler.
//
// Phase B.3: every Command goes through the pure reducer
// (`crate::reducer::update`) which mutates the shared State and
// returns a Vec<Event>. The handler then writes those events back
// over the connection. State is held inside Arc<Mutex<State>> and
// the mutex is acquired only for the duration of the reducer call —
// never across an await.
//
// What this commit does NOT do:
//   * Per-subscriber broadcast routing (B.4 splits replies vs broadcasts;
//     today every event goes back over the originating connection).
//   * Server-initiated events (no spontaneous emissions yet — only
//     reducer outputs).
//   * Persisted client_id (still per-launcher-run).
//
// Connection lifecycle: each accepted pipe instance handles one
// client connection end-to-end. When the client drops, the per-
// connection task ends and the accept loop continues with a fresh
// instance.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use agentmux_common::ipc::{ClientKind, Command, ErrorCode, Event};

use crate::reducer;
use crate::state::State;

/// State the IPC server shares across connections. Carries the
/// launcher's identity (for patching into Registered events the
/// reducer emits with sentinel values) plus the canonical State.
#[derive(Debug)]
pub struct ServerCtx {
    pub launcher_pid: u32,
    pub launcher_version: String,
    /// Canonical state owned by the server. Mutex held only during
    /// reducer dispatch — sub-millisecond.
    pub state: Mutex<State>,
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
            // On error: log + recreate the instance + retry. Without
            // the explicit `continue`, the failed (un-connected) pipe
            // instance below would be moved into `accepted` and
            // spawned in a handler that immediately fails to read,
            // wasting a per-connection task slot. (reagent P1 + codex
            // P1 PR #573 round-1.)
            if let Err(e) = current.connect().await {
                crate::log(&format!("[ipc] connect failed: {} — recreating instance", e));
                current = match ServerOptions::new().create(&pipe_name) {
                    Ok(s) => s,
                    Err(create_err) => {
                        crate::log(&format!(
                            "[ipc] FATAL: failed to recreate pipe after connect error: {} (server stopping)",
                            create_err
                        ));
                        return;
                    }
                };
                continue;
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
///
/// Phase B.3: every Command goes through `reducer::update`. The
/// reducer is sync; we hold the state mutex only while it runs.
/// Events come back from the reducer as Vec<Event>; we patch sentinel
/// fields (Registered.launcher_pid / launcher_version — the reducer
/// can't read those) and write each line.
#[cfg(target_os = "windows")]
async fn handle_connection(stream: NamedPipeServer, ctx: Arc<ServerCtx>) {
    let (read_half, write_half) = tokio::io::split(stream);
    let writer = Arc::new(Mutex::new(write_half));
    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();

    // Per-connection state the server (not reducer) tracks: have we
    // seen a Register yet? Reducer-level dedup is keyed by PID across
    // all connections; this is the per-connection enforcement so a
    // single connection can't send Ping before Register.
    let mut registered_kind: Option<ClientKind> = None;
    let mut registered_pid: Option<u32> = None;
    // Connection ID is server-allocated (not state-allocated) and
    // exists only for log correlation; the reducer-allocated
    // client_id (returned in Registered) is the wire-visible one.
    let conn_id = NEXT_CONN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => {
                crate::log(&format!(
                    "[ipc] connection conn_id={} closed (kind={:?}, pid={:?})",
                    conn_id, registered_kind, registered_pid
                ));
                return;
            }
            Err(e) => {
                crate::log(&format!(
                    "[ipc] read error conn_id={}: {}",
                    conn_id, e
                ));
                return;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let cmd = match serde_json::from_str::<Command>(&line) {
            Ok(c) => c,
            Err(e) => {
                let mut state = ctx.state.lock().await;
                let v = state.bump_version();
                drop(state);
                let _ = send_event(
                    &writer,
                    Event::Error {
                        code: ErrorCode::InvalidCommand,
                        message: format!("parse failed: {}", e),
                        fatal: false,
                        version: v,
                    },
                )
                .await;
                continue;
            }
        };

        // Per-connection invariants: enforce here so reducer doesn't
        // need to know about connection identity. Honor the `fatal`
        // bit — Ping-before-Register is non-fatal (clients can
        // recover by sending Register next), Goodbye-before-Register
        // is fatal (can't recover from a closed-by-them connection).
        // (reagent P1 + codex P1 PR #574 round-1.)
        if let Some(reply) = enforce_register_first(&cmd, &registered_kind, &ctx).await {
            let close = matches!(&reply, Event::Error { fatal: true, .. });
            let _ = send_event(&writer, reply).await;
            if close {
                return;
            }
            continue;
        }
        if let Command::Register { .. } = &cmd {
            if registered_kind.is_some() {
                let mut state = ctx.state.lock().await;
                let v = state.bump_version();
                drop(state);
                let _ = send_event(
                    &writer,
                    Event::Error {
                        code: ErrorCode::AlreadyRegistered,
                        message: "Register sent twice on the same connection".into(),
                        fatal: false,
                        version: v,
                    },
                )
                .await;
                continue;
            }
        }

        // Track local registration before dispatch so we can update
        // our per-connection state if the reducer accepts it. The
        // reducer's PID-uniqueness check might reject the Register;
        // we re-check the events for that case below.
        let pre_register = if let Command::Register { kind, pid, .. } = &cmd {
            Some((*kind, *pid))
        } else {
            None
        };

        // Dispatch through the reducer. Mutex held briefly — compute
        // the timestamp BEFORE acquiring so syscalls + string
        // formatting don't show up in lock-hold time. (gemini
        // MEDIUM @ server.rs:259, PR #574 round-1.)
        let now_rfc3339 = chrono::Utc::now().to_rfc3339();
        let events = {
            let mut state = ctx.state.lock().await;
            let rctx = reducer::Ctx {
                now_rfc3339,
                conn_id,
                registered_pid,
            };
            reducer::update(&mut state, cmd.clone(), &rctx)
        };

        // If the reducer accepted the Register (no AlreadyRegistered
        // error in the output), commit the local connection state.
        if let Some((kind, pid)) = pre_register {
            let rejected = events
                .iter()
                .any(|e| matches!(e, Event::Error { code: ErrorCode::AlreadyRegistered, .. }));
            if !rejected {
                registered_kind = Some(kind);
                registered_pid = Some(pid);
            }
        }

        // Write events back over the same connection. Patch the
        // sentinel launcher_pid / launcher_version fields the reducer
        // left blank.
        let goodbye = matches!(cmd, Command::Goodbye);
        for event in events {
            let event = patch_launcher_identity(event, &ctx);
            if send_event(&writer, event).await.is_err() {
                return;
            }
        }
        if goodbye {
            crate::log(&format!(
                "[ipc] goodbye from conn_id={} kind={:?} pid={:?}",
                conn_id, registered_kind, registered_pid
            ));
            return;
        }
    }
}

/// Per-connection counter for log-correlation IDs (NOT the wire
/// client_id — that comes from the reducer). Allocated even for
/// pre-Register failures so log lines can be correlated.
static NEXT_CONN_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Enforce the "first message must be Register" invariant. Returns
/// `Some(Event::Error)` if the command violates the contract; the
/// caller sends it and closes the connection.
#[cfg(target_os = "windows")]
async fn enforce_register_first(
    cmd: &Command,
    registered_kind: &Option<ClientKind>,
    ctx: &Arc<ServerCtx>,
) -> Option<Event> {
    if registered_kind.is_some() {
        return None;
    }
    let (msg, fatal) = match cmd {
        Command::Register { .. } => return None,
        Command::Ping { .. } => ("Ping before Register".to_string(), false),
        Command::Goodbye => ("Goodbye before Register".to_string(), true),
    };
    let mut state = ctx.state.lock().await;
    let v = state.bump_version();
    Some(Event::Error {
        code: ErrorCode::NotRegistered,
        message: msg,
        fatal,
        version: v,
    })
}

/// Patch `launcher_pid` + `launcher_version` into `Event::Registered`.
/// The reducer leaves these as sentinels (it can't read env without
/// breaking determinism) — the server fills them in here, just
/// before serializing to the wire.
fn patch_launcher_identity(event: Event, ctx: &Arc<ServerCtx>) -> Event {
    if let Event::Registered {
        client_id, version, ..
    } = event
    {
        Event::Registered {
            client_id,
            launcher_pid: ctx.launcher_pid,
            launcher_version: ctx.launcher_version.clone(),
            version,
        }
    } else {
        event
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
