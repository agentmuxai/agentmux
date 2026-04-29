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

/// Bind the first named-pipe instance synchronously.
///
/// Phase B.6: the bind is the single-instance signal. Splitting it
/// out of `run_ipc_server` lets the caller (main.rs) detect a
/// collision BEFORE spawning srv/host and surface a user-visible
/// error ("AgentMux is already running"). `ServerOptions::create`
/// requires a Tokio runtime context for IOCP registration, so this
/// must be called from inside `#[tokio::main]` (or any task on the
/// runtime) — not from a plain sync entrypoint.
///
/// On Windows, a second launcher hitting the same pipe gets
/// `ERROR_ACCESS_DENIED` (raw OS error 5); other errors mean the
/// pipe namespace itself is misconfigured.
#[cfg(target_os = "windows")]
pub fn bind_first_pipe_instance(pipe_name: &str) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new()
        .first_pipe_instance(true)
        .create(pipe_name)
}

/// Run the named-pipe IPC server until cancelled (or task panics).
///
/// Returns a JoinHandle the caller (main.rs) holds for the life of
/// the launcher. The server keeps accepting until the launcher's
/// Tokio runtime shuts down.
///
/// The first pipe instance is passed in pre-bound by the caller (see
/// `bind_first_pipe_instance`) so a collision can be surfaced
/// synchronously before any children are spawned (Phase B.6).
///
/// Each accepted connection becomes a new tokio task running
/// `handle_connection`. The accept loop creates a fresh
/// `NamedPipeServer` instance for the next client BEFORE spawning
/// the handler — without this, a slow handler could starve the next
/// connect. Standard Win32 named-pipe pattern.
#[cfg(target_os = "windows")]
pub fn run_ipc_server(
    pipe_name: String,
    first: NamedPipeServer,
    ctx: ServerCtx,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let ctx = Arc::new(ctx);
        crate::log(&format!("[ipc] server starting on {}", pipe_name));

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
        // Phase B.9.1 — monotonic ms since launcher start. Used by
        // the WRR arm for per-window observability ages.
        // `LAUNCHER_START_INSTANT` is a once-init `Instant`; the
        // first request seeds it, subsequent ones read its delta.
        let now_ms = launcher_start_ms();
        let events = {
            let mut state = ctx.state.lock().await;
            let rctx = reducer::Ctx {
                now_rfc3339,
                conn_id,
                registered_pid,
                now_ms,
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
        // left blank. Drift events get a WARN-level log line so
        // operators see mirror divergence in launcher logs without
        // needing a subscriber to be wired up. (B.4 follow-up.)
        let goodbye = matches!(cmd, Command::Goodbye);
        for event in events {
            if let Event::DriftDetected {
                kind,
                host_count,
                mirror_count,
                ..
            } = &event
            {
                crate::log(&format!(
                    "[ipc] DRIFT {:?}: host={} mirror={} (conn_id={})",
                    kind, host_count, mirror_count, conn_id
                ));
            }
            // Phase B.9.1 (WRR) — drift logged at the launcher level
            // so operators see Win32 reality divergence in
            // launcher.log regardless of subscriber wiring. Severity
            // tag in the log line lets a future `--diag wrr` (B.9.2)
            // grep precisely.
            if let Event::HwndDriftDetected {
                kind,
                label,
                hwnd,
                detail,
                severity,
                ..
            } = &event
            {
                crate::log(&format!(
                    "[ipc] WRR-DRIFT [{:?}] {:?} label={:?} hwnd={:?}: {} (conn_id={})",
                    severity, kind, label, hwnd, detail, conn_id
                ));
            }
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

/// Phase B.9.1 — milliseconds since the launcher's IPC server
/// started (first call seeds the epoch). Used as the monotonic
/// clock for WRR observability timestamps in `reducer::Ctx::now_ms`.
/// Distinct from `chrono::Utc::now()` because the WRR arm wants
/// elapsed time, not wall clock — and we don't want clock-skew
/// jitter (NTP adjustment, DST) showing up as drift.
fn launcher_start_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    start.elapsed().as_millis() as u64
}

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
        Command::ReportWindowOpened { .. } => {
            ("ReportWindowOpened before Register".to_string(), true)
        }
        Command::ReportWindowClosed { .. } => {
            ("ReportWindowClosed before Register".to_string(), true)
        }
        Command::ReportPoolWindowAdded { .. } => {
            ("ReportPoolWindowAdded before Register".to_string(), true)
        }
        Command::ReportPoolWindowRemoved { .. } => {
            ("ReportPoolWindowRemoved before Register".to_string(), true)
        }
        Command::ReportHostCounts { .. } => {
            ("ReportHostCounts before Register".to_string(), true)
        }
        Command::ReportHostPoolCount { .. } => {
            ("ReportHostPoolCount before Register".to_string(), true)
        }
        Command::ReportBackendWindowIdRegistered { .. } => {
            (
                "ReportBackendWindowIdRegistered before Register".to_string(),
                true,
            )
        }
        Command::ReportBackendWindowIdUnregistered { .. } => {
            (
                "ReportBackendWindowIdUnregistered before Register".to_string(),
                true,
            )
        }
        // Phase B.9.1 (WRR) — host-only Win32 reality reports.
        Command::ReportHwndOpened { .. } => {
            ("ReportHwndOpened before Register".to_string(), true)
        }
        Command::ReportHwndDestroyed { .. } => {
            ("ReportHwndDestroyed before Register".to_string(), true)
        }
        Command::ReportHwndVisibilityChanged { .. } => {
            (
                "ReportHwndVisibilityChanged before Register".to_string(),
                true,
            )
        }
        Command::ReportHwndForegroundChanged { .. } => {
            (
                "ReportHwndForegroundChanged before Register".to_string(),
                true,
            )
        }
        Command::ReportHwndIconicChanged { .. } => {
            ("ReportHwndIconicChanged before Register".to_string(), true)
        }
        Command::ReportHwndPositionChanged { .. } => {
            (
                "ReportHwndPositionChanged before Register".to_string(),
                true,
            )
        }
        Command::ReportMonitorTopologyChanged { .. } => {
            (
                "ReportMonitorTopologyChanged before Register".to_string(),
                true,
            )
        }
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
