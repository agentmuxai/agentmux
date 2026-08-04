// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1b — srv pipe IPC server. Adapted from
// agentmux-launcher::ipc::server with srv-specific tweaks:
//
//   * No "host-only" enforcement gate — srv accepts commands from
//     any registered client (per Phase E §5: workspace / tab / block
//     commands eventually originate from renderer-via-host or from
//     Tools).
//   * No WRR drift logging — that's launcher-domain.
//   * Identity patching is sentinel-aware: the srv reducer emits
//     `Event::Registered { launcher_pid: 0, launcher_version: "" }`
//     and the server fills both fields with srv's identity (process
//     id + crate version) before broadcast — same convention the
//     launcher uses for its own pipe replies.
//
// Future refactor: lift the shared parts (broadcast bus, fanout
// task, GetEvents intercept) into agentmux-common. Phase E.7
// cleanup PR. For E.1b copy/adapt to keep the diff scoped.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use agentmux_common::ipc::{ClientKind, Command, ErrorCode, Event};

use crate::reducer;
use crate::state::State;

#[derive(Debug)]
pub struct ServerCtx {
    pub srv_pid: u32,
    pub srv_version: String,
    pub state: Arc<Mutex<State>>,
    pub events_tx: tokio::sync::broadcast::Sender<Event>,
    pub event_log: Arc<crate::event_log::EventLog>,
}

#[cfg(target_os = "windows")]
pub fn bind_first_pipe_instance(pipe_name: &str) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new()
        .first_pipe_instance(true)
        .create(pipe_name)
}

#[cfg(target_os = "windows")]
pub fn run_srv_ipc_server(
    pipe_name: String,
    first: NamedPipeServer,
    ctx: ServerCtx,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let ctx = Arc::new(ctx);
        tracing::info!(target: "srv-ipc", "[srv-ipc] server starting on {}", pipe_name);

        let mut current = first;

        loop {
            if let Err(e) = current.connect().await {
                tracing::warn!(target: "srv-ipc", "[srv-ipc] connect failed: {} — recreating instance", e);
                current = match ServerOptions::new().create(&pipe_name) {
                    Ok(s) => s,
                    Err(create_err) => {
                        tracing::error!(target: "srv-ipc", "[srv-ipc] FATAL: failed to recreate pipe after connect error: {}", create_err);
                        return;
                    }
                };
                continue;
            }

            let accepted = current;
            current = match ServerOptions::new().create(&pipe_name) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(target: "srv-ipc", "[srv-ipc] FATAL: failed to create next pipe instance: {}", e);
                    tokio::spawn(handle_connection(accepted, Arc::clone(&ctx)));
                    return;
                }
            };

            tokio::spawn(handle_connection(accepted, Arc::clone(&ctx)));
        }
    })
}

#[cfg(not(target_os = "windows"))]
pub fn run_srv_ipc_server(
    _pipe_name: String,
    _ctx: ServerCtx,
) -> tokio::task::JoinHandle<()> {
    // Phase 7 will add Unix domain socket support.
    tokio::spawn(async {})
}

#[cfg(target_os = "windows")]
async fn handle_connection(stream: NamedPipeServer, ctx: Arc<ServerCtx>) {
    let (read_half, write_half) = tokio::io::split(stream);
    let writer = Arc::new(Mutex::new(write_half));
    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();

    let mut registered_kind: Option<ClientKind> = None;
    let mut registered_pid: Option<u32> = None;
    let conn_id = NEXT_CONN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // Per-connection fanout task — subscribed BEFORE we start reading
    // commands so concurrent broadcasts aren't lost in the pre-recv
    // window. Same pattern as launcher's server.
    let fanout_handle = {
        let writer = Arc::clone(&writer);
        let mut events_rx = ctx.events_tx.subscribe();
        tokio::spawn(async move {
            loop {
                match events_rx.recv().await {
                    Ok(event) => {
                        // Identity is patched at the publisher
                        // (before log + bus); no re-patch here.
                        if send_event(&writer, event).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(target: "srv-ipc", "[srv-ipc] conn_id={} lagged event bus, missed {} events", conn_id, n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        })
    };

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => {
                tracing::info!(target: "srv-ipc", "[srv-ipc] connection conn_id={} closed (kind={:?}, pid={:?})", conn_id, registered_kind, registered_pid);
                // Phase E.1b — synthetic Goodbye on ungraceful
                // disconnect so the reducer marks the PID Exited;
                // otherwise reconnect-from-same-PID hits
                // AlreadyRegistered. (codex P1 #610.)
                dispatch_synthetic_goodbye(&ctx, conn_id, registered_pid).await;
                fanout_handle.abort();
                return;
            }
            Err(e) => {
                tracing::warn!(target: "srv-ipc", "[srv-ipc] read error conn_id={}: {}", conn_id, e);
                dispatch_synthetic_goodbye(&ctx, conn_id, registered_pid).await;
                fanout_handle.abort();
                return;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let cmd = match serde_json::from_str::<Command>(&line) {
            Ok(c) => c,
            Err(e) => {
                // Phase E.1b — parse errors are connection-private
                // (sent only to the offender, not broadcast, not
                // appended to the event log). Don't bump the global
                // event_version: other subscribers would see version
                // gaps and treat them as missed events. Use 0 as a
                // sentinel for "not part of the ordered stream."
                // (codex P2 #610.)
                let _ = send_event(
                    &writer,
                    Event::Error {
                        code: ErrorCode::InvalidCommand,
                        message: format!("parse failed: {}", e),
                        fatal: false,
                        version: 0,
                    },
                )
                .await;
                continue;
            }
        };

        // Enforce Register-first at the connection level.
        if let Some(reply) = enforce_register_first(&cmd, &registered_kind, &ctx).await {
            let close = matches!(&reply, Event::Error { fatal: true, .. });
            let _ = send_event(&writer, reply).await;
            if close {
                fanout_handle.abort();
                return;
            }
            continue;
        }
        if let Command::Register { .. } = &cmd {
            if registered_kind.is_some() {
                // Phase E.1b — connection-private error; sentinel
                // version=0 (codex P2 #610).
                let _ = send_event(
                    &writer,
                    Event::Error {
                        code: ErrorCode::AlreadyRegistered,
                        message: "Register sent twice on the same connection".into(),
                        fatal: false,
                        version: 0,
                    },
                )
                .await;
                continue;
            }
        }

        let pre_register = if let Command::Register { kind, pid, .. } = &cmd {
            Some((*kind, *pid))
        } else {
            None
        };

        // Phase D.3 — `GetEvents` is intercepted before the reducer
        // (log query is I/O-adjacent; reducer stays pure).
        //
        // The reply (`Event::EventList`) is sent DIRECTLY to the
        // requesting connection — NOT broadcast on the shared bus.
        // EventList is request/response, not a state transition;
        // broadcasting it would force every subscriber to process
        // foreign replay payloads (potentially treating them as their
        // own catch-up data, duplicating state application). (codex
        // P1 #610.)
        if let Command::GetEvents { since } = &cmd {
            // Phase E.1b — read the current version WITHOUT bumping
            // (codex P2 #610). EventList is connection-private; the
            // version it carries is the "as-of" point for the
            // requester's next resync, not a new state-transition
            // marker. Bumping would create a global gap that other
            // subscribers see as missed events.
            let v = {
                let state = ctx.state.lock().await;
                state.event_version
            };
            let replay = ctx.event_log.events_since(*since);
            if ctx.event_log.replay_truncated(*since) {
                tracing::warn!(target: "srv-ipc", "[srv-ipc] conn_id={} GetEvents since={} truncated", conn_id, since);
            }
            let _ = send_event(
                &writer,
                Event::EventList {
                    events: replay,
                    version: v,
                },
            )
            .await;
            continue;
        }

        let now_rfc3339 = chrono::Utc::now().to_rfc3339();
        let events = {
            let mut state = ctx.state.lock().await;
            let rctx = reducer::Ctx {
                now_rfc3339,
                registered_pid,
            };
            reducer::update(&mut state, cmd.clone(), &rctx)
        };

        if let Some((kind, pid)) = pre_register {
            let rejected = events
                .iter()
                .any(|e| matches!(e, Event::Error { code: ErrorCode::AlreadyRegistered, .. }));
            if !rejected {
                registered_kind = Some(kind);
                registered_pid = Some(pid);
            }
        }

        let goodbye = matches!(cmd, Command::Goodbye);
        for event in events {
            // Phase E.1b — patch sentinel identity BEFORE log + bus
            // (per launcher's E.1a fix for codex P2 #608: replay must
            // match live broadcast).
            let event = patch_srv_identity(event, &ctx);
            // Append to the in-memory ring before broadcasting so a
            // concurrent GetEvents query sees consistent results.
            // Snapshot / EventList / Error are excluded — meta-events
            // not state transitions.
            if !matches!(
                event,
                Event::Snapshot { .. }
                    | Event::EventList { .. }
                    | Event::SrvSnapshot { .. }
                    | Event::Error { .. }
            ) {
                ctx.event_log.append(event.clone());
            }
            let _ = ctx.events_tx.send(event);
        }
        if goodbye {
            tracing::info!(target: "srv-ipc", "[srv-ipc] goodbye from conn_id={} kind={:?} pid={:?}", conn_id, registered_kind, registered_pid);
            fanout_handle.abort();
            return;
        }
    }
}

/// Phase E.1b — synthetic Goodbye dispatch for ungraceful disconnects
/// (EOF / read error before the client sent an explicit Goodbye).
/// Without this, the reducer's process record stays Running and a
/// reconnect from the same live PID hits AlreadyRegistered. Goodbye
/// transitions the record to Exited so re-Register is accepted.
/// (codex P1 #610.)
///
/// Idempotent: handle_goodbye is a no-op if no PID is registered or
/// the record is already Exited. Errors during the synthetic dispatch
/// are logged but non-fatal — we're already on a disconnect path.
#[cfg(target_os = "windows")]
async fn dispatch_synthetic_goodbye(
    ctx: &Arc<ServerCtx>,
    conn_id: u64,
    registered_pid: Option<u32>,
) {
    let Some(pid) = registered_pid else {
        return;
    };
    let now_rfc3339 = chrono::Utc::now().to_rfc3339();
    let events = {
        let mut state = ctx.state.lock().await;
        let rctx = reducer::Ctx {
            now_rfc3339,
            conn_id,
            registered_pid: Some(pid),
        };
        reducer::update(&mut state, Command::Goodbye, &rctx)
    };
    for event in events {
        let event = patch_srv_identity(event, ctx);
        if !matches!(
            event,
            Event::Snapshot { .. }
                | Event::EventList { .. }
                | Event::SrvSnapshot { .. }
                | Event::Error { .. }
        ) {
            ctx.event_log.append(event.clone());
        }
        let _ = ctx.events_tx.send(event);
    }
}

static NEXT_CONN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Same Register-first invariant the launcher enforces.
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
        Command::GetSrvSnapshot => ("GetSrvSnapshot before Register".to_string(), false),
        Command::GetEvents { .. } => ("GetEvents before Register".to_string(), false),
        Command::Goodbye => ("Goodbye before Register".to_string(), true),
        // Anything else hitting srv pre-Register is also a soft error
        // — same Ping precedent. The reducer arm for un-accepted
        // commands also returns a soft InvalidCommand error if the
        // dispatch reaches it.
        _ => ("Command before Register".to_string(), false),
    };
    // Phase E.1b — connection-private error; sentinel version=0
    // (codex P2 #610).
    let v = 0;
    // Phase E.1b — match launcher's `NotRegistered` error code for
    // pre-Register violations (vs InvalidCommand which is for
    // parse/shape problems). Clients dispatching on error code
    // need consistent semantics across both pipes.
    // (reagent + codex P2 #610.)
    Some(Event::Error {
        code: ErrorCode::NotRegistered,
        message: msg,
        fatal,
        version: v,
    })
}

/// Patch the sentinel `launcher_pid: 0` / empty `launcher_version`
/// the reducer emits in `Event::Registered` with srv's actual
/// identity. Idempotent — applying twice is a no-op (after the
/// first patch, fields are non-sentinel).
fn patch_srv_identity(event: Event, ctx: &Arc<ServerCtx>) -> Event {
    if let Event::Registered {
        client_id,
        launcher_pid,
        launcher_version,
        version,
    } = event
    {
        if launcher_pid == 0 && launcher_version.is_empty() {
            return Event::Registered {
                client_id,
                launcher_pid: ctx.srv_pid,
                launcher_version: ctx.srv_version.clone(),
                version,
            };
        }
        return Event::Registered {
            client_id,
            launcher_pid,
            launcher_version,
            version,
        };
    }
    event
}

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
