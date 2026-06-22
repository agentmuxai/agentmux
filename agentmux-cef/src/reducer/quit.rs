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

/// Background PENDING-CREATION labels — warm-pool tab refills (`window-pool-`),
/// browser-pane children (`browser-pane-`), and floating panes (`floating-`, the
/// BROAD prefix). A pending creation with one of these prefixes is not a user
/// "New Window" and must not block drain. Mirrors EXACTLY the exclusion
/// `commands::orphan_reconcile` applies to `pending_window_creations`
/// (orphan_reconcile.rs:302-304) — including the broad `floating-`: a #810/#811
/// failure path can leak a pending `floating-` entry, and gating drain on it
/// would block reconciliation forever (reagent P2 #1676).
///
/// Used ONLY for pending creations. Registered browsers are classified by
/// `is_live_user_window` (the `is_pool` flag), NOT by label — see its doc.
#[allow(dead_code)]
fn is_background_pending_creation_label(label: &str) -> bool {
    label.starts_with("window-pool-")
        || label.starts_with("browser-pane-")
        || label.starts_with("floating-")
}

/// Whether a registered browser is a live, user-visible top-level window — one
/// that keeps the instance alive. The authoritative signal is the per-browser
/// `BrowserKind::is_pool` flag, flipped `false` atomically at promote time
/// (`pool.rs`). We must NOT classify by label here: a PROMOTED pool window keeps
/// its `window-pool-` label forever yet IS the user's real window — classifying
/// by label would drop it and quit with a window open (reagent P1 #1676).
/// Unpromoted pool windows are `is_pool: true`; panes are `BrowserKind::Pane`;
/// both are correctly excluded by the `is_pool: false` match.
///
/// `pub(crate)` + re-exported from `reducer` so the live last-window quit gate
/// (`AppState::count_live_user_windows`, used by `client::on_before_close`) shares
/// this one definition rather than duplicating the predicate.
pub(crate) fn is_live_user_window(kind: &BrowserKind) -> bool {
    matches!(kind, BrowserKind::TopLevel { is_pool: false })
}

/// Whether a REGISTERED browser (label + kind) counts as a live, user-visible
/// top-level window for the last-window quit gate.
///
/// `is_live_user_window` handles the per-browser `is_pool` flag (the window
/// pool), but **pane-pool** windows (`floating-pool-*`) register as
/// `TopLevel { is_pool: false }` — the `on_after_created` classification only
/// special-cases `window-pool-`, and `PanePoolState` has no `is_pool` flag — so
/// the flag alone would count the always-seeded warm pane-pool window
/// (macOS/Linux `init_pane_pool`, `PANE_POOL_TARGET_SIZE = 1`) as a real user
/// window, and the gate would never reach 0 (reagent P0 #1676). Exclude them by
/// label, matching the canonical `compute_and_report_host_counts` exclusion
/// (state.rs:~1121). A PROMOTED `window-pool-*` window (is_pool:false, still
/// `window-pool-` labelled) correctly still counts.
pub(crate) fn counts_as_live_user_window(label: &str, kind: &BrowserKind) -> bool {
    is_live_user_window(kind) && !label.starts_with("floating-pool-")
}

/// Count of live, user-visible top-level windows — the windows that keep the
/// instance alive. Live last-window quit gate via `AppState::count_live_user_windows`
/// → `client::on_before_close` and `wrr::win_event::maybe_quit_on_last_user_window`.
pub(crate) fn count_live_user_windows(state: &HostState) -> usize {
    state
        .browsers
        .iter()
        .filter(|(label, h)| counts_as_live_user_window(label, &h.kind))
        .count()
}

/// Whether a USER-initiated top-level window creation is in flight (enqueued but
/// not yet registered). Such a creation has no registered browser yet — so it is
/// invisible to `count_live_user_windows` — which is exactly why the gate must
/// consult the PRE-registration `pending_window_creations` queue (spec §10.2):
/// draining while a user's "New Window" is still loading would quit the instance
/// out from under it. Background creations (pool refill / panes) do not block drain.
#[allow(dead_code)]
pub(super) fn user_creation_in_flight(state: &HostState) -> bool {
    state
        .pending_window_creations
        .iter()
        .any(|p| !is_background_pending_creation_label(&p.label))
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

