// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sync report_* API — the host-side "tell the launcher what just
// happened" calls. Every function here follows the same shape: look
// up `COMMAND_TX` (published by `connect_to_launcher` in the parent
// `launcher_ipc` module once the launcher pipe is connected), build a
// `Command` variant, and push it onto the outbound channel. `None` /
// send-failure is always a silent no-op — the host's authoritative
// state is unaffected either way, only the launcher's mirror falls
// behind. See the module docs on `super::COMMAND_TX` for the full
// rationale.
//
// Split out of `launcher_ipc.rs` (now `launcher_ipc/mod.rs`) — this
// file is the uniform, stateless "report a fact to the launcher"
// family; connection setup, shadow projection, and the reader/drain
// tasks stay in the parent module.

use agentmux_common::ipc::Command;

/// Workstream 0 Phase 1 (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`
/// §7) — tell the launcher whether background-service mode
/// (`AGENTMUX_BACKGROUND_SERVICE`) is enabled for this host process. Called
/// once, right after connecting (see `connect_to_launcher`'s
/// `request_snapshot()` call for the sibling once-per-connect signal). The
/// launcher's last-window orphan-drift detection and the teardown backstop
/// it arms must know this to avoid treating an intentionally-resting host
/// as a stuck/orphaned one — see PR #2983 review (Codex P2). Same
/// no-op-if-disconnected semantics as every other `report_*` helper.
pub fn report_background_service_enabled(enabled: bool) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportBackgroundServiceEnabled { enabled };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_background_service_enabled: channel closed ({})",
            e
        );
    }
}

/// Phase B.4 — sync API: report a window open to the launcher's
/// state mirror. Called from CEF lifecycle callbacks on the UI
/// thread. No-op if the launcher pipe isn't connected (`task dev`
/// mode); failures to enqueue (channel closed, drain task died)
/// are logged but don't propagate — the host's authoritative state
/// is unaffected, the mirror just falls behind. B.5 tightens.
pub fn report_window_opened(
    label: String,
    kind: agentmux_common::ipc::WindowKind,
    parent_label: Option<String>,
) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return; // launcher not in the loop
    };
    let cmd = Command::ReportWindowOpened {
        label,
        kind,
        parent_label,
    };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_window_opened: channel closed ({})", e);
    }
}

/// Phase B.4 — sync API: report a window close to the launcher's
/// state mirror. Same no-op-if-disconnected semantics as
/// `report_window_opened`.
pub fn report_window_closed(label: String) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportWindowClosed { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_window_closed: channel closed ({})", e);
    }
}

/// Phase B.4 follow-up — sync API: report a pool window being added
/// to the warm pool inventory. Called from `spawn_pool_window` on
/// the UI thread. No-op when launcher pipe is absent.
pub fn report_pool_window_added(label: String) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    // CPD-1 (schema-only): host's existing `report_pool_window_added`
    // call sites are organic refills (not yet saga-driven); pass
    // `saga_id: None` per spec §3.3. CPD-3 wires the saga-driven
    // path that will pass `Some(N)` through here.
    let cmd = Command::ReportPoolWindowAdded { label, saga_id: None };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_pool_window_added: channel closed ({})", e);
    }
}

/// Phase B.4 follow-up — sync API: report a pool window leaving the
/// pool (promote, destroy). On promote callers should also call
/// `report_window_opened` so the label transitions atomically (from
/// the launcher's perspective) from `pool` to `windows`.
pub fn report_pool_window_removed(label: String) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportPoolWindowRemoved { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_pool_window_removed: channel closed ({})", e);
    }
}

/// Phase F.5 — sync API: tell the launcher that a pool window is
/// promoting to a user-visible top-level window. Sent BETWEEN
/// `report_pool_window_removed` and `report_window_opened` so the
/// launcher's pool-respawn saga (state-machine bracket around the
/// implicit refill) can correlate the promote event with the
/// subsequent `PoolWindowAdded` for the freshly-spawned replacement
/// pool slot.
///
/// Same no-op-if-disconnected semantics as the other `report_*`
/// helpers — `task dev` mode (no launcher in the loop) silently
/// drops; the host's authoritative state and refill mechanism are
/// unaffected.
pub fn report_pool_window_promoted(label: String) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportPoolWindowPromoted { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_pool_window_promoted: channel closed ({})", e);
    }
}

/// Phase F.6 — sync API: tell the launcher that all browser-pane
/// HWNDs belonging to a closing top-level window have been reaped
/// (lifecycle entries drained, pane HWND map cleared, subwindow
/// cascade closes initiated). Sent from `client.rs::on_before_close`
/// AFTER the pane drain step, BEFORE the post-close pool-drain
/// decision is reported.
///
/// The launcher's window-cleanup-cascade saga uses this as the
/// Step 1 → Step 2 transition signal: it marks the implicit pane
/// reap as observed and lets the saga issue its `DrainPoolIfLast`
/// IssueCmd (currently log-only — see `saga/window_cleanup.rs`
/// module docstring for the saga-as-narrator scope decision).
///
/// Same no-op-if-disconnected semantics as the other `report_*`
/// helpers; `task dev` mode silently drops.
pub fn report_panes_reaped(label: String) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    // CPD-1 (schema-only): existing call sites are organic; CPD-3
    // adds the saga-driven path that fills `saga_id`.
    let cmd = Command::ReportPanesReaped { label, saga_id: None };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_panes_reaped: channel closed ({})", e);
    }
}

/// Phase F.6 — sync API: tell the launcher the result of the
/// post-close drain-pool-if-last decision. `was_last == true` when
/// the closing window was the last user-visible window (Stage 1 of
/// the wrr two-stage close cascade just kicked off in
/// `client.rs::on_before_close`); `false` when other windows
/// remain and the warm pool stays warm.
///
/// Step 2 terminal signal for the launcher's
/// window-cleanup-cascade saga. Both branches close the
/// `SagaStarted` bracket successfully — the saga's job is to
/// narrate the decision, not enforce a particular outcome.
///
/// Same no-op-if-disconnected semantics as the other `report_*`
/// helpers.
pub fn report_pool_drain_decision(label: String, was_last: bool) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    // CPD-1 (schema-only): existing call sites are organic; CPD-3
    // adds the saga-driven path that fills `saga_id`.
    let cmd = Command::ReportPoolDrainDecision {
        label,
        was_last,
        saga_id: None,
    };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_pool_drain_decision: channel closed ({})",
            e
        );
    }
}

/// Phase B.4 follow-up — sync API: report the host's current
/// authoritative counts so the launcher reducer can compare against
/// its mirror and emit `Event::DriftDetected` on mismatch. Callers
/// invoke this AFTER each window-level transition so the launcher
/// gets a fresh snapshot to compare against its just-applied
/// transition.
pub fn report_host_counts(windows: u32, pool: u32) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHostCounts { windows, pool };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_host_counts: channel closed ({})", e);
    }
}

/// Phase B.5 (window_id_map step b) — sync API: report the
/// frontend's `register_backend_window` IPC to the launcher.
/// Called from `commands/window.rs::register_backend_window`
/// after the host's local `window_id_map` insert. No-op if the
/// launcher pipe isn't connected.
pub fn report_backend_window_id_registered(label: String, window_id: String) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportBackendWindowIdRegistered { label, window_id };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_backend_window_id_registered: channel closed ({})",
            e
        );
    }
}

/// SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 1 — sync API: answer a
/// `ProbeUiThread` once its posted UI task actually executes. Called
/// EXCLUSIVELY from `ProbeUiThreadReplyTask::execute()` on the UI thread —
/// calling it from anywhere else would forge the exact liveness evidence
/// the probe exists to collect. No-op if the launcher pipe is absent
/// (standalone mode has no prober).
pub fn report_ui_thread_alive(nonce: u64) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportUiThreadAlive { nonce };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_ui_thread_alive: channel closed ({})",
            e
        );
    }
}

/// Phase B.5 (window_id_map step b) — sync API: report a window's
/// backend ID being dropped (close path). Called from
/// `client.rs::on_before_close` after the host's local
/// `window_id_map.remove`. No-op if launcher pipe absent.
pub fn report_backend_window_id_unregistered(label: String) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportBackendWindowIdUnregistered { label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!(
            "[launcher-ipc] report_backend_window_id_unregistered: channel closed ({})",
            e
        );
    }
}

/// Phase B.4 follow-up — sync API: report the host's pool count
/// only. Used by `spawn_pool_window` where the windows dimension
/// is mid-flight relative to the launcher mirror (refill happens
/// during a close path that hasn't sent `ReportWindowClosed` yet);
/// snapshotting only the pool dimension preserves the
/// check-every-transition guarantee without producing false
/// windows-drift. (codex P2 PR #578 round-3.)
pub fn report_host_pool_count(count: u32) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHostPoolCount { count };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_host_pool_count: channel closed ({})", e);
    }
}

/// Phase B.9.1 (WRR) — sync API: report a Win32 HWND created.
/// Called from the WRR `SetWinEventHook` callback. No-op if the
/// launcher pipe isn't connected (`task dev` mode); reducer arm
/// stashes pending-without-label until reconciliation.
pub fn report_hwnd_opened(
    hwnd: u64,
    class_name: String,
    title: String,
    label_hint: Option<String>,
) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndOpened {
        hwnd,
        class_name,
        title,
        label_hint,
    };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_opened: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report a Win32 HWND destroyed.
pub fn report_hwnd_destroyed(hwnd: u64) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndDestroyed { hwnd };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_destroyed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report visibility change.
pub fn report_hwnd_visibility_changed(hwnd: u64, visible: bool) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndVisibilityChanged { hwnd, visible };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_visibility_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report foreground change.
pub fn report_hwnd_foreground_changed(hwnd: u64) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndForegroundChanged { hwnd };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_foreground_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report iconic (minimized) change.
pub fn report_hwnd_iconic_changed(hwnd: u64, iconic: bool) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndIconicChanged { hwnd, iconic };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_iconic_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report position change. Caller is
/// responsible for debouncing — see `wrr/position_debounce.rs`.
pub fn report_hwnd_position_changed(hwnd: u64, rect: agentmux_common::ipc::Rect) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportHwndPositionChanged { hwnd, rect };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_hwnd_position_changed: channel closed ({})", e);
    }
}

/// Phase B.9.1 — sync API: report current monitor topology. Sent
/// once at install time; mid-session topology changes are a B.9.2
/// follow-up.
pub fn report_monitor_topology_changed(rects: Vec<agentmux_common::ipc::Rect>) {
    let Some(tx) = super::COMMAND_TX.get() else {
        return;
    };
    let cmd = Command::ReportMonitorTopologyChanged { rects };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_monitor_topology_changed: channel closed ({})", e);
    }
}

/// Startup-stage telemetry — sync API: report the start of a named
/// host-side startup phase. No-op if the launcher isn't in the loop
/// (`dev:standalone`, or `connect_to_launcher` hasn't run/succeeded
/// yet — see the doc comment on `Command::ReportStartupStageBegin`
/// for which phases can and can't use this live). Forwarded by the
/// launcher into its `StartupEventSink`, rendering in the splash
/// telemetry panel alongside the launcher's own `saga`/`backend`/
/// `host` stages. See SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_
/// 2026_07_02.md.
pub fn report_startup_stage_begin(stage: impl Into<String>, label: impl Into<String>) {
    let Some(tx) = super::COMMAND_TX.get() else {
        tracing::debug!("[launcher-ipc] report_startup_stage_begin: no launcher connection, skipped");
        return;
    };
    let stage = stage.into();
    let label = label.into();
    tracing::debug!(stage = %stage, label = %label, "[launcher-ipc] report_startup_stage_begin");
    let cmd = Command::ReportStartupStageBegin { stage, label };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_startup_stage_begin: channel closed ({})", e);
    }
}

/// Companion to `report_startup_stage_begin`. `status` is one of
/// `"ok"` / `"warn"` / `"error"`.
pub fn report_startup_stage_end(
    stage: impl Into<String>,
    duration_ms: u64,
    status: &str,
    detail: Option<String>,
) {
    let Some(tx) = super::COMMAND_TX.get() else {
        tracing::debug!("[launcher-ipc] report_startup_stage_end: no launcher connection, skipped");
        return;
    };
    let stage = stage.into();
    tracing::debug!(
        stage = %stage,
        duration_ms,
        status,
        "[launcher-ipc] report_startup_stage_end"
    );
    let cmd = Command::ReportStartupStageEnd {
        stage,
        duration_ms,
        status: status.to_string(),
        detail,
    };
    if let Err(e) = tx.send(cmd) {
        tracing::warn!("[launcher-ipc] report_startup_stage_end: channel closed ({})", e);
    }
}

/// Phase B.4 follow-up — compute the host's authoritative counts
/// from `AppState` and report them. Callers invoke this AFTER
/// each window/pool transition.
///
/// Atomic snapshot: holds both `unpromoted_pool_labels` and
/// `browsers` simultaneously so the reported `(windows, pool)`
/// pair is from one consistent state. Without this, a concurrent
/// mutation between the two lock acquisitions (CEF lifecycle on
/// the UI thread vs. IPC handler in `commands/drag.rs`) could
/// produce a mismatched snapshot and trigger a spurious
/// `Event::DriftDetected`. (codex P2 PR #578 round-1.)
///
/// Lock order: `unpromoted_pool_labels` first, then `browsers`.
/// Matches the existing snapshot pattern in
/// `client.rs::on_before_close` (line ~418) and is the only place
/// in the codebase that holds both locks simultaneously, so no
/// other path can race in the reverse order.
///
/// Counts (matching the launcher's mirror semantics):
/// * `windows` — top-level user-visible windows in `browsers`,
///   excluding `browser-pane-*` child HWNDs and any label still
///   in `unpromoted_pool_labels`.
/// * `pool` — pre-promote pool labels (`unpromoted_pool_labels.len()`).
///
/// **Why this reads host's `browsers` and `unpromoted_pool_labels`
/// directly (not the shadow):** this fn IS the source for the
/// launcher's mirror — its output is what gets compared against
/// `state.windows.len()` / `state.pool.len()` in the drift-detection
/// path. Reading from the shadow would compare the shadow against
/// itself (always agrees) and defeat the entire B.4 drift-detection
/// design. Once the host reducer arrives in Phase F (see
/// `docs/retro/multi-reducer-proposal-2026-04-28.md`), this becomes
/// "report host's authoritative reducer-state to the launcher."
pub fn compute_and_report_host_counts(state: &std::sync::Arc<crate::state::AppState>) {
    // Atomic snapshot — pool inventory + browsers under ONE
    // `host_state` lock. Two-lock variants race against
    // `promote_pool_window` between reads and let queued pool
    // windows leak into the user-window count, triggering spurious
    // launcher drift-detection.
    let (windows, pool) = state.host_counts_snapshot();
    report_host_counts(windows, pool);
}
