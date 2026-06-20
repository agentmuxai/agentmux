// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane pool reducer handlers — mirrors reducer/pool.rs for the
//! `floating-pool-{uuid}` frameless window pool.

use crate::state::*;
use super::{DispatchOutput, HostState};

pub(super) fn handle_pane_pool_spawn_start(state: &mut HostState, label: String) -> DispatchOutput {
    if state.quit_state != QuitState::Running {
        return DispatchOutput {
            pane_pool_spawn_proceeding: false,
            ..Default::default()
        };
    }
    if state.pane_pool.respawn_in_flight {
        return DispatchOutput {
            pane_pool_spawn_proceeding: false,
            ..Default::default()
        };
    }
    state.pane_pool.unpromoted.insert(label);
    state.pane_pool.respawn_in_flight = true;
    DispatchOutput {
        pane_pool_spawn_proceeding: true,
        ..Default::default()
    }
}

pub(super) fn handle_pane_pool_ready(state: &mut HostState, label: String) -> DispatchOutput {
    if !state.pane_pool.unpromoted.remove(&label) {
        return DispatchOutput {
            pane_pool_size_after: Some(state.pane_pool.queue.len()),
            ..Default::default()
        };
    }
    if !state.pane_pool.queue.iter().any(|l| l == &label) {
        state.pane_pool.queue.push_back(label.clone());
    }
    state.pane_pool.respawn_in_flight = false;
    let queue_len_after = state.pane_pool.queue.len();
    DispatchOutput {
        pane_pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

pub(super) fn handle_pane_pool_destroyed_before_promote(state: &mut HostState, label: String) -> DispatchOutput {
    let was_unpromoted = state.pane_pool.unpromoted.remove(&label);
    let queue_len_before = state.pane_pool.queue.len();
    state.pane_pool.queue.retain(|l| l != &label);
    let was_in_queue = state.pane_pool.queue.len() < queue_len_before;
    state.pane_pool.respawn_in_flight = false;
    let queue_len_after = state.pane_pool.queue.len();
    DispatchOutput {
        pane_pool_destroyed_was_unpromoted: was_unpromoted || was_in_queue,
        pane_pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

pub(super) fn handle_pop_and_promote_front_pane_pool_window(state: &mut HostState) -> DispatchOutput {
    let label = match state.pane_pool.queue.pop_front() {
        Some(l) => l,
        None => return DispatchOutput::default(),
    };
    state.pane_pool.unpromoted.remove(&label);
    // Mark the BrowserHandle as no longer a pool window.
    if let Some(handle) = state.browsers.get_mut(&label) {
        if let BrowserKind::TopLevel { is_pool } = &mut handle.kind {
            *is_pool = false;
        }
    }
    let queue_len_after = state.pane_pool.queue.len();
    DispatchOutput {
        promoted_pane_pool_label: Some(label),
        pane_pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}
