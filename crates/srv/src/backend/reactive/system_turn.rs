// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A turn started by AgentMux itself (a note that asks the agent for something, such as its status
//! after a sign-in), by the same sender a message takes. Split out of `handler.rs` (over the file-size
//! baseline) the way `sensitive_audit.rs` is.

use super::handler::ReactiveHandler;
use super::types::SenderDelivery;

impl ReactiveHandler {
    /// `text` as a turn for the agent in `block_id`, from AgentMux itself: a running persistent agent has it on
    /// its stdin, a one-shot or not-yet-started one gets a turn started. Not a jekt: no marker, no hold, no
    /// audit row. The sender is cloned out of the lock first, since starting a turn can wait for a process.
    pub fn send_system_turn(&self, block_id: &str, text: &str) -> Result<SenderDelivery, String> {
        let sender = self.inner.lock().unwrap().message_sender.clone().ok_or_else(|| "no message sender is installed".to_string())?;
        sender(block_id, text, &[], &crate::backend::message_origin::MessageOrigin::system(), None)
    }
}
