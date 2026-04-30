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
    ///
    /// Phase E.1a — moved from `Mutex<State>` to `Arc<Mutex<State>>`
    /// so the saga coordinator can share access. The coordinator
    /// needs `bump_version` when emitting saga lifecycle events and
    /// will (in E.5) inspect state during saga decisions. Sharing
    /// via `Arc` keeps the existing single-writer-mutex discipline.
    pub state: std::sync::Arc<Mutex<State>>,
    /// Phase B.8 — broadcast bus for reducer-emitted events. Every
    /// reducer event from `reducer::update` is published here; each
    /// connection subscribes and writes received events to its own
    /// pipe. Lets observability clients (`--diag wrr`, future Tools)
    /// see cross-process activity, not just replies to their own
    /// commands. Per-connection direct sends (Error replies for
    /// parse failures, register-first violations) bypass the bus —
    /// they're response-to-this-client-only by intent. (codex P1
    /// PR #605.)
    pub events_tx: tokio::sync::broadcast::Sender<Event>,
    /// Phase D.2 — event log: in-memory ring of recent reducer
    /// events (replay source for D.3's `GetEvents`) + disk
    /// persistence stream for crash forensics. Server appends to
    /// the in-memory ring synchronously after each reducer
    /// dispatch; the disk writer is a separate task spawned in
    /// main.rs that subscribes to the broadcast bus.
    pub event_log: std::sync::Arc<crate::event_log::EventLog>,
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
/// can't read those) and publish them on the broadcast bus.
///
/// Phase B.8 — events from the reducer flow through the broadcast
/// bus (`ctx.events_tx`); a per-connection fanout task subscribes
/// and writes events to this connection's pipe. Per-connection
/// direct writes are reserved for "response-to-this-client-only"
/// errors (parse failure, register-first violation). (codex P1
/// PR #605.)
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

    // Phase B.8 — fanout task: subscribe to the server-wide event
    // bus and write each event to THIS connection's pipe. Started
    // before any commands are processed so a registered client can
    // start receiving events from concurrent activity immediately.
    // Aborted when the connection's read loop returns (below).
    let fanout_handle = {
        let writer = Arc::clone(&writer);
        let ctx = Arc::clone(&ctx);
        let mut events_rx = ctx.events_tx.subscribe();
        tokio::spawn(async move {
            loop {
                match events_rx.recv().await {
                    Ok(event) => {
                        // Identity is patched at the publisher (before
                        // log + bus). No need to re-patch here.
                        // Errors here mean the pipe is closed; the
                        // read loop will detect EOF on the next
                        // iteration. Swallow + continue to drain
                        // any remaining buffered events so we don't
                        // accidentally hold onto channel slots.
                        if send_event(&writer, event).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Slow client missed `n` events. Phase D's
                        // GetSnapshot resync covers this case
                        // properly; for now the client has to
                        // reconnect to recover. Log so operators
                        // can see when this happens.
                        crate::log(&format!(
                            "[ipc] conn_id={} lagged event bus, missed {} events",
                            conn_id, n
                        ));
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
                crate::log(&format!(
                    "[ipc] connection conn_id={} closed (kind={:?}, pid={:?})",
                    conn_id, registered_kind, registered_pid
                ));
                // Phase E.1b — synthetic Goodbye on ungraceful
                // disconnect so the reducer marks the PID Exited;
                // otherwise reconnect-from-same-PID hits
                // AlreadyRegistered. (codex P1 #610.)
                dispatch_synthetic_goodbye(&ctx, conn_id, registered_pid).await;
                fanout_handle.abort();
                return;
            }
            Err(e) => {
                crate::log(&format!(
                    "[ipc] read error conn_id={}: {}",
                    conn_id, e
                ));
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
                fanout_handle.abort();
                return;
            }
            continue;
        }
        if let Command::Register { .. } = &cmd {
            if registered_kind.is_some() {
                // Phase E.1b — connection-private error; same
                // version-sentinel rationale as parse-error path
                // above. (codex P2 #610.)
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

        // Track local registration before dispatch so we can update
        // our per-connection state if the reducer accepts it. The
        // reducer's PID-uniqueness check might reject the Register;
        // we re-check the events for that case below.
        let pre_register = if let Command::Register { kind, pid, .. } = &cmd {
            Some((*kind, *pid))
        } else {
            None
        };

        // Phase D.3 — `GetEvents { since }` is handled here, not in
        // the reducer. The reducer is pure (no I/O); querying the
        // event log is a non-mutating read against the in-memory
        // ring + disk fallback.
        //
        // Phase E.1b — the reply (`Event::EventList`) is sent
        // DIRECTLY to the requesting connection, NOT broadcast on
        // the shared bus. EventList is request/response, not a
        // state transition; broadcasting it would force every
        // subscriber to process foreign replay payloads
        // (potentially treating them as their own catch-up data,
        // duplicating state application). (codex P1 #610.)
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
            // Don't log truncation as an error — it's a valid
            // "subscriber missed events that have already been
            // evicted" signal; the subscriber's resync logic
            // handles it (re-fetch a fresh snapshot).
            if ctx.event_log.replay_truncated(*since) {
                crate::log(&format!(
                    "[ipc] conn_id={} GetEvents since={} truncated (oldest retained event > since+1)",
                    conn_id, since
                ));
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

        // Phase B.8 — publish reducer events on the broadcast bus
        // instead of writing them directly to this connection. The
        // per-connection fanout task (spawned above) subscribes and
        // writes them to its own pipe — including this connection,
        // which sees its own events back. Drift events still log at
        // the launcher level so operators see them regardless of
        // subscriber wiring. (codex P1 PR #605.)
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
            // Phase E.1a (codex P2 #608) — patch sentinel identity
            // BEFORE appending to the log. Reducer emits
            // `Event::Registered { launcher_pid: 0, launcher_version: "" }`
            // because it doesn't know the launcher's identity; the
            // server fills it in. Pre-fix, the patch happened only at
            // per-connection write, so `GetEvents` replay returned
            // stored sentinels, inconsistent with live broadcast.
            let event = patch_launcher_identity(event, &ctx);

            // Phase D.2 — append to the in-memory ring BEFORE
            // broadcasting so a connection's GetEvents query that
            // races a just-published event sees consistent
            // results. Disk persistence (separate task) is best-
            // effort and may lag. Snapshot / EventList variants
            // are NOT appended — they're meta-events about the
            // event stream itself; including them would create
            // recursive replay (an EventList containing EventLists
            // is meaningless). Errors are also skipped — they're
            // per-client diagnostics, not state transitions.
            if !matches!(event, Event::Snapshot { .. } | Event::EventList { .. } | Event::Error { .. }) {
                ctx.event_log.append(event.clone());
            }
            // Send may fail when no receivers exist (e.g., during
            // shutdown). That's fine — events are advisory in that
            // window and the per-connection fanout tasks own retry
            // semantics via subscribe().
            let _ = ctx.events_tx.send(event);
        }
        if goodbye {
            crate::log(&format!(
                "[ipc] goodbye from conn_id={} kind={:?} pid={:?}",
                conn_id, registered_kind, registered_pid
            ));
            fanout_handle.abort();
            return;
        }
    }
}

/// Per-connection counter for log-correlation IDs (NOT the wire
/// client_id — that comes from the reducer). Allocated even for
/// pre-Register failures so log lines can be correlated.
static NEXT_CONN_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Phase E.1b — synthetic Goodbye dispatch for ungraceful disconnects
/// (EOF / read error before the client sent an explicit Goodbye).
/// Without this, the reducer's process record stays Running and a
/// reconnect from the same live PID hits AlreadyRegistered.
/// (codex P1 #610.)
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
    let now_ms = launcher_start_ms();
    let events = {
        let mut state = ctx.state.lock().await;
        let rctx = reducer::Ctx {
            now_rfc3339,
            conn_id,
            registered_pid: Some(pid),
            now_ms,
        };
        reducer::update(&mut state, Command::Goodbye, &rctx)
    };
    for event in events {
        let event = patch_launcher_identity(event, ctx);
        if !matches!(
            event,
            Event::Snapshot { .. } | Event::EventList { .. } | Event::Error { .. }
        ) {
            ctx.event_log.append(event.clone());
        }
        let _ = ctx.events_tx.send(event);
    }
}

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
        // Phase D.1 — GetSnapshot before Register is non-fatal: any
        // sane diagnostic client can fix it by retrying with Register
        // first. (Same Ping-before-Register precedent — soft error.)
        Command::GetSnapshot => ("GetSnapshot before Register".to_string(), false),
        // Phase D.3 — GetEvents before Register: same non-fatal
        // semantics as GetSnapshot.
        Command::GetEvents { .. } => ("GetEvents before Register".to_string(), false),
        // Phase E.1b — GetSrvSnapshot is a srv-pipe command; if a
        // client sends it to the launcher pipe by mistake, soft
        // error: not the launcher's command.
        Command::GetSrvSnapshot => (
            "GetSrvSnapshot is a srv-pipe command; sent to launcher pipe by mistake".to_string(),
            false,
        ),
        // Phase E.2 — srv-pipe commands sent to launcher pipe by
        // mistake. Soft error — clients can recover by routing to
        // the right pipe.
        Command::CreateWorkspace { .. } => (
            "CreateWorkspace is a srv-pipe command; sent to launcher pipe by mistake".to_string(),
            false,
        ),
        Command::DeleteWorkspace { .. } => (
            "DeleteWorkspace is a srv-pipe command; sent to launcher pipe by mistake".to_string(),
            false,
        ),
    };
    // Phase E.1b — connection-private error; sentinel version=0
    // (codex P2 #610). See parse-error path for rationale.
    let v = 0;
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
