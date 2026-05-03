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
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserRegistered { label, kind, version: v }],
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

