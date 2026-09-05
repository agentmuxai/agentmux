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
// WIRED (Pillar 2, Stage 1 — SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md):
// `reducer::update` now calls `reconcile_quit` after every quit-relevant command
// (gated by `is_quit_relevant`) and surfaces the result via
// `DispatchOutput::request_drain`. The decision is computed here, under the
// host-state lock, as a pure read.
//
// WIRED, PARTIALLY (Pillar 2, Stage 2 — same spec, §3.2/§4#3):
// `client::on_before_close` now consumes `request_drain` (from the
// `UnregisterBrowser` dispatch it already makes) and calls the extracted
// `AgentMuxHandler::begin_drain_and_cascade` executor instead of re-deriving
// `count_live_user_windows() == 0` itself — the close-edge path is now a pure
// executor of this module's decision, not a second decision-maker.
//
// WIRED (Pillar 2, sanitize-then-decide Phases 1–2 —
// SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md): every dispatch site whose
// command can flip the verdict to Some now consumes `request_drain` via
// `ui_tasks::consume_request_drain` (posts the shared Stage-1 executor):
// `unregister_after_parking_close` (the dominant Windows close path), the WRR
// LOCATIONCHANGE recycle-close detector, `DemotePoolWindow` (the one close
// flow that lowers the count without an UnregisterBrowser), and the two
// failed-promote orphan cleanups. `commands::orphan_reconcile` no longer has
// an independent `begin_drain` predicate — it sanitizes the browsers
// projection, then reads this module's verdict via the `ReconcileQuit` poke.
// Deliberately NON-consuming: `on_after_created`'s Dequeue→Register pair (the
// gap between them would surface a spurious verdict for a second window
// opening into an otherwise-empty session; the Register lands in the same
// lock scope of activity and re-blocks it — do not consume there).
//
// WIRED (Phase 3): `wrr::win_event::maybe_quit_on_last_user_window` now
// requires `QuitState::Draining` (a decision this module made and a Phase-2
// site consumed) before its happy-path `quit_message_loop()` — it is the
// Windows Stage-2 EXECUTOR of this module's decision, not an authority
// (parked browsers never fire `on_before_close` on Windows, so the
// on_before_close Stage-2 gate is structurally unreachable there). The quit
// watchdog remains the bounded backstop for both desync flavors
// (registered > 0, or counts-zero-but-never-decided) and is the only
// remaining direct `quit_message_loop` caller outside the documented
// Stage-2 executors. `reconcile_quit` is the sole quit decision-maker.

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
fn is_background_pending_creation_label(label: &str) -> bool {
    label.starts_with("window-pool-")
        || label.starts_with("browser-pane-")
        || label.starts_with("floating-")
}

/// Whether a registered browser is a live, user-visible top-level window — one
/// that keeps the instance alive. Decided PURELY BY TYPE (`BrowserKind`), never
/// by label-prefix string:
/// - `TopLevel { is_pool: false }` → YES (the user's real windows, including a
///   PROMOTED pool window which keeps its `window-pool-` label forever yet IS a
///   real window — classifying by label would drop it and quit with a window
///   open, reagent P1 #1676).
/// - `TopLevel { is_pool: true }` (warm window pool) → no.
/// - `Floater { .. }` (floating panes / tear-offs, `floating-`/`floating-pool-`)
///   → no — floaters die with the last top-level window (invariant FP-LIFE;
///   docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md §1.1).
///   This replaced a `!label.starts_with("floating-pool-")` string check that
///   missed direct `floating-<uuid>` floaters (SPEC_REDUCER_SSOT_CONSOLIDATION L4).
/// - `Pane { .. }` (browser-pane children) → no.
///
/// `pub(crate)` + re-exported from `reducer` so the live last-window quit gate
/// (`AppState::count_live_user_windows`, used by `client::on_before_close`) shares
/// this one definition rather than duplicating the predicate.
pub(crate) fn is_live_user_window(kind: &BrowserKind) -> bool {
    matches!(kind, BrowserKind::TopLevel { is_pool: false })
}

/// Count of live, user-visible top-level windows — the windows that keep the
/// instance alive. Live last-window quit gate via `AppState::count_live_user_windows`
/// → `client::on_before_close` and `wrr::win_event::maybe_quit_on_last_user_window`.
/// Excludes pool windows, floaters, and panes BY TYPE (`is_live_user_window`).
pub(crate) fn count_live_user_windows(state: &HostState) -> usize {
    state
        .browsers
        .values()
        .filter(|h| is_live_user_window(&h.kind))
        .count()
}

/// Should a browser that is registering RIGHT NOW be closed on arrival?
///
/// True only for a live user window arriving while the instance has already
/// left `Running` — a creation that was in flight when a quit began
/// (ReAgent P1 on PR #2996). The pre-checks on the creation paths narrow that
/// race but cannot close it; registration is the last step, so this is the
/// only unraceable point to decide.
///
/// Background browsers are deliberately excluded: pool browsers legitimately
/// register during a drain (the drain cascade closes them itself), and
/// panes/floaters are not top-level windows the quit is responsible for.
///
/// Pure, and separate from `handle_register_browser`, because that arm takes
/// a real `cef::Browser` which no test can construct — same reason
/// `live_user_window_labels_from` exists.
pub(crate) fn should_close_on_arrival(kind: &BrowserKind, quit_state: &QuitState) -> bool {
    is_live_user_window(kind) && !matches!(quit_state, QuitState::Running)
}

/// Labels of every live, user-visible top-level window, by the same
/// `is_live_user_window` classification `count_live_user_windows` uses.
///
/// Exists so an explicit quit (`commands::window::quit_app`, the tray's "Quit
/// AgentMux") can close exactly the windows that count, without
/// `is_live_user_window` having to leave this module — the classification
/// stays here, callers get purpose-built accessors. Same reason
/// `count_live_user_windows` is shaped the way it is.
/// Split from the `HostState` wrapper below so the classification is
/// unit-testable: a `BrowserHandle` owns a real `cef::Browser`, which no test
/// can construct, so anything reading `state.browsers` directly is untestable
/// by construction. Same pure-core/thin-wrapper shape as `should_begin_drain`
/// vs `reconcile_quit`.
pub(crate) fn live_user_window_labels_from<'a>(
    entries: impl Iterator<Item = (&'a String, &'a BrowserKind)>,
) -> Vec<String> {
    entries
        .filter(|(_, kind)| is_live_user_window(kind))
        .map(|(label, _)| label.clone())
        .collect()
}

pub(crate) fn live_user_window_labels(state: &HostState) -> Vec<String> {
    live_user_window_labels_from(state.browsers.iter().map(|(label, h)| (label, &h.kind)))
}

/// Whether a USER-initiated top-level window creation is in flight (enqueued but
/// not yet registered). Such a creation has no registered browser yet — so it is
/// invisible to `count_live_user_windows` — which is exactly why the gate must
/// consult the PRE-registration `pending_window_creations` queue (spec §10.2):
/// draining while a user's "New Window" is still loading would quit the instance
/// out from under it. Background creations (pool refill / panes) do not block drain.
pub(super) fn user_creation_in_flight(state: &HostState) -> bool {
    state
        .pending_window_creations
        .iter()
        .any(|p| !is_background_pending_creation_label(&p.label))
}

/// Pure decision: should the host begin draining NOW? `Some(reason)` iff the host
/// is armed (a live user window has registered at least once this process —
/// `HostState::saw_live_user_window`, §1.E of the sanitize-then-decide spec),
/// `Running`, no live user-visible window remains, no user-initiated
/// creation is in flight, and background-service mode is not enabled. Unarmed
/// covers the startup gap: main's creation path enqueues no
/// `PendingWindowCreation`, so before its `RegisterBrowser` lands both other
/// inputs read "drainable" — without the arming gate, any quit-relevant
/// dispatch in that window would surface a spurious drain request. Safe to
/// call after every transition — once `Draining`/`Quit` it returns `None`
/// (the transition is monotonic — see `handle_begin_drain`). CEF-free so the
/// full truth table is unit-testable.
///
/// `background_service_enabled` (Workstream 0 Phase 1,
/// `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §7) gates only the
/// `LastWindowClosed` reason produced here — `QuitReason::LauncherRequested`
/// and `QuitReason::External` are dispatched directly to `handle_begin_drain`
/// (see `ui_tasks::window::begin_drain_and_cascade`'s callers), never through
/// this function, so a genuine launcher-requested or external quit always
/// works regardless of this flag.
pub(super) fn should_begin_drain(
    armed: bool,
    live_user_windows: usize,
    user_creation_in_flight: bool,
    quit_state: &QuitState,
    background_service_enabled: bool,
) -> Option<QuitReason> {
    if !armed {
        return None;
    }
    if !matches!(quit_state, QuitState::Running) {
        return None;
    }
    if live_user_windows > 0 || user_creation_in_flight {
        return None;
    }
    if background_service_enabled {
        return None;
    }
    Some(QuitReason::LastWindowClosed)
}

/// Level-triggered quit reconciliation over the full `HostState`. Composes the
/// reads above. Returns the drain reason iff it is safe to begin draining.
pub(super) fn reconcile_quit(state: &HostState) -> Option<QuitReason> {
    should_begin_drain(
        state.saw_live_user_window,
        count_live_user_windows(state),
        user_creation_in_flight(state),
        &state.quit_state,
        state.background_service_enabled,
    )
}

