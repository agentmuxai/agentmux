// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.3 — pure reducer.
//
// Per `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §5.1:
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
//      the duration of update — must stay sub-millisecond.
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

use agentmux_common::ipc::{ClientKind, Command, DriftKind, ErrorCode, Event, HwndDriftKind, WindowKind};

use crate::state::{LifecyclePhase, ProcessRecord, ProcessState, State, WindowMirror};

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

/// Apply one Command to State, returning the resulting Events. State
/// is mutated in place. Total function — never panics on input
/// (panics are reserved for internal invariant violations).
mod pool;

pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    let _ = ctx.conn_id; // reserved for B.4 routing
    match cmd {
        Command::Register {
            kind,
            pid,
            version,
        } => handle_register(state, ctx, kind, pid, version),
        Command::Ping { nonce } => {
            let v = state.bump_version();
            vec![Event::Pong { nonce, version: v }]
        }
        Command::Goodbye => handle_goodbye(state, ctx.registered_pid.unwrap_or(0)),
        Command::ReportWindowOpened {
            label,
            kind,
            parent_label,
        } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportWindowOpened") {
                return vec![err];
            }
            handle_report_window_opened(state, ctx, label, kind, parent_label)
        }
        Command::ReportWindowClosed { label } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportWindowClosed") {
                return vec![err];
            }
            handle_report_window_closed(state, label)
        }
        Command::ReportPoolWindowAdded { label, saga_id } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportPoolWindowAdded") {
                return vec![err];
            }
            pool::handle_report_pool_window_added(state, label, saga_id)
        }
        Command::ReportPoolWindowRemoved { label } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportPoolWindowRemoved") {
                return vec![err];
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
            if let Some(err) = enforce_host_only(state, ctx, "ReportPoolWindowPromoted") {
                return vec![err];
            }
            pool::handle_report_pool_window_promoted(state, label)
        }
        // Phase F.5 — `SpawnPoolWindow` is a launcher→host direction
        // command, NOT a host→launcher report. If a registered client
        // sends it to the launcher pipe by mistake, return a non-fatal
        // error so the client knows the dispatch was wrong (vs silently
        // appearing successful with no reply). Same misrouted-error
        // pattern as the srv-pipe commands below.
        Command::SpawnPoolWindow { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "SpawnPoolWindow is a launcher→host command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
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
            if let Some(err) = enforce_host_only(state, ctx, "ReportPanesReaped") {
                return vec![err];
            }
            handle_report_panes_reaped(state, label, saga_id)
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
            if let Some(err) = enforce_host_only(state, ctx, "ReportPoolDrainDecision") {
                return vec![err];
            }
            pool::handle_report_pool_drain_decision(state, label, was_last, saga_id)
        }
        // Phase F.6 — `ReapPanes` and `DrainPoolIfLast` are
        // launcher→host direction commands, NOT host→launcher
        // reports. Same misrouted-error pattern as `SpawnPoolWindow`
        // above.
        Command::ReapPanes { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "ReapPanes is a launcher→host command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::DrainPoolIfLast { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "DrainPoolIfLast is a launcher→host command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase CPD-1 — host-emitted report that a saga-issued action
        // failed. Pure pass-through arm: translates the wire command
        // into `Event::SagaActionFailed`. The saga coordinator's bus
        // loop (CPD-3) will treat this as a terminal signal for the
        // matching `saga_id` and emit `Event::SagaFailed`. Host-only
        // gate same as other Report* arms.
        Command::ReportSagaActionFailed { saga_id, reason } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportSagaActionFailed") {
                return vec![err];
            }
            handle_report_saga_action_failed(state, saga_id, reason)
        }
        Command::ReportHostCounts { windows, pool } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHostCounts") {
                return vec![err];
            }
            handle_report_host_counts(state, windows, pool)
        }
        Command::ReportHostPoolCount { count } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHostPoolCount") {
                return vec![err];
            }
            pool::handle_report_host_pool_count(state, count)
        }
        Command::ReportBackendWindowIdRegistered { label, window_id } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportBackendWindowIdRegistered") {
                return vec![err];
            }
            handle_report_backend_window_id_registered(state, label, window_id)
        }
        Command::ReportBackendWindowIdUnregistered { label } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportBackendWindowIdUnregistered") {
                return vec![err];
            }
            handle_report_backend_window_id_unregistered(state, label)
        }
        // Phase B.9.1 (WRR) — Win32 reality events. Host-only
        // because only the host installs the SetWinEventHook /
        // wndproc wrapper. Same enforce-host pattern as the other
        // observability reports.
        Command::ReportHwndOpened { hwnd, class_name, title, label_hint } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHwndOpened") {
                return vec![err];
            }
            crate::wrr::apply_hwnd_opened(state, hwnd, class_name, title, label_hint, ctx.now_ms)
        }
        Command::ReportHwndDestroyed { hwnd } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHwndDestroyed") {
                return vec![err];
            }
            crate::wrr::apply_hwnd_destroyed(state, hwnd)
        }
        Command::ReportHwndVisibilityChanged { hwnd, visible } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHwndVisibilityChanged") {
                return vec![err];
            }
            crate::wrr::apply_hwnd_visibility_changed(state, hwnd, visible)
        }
        Command::ReportHwndForegroundChanged { hwnd } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHwndForegroundChanged") {
                return vec![err];
            }
            crate::wrr::apply_hwnd_foreground_changed(state, hwnd, ctx.now_ms)
        }
        Command::ReportHwndIconicChanged { hwnd, iconic } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHwndIconicChanged") {
                return vec![err];
            }
            crate::wrr::apply_hwnd_iconic_changed(state, hwnd, iconic)
        }
        Command::ReportHwndPositionChanged { hwnd, rect } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportHwndPositionChanged") {
                return vec![err];
            }
            crate::wrr::apply_hwnd_position_changed(state, hwnd, rect)
        }
        Command::ReportMonitorTopologyChanged { rects } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportMonitorTopologyChanged") {
                return vec![err];
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
        Command::GetSnapshot => handle_get_snapshot(state),
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
        Command::CreateWorkspace { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "CreateWorkspace is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::DeleteWorkspace { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "DeleteWorkspace is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase E.2b — Tab arms are also srv-pipe commands.
        Command::CreateTab { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "CreateTab is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::DeleteTab { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "DeleteTab is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::SetActiveTab { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "SetActiveTab is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::ReorderTab { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "ReorderTab is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase E.5 — window↔workspace mapping commands are srv-pipe.
        Command::CreateWindow { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "CreateWindow is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::CloseWindowInternal { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "CloseWindowInternal is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::SwitchWorkspace { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "SwitchWorkspace is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase E.5.3 — atomic single-step domain commands are srv-pipe.
        Command::ReorderTabsBulk { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "ReorderTabsBulk is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::RenameWorkspace { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "RenameWorkspace is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::RenameTab { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "RenameTab is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::UpdateWorkspaceMeta { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "UpdateWorkspaceMeta is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::UpdateTabMeta { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "UpdateTabMeta is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::UpdateBlockMeta { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "UpdateBlockMeta is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase E.3 — Block arms are also srv-pipe commands.
        Command::CreateBlock { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "CreateBlock is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::DeleteBlock { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "DeleteBlock is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase E.5.5 — saga-driven move commands are srv-pipe.
        Command::MoveTab { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "MoveTab is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::MoveBlock { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "MoveBlock is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase E.4 (Option A) — layout-focused/magnified setters are
        // srv-pipe commands. Misrouted to the launcher pipe, return
        // a non-fatal error so the client knows the dispatch was wrong.
        Command::SetFocusedNode { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "SetFocusedNode is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        Command::SetMagnifiedNode { .. } => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "SetMagnifiedNode is a srv-pipe command; sent to launcher pipe by mistake".into(),
                fatal: false,
                version: v,
            }]
        }
        // Phase E.1b — `GetSrvSnapshot` is a srv-pipe command. If a
        // registered client misroutes it to the launcher pipe, return
        // an explicit error so the client knows the dispatch was
        // wrong (vs silently appearing successful with no reply).
        // Pre-Register, `enforce_register_first` already returns a
        // soft error. (codex P2 #610.)
        Command::GetSrvSnapshot => {
            let v = state.bump_version();
            vec![Event::Error {
                code: agentmux_common::ipc::ErrorCode::InvalidCommand,
                message: "GetSrvSnapshot is a srv-pipe command; sent to launcher pipe by mistake".to_string(),
                fatal: false,
                version: v,
            }]
        }
    }
}

/// Phase B.5 (window_id_map step a) — record the host-reported
/// label → backend_window_id mapping. Idempotent on duplicate
/// label (overwrites with the new ID and emits a fresh event so
/// subscribers see the latest mapping).
fn handle_report_backend_window_id_registered(
    state: &mut State,
    label: String,
    window_id: String,
) -> Vec<Event> {
    state
        .backend_window_ids
        .insert(label.clone(), window_id.clone());
    let v = state.bump_version();
    vec![Event::BackendWindowIdRegistered {
        label,
        window_id,
        version: v,
    }]
}

/// Phase B.5 (window_id_map step a) — drop the host-reported label
/// from the map. Strict pairing: emits `BackendWindowIdUnregistered`
/// only when the label was present (mirrors `WindowClosed` and
/// `PoolWindowRemoved` semantics — codex P2 PR #577 round-2).
fn handle_report_backend_window_id_unregistered(
    state: &mut State,
    label: String,
) -> Vec<Event> {
    let removed = state.backend_window_ids.remove(&label);
    let Some(window_id) = removed else {
        return vec![];
    };
    let v = state.bump_version();
    vec![Event::BackendWindowIdUnregistered {
        label,
        window_id,
        version: v,
    }]
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


/// Phase F.6 — host-emitted signal that browser-pane HWNDs for a
/// closing top-level window have been reaped. Pure pass-through:
/// state stays untouched (the host owns pane bookkeeping); the
/// reducer just translates the wire command into the typed event so
/// the window-cleanup-cascade saga can advance.
///
/// Idempotent / context-free: the saga matches the `label` against
/// its own `closed_label`, so a stray report for a label that no
/// in-flight saga is tracking is a harmless broadcast.
fn handle_report_panes_reaped(
    state: &mut State,
    label: String,
    saga_id: Option<u64>,
) -> Vec<Event> {
    // No state.windows gate — round 4's gate had an ordering bug:
    // host sends ReportWindowClosed BEFORE ReportPanesReaped on the
    // same channel, so by the time the reducer processes this, the
    // label is already gone from state.windows (closed by the prior
    // command's reducer arm). The gate then dropped EVERY
    // PanesReaped, leaving the F.6 saga stuck in WaitingForPanesReaped
    // indefinitely. Round 5 reversal: emit unconditionally; for
    // unpromoted-pool drains where no saga is in flight, the event
    // appears stray on the bus but is harmless (no subscriber acts
    // on it). Cosmetic only; correct saga lifecycle restored.
    //
    // CPD-1: `saga_id` flows through unchanged (None for organic
    // reports, Some(N) once CPD-3 hosts echo back the saga's id).
    let v = state.bump_version();
    vec![Event::PanesReaped {
        label,
        version: v,
        saga_id,
    }]
}

fn handle_report_saga_action_failed(
    state: &mut State,
    saga_id: u64,
    reason: String,
) -> Vec<Event> {
    let v = state.bump_version();
    vec![Event::SagaActionFailed {
        saga_id,
        reason,
        version: v,
    }]
}

/// Phase B.4 — gate window-mirror reports to Host clients only. The
/// host is the only source of truth about its own window lifecycle;
/// allowing Renderer/Srv/Tool clients to mutate the mirror would let
/// any registered process spoof open/close traffic and break the
/// host-authoritative model. (codex P1 PR #576 round-1.)
///
/// Returns `Some(Error)` if the calling connection is not a Host;
/// `None` if the call is allowed to proceed. Looks up the kind by
/// PID rather than threading it through `Ctx` because `processes`
/// is already the canonical source — single source of truth, no
/// extra plumbing.
fn enforce_host_only(state: &mut State, ctx: &Ctx, op: &'static str) -> Option<Event> {
    let kind = ctx
        .registered_pid
        .and_then(|pid| state.processes.get(&pid).map(|r| r.kind));
    if kind == Some(ClientKind::Host) {
        return None;
    }
    let v = state.bump_version();
    Some(Event::Error {
        code: ErrorCode::NotRegistered,
        message: format!(
            "{} is Host-only; caller kind={:?}",
            op, kind
        ),
        fatal: false,
        version: v,
    })
}

/// Phase B.4 — record a host-reported window opening in the launcher's
/// read-only mirror. Idempotent on duplicate opens (same label twice
/// in a row): the second insert overwrites with fresh metadata and
/// emits a fresh event. Subscribers must tolerate seeing the same
/// label twice; cleaner once B.5 makes the launcher authoritative.
///
/// Phase B.5: also assigns an authoritative instance number from
/// `state.instance_registry` and emits `WindowInstanceAssigned`.
/// "main" is pre-seeded with 1; other labels get the next value of
/// `next_instance_num`. Re-opens of an existing label preserve the
/// original number — instance numbers are stable per-label-per-run.
fn handle_report_window_opened(
    state: &mut State,
    ctx: &Ctx,
    label: String,
    kind: WindowKind,
    parent_label: Option<String>,
) -> Vec<Event> {
    // PR #664 codex P1 round 2 — drain-on-WindowOpened RESTORED as
    // best-effort fallback. The explicit `ReportHwndOpened(Some(label))`
    // from `client.rs::on_after_created` remains the AUTHORITATIVE
    // link, and `apply_hwnd_opened` REPAIRS stale links it finds.
    // But that explicit dispatch is gated on `hwnd_val != 0` host-side;
    // if the HWND can't be resolved at on_after_created time from
    // either of the 2 sources (Views, host), the explicit dispatch
    // is skipped and the mirror would otherwise stay permanently
    // unlinked, breaking WRR drift detection AND orphan-destroy
    // reconciliation (no WindowClosed when OS destroys the HWND →
    // permanent ghost InstancePanel rows).
    //
    // (PR #664 round 4 dropped a 3rd fallback `find_own_top_level_window`
    // because it returns the FIRST visible window in the process —
    // some other window's HWND in a multi-window session — which
    // would corrupt other labels' mirrors via the `Repaired` arm.
    // See client.rs::on_after_created comment for details.)
    //
    // The drain provides a fallback link from `pending_hwnds`. If the
    // drain picks the WRONG HWND (the original burst-create race), the
    // subsequent `apply_hwnd_opened` call from on_after_created will
    // detect the mismatch and REPAIR — see the `Repaired` arm there.
    // Net: best-effort link via drain, authoritative repair via
    // explicit dispatch. The combination addresses both the
    // hwnd_val=0 case and the burst-create race.
    const PENDING_AGE_LIMIT_MS: u64 = 2_000;
    let drained_hwnd: Option<u64> = state
        .pending_hwnds
        .iter()
        .filter(|(_, p)| p.label_hint.is_none())
        .filter(|(_, p)| ctx.now_ms.saturating_sub(p.arrived_at_ms) <= PENDING_AGE_LIMIT_MS)
        .max_by_key(|(_, p)| p.arrived_at_ms)
        .map(|(hwnd, _)| *hwnd);
    if let Some(hwnd) = drained_hwnd {
        state.pending_hwnds.remove(&hwnd);
    }

    state.windows.insert(
        label.clone(),
        WindowMirror {
            label: label.clone(),
            kind,
            parent_label: parent_label.clone(),
            opened_at: ctx.now_rfc3339.clone(),
            // Best-effort drain above; authoritative explicit
            // ReportHwndOpened from on_after_created arrives a few
            // ms later via `apply_hwnd_opened` and REPAIRS any wrong
            // link the drain picked.
            hwnd: drained_hwnd,
            visible: false,
            iconic: false,
            last_rect: None,
            last_foreground_at_ms: None,
            foregrounded_since_open: false,
        },
    );
    let mut out = Vec::with_capacity(2);
    let v = state.bump_version();
    out.push(Event::WindowOpened {
        label: label.clone(),
        kind,
        parent_label,
        version: v,
    });

    // Assign instance number if this label isn't already in the
    // registry. Re-opens of an existing label keep the original
    // number — matches host's `WindowInstanceRegistry` semantics
    // where a label is only registered once per session.
    let num = if let Some(existing) = state.instance_registry.get(&label).copied() {
        existing
    } else {
        let n = state.next_instance_num;
        state.instance_registry.insert(label.clone(), n);
        state.next_instance_num += 1;
        n
    };
    let v = state.bump_version();
    out.push(Event::WindowInstanceAssigned { label, num, version: v });
    out
}

/// Phase B.4 — drop a host-reported window from the mirror. Returns
/// `Event::WindowClosed` only when the label was actually in the
/// mirror; an unknown-label close is a silent no-op (codex P2 PR
/// #577 round-2). Without this gate, a `ReportWindowClosed` for a
/// label the launcher never saw (e.g. a pool window that was popped
/// from the queue but failed HWND validation in
/// `promote_pool_window` — the orphan window's eventual
/// `on_before_close` reaches us without a matching open) would
/// emit an unpaired `WindowClosed` broadcast and break subscribers
/// that assume open/close pairing.
///
/// Phase B.5 — also drops the label from `instance_registry` and
/// emits `WindowInstanceReleased` if a number was assigned.
/// `next_instance_num` is NOT decremented — instance numbers are
/// monotonic per-launcher-run.
fn handle_report_window_closed(state: &mut State, label: String) -> Vec<Event> {
    let was_present = state.windows.remove(&label).is_some();
    if !was_present {
        // Silent: only emit when the close pairs with a known open.
        return vec![];
    }
    let mut out = Vec::with_capacity(4);
    let v = state.bump_version();
    out.push(Event::WindowClosed {
        label: label.clone(),
        version: v,
        // Clean close — host ran on_before_close before sending
        // ReportWindowClosed. F.6 saga is safe to trigger.
        crash_detected: false,
    });
    if let Some(num) = state.instance_registry.remove(&label) {
        let v = state.bump_version();
        out.push(Event::WindowInstanceReleased { label: label.clone(), num, version: v });
    }

    // Phase B.9.3 — OrphanInstance transition. The label we just
    // removed was the LAST user-visible window (state.windows is
    // now empty). If a Host is still registered as Running, its
    // own close path won't quit_message_loop because the warm
    // pool is keeping state.browsers non-empty. Emit drift +
    // saga-style HostShouldQuit so the host can reap pool and
    // quit cleanly. See B.9.3 in
    // docs/retro/next-steps-2026-04-29.md.
    if state.windows.is_empty() && host_is_running(state) {
        let v_drift = state.bump_version();
        out.push(Event::HwndDriftDetected {
            kind: HwndDriftKind::OrphanInstance,
            label: Some(label),
            hwnd: None,
            detail: "Last user-visible window closed; host still alive (likely holding warm pool)"
                .to_string(),
            severity: agentmux_common::ipc::Severity::Warn,
            version: v_drift,
        });
        let v_quit = state.bump_version();
        out.push(Event::HostShouldQuit { version: v_quit });
    }
    out
}

/// Phase B.9.3 — does state.processes contain a Host in the
/// `Running` lifecycle? Used by the OrphanInstance transition
/// check; without this guard, a stale Exited record would fire
/// HostShouldQuit on every benign close. Tool-only sessions
/// (`agentmux.exe --diag`) also correctly skip the saga because
/// they never register a Host.
fn host_is_running(state: &State) -> bool {
    use crate::state::ProcessState;
    state.processes.values().any(|r| {
        r.kind == ClientKind::Host && matches!(r.state, ProcessState::Running)
    })
}

/// Phase D.1 — clone the reducer's canonical state into a `Snapshot`
/// event. Read-only; doesn't mutate state except for bumping
/// `event_version` so the snapshot's version is monotonically
/// distinct from prior events (subscribers applying snapshot + delta
/// events know the snapshot is "as-of" this version).
///
/// Sorted-vec serialization (rather than HashMap-as-JSON-object) for:
/// 1. Deterministic ordering across snapshots (idempotent diffs in
///    operator output, easier test assertions).
/// 2. Wire compatibility with `Vec<(K, V)>` decoders that don't
///    require canonical-string-keyed JSON objects.
fn handle_get_snapshot(state: &mut State) -> Vec<Event> {
    let v = state.bump_version();

    let mut windows: Vec<agentmux_common::ipc::WindowSnapshot> = state
        .windows
        .values()
        .map(|w| agentmux_common::ipc::WindowSnapshot {
            label: w.label.clone(),
            kind: w.kind,
            parent_label: w.parent_label.clone(),
            hwnd: w.hwnd,
            visible: w.visible,
            iconic: w.iconic,
            last_rect: w.last_rect,
            foregrounded_since_open: w.foregrounded_since_open,
        })
        .collect();
    windows.sort_by(|a, b| a.label.cmp(&b.label));

    let mut pool: Vec<String> = state.pool.iter().cloned().collect();
    pool.sort();

    let mut instance_registry: Vec<(String, u32)> =
        state.instance_registry.iter().map(|(k, v)| (k.clone(), *v)).collect();
    instance_registry.sort_by(|a, b| a.0.cmp(&b.0));

    let mut backend_window_ids: Vec<(String, String)> = state
        .backend_window_ids
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    backend_window_ids.sort_by(|a, b| a.0.cmp(&b.0));

    vec![Event::Snapshot {
        version: v,
        lifecycle: state.lifecycle,
        windows,
        pool,
        instance_registry,
        backend_window_ids,
        monitors: state.monitors.clone(),
    }]
}

fn handle_register(
    state: &mut State,
    ctx: &Ctx,
    kind: ClientKind,
    pid: u32,
    version: String,
) -> Vec<Event> {
    let mut out = Vec::with_capacity(3);

    // Cross-connection invariant: only ONE live ProcessRecord per
    // PID. We DO allow re-registration if the existing record is
    // Exited — the OS recycles PIDs over a long-running launcher,
    // so a new process can legitimately end up with a PID that was
    // previously held by a process that has cleanly Goodbye'd.
    // Without this carve-out, the process map would accumulate dead
    // records and the launcher would reject increasingly many real
    // registrations. (gemini MEDIUM PR #574 round-1.)
    let existing_state = state.processes.get(&pid).map(|r| r.state);
    if let Some(existing_state) = existing_state {
        if !matches!(existing_state, ProcessState::Exited { .. }) {
            let v = state.bump_version();
            out.push(Event::Error {
                code: ErrorCode::AlreadyRegistered,
                message: format!(
                    "pid {} already in process registry (state={:?})",
                    pid, existing_state
                ),
                fatal: true,
                version: v,
            });
            return out;
        }
        // Else: fall through. The insert below replaces the Exited
        // record with the new live one — same entry, fresh state.
    }

    let record = ProcessRecord {
        pid,
        kind,
        state: ProcessState::Running,
        spawned_at: ctx.now_rfc3339.clone(),
        version: version.clone(),
    };
    state.processes.insert(pid, record);

    let spawned_v = state.bump_version();
    out.push(Event::ProcessSpawned {
        pid,
        kind,
        client_version: version,
        version: spawned_v,
    });

    // Lifecycle: Starting → Running when the first Host registers.
    // Subsequent Host re-registers (after a host crash + restart in
    // some future world) won't double-fire because we'd already be
    // in Running. Other client kinds (Renderer, Srv, Tool) don't
    // drive the transition.
    if state.lifecycle == LifecyclePhase::Starting && kind == ClientKind::Host {
        let from = state.lifecycle;
        state.lifecycle = LifecyclePhase::Running;
        let v = state.bump_version();
        out.push(Event::LifecyclePhaseChanged {
            from,
            to: LifecyclePhase::Running,
            version: v,
        });
    }

    let registered_v = state.bump_version();
    let client_id = state.alloc_client_id();
    out.push(Event::Registered {
        client_id,
        // launcher_pid + launcher_version are filled in by the
        // server before broadcast — they don't belong in the pure
        // reducer (env reads). We use a sentinel here; the server
        // patches these before sending.
        launcher_pid: 0,
        launcher_version: String::new(),
        version: registered_v,
    });

    out
}

fn handle_goodbye(state: &mut State, pid: u32) -> Vec<Event> {
    if pid == 0 {
        // No pid known for this connection (B.3 limitation). Just
        // emit a synthetic ProcessExited with pid=0 to signal the
        // graceful close; the server will log + close.
        let v = state.bump_version();
        return vec![Event::ProcessExited {
            pid: 0,
            code: 0,
            version: v,
        }];
    }
    if let Some(record) = state.processes.get_mut(&pid) {
        record.state = ProcessState::Exited { code: 0 };
    }
    let v = state.bump_version();
    vec![Event::ProcessExited {
        pid,
        code: 0,
        version: v,
    }]
}


#[cfg(test)]
mod tests;
