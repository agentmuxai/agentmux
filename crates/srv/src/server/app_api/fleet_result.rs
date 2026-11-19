// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a broadcast reports per target (`fleet::fleet_broadcast_impl`): delivered, kept for an agent
//! out of quota (`person_hold`), or failed. Each is recorded in the audit log as it happened.

use crate::backend::rpc_types::{FleetActionFailure, FleetActionHeld, FleetActionResult};

use super::fleet::{BroadcastOutcome, FLEET_BROADCAST_AUDIT_ACTION};
use super::AppState;

/// Sorts the outcomes of a broadcast into a [`FleetActionResult`]. `is_local` says whether a block is
/// an agent of this instance, whose history gets what the person typed.
pub(super) fn settle_broadcast(
    state: &AppState,
    outcomes: Vec<BroadcastOutcome>,
    is_local: impl Fn(&str) -> bool,
    msg_id: &str,
    message: &str,
) -> FleetActionResult {
    let mut result = FleetActionResult::default();
    for outcome in outcomes {
        let target_agent = outcome.agent_id.as_deref().unwrap_or(&outcome.block_id);
        let audit = |success: bool, error: Option<&str>, note: Option<&str>| {
            state.reactive_handler.log_fleet_action_audit(None, target_agent, &outcome.block_id, FLEET_BROADCAST_AUDIT_ACTION, success, error, msg_id, note);
        };
        match &outcome.result {
            Ok(()) => {
                audit(true, None, None);
                // What the user typed, in each recipient of this instance's history.
                if is_local(&outcome.block_id) {
                    super::input_history::record_broadcast(state, &outcome.block_id, message);
                }
                result.succeeded.push(outcome.block_id);
            }
            // Kept for an agent out of quota, with its check started: not a failure.
            Err(error) if crate::server::person_hold::held_note(error).is_some() => {
                audit(true, None, Some("held for an agent out of quota"));
                let note = crate::server::person_hold::held_note(error).unwrap_or_default().to_string();
                result.held.push(FleetActionHeld { id: outcome.block_id, note });
            }
            Err(error) => {
                audit(false, Some(error), None);
                result.failed.push(FleetActionFailure { id: outcome.block_id, error: error.clone() });
            }
        }
    }
    result
}
