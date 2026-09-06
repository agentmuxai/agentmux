// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser handle registry (Phase H.2) reducer handlers. Extracted from reducer/mod.rs in
//! task #182 PR-F-2 for navigability.

use std::time::Instant;

use cef::Browser;

use crate::state::*;

use super::{DispatchOutput, HostEvent, HostLifecyclePhase, HostState, emit_error};

// ── H.2 — browser handle registry ────────────────────────────────────────

pub(super) fn handle_register_browser(
    state: &mut HostState,
    label: String,
    browser: Browser,
    kind: BrowserKind,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        return emit_error(state, format!("register_browser: shutting down (label={})", label));
    }
    if state.browsers.contains_key(&label) {
        return emit_error(state, format!("register_browser: label {} already registered", label));
    }
    state.browsers.insert(
        label.clone(),
        BrowserHandle {
            label: label.clone(),
            browser,
            kind: kind.clone(),
            registered_at: Instant::now(),
        },
    );
    // Quit arming (SPEC_PILLAR2_SANITIZE_THEN_DECIDE §1.E): the first live
    // user window arms `should_begin_drain`. Monotonic — never cleared.
    if super::quit::is_live_user_window(&kind) {
        state.saw_live_user_window = true;
    }
    // LEVEL-TRIGGERED close-on-arrival for a window that registers after the
    // instance already decided to quit (ReAgent P1 on PR #2996).
    //
    // The pre-checks in `open_window_with_kind` / `open_window_at_position`
    // narrow this race but cannot close it: a creation that passed them
    // microseconds before the drain began still runs enqueue →
    // `post_create_window` → here, and none of those steps re-read
    // `QuitState`. Registration is the LAST step, so checking here cannot be
    // raced by construction — the same level-triggered reasoning
    // `reducer/quit.rs`'s header documents for `reconcile_quit`, and the same
    // reason `promote_liveness::should_open_fallback` re-checks liveness at
    // its decision point rather than trusting every close path to cancel.
    //
    // Only user windows: pool browsers legitimately register during a drain
    // (the drain cascade closes them itself), and panes/floaters are not
    // top-level windows the quit is responsible for.
    let registered_during_drain = super::quit::should_close_on_arrival(&kind, &state.quit_state);

    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserRegistered { label, kind, version: v }],
        registered_during_drain,
        ..Default::default()
    }
}

/// Rename a browser entry `old_label` → `new_label`, re-keying the per-label
/// host state that persists for the window's life: the `browsers` map (and the
/// duplicated `BrowserHandle.label` field), plus `window_opacities` and
/// `pane_window_states` when an entry exists. `window_meta` / `window_hwnds`
/// live on `AppState` and are re-keyed by the caller
/// (`promote_pane_pool_window`). Errors if `old_label` is absent or `new_label`
/// is already registered. Emits no event — the frontend learns the new label
/// via the `pool:pane-promote` payload, not a host event.
/// See `SPEC_FLOATING_PANE_POOL_RELABEL_2026_06_30`.
pub(super) fn handle_relabel_browser(
    state: &mut HostState,
    old_label: String,
    new_label: String,
) -> DispatchOutput {
    if old_label == new_label {
        // No-op: label already correct. Treated as success so the caller
        // proceeds (nothing to re-key).
        return DispatchOutput {
            relabel_ok: true,
            ..Default::default()
        };
    }
    if !state.browsers.contains_key(&old_label) {
        return emit_error(
            state,
            format!("relabel_browser: old label {} not found", old_label),
        );
    }
    if state.browsers.contains_key(&new_label) {
        return emit_error(
            state,
            format!("relabel_browser: new label {} already registered", new_label),
        );
    }
    // Move the handle and update its duplicated `.label` field.
    let mut handle = state
        .browsers
        .remove(&old_label)
        .expect("presence checked above");
    handle.label = new_label.clone();
    state.browsers.insert(new_label.clone(), handle);
    // Re-key sibling per-label state only when an entry exists (both are
    // created lazily and are usually absent at promote time).
    if let Some(opacity) = state.window_opacities.remove(&old_label) {
        state.window_opacities.insert(new_label.clone(), opacity);
    }
    if let Some(win_state) = state.pane_window_states.remove(&old_label) {
        state.pane_window_states.insert(new_label.clone(), win_state);
    }
    tracing::info!(
        target: "pool:pane",
        old_label = %old_label,
        new_label = %new_label,
        "[relabel] browser re-keyed on pane-pool promotion"
    );
    DispatchOutput {
        relabel_ok: true,
        ..Default::default()
    }
}

pub(super) fn handle_unregister_browser(state: &mut HostState, label: String) -> DispatchOutput {
    // Atomic remove + return the Browser handle in `removed_browser`
    // (codex P2 PR #660). The pane close path in
    // `browser_panes::AppStateCloseOps::take_browser_hwnd` uses the
    // returned Browser to extract its HWND. Any caller that doesn't
    // need the handle can simply ignore `removed_browser`.
    let removed = state.browsers.remove(&label);
    let removed_browser = removed.map(|h| h.browser);
    if removed_browser.is_none() {
        return DispatchOutput::default(); // idempotent
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserUnregistered { label, version: v }],
        removed_browser,
        ..Default::default()
    }
}

