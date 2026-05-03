// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser pane lifecycle (Phase H.1) reducer handlers. Extracted from reducer/mod.rs in
//! task #182 PR-F-2 for navigability.

use std::time::Instant;

use crate::state::*;

use super::{DispatchOutput, HostEvent, HostLifecyclePhase, HostState, RegisterResult, emit_error};

// ── H.1 — pane lifecycle ─────────────────────────────────────────────────

pub(super) fn handle_enqueue_browser_pane_create(
    state: &mut HostState,
    block_id: String,
    label: String,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        return emit_error(state, format!("enqueue_browser_pane_create: shutting down (block_id={})", block_id));
    }
    if state.browser_panes.contains_key(&block_id) {
        return emit_error(state, format!("enqueue_browser_pane_create: block_id {} already has a pane", block_id));
    }
    state.browser_panes.insert(
        block_id.clone(),
        BrowserPaneEntry {
            block_id: block_id.clone(),
            label: label.clone(),
            lifecycle: BrowserPaneLifecycle::Live,
        },
    );
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneCreateRequested { block_id, label, version: v }],
        ..Default::default()
    }
}

pub(super) fn handle_complete_browser_pane_create(state: &mut HostState, block_id: String) -> DispatchOutput {
    let entry = match state.browser_panes.get(&block_id) {
        Some(e) => e.clone(),
        None => return DispatchOutput::default(), // late callback for already-removed pane; idempotent no-op
    };
    // Already Live by EnqueueBrowserPaneCreate's invariant; this is a no-op
    // confirmation event for observers.
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneLive { block_id, label: entry.label, version: v }],
        ..Default::default()
    }
}

pub(super) fn handle_enqueue_browser_pane_close(state: &mut HostState, block_id: String) -> DispatchOutput {
    let entry = match state.browser_panes.get_mut(&block_id) {
        Some(e) => e,
        None => return DispatchOutput::default(), // close request for already-gone pane; idempotent
    };
    if matches!(entry.lifecycle, BrowserPaneLifecycle::Closing { .. }) {
        return DispatchOutput::default(); // already Closing; idempotent
    }
    entry.lifecycle = BrowserPaneLifecycle::Closing { since: Instant::now() };
    let label = entry.label.clone();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneClosing { block_id, version: v }],
        closed_browser_pane_label: Some(label),
        ..Default::default()
    }
}

/// PR #5 — sole pane registration entry point. Replaces
/// `pane::lifecycle::PaneStateMachine::try_register_live`.
///
/// - Live entry exists → `AlreadyLive(label)`
/// - Closing entry exists → `Closing`
/// - No entry → generate label, insert Live, `Fresh(label)` + emit
///   `BrowserPaneCreateRequested`
pub(super) fn handle_try_register_browser_pane_live(state: &mut HostState, block_id: String) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        return emit_error(
            state,
            format!("try_register_browser_pane_live: shutting down (block_id={})", block_id),
        );
    }
    if let Some(entry) = state.browser_panes.get(&block_id) {
        let result = match entry.lifecycle {
            BrowserPaneLifecycle::Live => RegisterResult::AlreadyLive(entry.label.clone()),
            BrowserPaneLifecycle::Closing { .. } => RegisterResult::Closing,
        };
        return DispatchOutput {
            browser_pane_register_result: Some(result),
            ..Default::default()
        };
    }
    let label = super::next_browser_pane_label(&block_id);
    state.browser_panes.insert(
        block_id.clone(),
        BrowserPaneEntry {
            block_id: block_id.clone(),
            label: label.clone(),
            lifecycle: BrowserPaneLifecycle::Live,
        },
    );
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneCreateRequested {
            block_id,
            label: label.clone(),
            version: v,
        }],
        browser_pane_register_result: Some(RegisterResult::Fresh(label)),
        ..Default::default()
    }
}

/// PR #5 — sole label-keyed drain entry point. Replaces
/// `pane::lifecycle::PaneStateMachine::drain_by_label`.
///
/// Removes whichever pane entry has `label`. Returns the drained block_id
/// in `drained_browser_pane_block_id`. Idempotent — `None` if no entry has that label
/// (e.g., explicit `close()` already cleared it; `on_before_close` arrives
/// later).
pub(super) fn handle_drain_browser_pane_by_label(state: &mut HostState, label: String) -> DispatchOutput {
    let victim = state
        .browser_panes
        .iter()
        .find(|(_, e)| e.label == label)
        .map(|(k, _)| k.clone());
    let block_id = match victim {
        Some(b) => b,
        None => return DispatchOutput::default(),
    };
    state.browser_panes.remove(&block_id);
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneClosed {
            block_id: block_id.clone(),
            version: v,
        }],
        drained_browser_pane_block_id: Some(block_id),
        ..Default::default()
    }
}

pub(super) fn handle_complete_browser_pane_close(state: &mut HostState, block_id: String) -> DispatchOutput {
    if state.browser_panes.remove(&block_id).is_none() {
        return DispatchOutput::default(); // idempotent
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneClosed { block_id, version: v }],
        ..Default::default()
    }
}

pub(super) fn handle_abort_browser_pane_create(
    state: &mut HostState,
    block_id: String,
    reason: String,
) -> DispatchOutput {
    state.browser_panes.remove(&block_id);
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneCreationFailed { block_id, reason, version: v }],
        ..Default::default()
    }
}

