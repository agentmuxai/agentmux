// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.3 — pure reducer.
//
// Shares its DISCIPLINE with `agentmux_srv::reducer` — pure, total,
// deterministic, no I/O — and its wire types: both import the same
// `agentmux_common::ipc::{Command, Event}`. What differs is `State` (each
// crate's own) and the command/event variants each `update` actually
// handles — this one drives the window/process state machine, not
// workspaces/tabs/blocks. Two reducers by design; see
// docs/architecture/DECISION_SAGA_REDUCER_TWO_FRAMEWORKS_2026_09_07.md.
//
// Per `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §5.1:
//
//   pub fn update(state: &State, cmd: Command) -> (State, Vec<Event>);
//
// We deviate slightly: the function takes `&mut State` (mutating in
// place rather than returning a new value) because cloning a HashMap
// per-command in the IPC hot path is wasteful and not needed for
// the testability properties we want — `proptest` works equally well
// against a mutating-reducer (apply commands one by one, assert
// invariants hold after each).
//
// Strict properties of `update`:
//   1. **Total**. Every (cmd, state) combination produces a result;
//      never panics on input. (Panics ARE used to enforce internal
//      invariants — those are bugs in the reducer or upstream code,
//      not user input. See spec §7 / §8.)
//   2. **Deterministic**. Same (state, cmd, conn) → same (state',
//      events). No clocks, no UUIDs, no env reads inside update —
//      injected via the `Reducer` context if needed (B.4 will need
//      it for spawned_at timestamps; for B.3 the conn carries one).
//   3. **No I/O**. update never blocks or awaits. Mutex is held for
//      the duration of update — must stay sub-millisecond. That includes
//      logging: `crate::log` opens, rotates and appends to a file, so an
//      arm that wants a log line pushes it onto `Reduced::diagnostics`
//      and the caller writes it after releasing the lock.
//
// Connection context: every command arrives over a specific
// connection and the resulting events that are *replies* (Registered,
// Pong) belong to that connection only, while *broadcasts*
// (ProcessSpawned, LifecyclePhaseChanged) belong to all subscribers.
// Phase B.3 ships in a simplified model where every event goes over
// the originating connection; B.4 splits the routing.
//
// Invariants checked:
//   * Register on a PID that's already registered → AlreadyRegistered
//     error. (Connection-level enforcement that "Register sent twice"
//     also lives in the server; this is the cross-connection guard.)
//   * Lifecycle transitions: Starting → Running on first Host
//     register. Running → Quitting on Quit (B.3 placeholder).
//     Quitting → Dead on cleanup-done (B.3 placeholder). No skipping.

use agentmux_common::ipc::{Command, DriftKind, ErrorCode, Event};

use crate::state::State;

/// Context the reducer needs but can't read from State (clocks,
/// connection identity). Passed in per-call so update remains pure.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// RFC3339 timestamp to stamp on new ProcessRecords. Injected
    /// (rather than read from chrono::Utc::now() inside update) to
    /// keep the function deterministic for tests.
    pub now_rfc3339: String,
    /// The connection on which this command arrived. Currently
    /// just used for log correlation; B.4+ will use it to route
    /// per-connection replies.
    pub conn_id: u64,
    /// PID the connection has Registered under, if any. The server
    /// tracks this server-side and passes it on every command after
    /// the initial Register so the reducer can mark the right
    /// process Exited on Goodbye. None for the very first command
    /// on a connection (which MUST be Register, enforced server-
    /// side). (codex P1 + gemini HIGH PR #574 round-1.)
    pub registered_pid: Option<u32>,
    /// Phase B.9.1 — monotonic milliseconds since some reference
    /// epoch (the IPC server uses launcher start as the epoch).
    /// Used by the WRR arm for per-window observability timestamps
    /// (`last_foreground_at_ms`, `pending_hwnds[hwnd].arrived_at_ms`).
    /// Distinct from `now_rfc3339` (wall clock) because operators
    /// reading `--diag wrr` want age, not absolute time.
    pub now_ms: u64,
}

mod pool;
mod window;
mod saga;
mod connection;

/// What one `update` call produced.
#[derive(Debug, Default)]
pub struct Reduced {
    /// Events to log, broadcast and reply with.
    pub events: Vec<Event>,
    /// Log lines for `crate::log`. The reducer does no I/O; the caller
    /// writes these after releasing the state lock.
    pub diagnostics: Vec<String>,
}

/// Apply one Command to State, returning the resulting Events and any
/// diagnostic log lines. State is mutated in place. Total function —
/// never panics on input (panics are reserved for internal invariant
/// violations).
pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Reduced {
    let _ = ctx.conn_id; // reserved for B.4 routing
    let mut diagnostics: Vec<String> = Vec::new();

    let mut cmd_events = match cmd {
        Command::Register {
            kind,
            pid,
            version,
        } => connection::handle_register(state, ctx, kind, pid, version),
        Command::Ping { nonce } => {
            let v = state.bump_version();
            vec![Event::Pong { nonce, version: v }]
        }
        // SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 1 — UI-liveness telemetry is
        // recorded at the transport layer (`ipc/server.rs` →
        // `crate::ui_liveness`), NOT here: it's thread/process telemetry
        // about the host, not domain state this reducer owns. This arm
        // exists only for match exhaustiveness — a defensive no-op.
        Command::ReportUiThreadAlive { .. } => Vec::new(),
        // The launcher SENDS ProbeUiThread (over the host pipe); receiving
        // one on its own IPC server is a wrong-direction message — ignore
        // (the reducer is pure and does not log; the pre-Register table in
        // ipc/server.rs already names this case for unregistered senders).
        Command::ProbeUiThread { .. } => Vec::new(),
        // Launcher → host only, like ProbeUiThread; a client sending it is a no-op.
        Command::NotifySrvLatency { .. } => Vec::new(),
        Command::Goodbye => connection::handle_goodbye(state, ctx.registered_pid.unwrap_or(0)),
        Command::ReportWindowOpened {
            label,
            kind,
            parent_label,
        } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportWindowOpened") {
                return Reduced { events: vec![err], diagnostics };
            }
            window::handle_report_window_opened(state, ctx, label, kind, parent_label)
        }
        Command::ReportWindowClosed { label } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportWindowClosed") {
                return Reduced { events: vec![err], diagnostics };
            }
            window::handle_report_window_closed(state, label)
        }
        Command::ReportBackgroundServiceEnabled { enabled } => {
            if let Some(err) =
                connection::enforce_host_only(state, ctx, "ReportBackgroundServiceEnabled")
            {
                return Reduced { events: vec![err], diagnostics };
            }
            window::handle_report_background_service_enabled(state, enabled)
        }
        Command::ReportPoolWindowAdded { label, saga_id } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportPoolWindowAdded") {
                return Reduced { events: vec![err], diagnostics };
            }
            pool::handle_report_pool_window_added(state, label, saga_id)
        }
        Command::ReportPoolWindowRemoved { label } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportPoolWindowRemoved") {
                return Reduced { events: vec![err], diagnostics };
            }
            pool::handle_report_pool_window_removed(state, label)
        }
        // Phase F.5 — host-only signal that a pool→window promote is
        // happening. Sent BETWEEN the matching `ReportPoolWindowRemoved`
        // and `ReportWindowOpened` so the launcher can disambiguate
        // promote from pre-promote destroy. Pure-reducer arm: emits
        // the typed event; saga side-effect (start the pool-respawn
        // saga) lives in the saga coordinator's bus subscription.
        Command::ReportPoolWindowPromoted { label } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportPoolWindowPromoted") {
                return Reduced { events: vec![err], diagnostics };
            }
            pool::handle_report_pool_window_promoted(state, label)
        }
        // Phase F.5 — `SpawnPoolWindow` is a launcher→host direction
        // command, NOT a host→launcher report. If a registered client
        // sends it to the launcher pipe by mistake, return a non-fatal
        // error so the client knows the dispatch was wrong (vs silently
        // appearing successful with no reply). Same misrouted-error
        // pattern as the srv-pipe commands below.
        Command::SpawnPoolWindow { .. } => host_pipe_only(state, "SpawnPoolWindow"),
        // Phase F.6 — host-only signal that all browser-pane HWNDs
        // belonging to a closing top-level window have been reaped.
        // Pure-reducer arm: emits the typed event so the
        // window-cleanup-cascade saga can advance from Step 1
        // (ReapingPanes) to Step 2 (DrainingPool). State is
        // unchanged — pane bookkeeping lives in the host's session
        // structures (lifecycle entries, pane HWND map), not the
        // launcher's mirror; the launcher just narrates the
        // transition for subscribers.
        Command::ReportPanesReaped { label, saga_id } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportPanesReaped") {
                return Reduced { events: vec![err], diagnostics };
            }
            saga::handle_report_panes_reaped(state, label, saga_id)
        }
        // Phase F.6 — host reports the result of the post-close
        // drain-pool-if-last decision. `was_last == true` →
        // `Event::PoolDrained`; `was_last == false` →
        // `Event::PoolNotLast`. Both are terminal alternatives for
        // the window-cleanup-cascade saga's Step 2.
        Command::ReportPoolDrainDecision {
            label,
            was_last,
            saga_id,
        } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportPoolDrainDecision") {
                return Reduced { events: vec![err], diagnostics };
            }
            pool::handle_report_pool_drain_decision(state, label, was_last, saga_id)
        }
        // Phase F.6 — `ReapPanes` and `DrainPoolIfLast` are
        // launcher→host direction commands, NOT host→launcher
        // reports. Same misrouted-error pattern as `SpawnPoolWindow`
        // above.
        Command::ReapPanes { .. } => host_pipe_only(state, "ReapPanes"),
        Command::DrainPoolIfLast { .. } => host_pipe_only(state, "DrainPoolIfLast"),
        // Phase CPD-1 — host-emitted report that a saga-issued action
        // failed. Pure pass-through arm: translates the wire command
        // into `Event::SagaActionFailed`. The saga coordinator's bus
        // loop (CPD-3) will treat this as a terminal signal for the
        // matching `saga_id` and emit `Event::SagaFailed`. Host-only
        // gate same as other Report* arms.
        Command::ReportSagaActionFailed { saga_id, reason } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportSagaActionFailed") {
                return Reduced { events: vec![err], diagnostics };
            }
            saga::handle_report_saga_action_failed(state, saga_id, reason)
        }
        // Startup-stage telemetry is NOT mirrored State — it's forwarded
        // directly into the launcher's StartupEventSink by a short-circuit
        // in ipc/server.rs BEFORE the command ever reaches this reducer
        // (same pattern as GetEvents). These arms exist only to satisfy
        // exhaustiveness; they are unreachable at runtime.
        Command::ReportStartupStageBegin { .. } | Command::ReportStartupStageEnd { .. } => {
            vec![]
        }
        Command::ReportHostCounts { windows, pool } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHostCounts") {
                return Reduced { events: vec![err], diagnostics };
            }
            handle_report_host_counts(state, windows, pool)
        }
        Command::ReportHostPoolCount { count } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHostPoolCount") {
                return Reduced { events: vec![err], diagnostics };
            }
            pool::handle_report_host_pool_count(state, count)
        }
        Command::ReportBackendWindowIdRegistered { label, window_id } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportBackendWindowIdRegistered") {
                return Reduced { events: vec![err], diagnostics };
            }
            window::handle_report_backend_window_id_registered(state, label, window_id)
        }
        Command::ReportBackendWindowIdUnregistered { label } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportBackendWindowIdUnregistered") {
                return Reduced { events: vec![err], diagnostics };
            }
            window::handle_report_backend_window_id_unregistered(state, label)
        }
        // Phase B.9.1 (WRR) — Win32 reality events. Host-only
        // because only the host installs the SetWinEventHook /
        // wndproc wrapper. Same enforce-host pattern as the other
        // observability reports.
        Command::ReportHwndOpened { hwnd, class_name, title, label_hint } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHwndOpened") {
                return Reduced { events: vec![err], diagnostics };
            }
            crate::wrr::apply_hwnd_opened(
                state,
                hwnd,
                class_name,
                title,
                label_hint,
                ctx.now_ms,
                &mut diagnostics,
            )
        }
        Command::ReportHwndDestroyed { hwnd } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHwndDestroyed") {
                return Reduced { events: vec![err], diagnostics };
            }
            let host_running = connection::host_is_running(state);
            crate::wrr::apply_hwnd_destroyed(state, hwnd, host_running)
        }
        Command::ReportHwndVisibilityChanged { hwnd, visible } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHwndVisibilityChanged") {
                return Reduced { events: vec![err], diagnostics };
            }
            crate::wrr::apply_hwnd_visibility_changed(state, hwnd, visible, ctx.now_ms)
        }
        Command::ReportHwndForegroundChanged { hwnd } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHwndForegroundChanged") {
                return Reduced { events: vec![err], diagnostics };
            }
            crate::wrr::apply_hwnd_foreground_changed(state, hwnd, ctx.now_ms)
        }
        Command::ReportHwndIconicChanged { hwnd, iconic } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHwndIconicChanged") {
                return Reduced { events: vec![err], diagnostics };
            }
            crate::wrr::apply_hwnd_iconic_changed(state, hwnd, iconic)
        }
        Command::ReportHwndPositionChanged { hwnd, rect } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportHwndPositionChanged") {
                return Reduced { events: vec![err], diagnostics };
            }
            crate::wrr::apply_hwnd_position_changed(state, hwnd, rect, &mut diagnostics)
        }
        Command::ReportMonitorTopologyChanged { rects } => {
            if let Some(err) = connection::enforce_host_only(state, ctx, "ReportMonitorTopologyChanged") {
                return Reduced { events: vec![err], diagnostics };
            }
            crate::wrr::apply_monitor_topology_changed(state, rects)
        }
        // Phase D.1 — read-only snapshot of canonical state. Any
        // registered client can ask; no host-only gate. Reducer state
        // is unchanged by this command (it's a query, not a mutation),
        // but we still bump `event_version` so the snapshot's version
        // is monotonically distinct from prior events — a subscriber
        // applying snapshot + delta events knows the snapshot's
        // version is the "as-of" point.
        Command::GetSnapshot => connection::handle_get_snapshot(state),
        // Phase D.3 — `GetEvents` is intercepted by the IPC server's
        // dispatch path BEFORE reaching the reducer (it's a non-
        // mutating read against the event log, which is I/O-adjacent
        // — keeping it out of the pure reducer preserves the
        // "reducer never blocks" invariant). This arm exists only
        // to satisfy the exhaustive match; in practice it's
        // unreachable. Returning empty Vec is the safe no-op.
        Command::GetEvents { .. } => Vec::new(),
        // Phase E.2 — srv-pipe domain commands routed to launcher
        // pipe by mistake.
        Command::CreateWorkspace { .. } => srv_pipe_only(state, "CreateWorkspace"),
        Command::DeleteWorkspace { .. } => srv_pipe_only(state, "DeleteWorkspace"),
        // Phase E.2b — Tab arms are also srv-pipe commands.
        Command::CreateTab { .. } => srv_pipe_only(state, "CreateTab"),
        Command::DeleteTab { .. } => srv_pipe_only(state, "DeleteTab"),
        Command::SetActiveTab { .. } => srv_pipe_only(state, "SetActiveTab"),
        Command::ReorderTab { .. } => srv_pipe_only(state, "ReorderTab"),
        // Phase E.5 — window↔workspace mapping commands are srv-pipe.
        Command::CreateWindow { .. } => srv_pipe_only(state, "CreateWindow"),
        Command::CloseWindowInternal { .. } => srv_pipe_only(state, "CloseWindowInternal"),
        Command::SwitchWorkspace { .. } => srv_pipe_only(state, "SwitchWorkspace"),
        // Phase E.5.3 — atomic single-step domain commands are srv-pipe.
        Command::ReorderTabsBulk { .. } => srv_pipe_only(state, "ReorderTabsBulk"),
        Command::RenameWorkspace { .. } => srv_pipe_only(state, "RenameWorkspace"),
        Command::RenameTab { .. } => srv_pipe_only(state, "RenameTab"),
        Command::UpdateWorkspaceMeta { .. } => srv_pipe_only(state, "UpdateWorkspaceMeta"),
        Command::UpdateTabMeta { .. } => srv_pipe_only(state, "UpdateTabMeta"),
        Command::UpdateBlockMeta { .. } => srv_pipe_only(state, "UpdateBlockMeta"),
        // Phase E.3 — Block arms are also srv-pipe commands.
        Command::CreateBlock { .. } => srv_pipe_only(state, "CreateBlock"),
        Command::DeleteBlock { .. } => srv_pipe_only(state, "DeleteBlock"),
        // Phase E.5.5 — saga-driven move commands are srv-pipe.
        Command::MoveTab { .. } => srv_pipe_only(state, "MoveTab"),
        Command::MoveBlock { .. } => srv_pipe_only(state, "MoveBlock"),
        // Phase E.4 (Option A) — layout-focused/magnified setters are
        // srv-pipe commands. Misrouted to the launcher pipe, return
        // a non-fatal error so the client knows the dispatch was wrong.
        Command::SetFocusedNode { .. } => srv_pipe_only(state, "SetFocusedNode"),
        Command::SetMagnifiedNode { .. } => srv_pipe_only(state, "SetMagnifiedNode"),
        // Phase E.1b — `GetSrvSnapshot` is a srv-pipe command. If a
        // registered client misroutes it to the launcher pipe, return
        // an explicit error so the client knows the dispatch was
        // wrong (vs silently appearing successful with no reply).
        // Pre-Register, `enforce_register_first` already returns a
        // soft error. (codex P2 #610.)
        Command::GetSrvSnapshot => srv_pipe_only(state, "GetSrvSnapshot"),
        // Phase E.4.B — layout-tree commands are srv-pipe only; same
        // soft-error treatment as GetSrvSnapshot above.
        Command::LayoutDeleteNode { .. }
        | Command::LayoutDeleteNodeByBlock { .. }
        | Command::LayoutStackPush { .. }
        | Command::LayoutStackActivate { .. }
        | Command::CreateBlockInStack { .. }
        | Command::LayoutStackMove { .. }
        | Command::LayoutApplyIntent { .. }
        | Command::LayoutQueueBackendActions { .. }
        | Command::LayoutSetTree { .. }
        | Command::UpdateWindowMeta { .. } => misrouted(
            state,
            "Srv-pipe command (Layout/UpdateWindowMeta) sent to launcher pipe by mistake".to_string(),
        ),
    };

    // Heartbeat-via-traffic: drain any deferred hidden-since-open
    // drifts AFTER the command processes. Running this AFTER (not
    // before) is critical — the command itself may legitimately clear
    // the deferred state (visible=true / foreground / window closed),
    // and a pre-command drain would fire spurious drift on an event
    // whose own purpose is the recovery (codex P2 PR #725 round 2).
    //
    // Catches windows that hid during placement and produced no
    // further visibility events: any subsequent unrelated command
    // past the grace promotes the deferred state to a fired drift.
    let mut deferred = crate::wrr::drain_deferred_hidden_since_open(state, ctx.now_ms);
    cmd_events.append(&mut deferred);
    Reduced { events: cmd_events, diagnostics }
}

/// A command that belongs on another pipe reached the launcher's. Reply
/// with a non-fatal error so the client knows its dispatch was wrong,
/// rather than seeing no reply at all.
fn misrouted(state: &mut State, message: String) -> Vec<Event> {
    let v = state.bump_version();
    vec![Event::Error {
        code: ErrorCode::InvalidCommand,
        message,
        fatal: false,
        version: v,
    }]
}

/// `misrouted` for a command only srv's pipe handles.
fn srv_pipe_only(state: &mut State, command: &str) -> Vec<Event> {
    misrouted(
        state,
        format!("{command} is a srv-pipe command; sent to launcher pipe by mistake"),
    )
}

/// `misrouted` for a command the launcher sends to the host, never receives.
fn host_pipe_only(state: &mut State, command: &str) -> Vec<Event> {
    misrouted(
        state,
        format!("{command} is a launcher→host command; sent to launcher pipe by mistake"),
    )
}

/// Phase B.4 follow-up — drift check. Compares host-reported counts
/// to launcher mirror counts; emits `DriftDetected` for each
/// disagreeing dimension. Returns `[]` when both counts match (the
/// happy path; mirrors are in sync).
///
/// Two events possible (windows + pool) when both diverge in the
/// same report — emitted in a stable order (windows first) so test
/// assertions don't depend on HashSet iteration order.
fn handle_report_host_counts(state: &mut State, host_windows: u32, host_pool: u32) -> Vec<Event> {
    let mut out = Vec::new();
    let mirror_windows = state.windows.len() as u32;
    let mirror_pool = state.pool.len() as u32;
    if mirror_windows != host_windows {
        let v = state.bump_version();
        out.push(Event::DriftDetected {
            kind: DriftKind::Windows,
            host_count: host_windows,
            mirror_count: mirror_windows,
            version: v,
        });
    }
    if mirror_pool != host_pool {
        let v = state.bump_version();
        out.push(Event::DriftDetected {
            kind: DriftKind::Pool,
            host_count: host_pool,
            mirror_count: mirror_pool,
            version: v,
        });
    }
    out
}

#[cfg(test)]
mod tests;
