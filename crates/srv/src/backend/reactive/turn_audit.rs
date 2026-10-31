// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a message did to the agent's turns, for the audit (`SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md` §7).

use super::handler::{Handler, ReactiveHandler};

/// Whether `block_id`'s agent has a turn in flight. Read just before a message is written, which flips the
/// turn on: a message into a running turn costs no new context read, one to an idle agent starts a turn.
pub(super) fn turn_in_flight(block_id: &str) -> bool {
    crate::backend::blockcontroller::get_controller(block_id).and_then(|c| c.turn_tracker()).is_some_and(|t| t.is_active_turn())
}

impl Handler {
    /// A reply that said nothing was refused before delivery (`backend::ack_policy`): an entry with outcome
    /// `ack_refused`, so the Audit counts the turns it saved.
    pub(super) fn log_ack_refused(&mut self, source_agent: &str, target_agent: &str, request_id: &str, why: &str) {
        self.log_audit(Some(source_agent), target_agent, "", "", false, Some(why), request_id, Some("ack_refused"), None);
    }
}

impl ReactiveHandler {
    /// See [`Handler::log_ack_refused`].
    pub fn log_ack_refused(&self, source_agent: &str, target_agent: &str, request_id: &str, why: &str) {
        self.inner.lock().unwrap().log_ack_refused(source_agent, target_agent, request_id, why);
    }
}
