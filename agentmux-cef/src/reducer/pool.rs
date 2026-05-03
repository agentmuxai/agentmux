// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pool state (Phase H.4) reducer handlers. Extracted from reducer/mod.rs in
//! task #182 PR-F-2 for navigability.

use std::time::Instant;

use crate::state::*;

use super::{DispatchOutput, HostEvent, HostLifecyclePhase, HostState, PoolLeaveReason, RegisterResult, emit_error};

// ── H.4 — pool state ─────────────────────────────────────────────────────

pub(super) fn handle_pool_spawn_start(state: &mut HostState, label: String) -> DispatchOutput {
    // PR #5 H.4 — single-flight semaphore. The legacy
    // `window_pool_respawn_in_flight.swap(true)` returns the prior
    // value; if true, caller skips. We replicate that here: prior
    // in-flight OR non-Running quit state both suppress the spawn.
    if state.quit_state != QuitState::Running {
        return DispatchOutput {
            pool_spawn_proceeding: false,
            ..Default::default()
        };
    }
    if state.pool.respawn_in_flight {
        return DispatchOutput {
            pool_spawn_proceeding: false,
            ..Default::default()
        };
    }
    state.pool.unpromoted.insert(label);
    state.pool.respawn_in_flight = true;
    DispatchOutput {
        pool_spawn_proceeding: true,
        ..Default::default()
    }
}

pub(super) fn handle_pool_ready(state: &mut HostState, label: String) -> DispatchOutput {
    if !state.pool.unpromoted.remove(&label) {
        // Not in unpromoted (race or duplicate signal); idempotent.
        return DispatchOutput {
            pool_size_after: Some(state.pool.queue.len()),
            ..Default::default()
        };
    }
    if !state.pool.queue.iter().any(|l| l == &label) {
        state.pool.queue.push_back(label.clone());
    }
    state.pool.respawn_in_flight = false;
    let queue_len_after = state.pool.queue.len();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowEntered {
            label,
            queue_len_after,
            version: v,
        }],
        pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

pub(super) fn handle_pool_destroyed_before_promote(state: &mut HostState, label: String) -> DispatchOutput {
    // Pool windows can be destroyed in two states (codex P1 PR #654 round 2):
    //   1. Still in `unpromoted` — never reached renderer-ready.
    //   2. Already in `queue` — passed renderer-ready, awaiting promotion,
    //      then closed externally before promote.
    // Both must be cleaned up; otherwise the queue retains a dead label
    // and a later `PromotePoolWindow` operates on stale inventory.
    let was_unpromoted = state.pool.unpromoted.remove(&label);
    let queue_len_before = state.pool.queue.len();
    state.pool.queue.retain(|l| l != &label);
    let was_in_queue = state.pool.queue.len() < queue_len_before;
    state.pool.respawn_in_flight = false;
    let queue_len_after = state.pool.queue.len();
    if !was_unpromoted && !was_in_queue {
        return DispatchOutput {
            pool_size_after: Some(queue_len_after),
            ..Default::default()
        };
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowLeft {
            label,
            queue_len_after,
            reason: PoolLeaveReason::DestroyedBeforePromote,
            version: v,
        }],
        pool_destroyed_was_unpromoted: was_unpromoted,
        pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

pub(super) fn handle_pop_and_promote_front_pool_window(state: &mut HostState) -> DispatchOutput {
    let label = match state.pool.queue.pop_front() {
        Some(l) => l,
        None => return DispatchOutput::default(),
    };
    state.pool.unpromoted.remove(&label);
    if let Some(handle) = state.browsers.get_mut(&label) {
        if let BrowserKind::TopLevel { is_pool } = &mut handle.kind {
            *is_pool = false;
        }
    }
    let queue_len_after = state.pool.queue.len();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowLeft {
            label: label.clone(),
            queue_len_after,
            reason: PoolLeaveReason::Promoted,
            version: v,
        }],
        promoted_pool_label: Some(label),
        pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

pub(super) fn handle_promote_pool_window(state: &mut HostState, label: String) -> DispatchOutput {
    // Idempotent no-op for truly unknown labels (reagent P2 PR #654 round 3).
    // Symmetric with `handle_pool_destroyed_before_promote`'s pattern: only
    // emit `PoolWindowLeft` if we actually removed the label from one of
    // the pool sets. Without this, a stale promote command (e.g., from a
    // race between PromotePoolWindow and PoolWindowDestroyedBeforePromote)
    // would emit a phantom `PoolWindowLeft` event that observers might act on.
    let queue_len_before = state.pool.queue.len();
    state.pool.queue.retain(|l| l != &label);
    let was_in_queue = state.pool.queue.len() < queue_len_before;
    let was_in_unpromoted = state.pool.unpromoted.remove(&label);
    if !was_in_queue && !was_in_unpromoted {
        return DispatchOutput::default();
    }
    // Mark the corresponding browser handle as no-longer-pool.
    if let Some(handle) = state.browsers.get_mut(&label) {
        if let BrowserKind::TopLevel { is_pool } = &mut handle.kind {
            *is_pool = false;
        }
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowLeft {
            label,
            queue_len_after: state.pool.queue.len(),
            reason: PoolLeaveReason::Promoted,
            version: v,
        }],
        ..Default::default()
    }
}

pub(super) fn handle_pool_drain_all(state: &mut HostState) -> DispatchOutput {
    let drained: Vec<String> = state
        .pool
        .queue
        .drain(..)
        .chain(state.pool.unpromoted.drain())
        .collect();
    state.pool.respawn_in_flight = false;
    let mut events = Vec::new();
    for label in drained {
        let v = state.bump_version();
        events.push(HostEvent::PoolWindowLeft {
            label,
            queue_len_after: 0,
            reason: PoolLeaveReason::DrainedOnShutdown,
            version: v,
        });
    }
    let v = state.bump_version();
    events.push(HostEvent::PoolEmpty { version: v });
    DispatchOutput {
        events,
        ..Default::default()
    }
}

