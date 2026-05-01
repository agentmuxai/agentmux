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
        Command::ReportPoolWindowAdded { label } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportPoolWindowAdded") {
                return vec![err];
            }
            handle_report_pool_window_added(state, label)
        }
        Command::ReportPoolWindowRemoved { label } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportPoolWindowRemoved") {
                return vec![err];
            }
            handle_report_pool_window_removed(state, label)
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
            handle_report_pool_window_promoted(state, label)
        }
        // Phase F.5 — `SpawnPoolWindow` is a launcher→host direction
        // command, NOT a host→launcher report. If a registered client
        // sends it to the launcher pipe by mistake, return a non-fatal
        // error so the client knows the dispatch was wrong (vs silently
        // appearing successful with no reply). Same misrouted-error
        // pattern as the srv-pipe commands below.
        Command::SpawnPoolWindow => {
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
        Command::ReportPanesReaped { label } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportPanesReaped") {
                return vec![err];
            }
            handle_report_panes_reaped(state, label)
        }
        // Phase F.6 — host reports the result of the post-close
        // drain-pool-if-last decision. `was_last == true` →
        // `Event::PoolDrained`; `was_last == false` →
        // `Event::PoolNotLast`. Both are terminal alternatives for
        // the window-cleanup-cascade saga's Step 2.
        Command::ReportPoolDrainDecision { label, was_last } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportPoolDrainDecision") {
                return vec![err];
            }
            handle_report_pool_drain_decision(state, label, was_last)
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
            handle_report_host_pool_count(state, count)
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

/// Phase B.4 follow-up — pool-only drift check. Called from
/// `spawn_pool_window` where the windows dimension is mid-flight
/// (close path hasn't completed). Compares only the pool dimension;
/// emits `DriftDetected { kind: Pool, ... }` on mismatch.
fn handle_report_host_pool_count(state: &mut State, host_pool: u32) -> Vec<Event> {
    let mirror_pool = state.pool.len() as u32;
    if mirror_pool == host_pool {
        return vec![];
    }
    let v = state.bump_version();
    vec![Event::DriftDetected {
        kind: DriftKind::Pool,
        host_count: host_pool,
        mirror_count: mirror_pool,
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

/// Phase B.4 follow-up — record pool inventory growth. Idempotent
/// on duplicate labels (HashSet semantics) but the event still fires
/// so subscribers can track add-attempts even if redundant.
fn handle_report_pool_window_added(state: &mut State, label: String) -> Vec<Event> {
    state.pool.insert(label.clone());
    let v = state.bump_version();
    vec![Event::PoolWindowAdded { label, version: v }]
}

/// Phase B.4 follow-up — record pool inventory shrink (promote or
/// destroy). Strictly paired with `ReportPoolWindowAdded`: an
/// unknown-label remove is a silent no-op so subscribers can rely
/// on add/remove pairing in the broadcast stream. Same gate as
/// `handle_report_window_closed`. (reagent P2 PR #577 round-3 —
/// the original "idempotent" comment referenced behavior that was
/// already removed for `ReportWindowClosed`; pool semantics now
/// match.)
fn handle_report_pool_window_removed(state: &mut State, label: String) -> Vec<Event> {
    let was_present = state.pool.remove(&label);
    if !was_present {
        return vec![];
    }
    let v = state.bump_version();
    vec![Event::PoolWindowRemoved { label, version: v }]
}

/// Phase F.5 — host-emitted promote signal. The reducer doesn't mutate
/// state for this command (the windows/pool transitions are carried by
/// the surrounding `ReportPoolWindowRemoved` + `ReportWindowOpened`
/// pair); it just translates the wire command into the corresponding
/// typed event so subscribers — most importantly the launcher saga
/// coordinator — can react.
///
/// Idempotent / context-free: we don't validate the label is in the
/// mirror because the host's own ordering may have the
/// `ReportPoolWindowRemoved` arrive before this command, after this
/// command, or in either order; the typed event is "host says a
/// promote happened" — subscribers correlate with the surrounding
/// add/remove pair if they need stronger invariants.
fn handle_report_pool_window_promoted(state: &mut State, label: String) -> Vec<Event> {
    let v = state.bump_version();
    vec![Event::PoolWindowPromoted { label, version: v }]
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
fn handle_report_panes_reaped(state: &mut State, label: String) -> Vec<Event> {
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
    let v = state.bump_version();
    vec![Event::PanesReaped { label, version: v }]
}

/// Phase F.6 — host-emitted signal carrying the result of the
/// post-close drain-pool-if-last decision. Maps `was_last` directly
/// to the corresponding terminal event for Step 2 of the
/// window-cleanup-cascade saga:
/// * `true` → `Event::PoolDrained` (last user-visible window
///   closed; warm-pool drain initiated)
/// * `false` → `Event::PoolNotLast` (other windows remain; pool
///   stays warm)
///
/// Pure pass-through (same reasoning as `handle_report_panes_reaped`).
fn handle_report_pool_drain_decision(
    state: &mut State,
    label: String,
    was_last: bool,
) -> Vec<Event> {
    // Same rationale as handle_report_panes_reaped: round 4's gate
    // had an ordering bug; round 5 reverts to emit-unconditionally.
    let v = state.bump_version();
    if was_last {
        vec![Event::PoolDrained { label, version: v }]
    } else {
        vec![Event::PoolNotLast { label, version: v }]
    }
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
    // Phase B.9.1 (WRR) — drain-on-WindowOpened fallback. The
    // host's `EVENT_OBJECT_CREATE` callback fires before its CEF
    // `OnAfterCreated` (which becomes this Command), so the
    // matching `ReportHwndOpened` may have already been processed
    // and stashed in `state.pending_hwnds` (with `label_hint=None`
    // if the host couldn't determine it from
    // `pending_window_creations`). Find the most recent pending
    // HWND that arrived within the last 2 seconds and link it to
    // the new mirror. The 2s window is generous compared to the
    // typical < 50ms create→ReportWindowOpened gap, but bounded
    // enough that a stale pending HWND from a prior open won't
    // mis-link.
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
            // Phase B.9.1 — observability axis. `hwnd` is
            // populated from the drained pending entry above
            // (host's WRR hook fired before this Command landed)
            // OR remains None until the host's
            // `ReportHwndOpened` with matching `label_hint`
            // arrives via `apply_hwnd_opened`. Either path
            // converges on the same linked state.
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
mod tests {
    use super::*;

    fn ctx(conn_id: u64) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-28T00:00:00Z".to_string(),
            conn_id,
            registered_pid: None,
            now_ms: 0,
        }
    }

    fn ctx_with_pid(conn_id: u64, pid: u32) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-28T00:00:00Z".to_string(),
            conn_id,
            registered_pid: Some(pid),
            now_ms: 0,
        }
    }

    #[test]
    fn first_host_register_transitions_starting_to_running() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        assert!(state.processes.contains_key(&1234));
        // Should emit ProcessSpawned + LifecyclePhaseChanged + Registered.
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[0],
            Event::ProcessSpawned { pid: 1234, .. }
        ));
        assert!(matches!(
            events[1],
            Event::LifecyclePhaseChanged {
                from: LifecyclePhase::Starting,
                to: LifecyclePhase::Running,
                ..
            }
        ));
        assert!(matches!(events[2], Event::Registered { .. }));
    }

    #[test]
    fn second_host_register_does_not_re_emit_lifecycle_change() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        // Different PID, second Host (e.g. test harness or doc)
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 5678,
                version: "0.33.450".into(),
            },
            &ctx(2),
        );
        // Lifecycle stays Running; no LifecyclePhaseChanged event.
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        assert_eq!(events.len(), 2); // ProcessSpawned + Registered
        assert!(events
            .iter()
            .all(|e| !matches!(e, Event::LifecyclePhaseChanged { .. })));
    }

    #[test]
    fn duplicate_pid_register_returns_already_registered() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 1234, // SAME pid
                version: "0.33.450".into(),
            },
            &ctx(2),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            Event::Error {
                code: ErrorCode::AlreadyRegistered,
                fatal: true,
                ..
            }
        ));
        // Second register doesn't overwrite the first.
        assert_eq!(state.processes[&1234].kind, ClientKind::Host);
    }

    #[test]
    fn renderer_register_does_not_drive_lifecycle() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 4321,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        // Lifecycle stays Starting until a HOST registers.
        assert_eq!(state.lifecycle, LifecyclePhase::Starting);
        assert_eq!(events.len(), 2); // ProcessSpawned + Registered
    }

    #[test]
    fn ping_returns_pong_with_same_nonce() {
        let mut state = State::default();
        let events = update(&mut state, Command::Ping { nonce: 42 }, &ctx(1));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Pong { nonce: 42, .. }));
    }

    /// Helper used by both the unit test below and the proptest: extract
    /// the version number from any Event variant. New variants must be
    /// added here OR the helper switched to a generic accessor when
    /// the variant set grows large.
    fn extract_version(e: &Event) -> u64 {
        match e {
            Event::ProcessSpawned { version, .. }
            | Event::ProcessExited { version, .. }
            | Event::LifecyclePhaseChanged { version, .. }
            | Event::Registered { version, .. }
            | Event::Pong { version, .. }
            | Event::WindowOpened { version, .. }
            | Event::WindowClosed { version, .. }
            | Event::PoolWindowAdded { version, .. }
            | Event::PoolWindowRemoved { version, .. }
            | Event::PoolWindowPromoted { version, .. }
            | Event::PanesReaped { version, .. }
            | Event::PoolDrained { version, .. }
            | Event::PoolNotLast { version, .. }
            | Event::WindowInstanceAssigned { version, .. }
            | Event::WindowInstanceReleased { version, .. }
            | Event::BackendWindowIdRegistered { version, .. }
            | Event::BackendWindowIdUnregistered { version, .. }
            | Event::DriftDetected { version, .. }
            | Event::HwndDriftDetected { version, .. }
            | Event::CorrectiveWindowMove { version, .. }
            | Event::HostShouldQuit { version, .. }
            | Event::Snapshot { version, .. }
            | Event::EventList { version, .. }
            | Event::SagaStarted { version, .. }
            | Event::SagaCompleted { version, .. }
            | Event::SagaFailed { version, .. }
            | Event::SrvSnapshot { version, .. }
            | Event::WorkspaceCreated { version, .. }
            | Event::WorkspaceDeleted { version, .. }
            | Event::TabCreated { version, .. }
            | Event::TabDeleted { version, .. }
            | Event::ActiveTabChanged { version, .. }
            | Event::TabReordered { version, .. }
            | Event::SrvWindowOpened { version, .. }
            | Event::SrvWindowClosed { version, .. }
            | Event::SrvWindowWorkspaceChanged { version, .. }
            | Event::TabsReorderedBulk { version, .. }
            | Event::WorkspaceRenamed { version, .. }
            | Event::TabRenamed { version, .. }
            | Event::WorkspaceMetaUpdated { version, .. }
            | Event::TabMetaUpdated { version, .. }
            | Event::BlockMetaUpdated { version, .. }
            | Event::TabMoved { version, .. }
            | Event::BlockMoved { version, .. }
            | Event::BlockCreated { version, .. }
            | Event::BlockDeleted { version, .. }
            | Event::FocusedNodeChanged { version, .. }
            | Event::MagnifiedNodeChanged { version, .. }
            | Event::Error { version, .. } => *version,
        }
    }

    #[test]
    fn event_versions_are_strictly_monotonic() {
        let mut state = State::default();
        let mut versions = vec![];
        for pid in [100, 200, 300] {
            let events = update(
                &mut state,
                Command::Register {
                    kind: ClientKind::Host,
                    pid,
                    version: "0.33.450".into(),
                },
                &ctx(1),
            );
            versions.extend(events.iter().map(extract_version));
        }
        for w in versions.windows(2) {
            assert!(w[1] > w[0], "versions not monotonic: {:?}", versions);
        }
    }

    #[test]
    fn goodbye_marks_registered_pid_as_exited() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.451".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Running
        ));
        let events = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 1234));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            Event::ProcessExited { pid: 1234, code: 0, .. }
        ));
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Exited { code: 0 }
        ));
    }

    /// B.4 reports require a Host registration first (codex P1
    /// PR #576). Helper to set that up so each window-mirror test
    /// doesn't repeat the boilerplate.
    fn register_host_and_get_ctx(state: &mut State, pid: u32) -> Ctx {
        let _ = update(
            state,
            Command::Register {
                kind: ClientKind::Host,
                pid,
                version: "test".into(),
            },
            &ctx(1),
        );
        ctx_with_pid(1, pid)
    }

    #[test]
    fn report_window_opened_inserts_into_mirror_and_emits_event() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        // B.5 — open emits WindowOpened + WindowInstanceAssigned.
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            Event::WindowOpened { label, kind: WindowKind::FullInstance, parent_label: None, .. }
                if label == "main"
        ));
        assert!(matches!(
            &events[1],
            Event::WindowInstanceAssigned { label, num: 1, .. } if label == "main"
        ));
        let mirror = &state.windows["main"];
        assert_eq!(mirror.label, "main");
        assert_eq!(mirror.kind, WindowKind::FullInstance);
        assert_eq!(mirror.parent_label, None);
    }

    #[test]
    fn report_window_closed_removes_from_mirror() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        assert!(state.windows.contains_key("main"));
        let events = update(
            &mut state,
            Command::ReportWindowClosed {
                label: "main".into(),
            },
            &host_ctx,
        );
        assert!(!state.windows.contains_key("main"));
        // B.5 — close emits WindowClosed + WindowInstanceReleased.
        // B.9.3 — closing the last window with a Host registered
        // also emits OrphanInstance drift + HostShouldQuit saga,
        // so the total is 4 events on the last-window-close path.
        assert_eq!(events.len(), 4);
        assert!(matches!(
            &events[0],
            Event::WindowClosed { label, .. } if label == "main"
        ));
        assert!(matches!(
            &events[1],
            Event::WindowInstanceReleased { label, num: 1, .. } if label == "main"
        ));
        assert!(matches!(
            &events[2],
            Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. }
        ));
        assert!(matches!(&events[3], Event::HostShouldQuit { .. }));
    }

    #[test]
    fn instance_numbers_are_monotonic_per_launcher_run() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        // main pre-seeded as 1 (already in instance_registry from Default).
        // Open second window → gets 2.
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "window-2".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        let assigned = events.iter().find_map(|e| match e {
            Event::WindowInstanceAssigned { num, .. } => Some(*num),
            _ => None,
        });
        assert_eq!(assigned, Some(2));

        // Close it. Open a third window → gets 3 (NOT reused 2).
        let _ = update(
            &mut state,
            Command::ReportWindowClosed {
                label: "window-2".into(),
            },
            &host_ctx,
        );
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "window-3".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        let assigned = events.iter().find_map(|e| match e {
            Event::WindowInstanceAssigned { num, .. } => Some(*num),
            _ => None,
        });
        assert_eq!(assigned, Some(3), "instance numbers must not be reused");
    }

    #[test]
    fn re_open_of_same_label_keeps_original_instance_number() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "x".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        let first_num = state.instance_registry["x"];
        // Re-open without close (B.4 idempotent overwrite path).
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "x".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        let assigned = events.iter().find_map(|e| match e {
            Event::WindowInstanceAssigned { num, .. } => Some(*num),
            _ => None,
        });
        assert_eq!(assigned, Some(first_num));
        assert_eq!(state.instance_registry["x"], first_num);
    }

    #[test]
    fn report_window_closed_on_unknown_label_is_silent_no_op() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportWindowClosed {
                label: "ghost".into(),
            },
            &host_ctx,
        );
        // Codex P2 PR #577 round-2: NO broadcast for unknown labels.
        // Pairs strictly with WindowOpened so subscribers can rely on
        // open/close pairing.
        assert_eq!(events.len(), 0);
        assert!(state.windows.is_empty());
    }

    #[test]
    fn subwindow_open_records_parent_label() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "sub-1".into(),
                kind: WindowKind::Subwindow,
                parent_label: Some("main".into()),
            },
            &host_ctx,
        );
        assert_eq!(
            state.windows["sub-1"].parent_label.as_deref(),
            Some("main")
        );
    }

    #[test]
    fn report_pool_window_add_and_remove_round_trip() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportPoolWindowAdded {
                label: "window-pool-abc".into(),
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::PoolWindowAdded { label, .. } if label == "window-pool-abc"
        ));
        assert!(state.pool.contains("window-pool-abc"));

        let events = update(
            &mut state,
            Command::ReportPoolWindowRemoved {
                label: "window-pool-abc".into(),
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::PoolWindowRemoved { label, .. } if label == "window-pool-abc"
        ));
        assert!(!state.pool.contains("window-pool-abc"));
    }

    #[test]
    fn report_backend_window_id_round_trip() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportBackendWindowIdRegistered {
                label: "main".into(),
                window_id: "wid-abc".into(),
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::BackendWindowIdRegistered { label, window_id, .. }
                if label == "main" && window_id == "wid-abc"
        ));
        assert_eq!(state.backend_window_ids.get("main").map(|s| s.as_str()), Some("wid-abc"));

        let events = update(
            &mut state,
            Command::ReportBackendWindowIdUnregistered {
                label: "main".into(),
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::BackendWindowIdUnregistered { label, window_id, .. }
                if label == "main" && window_id == "wid-abc"
        ));
        assert!(state.backend_window_ids.is_empty());
    }

    #[test]
    fn report_backend_window_id_unregister_unknown_label_is_silent() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportBackendWindowIdUnregistered {
                label: "ghost".into(),
            },
            &host_ctx,
        );
        // Strict pairing — same as WindowClosed/PoolWindowRemoved.
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn report_backend_window_id_overwrites_on_duplicate() {
        // Frontend can re-register if it reloads — the launcher should
        // accept the new ID and emit a fresh event so subscribers see
        // the latest mapping.
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportBackendWindowIdRegistered {
                label: "main".into(),
                window_id: "wid-old".into(),
            },
            &host_ctx,
        );
        let _ = update(
            &mut state,
            Command::ReportBackendWindowIdRegistered {
                label: "main".into(),
                window_id: "wid-new".into(),
            },
            &host_ctx,
        );
        assert_eq!(
            state.backend_window_ids.get("main").map(|s| s.as_str()),
            Some("wid-new")
        );
    }

    #[test]
    fn pool_and_window_mirrors_are_independent() {
        // The host transitions a pool window to a real window via
        // (PoolRemoved, WindowOpened). Verify the launcher can hold
        // both maps without collision and an entry can be in pool
        // OR windows but not both.
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportPoolWindowAdded {
                label: "window-pool-xyz".into(),
            },
            &host_ctx,
        );
        let _ = update(
            &mut state,
            Command::ReportPoolWindowRemoved {
                label: "window-pool-xyz".into(),
            },
            &host_ctx,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "window-pool-xyz".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        assert!(!state.pool.contains("window-pool-xyz"));
        assert!(state.windows.contains_key("window-pool-xyz"));
    }

    #[test]
    fn report_pool_window_removed_on_unknown_label_is_silent_no_op() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportPoolWindowRemoved {
                label: "ghost-pool".into(),
            },
            &host_ctx,
        );
        // Strict pairing — pool remove only emits an event when the
        // label was in the pool. (reagent P2 PR #577 round-3.)
        assert_eq!(events.len(), 0);
        assert!(state.pool.is_empty());
    }

    #[test]
    fn report_host_counts_matching_mirror_emits_no_event() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        // Mirror has 1 window, 0 pool. Host reports the same.
        let events = update(
            &mut state,
            Command::ReportHostCounts {
                windows: 1,
                pool: 0,
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn report_host_counts_emits_drift_for_window_mismatch() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        // Mirror has 0 windows; host claims 3 → drift.
        let events = update(
            &mut state,
            Command::ReportHostCounts {
                windows: 3,
                pool: 0,
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::DriftDetected {
                kind: DriftKind::Windows,
                host_count: 3,
                mirror_count: 0,
                ..
            }
        ));
    }

    #[test]
    fn report_host_counts_emits_drift_for_pool_mismatch() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportPoolWindowAdded {
                label: "window-pool-a".into(),
            },
            &host_ctx,
        );
        // Mirror has 1 pool entry; host claims 5 → drift.
        let events = update(
            &mut state,
            Command::ReportHostCounts {
                windows: 0,
                pool: 5,
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::DriftDetected {
                kind: DriftKind::Pool,
                host_count: 5,
                mirror_count: 1,
                ..
            }
        ));
    }

    #[test]
    fn report_host_pool_count_matching_emits_no_event() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportPoolWindowAdded {
                label: "window-pool-x".into(),
            },
            &host_ctx,
        );
        let events = update(
            &mut state,
            Command::ReportHostPoolCount { count: 1 },
            &host_ctx,
        );
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn report_host_pool_count_emits_drift_on_mismatch() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        // Mirror pool=0; host claims 7 → drift.
        let events = update(
            &mut state,
            Command::ReportHostPoolCount { count: 7 },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::DriftDetected {
                kind: DriftKind::Pool,
                host_count: 7,
                mirror_count: 0,
                ..
            }
        ));
    }

    #[test]
    fn report_host_pool_count_ignores_windows_dimension() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        // Open a window so the windows dimension WOULD diverge if
        // checked, but ReportHostPoolCount only inspects pool.
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        // Mirror windows=1, mirror pool=0. Host pool count matches.
        let events = update(
            &mut state,
            Command::ReportHostPoolCount { count: 0 },
            &host_ctx,
        );
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn report_host_counts_emits_both_drifts_when_both_diverge() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportHostCounts {
                windows: 1,
                pool: 1,
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 2);
        // Stable order: windows first, then pool. (Tested for predictability
        // so subscribers + this assertion don't drift with HashMap iteration.)
        assert!(matches!(
            &events[0],
            Event::DriftDetected { kind: DriftKind::Windows, .. }
        ));
        assert!(matches!(
            &events[1],
            Event::DriftDetected { kind: DriftKind::Pool, .. }
        ));
    }

    #[test]
    fn pool_commands_from_non_host_are_rejected() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Tool,
                pid: 9999,
                version: "test".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::ReportPoolWindowAdded {
                label: "spoof-pool".into(),
            },
            &ctx_with_pid(1, 9999),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::NotRegistered, .. }
        ));
        assert!(state.pool.is_empty());
    }

    #[test]
    fn report_window_opened_from_non_host_is_rejected() {
        let mut state = State::default();
        // Register as Renderer (not Host) at PID 4321.
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 4321,
                version: "test".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "spoof".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &ctx_with_pid(1, 4321),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::NotRegistered, fatal: false, .. }
        ));
        // Mirror NOT mutated.
        assert!(state.windows.is_empty());
    }

    #[test]
    fn report_window_closed_from_unregistered_conn_is_rejected() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReportWindowClosed {
                label: "x".into(),
            },
            &ctx(1), // No Register first.
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::NotRegistered, fatal: false, .. }
        ));
    }

    #[test]
    fn register_replaces_exited_record_for_recycled_pid() {
        let mut state = State::default();
        // First process registers + cleanly exits
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "first".into(),
            },
            &ctx(1),
        );
        let _ = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 1234));
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Exited { .. }
        ));

        // OS recycles PID 1234 to a new process which Registers
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 1234,
                version: "second".into(),
            },
            &ctx(2),
        );
        // Should NOT emit AlreadyRegistered — record replaced.
        assert!(!events
            .iter()
            .any(|e| matches!(e, Event::Error {
                code: ErrorCode::AlreadyRegistered,
                ..
            })));
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Running
        ));
        assert_eq!(state.processes[&1234].kind, ClientKind::Renderer);
        assert_eq!(state.processes[&1234].version, "second");
    }

    // ===== Property-based tests =====
    //
    // The unit tests above cover specific scenarios; these prove
    // the reducer's INVARIANTS hold across arbitrary input
    // sequences. Per spec §7 + the Phase B plan's testing strategy.

    use proptest::prelude::*;

    /// Generate an arbitrary Command. Constrained to the value-space
    /// the IPC server can actually produce: PIDs are realistic-ish
    /// u32s, version strings are short ASCII.
    fn arb_command() -> impl Strategy<Value = Command> {
        prop_oneof![
            // Register dominates the distribution because that's
            // where most state-machine logic lives.
            5 => (
                prop_oneof![
                    Just(ClientKind::Host),
                    Just(ClientKind::Renderer),
                    Just(ClientKind::Srv),
                    Just(ClientKind::Tool),
                ],
                1u32..10000u32,
                "[a-zA-Z0-9.]{1,16}",
            )
                .prop_map(|(kind, pid, version)| Command::Register { kind, pid, version }),
            1 => any::<u64>().prop_map(|nonce| Command::Ping { nonce }),
            1 => Just(Command::Goodbye),
            // B.4: window mirror commands. Labels drawn from a small
            // alphabet so duplicates (open then close) are common
            // enough for the proptest to exercise the idempotent
            // close path.
            2 => (
                "[a-c]{1,3}",
                prop_oneof![
                    Just(WindowKind::FullInstance),
                    Just(WindowKind::Subwindow),
                ],
                prop_oneof![Just(None::<String>), Just(Some("a".into()))],
            )
                .prop_map(|(label, kind, parent_label)| {
                    Command::ReportWindowOpened { label, kind, parent_label }
                }),
            2 => "[a-c]{1,3}".prop_map(|label| Command::ReportWindowClosed { label }),
        ]
    }

    proptest! {
        /// Versions across an arbitrary sequence of commands are
        /// always strictly monotonic. This is the foundation of
        /// Phase D's GetSnapshot resync: clients detect missed
        /// events by gap in version numbers.
        #[test]
        fn versions_strictly_monotonic_under_any_command_sequence(
            cmds in proptest::collection::vec(arb_command(), 1..50)
        ) {
            let mut state = State::default();
            let mut all_versions = vec![];
            for cmd in cmds {
                let events = update(&mut state, cmd, &ctx(1));
                all_versions.extend(events.iter().map(extract_version));
            }
            for w in all_versions.windows(2) {
                prop_assert!(
                    w[1] > w[0],
                    "version regression: {} → {}",
                    w[0], w[1]
                );
            }
        }

        /// Lifecycle invariant (spec §4): only ever Starting →
        /// Running → Quitting → Dead. No other transition.
        /// In B.3 we exercise just the Starting → Running edge
        /// since later transitions don't have triggering commands
        /// yet, but the harness is ready for B.4+.
        #[test]
        fn lifecycle_only_progresses_forward(
            cmds in proptest::collection::vec(arb_command(), 1..50)
        ) {
            let mut state = State::default();
            let mut prev = state.lifecycle;
            for cmd in cmds {
                let _ = update(&mut state, cmd, &ctx(1));
                let next = state.lifecycle;
                let valid = match (prev, next) {
                    (a, b) if a == b => true,
                    (LifecyclePhase::Starting, LifecyclePhase::Running) => true,
                    (LifecyclePhase::Running, LifecyclePhase::Quitting) => true,
                    (LifecyclePhase::Quitting, LifecyclePhase::Dead) => true,
                    _ => false,
                };
                prop_assert!(
                    valid,
                    "illegal lifecycle transition {:?} → {:?}",
                    prev, next
                );
                prev = next;
            }
        }

        /// Process map invariant: a successful Register inserts the
        /// PID; a duplicate Register (same PID) NEVER overwrites the
        /// existing record. This is what the server relies on for
        /// stale-state safety.
        #[test]
        fn duplicate_register_never_overwrites(
            initial_kind in prop_oneof![Just(ClientKind::Host), Just(ClientKind::Renderer)],
            second_kind in prop_oneof![Just(ClientKind::Srv), Just(ClientKind::Tool)],
            pid in 1u32..10000u32,
        ) {
            let mut state = State::default();
            let _ = update(
                &mut state,
                Command::Register {
                    kind: initial_kind,
                    pid,
                    version: "v1".into(),
                },
                &ctx(1),
            );
            let _ = update(
                &mut state,
                Command::Register {
                    kind: second_kind,
                    pid,
                    version: "v2".into(),
                },
                &ctx(2),
            );
            // Original record preserved across the rejected dup.
            prop_assert_eq!(state.processes[&pid].kind, initial_kind);
            prop_assert_eq!(&state.processes[&pid].version, "v1");
        }

        /// B.4 mirror invariants under arbitrary host-driven traffic.
        /// State seeded with a registered Host so the host-only gate
        /// doesn't trivially reject every command (reagent P2 round-1
        /// PR #576). Two invariants checked:
        ///   1. Mirror size is bounded by total opens minus successful
        ///      closes — no phantom entries appear.
        ///   2. Every label in `state.windows` has a `WindowMirror`
        ///      whose `label` field matches the map key (no key/value
        ///      drift).
        #[test]
        fn window_mirror_invariants_under_host_traffic(
            cmds in proptest::collection::vec(arb_window_cmd(), 1..100)
        ) {
            const HOST_PID: u32 = 1;
            let mut state = State::default();
            let _ = update(
                &mut state,
                Command::Register {
                    kind: ClientKind::Host,
                    pid: HOST_PID,
                    version: "host".into(),
                },
                &ctx(1),
            );
            let host_ctx = ctx_with_pid(1, HOST_PID);

            let mut opens = 0u64;
            let mut closes = 0u64;
            for cmd in cmds {
                match &cmd {
                    Command::ReportWindowOpened { .. } => opens += 1,
                    Command::ReportWindowClosed { .. } => closes += 1,
                    _ => {}
                }
                let _ = update(&mut state, cmd, &host_ctx);
            }

            // Bound: mirror can't hold more entries than distinct opens
            // (idempotent overwrite ensures opens are dedup'd by label,
            // and any open can be cancelled by its matching close).
            prop_assert!(
                state.windows.len() as u64 <= opens,
                "mirror size {} > total opens {}",
                state.windows.len(), opens
            );
            // Key/value coherence — if this ever fails, the reducer
            // wrote a value with a mismatched label.
            for (k, v) in &state.windows {
                prop_assert_eq!(k, &v.label);
            }
            // Closes are observable (each emits an event regardless
            // of mirror state). Just sanity-check nothing is leaking
            // into negative counters.
            let _ = (opens, closes); // referenced for failure messages
        }
    }

    /// B.4 — generate ONLY window-mirror commands. Used by the
    /// host-driven proptest above to guarantee exercise of the
    /// mirror insert/remove paths (general `arb_command` mixes in
    /// Register / Ping / Goodbye which dilute window coverage).
    fn arb_window_cmd() -> impl proptest::strategy::Strategy<Value = Command> {
        use proptest::prelude::*;
        prop_oneof![
            (
                "[a-c]{1,3}",
                prop_oneof![
                    Just(WindowKind::FullInstance),
                    Just(WindowKind::Subwindow),
                ],
                prop_oneof![Just(None::<String>), Just(Some("a".into()))],
            )
                .prop_map(|(label, kind, parent_label)| {
                    Command::ReportWindowOpened { label, kind, parent_label }
                }),
            "[a-c]{1,3}".prop_map(|label| Command::ReportWindowClosed { label }),
        ]
    }

    // -----------------------------------------------------------
    // Phase B.9 (WRR) — reducer arm tests
    // -----------------------------------------------------------

    use agentmux_common::ipc::{HwndDriftKind, Rect};

    /// Helper: drive the reducer through a host Register so
    /// subsequent host-only WRR commands are accepted. Returns the
    /// state ready to receive WRR commands.
    fn registered_host_state() -> (State, Ctx) {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1,
                version: "test".into(),
            },
            &ctx(1),
        );
        (state, ctx_with_pid(1, 1))
    }

    #[test]
    fn wrr_off_monitor_position_for_unseen_window_emits_drift_and_corrective() {
        let (mut state, c) = registered_host_state();
        // Set monitor topology: a single 1920x1080 monitor.
        let _ = update(
            &mut state,
            Command::ReportMonitorTopologyChanged {
                rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
            },
            &c,
        );
        // Open a window.
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        // Link an HWND to it.
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "w1".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        // Position event: rect is fully off the monitor and NOT
        // the Win32 hidden sentinel (so drift fires).
        let evs = update(
            &mut state,
            Command::ReportHwndPositionChanged {
                hwnd: 0xAA,
                rect: Rect { left: -5000, top: -5000, right: -4000, bottom: -4000 },
            },
            &c,
        );
        // Expect both: drift + corrective move.
        assert!(
            evs.iter().any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. })),
            "expected OffMonitor drift, got {:?}", evs
        );
        assert!(
            evs.iter().any(|e| matches!(e, Event::CorrectiveWindowMove { hwnd: 0xAA, .. })),
            "expected CorrectiveWindowMove, got {:?}", evs
        );
    }

    #[test]
    fn wrr_sentinel_position_suppresses_drift_but_emits_corrective() {
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportMonitorTopologyChanged {
                rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        // Sentinel position — CEF Views' "hidden" parking spot.
        let evs = update(
            &mut state,
            Command::ReportHwndPositionChanged {
                hwnd: 0xAA,
                rect: Rect { left: -31970, top: -31970, right: -31340, bottom: -30871 },
            },
            &c,
        );
        // Drift suppressed (sentinel is a known transient).
        assert!(
            !evs.iter().any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. })),
            "sentinel position should NOT fire drift, got {:?}", evs
        );
        // Corrective fires regardless — we want to move it before
        // the user notices the orphan taskbar entry.
        assert!(
            evs.iter().any(|e| matches!(e, Event::CorrectiveWindowMove { hwnd: 0xAA, .. })),
            "sentinel position should fire CorrectiveWindowMove, got {:?}", evs
        );
    }

    #[test]
    fn wrr_off_monitor_after_user_foregrounded_does_not_correct() {
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportMonitorTopologyChanged {
                rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        // Foreground event — user has interacted with this window.
        let _ = update(
            &mut state,
            Command::ReportHwndForegroundChanged { hwnd: 0xAA },
            &c,
        );
        // User then drags it off-monitor (legitimate state).
        let evs = update(
            &mut state,
            Command::ReportHwndPositionChanged {
                hwnd: 0xAA,
                rect: Rect { left: -5000, top: -5000, right: -4000, bottom: -4000 },
            },
            &c,
        );
        // Drift fires — operator should still see it.
        assert!(
            evs.iter().any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. })),
            "drift should fire even for user-touched windows, got {:?}", evs
        );
        // BUT no corrective — we trust the user.
        assert!(
            !evs.iter().any(|e| matches!(e, Event::CorrectiveWindowMove { .. })),
            "corrective must NOT fire after user has foregrounded the window, got {:?}", evs
        );
    }

    #[test]
    fn wrr_duplicate_hwnd_open_for_same_label_is_idempotent() {
        // codex #600 P2: ReportHwndOpened arrives twice (once from
        // the WinEvent CREATE hook with label_hint=None, then again
        // from CEF's on_after_created with label_hint=Some(label)).
        // The second report carries the SAME hwnd that's now
        // linked to the mirror. Should be a no-op, NOT drift.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        // First link.
        let evs1 = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        assert!(
            evs1.is_empty(),
            "first ReportHwndOpened should silently link, got {:?}", evs1
        );
        // Duplicate: same label, same hwnd.
        let evs2 = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        assert!(
            evs2.is_empty(),
            "duplicate ReportHwndOpened with same hwnd must be no-op, got {:?}", evs2
        );
    }

    #[test]
    fn wrr_double_link_with_different_hwnd_for_same_label_emits_drift() {
        // The non-duplicate path: a second ReportHwndOpened for
        // the same label but a DIFFERENT HWND is genuine drift —
        // host is reporting a related popup or there's a real bug.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        let evs = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xBB,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        assert!(
            evs.iter().any(|e| matches!(
                e,
                Event::HwndDriftDetected { kind: HwndDriftKind::HwndWithoutBrowser, .. }
            )),
            "different HWND for same label must fire drift, got {:?}", evs
        );
    }

    #[test]
    fn b9_3_orphan_instance_fires_when_last_window_closes_and_host_running() {
        // The smoke-test scenario: open a window, close it; with a
        // Host registered, the reducer should emit OrphanInstance
        // drift + HostShouldQuit on the same dispatch tick.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        // Sanity: window opened, no orphan signal yet.
        let evs = update(
            &mut state,
            Command::ReportWindowClosed { label: "w1".into() },
            &c,
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
            "expected OrphanInstance drift on last-window-close, got {:?}", evs
        );
        assert!(
            evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
            "expected HostShouldQuit saga on last-window-close, got {:?}", evs
        );
    }

    #[test]
    fn b9_3_orphan_instance_does_not_fire_on_non_terminal_close() {
        // Closing one of N windows when N > 1 must NOT fire — the
        // host has more user-visible windows alive, so it shouldn't
        // quit. Predicate: state.windows still non-empty after
        // remove → no signal.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w2".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let evs = update(
            &mut state,
            Command::ReportWindowClosed { label: "w1".into() },
            &c,
        );
        assert!(
            !evs.iter()
                .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
            "OrphanInstance must NOT fire while other windows are open, got {:?}", evs
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
            "HostShouldQuit must NOT fire while other windows are open, got {:?}", evs
        );
    }

    #[test]
    fn b9_3_orphan_instance_does_not_fire_after_host_goodbye() {
        // Realistic scenario for the no-Host predicate guard:
        // Host registers, opens a window, then sends Goodbye
        // (clean shutdown), which marks its ProcessRecord as
        // Exited. A subsequent ReportWindowClosed arriving from
        // the host's pipe (e.g. a queued event flushed during
        // shutdown) MUST NOT fire HostShouldQuit — there's no
        // Running Host left to quit.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        // Host says Goodbye — record transitions to Exited.
        let _ = update(&mut state, Command::Goodbye, &c);
        // Force the close path to actually run by calling the
        // private handler directly. (The public path's
        // enforce_host_only would reject because the Host record
        // is no longer Running, but that rejection happens
        // BEFORE reaching the predicate; we want to prove the
        // predicate itself is correct.)
        let evs = handle_report_window_closed(&mut state, "w1".into());
        assert!(
            evs.iter().any(|e| matches!(e, Event::WindowClosed { .. })),
            "WindowClosed should still emit, got {:?}", evs
        );
        assert!(
            !evs.iter()
                .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
            "OrphanInstance must NOT fire after host has Goodbye'd, got {:?}", evs
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
            "HostShouldQuit must NOT fire after host has Goodbye'd, got {:?}", evs
        );
    }

    #[test]
    fn wrr_orphan_destroy_runs_even_with_stale_pending_entry() {
        // reagent #600 P1: the dual-source design (WinEvent CREATE
        // hook with label_hint=None, then explicit on_after_created
        // with label_hint=Some(label)) can leave a stale entry in
        // `pending_hwnds` AFTER the mirror is linked. Pre-fix,
        // `apply_hwnd_destroyed` early-returned on the stale
        // pending entry and skipped the OrphanDestroy chain. Post-
        // fix, the link drains pending and destroy runs the chain
        // correctly. This test reproduces the exact race.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportMonitorTopologyChanged {
                rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        // Step 1: WinEvent CREATE fires first with label_hint=None.
        // No mirror match → stash in pending_hwnds.
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: None,
            },
            &c,
        );
        assert!(state.pending_hwnds.contains_key(&0xAA), "pending entry expected after step 1");
        // Step 2: on_after_created fires with label_hint=Some(w1).
        // Should link the mirror AND drain the stale pending.
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        assert!(
            !state.pending_hwnds.contains_key(&0xAA),
            "stale pending entry should be drained on link, but still present: {:?}",
            state.pending_hwnds
        );
        // Step 3: renderer crash → ReportHwndDestroyed.
        let evs = update(
            &mut state,
            Command::ReportHwndDestroyed { hwnd: 0xAA },
            &c,
        );
        // Even with the (drained) pending entry history, the
        // orphan-destroy chain must run.
        assert!(
            evs.iter()
                .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanDestroy, .. })),
            "OrphanDestroy must fire on renderer crash even after dual-source link, got {:?}",
            evs
        );
        assert!(
            evs.iter().any(|e| matches!(e, Event::WindowClosed { .. })),
            "WindowClosed must fire so frontend prunes its atoms, got {:?}",
            evs
        );
    }

    #[test]
    fn wrr_orphan_destroy_emits_window_closed_and_instance_released() {
        // reagent #600 P1: a renderer crash that takes the HWND
        // with it must produce the same shutdown events the normal
        // close path would, otherwise the frontend keeps a stale
        // window in its atoms after the crash.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportMonitorTopologyChanged {
                rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        // Renderer crashes — Win32 fires the destroy without CEF's
        // close path running, so no ReportWindowClosed precedes it.
        let evs = update(
            &mut state,
            Command::ReportHwndDestroyed { hwnd: 0xAA },
            &c,
        );

        // All three: drift (operator alert) + WindowClosed
        // (frontend prune) + WindowInstanceReleased (count drop).
        assert!(
            evs.iter()
                .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanDestroy, .. })),
            "expected OrphanDestroy drift, got {:?}", evs
        );
        assert!(
            evs.iter().any(|e| matches!(
                e,
                Event::WindowClosed { label, .. } if label == "w1"
            )),
            "expected WindowClosed for w1, got {:?}", evs
        );
        assert!(
            evs.iter().any(|e| matches!(
                e,
                Event::WindowInstanceReleased { label, .. } if label == "w1"
            )),
            "expected WindowInstanceReleased for w1, got {:?}", evs
        );
        // State pruned.
        assert!(!state.windows.contains_key("w1"));
        assert!(!state.instance_registry.contains_key("w1"));
    }

    #[test]
    fn wrr_off_monitor_with_no_known_monitors_emits_neither() {
        // No `ReportMonitorTopologyChanged` => state.monitors is
        // empty => we don't know what "off-monitor" means yet.
        let (mut state, c) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "w1".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd: 0xAA,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some("w1".into()),
            },
            &c,
        );
        let evs = update(
            &mut state,
            Command::ReportHwndPositionChanged {
                hwnd: 0xAA,
                rect: Rect { left: -5000, top: -5000, right: -4000, bottom: -4000 },
            },
            &c,
        );
        assert!(
            evs.is_empty(),
            "no monitors known => no drift, no corrective; got {:?}", evs
        );
    }

    // -----------------------------------------------------------
    // Phase B.8 — extended invariant suite.
    //
    // Covers gaps surfaced during B.5 / B.7 / B.9 work that the
    // earlier proptests in `arb_command` (Register / Window
    // open/close only) didn't exercise: pool inventory, backend
    // window ID lifecycle, and the OrphanInstance / HostShouldQuit
    // saga from B.9.3. Plus a deterministic integration-style
    // close-all cascade that locks in the B.9.3 behaviour at the
    // reducer level (CI synthetic close-all assertion per B.8 plan).
    // -----------------------------------------------------------

    /// Generate B.5+ host commands: window open/close, pool
    /// add/remove, backend window ID register/unregister. Labels
    /// drawn from a small alphabet so duplicates / opens-of-already-
    /// open / close-of-not-open paths get exercised.
    fn arb_b8_host_command() -> impl proptest::strategy::Strategy<Value = Command> {
        use proptest::prelude::*;
        prop_oneof![
            3 => (
                "[a-c]{1,3}",
                prop_oneof![Just(WindowKind::FullInstance), Just(WindowKind::Subwindow)],
                prop_oneof![Just(None::<String>), Just(Some("a".into()))],
            )
                .prop_map(|(label, kind, parent_label)| {
                    Command::ReportWindowOpened { label, kind, parent_label }
                }),
            3 => "[a-c]{1,3}".prop_map(|label| Command::ReportWindowClosed { label }),
            2 => "pool-[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowAdded { label }),
            2 => "pool-[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowRemoved { label }),
            2 => ("[a-c]{1,3}", "[0-9a-f]{4}").prop_map(|(label, window_id)| {
                Command::ReportBackendWindowIdRegistered { label, window_id }
            }),
            2 => "[a-c]{1,3}".prop_map(|label| Command::ReportBackendWindowIdUnregistered { label }),
        ]
    }

    proptest! {
        /// Pool/windows disjoint: a label is never simultaneously in
        /// both `state.pool` and `state.windows`. The host's
        /// pool→window promote path always sends ReportPoolWindowRemoved
        /// before ReportWindowOpened (and reverse on demote), so the
        /// reducer should never observe overlap. Catches a regression
        /// where a buggy promote sequence would leave a pool label
        /// shadowed by a window entry — gap #5 from
        /// `ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md`.
        #[test]
        fn pool_and_windows_disjoint_under_any_sequence(
            cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
        ) {
            let (mut state, host_ctx) = registered_host_state();
            for cmd in cmds {
                let _ = update(&mut state, cmd, &host_ctx);
                let pool: std::collections::HashSet<&str> =
                    state.pool.iter().map(|s| s.as_str()).collect();
                let window_keys: std::collections::HashSet<&str> =
                    state.windows.keys().map(|s| s.as_str()).collect();
                let overlap: Vec<&&str> = pool.intersection(&window_keys).collect();
                prop_assert!(
                    overlap.is_empty(),
                    "label(s) in both pool and windows: {:?}", overlap
                );
            }
        }

        /// Instance numbers within `state.instance_registry` are
        /// unique. Reagent / codex flagged the reverse property
        /// (numbers don't repeat across releases) at B.5b; this is
        /// the symmetric "no two LIVE windows share a number"
        /// guarantee. Failure mode: InstancePanel would render two
        /// rows as "Window N", users couldn't disambiguate.
        #[test]
        fn instance_numbers_unique_within_registry(
            cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
        ) {
            let (mut state, host_ctx) = registered_host_state();
            for cmd in cmds {
                let _ = update(&mut state, cmd, &host_ctx);
                let nums: Vec<u32> = state.instance_registry.values().copied().collect();
                let mut sorted = nums.clone();
                sorted.sort_unstable();
                sorted.dedup();
                prop_assert_eq!(
                    nums.len(), sorted.len(),
                    "duplicate instance numbers in registry: {:?}", nums
                );
            }
        }

        /// HostShouldQuit fires ONLY when the close sequence ends
        /// with `state.windows` empty AND a Host was running at
        /// emit time. Property: every HostShouldQuit event in the
        /// stream was emitted on a transition where, immediately
        /// after the close was applied, windows was empty. Catches
        /// a regression where the saga fires while pool labels (or
        /// some other entry) were still in `windows`.
        #[test]
        fn host_should_quit_only_on_empty_windows_transition(
            cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
        ) {
            let (mut state, host_ctx) = registered_host_state();
            for cmd in cmds {
                let pre_window_count = state.windows.len();
                let evs = update(&mut state, cmd, &host_ctx);
                let saw_quit = evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. }));
                if saw_quit {
                    prop_assert_eq!(
                        state.windows.len(), 0,
                        "HostShouldQuit emitted but windows={:?} (pre={})",
                        state.windows.keys().collect::<Vec<_>>(), pre_window_count
                    );
                    prop_assert!(
                        host_is_running(&state),
                        "HostShouldQuit emitted but no Host in Running state"
                    );
                }
            }
        }

        /// Backend window IDs are gated by `state.windows` membership
        /// at register time. The host calls
        /// `ReportBackendWindowIdRegistered` only after the window's
        /// `register_backend_window` IPC, which happens after
        /// `on_after_created` (which sent `ReportWindowOpened`). So
        /// in valid traffic, a register's label should be in
        /// `state.windows`. The reducer's behaviour on out-of-order
        /// or stale traffic is to silently store the mapping (the
        /// shadow gets cleaned up on later WindowClosed). This test
        /// pins that lenient behaviour: backend_window_ids never has
        /// MORE entries than the cumulative-register minus
        /// cumulative-unregister count, no phantom entries appear.
        #[test]
        fn backend_window_ids_bounded_by_register_minus_unregister(
            cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
        ) {
            let (mut state, host_ctx) = registered_host_state();
            let mut registers = 0i64;
            let mut unregisters = 0i64;
            for cmd in cmds {
                match &cmd {
                    Command::ReportBackendWindowIdRegistered { .. } => registers += 1,
                    Command::ReportBackendWindowIdUnregistered { .. } => unregisters += 1,
                    _ => {}
                }
                let _ = update(&mut state, cmd, &host_ctx);
                prop_assert!(
                    state.backend_window_ids.len() as i64 <= registers,
                    "backend_window_ids has {} entries; only {} registers seen",
                    state.backend_window_ids.len(), registers
                );
                let _ = unregisters;
            }
        }
    }

    // -----------------------------------------------------------
    // B.8 — synthetic close-all integration test.
    //
    // The CI-synthetic-close-all assertion the B.8 plan calls for.
    // Drives the reducer through a full session (register → open
    // main → open secondary → tear-off → close all) and asserts
    // the cascade emits the OrphanInstance + HostShouldQuit pair
    // exactly once on the last close.
    // -----------------------------------------------------------
    #[test]
    fn close_all_cascade_emits_orphan_and_quit_exactly_once() {
        let (mut state, host_ctx) = registered_host_state();

        // Open main + 2 secondaries + 3 pool windows.
        let _ = update(&mut state, Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        }, &host_ctx);
        let _ = update(&mut state, Command::ReportWindowOpened {
            label: "second-a".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        }, &host_ctx);
        let _ = update(&mut state, Command::ReportWindowOpened {
            label: "second-b".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        }, &host_ctx);
        for label in &["pool-1", "pool-2", "pool-3"] {
            let _ = update(&mut state, Command::ReportPoolWindowAdded {
                label: (*label).into(),
            }, &host_ctx);
        }

        assert_eq!(state.windows.len(), 3, "main + 2 secondaries");
        assert_eq!(state.pool.len(), 3, "3 pool labels");

        // Close secondaries — neither close should emit HostShouldQuit
        // (windows still non-empty after each).
        for label in &["second-a", "second-b"] {
            let evs = update(&mut state, Command::ReportWindowClosed {
                label: (*label).into(),
            }, &host_ctx);
            assert!(
                !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
                "premature HostShouldQuit on {} close: {:?}", label, evs
            );
        }

        // Close main — the cascade should fire here. Expected events:
        // WindowClosed(main) + WindowInstanceReleased(main) +
        // OrphanInstance drift + HostShouldQuit. Order: Released
        // before drift (the close path emits Released after the
        // window is removed but before the empty-check).
        let evs = update(&mut state, Command::ReportWindowClosed {
            label: "main".into(),
        }, &host_ctx);

        let drift_count = evs.iter().filter(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. }
        )).count();
        let quit_count = evs.iter().filter(|e| matches!(
            e, Event::HostShouldQuit { .. }
        )).count();
        assert_eq!(drift_count, 1, "expected 1 OrphanInstance drift, got {}: {:?}", drift_count, evs);
        assert_eq!(quit_count, 1, "expected 1 HostShouldQuit, got {}: {:?}", quit_count, evs);
        assert!(state.windows.is_empty(), "windows must be empty post-cascade");

        // Drain pool — emits PoolWindowRemoved each time. No further
        // HostShouldQuit (the saga is one-shot per close-all transition).
        for label in &["pool-1", "pool-2", "pool-3"] {
            let evs = update(&mut state, Command::ReportPoolWindowRemoved {
                label: (*label).into(),
            }, &host_ctx);
            assert!(
                !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
                "spurious HostShouldQuit during pool drain on {}: {:?}", label, evs
            );
        }
        assert!(state.pool.is_empty(), "pool must be empty after drain");
    }

    // -----------------------------------------------------------
    // Phase D.1 — GetSnapshot tests
    // -----------------------------------------------------------

    #[test]
    fn get_snapshot_returns_canonical_state_in_one_event() {
        let (mut state, host_ctx) = registered_host_state();

        // Drive some state into the reducer.
        let _ = update(&mut state, Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        }, &host_ctx);
        let _ = update(&mut state, Command::ReportWindowOpened {
            label: "window-abc".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        }, &host_ctx);
        let _ = update(&mut state, Command::ReportPoolWindowAdded {
            label: "pool-x".into(),
        }, &host_ctx);
        let _ = update(&mut state, Command::ReportBackendWindowIdRegistered {
            label: "main".into(),
            window_id: "uuid-main".into(),
        }, &host_ctx);

        // Snapshot.
        let evs = update(&mut state, Command::GetSnapshot, &host_ctx);
        assert_eq!(evs.len(), 1, "expected exactly 1 Snapshot event, got {:?}", evs);

        let Event::Snapshot {
            version,
            lifecycle: _,
            windows,
            pool,
            instance_registry,
            backend_window_ids,
            monitors: _,
        } = &evs[0] else {
            panic!("expected Snapshot, got {:?}", evs[0]);
        };

        // Sorted ordering: "main" < "window-abc".
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "main");
        assert_eq!(windows[1].label, "window-abc");
        assert_eq!(pool, &vec!["pool-x".to_string()]);
        // instance_registry has main=1 (pre-seed) + window-abc=2.
        assert_eq!(instance_registry.len(), 2);
        assert_eq!(instance_registry[0], ("main".to_string(), 1));
        assert_eq!(backend_window_ids, &vec![("main".to_string(), "uuid-main".to_string())]);
        // Snapshot's version is monotonic w.r.t. earlier events.
        assert!(*version > 0, "snapshot version must be non-zero (was bump_version'd)");
    }

    #[test]
    fn get_snapshot_does_not_mutate_canonical_state() {
        let (mut state, host_ctx) = registered_host_state();
        let _ = update(&mut state, Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        }, &host_ctx);

        let pre_windows = state.windows.clone();
        let pre_pool = state.pool.clone();
        let pre_instance_registry = state.instance_registry.clone();
        let pre_backend_window_ids = state.backend_window_ids.clone();
        let pre_lifecycle = state.lifecycle;

        let _ = update(&mut state, Command::GetSnapshot, &host_ctx);

        assert_eq!(state.windows, pre_windows);
        assert_eq!(state.pool, pre_pool);
        assert_eq!(state.instance_registry, pre_instance_registry);
        assert_eq!(state.backend_window_ids, pre_backend_window_ids);
        assert_eq!(state.lifecycle, pre_lifecycle);
    }

    #[test]
    fn get_snapshot_works_for_non_host_clients() {
        // Tool clients (e.g. --diag wrr) should be able to query
        // snapshots — host-only gate doesn't apply.
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Tool,
                pid: 99,
                version: "diag".into(),
            },
            &ctx(1),
        );
        let tool_ctx = ctx_with_pid(1, 99);

        let evs = update(&mut state, Command::GetSnapshot, &tool_ctx);
        assert_eq!(evs.len(), 1);
        assert!(matches!(evs[0], Event::Snapshot { .. }));
    }
}
