// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Quit lifecycle (Phase H.5) reducer handlers. Extracted from reducer/mod.rs in
//! task #182 PR-F-2 for navigability.


use crate::state::*;

use super::{DispatchOutput, HostEvent, HostState};

// ── H.5 — quit lifecycle ─────────────────────────────────────────────────

pub(super) fn handle_begin_drain(state: &mut HostState, reason: QuitReason) -> DispatchOutput {
    if state.quit_state != QuitState::Running {
        return DispatchOutput::default(); // already draining or quit; idempotent
    }
    state.quit_state = QuitState::Draining { reason: reason.clone() };
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::QuitDraining { reason, version: v }],
        ..Default::default()
    }
}

pub(super) fn handle_confirm_drained(state: &mut HostState) -> DispatchOutput {
    if !matches!(state.quit_state, QuitState::Draining { .. }) {
        return DispatchOutput::default(); // not draining; idempotent
    }
    state.quit_state = QuitState::Quit;
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::QuitReady { version: v }],
        ..Default::default()
    }
}

// ── Level-triggered quit reconciliation ──────────────────────────────────────
//   (SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md §5.1 / §10)
//
// The reported regression — closing the last window orphaned the whole process
// tree — was an EDGE-triggered quit gate: "should we quit?" was computed only
// inside `client::on_before_close`, racing concurrent pool refill/promotion with
// nothing to re-trigger the evaluation (confirmed mechanism A: the orphan host
// log shows `BeginDrain` never fired). `reconcile_quit` is the pure,
// LEVEL-triggered replacement — a function of current `HostState` that can be
// re-run after ANY window/pool transition and always reaches the right answer.
//
// THREADING CONTRACT (spec §10.1 — CRITICAL): these are pure reads/decisions.
// They MUST NOT call `quit_message_loop()`, re-lock `host_state`, or take any
// destructive action. The caller flips `QuitState` (which suppresses pool
// refill) via the existing `handle_begin_drain` transition; the UI-thread
// executor that actually closes pool browsers (Stage 1) and exits the message
// loop (Stage 2) stays exactly where it is today. `reconcile_quit` only DECIDES.
//
// NOT YET WIRED: this PR lands the decision + its test coverage (the safety net
// the regression slipped through). Calling it from the browser-deregister /
// promote-complete / creation-abort transitions — behind the §10.1 UI-thread
// executor — is the explicit next step. Hence `#[allow(dead_code)]`, matching the
// reducer-scaffolding convention documented in `state.rs`.

/// Labels for background (non-user-visible) browsers / pending creations: a
/// warm-pool tab window, a warm-pool floating pane, or a browser-pane child
/// view — none keeps the instance alive. (Spec Stage 1 upgrades this prefix test
/// to a typed `source`/`BrowserKind` field; this mirrors the predicate
/// `commands::orphan_reconcile` already trusts in production, so it is a faithful
/// minimal stopgap, not a new taxonomy.)
#[allow(dead_code)]
pub(super) fn is_background_creation_label(label: &str) -> bool {
    label.starts_with("window-pool-")
        || label.starts_with("floating-pool-")
        || label.starts_with("browser-pane-")
}

/// Count of live, user-visible top-level windows — the windows that keep the
/// instance alive. A promoted pool window has `is_pool == false` (flipped
/// atomically at promote time, `pool.rs`), so it counts; an unpromoted pool
/// window (`is_pool == true`) does not; `BrowserKind::Pane` children never do.
#[allow(dead_code)]
pub(super) fn count_live_user_windows(state: &HostState) -> usize {
    state
        .browsers
        .iter()
        .filter(|(label, h)| {
            matches!(h.kind, BrowserKind::TopLevel { is_pool: false })
                && !is_background_creation_label(label)
        })
        .count()
}

/// Whether a USER-initiated top-level window creation is in flight (enqueued but
/// not yet registered). Such a creation has no registered browser yet — so it is
/// invisible to `count_live_user_windows` — which is exactly why the gate must
/// consult the PRE-registration `pending_window_creations` queue (spec §10.2):
/// draining while a user's "New Window" is still loading would quit the instance
/// out from under it. Background creations (pool refill, panes) do not block drain.
#[allow(dead_code)]
pub(super) fn user_creation_in_flight(state: &HostState) -> bool {
    state
        .pending_window_creations
        .iter()
        .any(|p| !is_background_creation_label(&p.label))
}

/// Pure decision: should the host begin draining NOW? `Some(reason)` iff the host
/// is `Running`, no live user-visible window remains, and no user-initiated
/// creation is in flight. Safe to call after every transition — once
/// `Draining`/`Quit` it returns `None` (the transition is monotonic — see
/// `handle_begin_drain`). CEF-free so the full truth table is unit-testable.
#[allow(dead_code)]
pub(super) fn should_begin_drain(
    live_user_windows: usize,
    user_creation_in_flight: bool,
    quit_state: &QuitState,
) -> Option<QuitReason> {
    if !matches!(quit_state, QuitState::Running) {
        return None;
    }
    if live_user_windows > 0 || user_creation_in_flight {
        return None;
    }
    Some(QuitReason::LastWindowClosed)
}

/// Level-triggered quit reconciliation over the full `HostState`. Composes the
/// three reads above. Returns the drain reason iff it is safe to begin draining.
#[allow(dead_code)]
pub(super) fn reconcile_quit(state: &HostState) -> Option<QuitReason> {
    should_begin_drain(
        count_live_user_windows(state),
        user_creation_in_flight(state),
        &state.quit_state,
    )
}

