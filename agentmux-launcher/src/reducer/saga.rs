// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Saga-related reducer handlers. Extracted from reducer/mod.rs
//! in task #182 PR-D for navigability.
//!
//! Both handlers are pure pass-through: state is untouched, the
//! reducer just translates the wire command into the typed event
//! so saga subscribers (in the saga coordinator's bus loop) can react.

use agentmux_common::ipc::Event;

use crate::state::State;

/// Phase F.6 — host-emitted signal that browser-pane HWNDs for a
/// closing top-level window have been reaped. Pure pass-through:
/// state stays untouched (the host owns pane bookkeeping); the
/// reducer just translates the wire command into the typed event so
/// the window-cleanup-cascade saga can advance.
///
/// Idempotent / context-free: the saga matches the `label` against
/// its own `closed_label`, so a stray report for a label that no
/// in-flight saga is tracking is a harmless broadcast.
pub(super) fn handle_report_panes_reaped(
    state: &mut State,
    label: String,
    saga_id: Option<u64>,
) -> Vec<Event> {
    // No state.windows gate — round 4's gate had an ordering bug:
    // host sends ReportWindowClosed BEFORE ReportPanesReaped on the
    // same channel, so by the time the reducer processes this, the
    // label is already gone from state.windows (closed by the prior
    // command's reducer arm). The gate then dropped EVERY
    // PanesReaped, leaving the F.6 saga stuck in WaitingForPanesReaped
    // indefinitely. Round 5 reversal: emit unconditionally; for
    // unpromoted-pool drains where no saga is in flight, the event
    // appears stray on the bus but is harmless (no subscriber acts
    // on it). Cosmetic only; correct saga lifecycle restored.
    //
    // CPD-1: `saga_id` flows through unchanged (None for organic
    // reports, Some(N) once CPD-3 hosts echo back the saga's id).
    let v = state.bump_version();
    vec![Event::PanesReaped {
        label,
        version: v,
        saga_id,
    }]
}

/// Phase CPD-1 — host reported a saga-issued action failed. Pure
/// pass-through translation into `Event::SagaActionFailed`. The
/// saga coordinator's bus loop will (CPD-3) treat the event as a
/// terminal signal for the matching `saga_id` and emit
/// `Event::SagaFailed`, dropping the saga from in-flight.
pub(super) fn handle_report_saga_action_failed(
    state: &mut State,
    saga_id: u64,
    reason: String,
) -> Vec<Event> {
    let v = state.bump_version();
    vec![Event::SagaActionFailed {
        saga_id,
        reason,
        version: v,
    }]
}
