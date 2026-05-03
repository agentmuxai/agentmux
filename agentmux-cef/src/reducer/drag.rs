// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drag state (Phase H.3) reducer handlers. Extracted from reducer/mod.rs in
//! task #182 PR-F-2 for navigability.


use crate::state::*;

use super::{DispatchOutput, DragOutcome, HostEvent, HostState, emit_error};

// ── H.3 — drag state ─────────────────────────────────────────────────────

pub(super) fn handle_start_drag(state: &mut HostState, session: DragSession) -> DispatchOutput {
    if state.active_drag.is_some() {
        return emit_error(state, "start_drag: drag session already active".to_string());
    }
    let drag_id = session.drag_id.clone();
    let source_window = session.source_window.clone();
    state.active_drag = Some(session);
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::DragStarted { drag_id, source_window, version: v }],
        ..Default::default()
    }
}

pub(super) fn handle_end_drag(
    state: &mut HostState,
    drag_id: String,
    outcome: DragOutcome,
) -> DispatchOutput {
    let active_id = state.active_drag.as_ref().map(|s| s.drag_id.clone());
    match active_id {
        Some(id) if id == drag_id => {
            // PR #5 H.3 — return the prior session via output so callers
            // (cross-drag complete / cancel) can build the renderer-side
            // event payload without a separate read of state.active_drag.
            let session = state.active_drag.take();
            let v = state.bump_version();
            DispatchOutput {
                events: vec![HostEvent::DragEnded { drag_id, outcome, version: v }],
                ended_drag_session: session,
                ..Default::default()
            }
        }
        _ => DispatchOutput::default(), // mismatched or no drag; idempotent no-op
    }
}

