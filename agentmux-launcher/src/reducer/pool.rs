// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pool-related reducer handlers. Extracted from reducer/mod.rs in
//! task #182 PR-B for navigability.

use agentmux_common::ipc::{DriftKind, Event};

use crate::state::State;

/// Phase B.4 follow-up — pool-only drift check. Called from
/// `spawn_pool_window` where the windows dimension is mid-flight
/// (close path hasn't completed). Compares only the pool dimension;
/// emits `DriftDetected { kind: Pool, ... }` on mismatch.
pub(super) fn handle_report_host_pool_count(state: &mut State, host_pool: u32) -> Vec<Event> {
    let mirror_pool = state.pool.len() as u32;
    if mirror_pool == host_pool {
        return vec![];
    }
    let v = state.bump_version();
    vec![Event::DriftDetected {
        kind: DriftKind::Pool,
        host_count: host_pool,
        mirror_count: mirror_pool,
        version: v,
    }]
}

/// Phase B.4 follow-up — record pool inventory growth. Idempotent
/// on duplicate labels (HashSet semantics) but the event still fires
/// so subscribers can track add-attempts even if redundant.
pub(super) fn handle_report_pool_window_added(
    state: &mut State,
    label: String,
    saga_id: Option<u64>,
) -> Vec<Event> {
    state.pool.insert(label.clone());
    let v = state.bump_version();
    vec![Event::PoolWindowAdded {
        label,
        version: v,
        saga_id,
    }]
}

/// Phase B.4 follow-up — record pool inventory shrink (promote or
/// destroy). Strictly paired with `ReportPoolWindowAdded`: an
/// unknown-label remove is a silent no-op so subscribers can rely
/// on add/remove pairing in the broadcast stream. Same gate as
/// `handle_report_window_closed`. (reagent P2 PR #577 round-3 —
/// the original "idempotent" comment referenced behavior that was
/// already removed for `ReportWindowClosed`; pool semantics now
/// match.)
pub(super) fn handle_report_pool_window_removed(state: &mut State, label: String) -> Vec<Event> {
    let was_present = state.pool.remove(&label);
    if !was_present {
        return vec![];
    }
    let v = state.bump_version();
    vec![Event::PoolWindowRemoved { label, version: v }]
}

/// Phase F.5 — host-emitted promote signal. The reducer doesn't mutate
/// state for this command (the windows/pool transitions are carried by
/// the surrounding `ReportPoolWindowRemoved` + `ReportWindowOpened`
/// pair); it just translates the wire command into the corresponding
/// typed event so subscribers — most importantly the launcher saga
/// coordinator — can react.
///
/// Idempotent / context-free: we don't validate the label is in the
/// mirror because the host's own ordering may have the
/// `ReportPoolWindowRemoved` arrive before this command, after this
/// command, or in either order; the typed event is "host says a
/// promote happened" — subscribers correlate with the surrounding
/// add/remove pair if they need stronger invariants.
///
/// **WRR side-effect:** records the label in `state.just_promoted_labels`
/// so the subsequent `ReportWindowOpened` initializes the new mirror
/// with `foregrounded_since_open: true`. Promote is the user explicitly
/// tearing off a tab — the open-transient corrective logic in
/// `apply_hwnd_visibility_changed` MUST stop firing for this label,
/// otherwise the post-promote reposition (multiple SetWindowPos calls
/// during HWND placement) re-fires `HiddenSinceOpen` indefinitely.
/// Each fire is a launcher event broadcast to all renderers; without
/// this guard the host fans the same drift event out across the
/// bridge until the renderer's V8 isolate runs out of stack and
/// crashes (`Crashpad_NotConnectedToHandler`, observed v0.33.655).
///
/// The actual host emit order is
/// `ReportPoolWindowRemoved → ReportPoolWindowPromoted →
/// ReportWindowOpened` (`agentmux-cef/src/commands/window_pool.rs`).
/// At promote-time the launcher has NO mirror for this label —
/// `state.pool.contains(label)` is also false (removed by the
/// preceding `ReportPoolWindowRemoved`), so we can't gate purely on
/// pool membership in `handle_report_window_opened` either. The
/// `just_promoted_labels` set bridges the microsecond gap.
///
/// See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`.
pub(super) fn handle_report_pool_window_promoted(state: &mut State, label: String) -> Vec<Event> {
    state.just_promoted_labels.insert(label.clone());
    let v = state.bump_version();
    vec![Event::PoolWindowPromoted { label, version: v }]
}

/// Phase F.6 — host-emitted signal carrying the result of the
/// post-close drain-pool-if-last decision. Maps `was_last` directly
/// to the corresponding terminal event for Step 2 of the
/// window-cleanup-cascade saga:
/// * `true` → `Event::PoolDrained` (last user-visible window
///   closed; warm-pool drain initiated)
/// * `false` → `Event::PoolNotLast` (other windows remain; pool
///   stays warm)
///
/// Pure pass-through (same reasoning as `handle_report_panes_reaped`).
pub(super) fn handle_report_pool_drain_decision(
    state: &mut State,
    label: String,
    was_last: bool,
    saga_id: Option<u64>,
) -> Vec<Event> {
    // Same rationale as handle_report_panes_reaped: round 4's gate
    // had an ordering bug; round 5 reverts to emit-unconditionally.
    //
    // CPD-1: `saga_id` flows through unchanged.
    let v = state.bump_version();
    if was_last {
        vec![Event::PoolDrained {
            label,
            version: v,
            saga_id,
        }]
    } else {
        vec![Event::PoolNotLast {
            label,
            version: v,
            saga_id,
        }]
    }
}

