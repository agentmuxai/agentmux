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

