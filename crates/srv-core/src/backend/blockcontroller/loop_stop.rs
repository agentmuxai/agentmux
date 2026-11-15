// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! What follows a turn the loop guard (`agents::loop_guard`) ended, the same for every controller: a note held for
//! the agent's next turn, and one to its leader (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 5). How the
//! turn is ended is the controller's own: Claude's control-protocol interrupt (`persistent/loop_stop.rs`), ACP's
//! `session/cancel` (`acp_spawn.rs`).

use crate::agents::loop_guard::Stuck;
use crate::backend::storage::store::Store;
use std::sync::Arc;

/// The key the agent's note is held under; a second one replaces the first.
const NOTE_KEY: &str = "loop-guard";

/// The note the agent finds at the start of its next turn.
pub(crate) fn agent_note(stuck: &Stuck) -> String {
    let what = crate::backend::reactive::sanitize::quote_in_system_note(&stuck.describe());
    format!("[AgentMux] Your last turn was ended: {what}. Do not make that call again as it was; read what it answered, change what you pass, or ask your leader.")
}

/// The note to the agent's leader, if it has one on this machine with a pane open.
fn tell_leader(mstore: &Arc<Store>, block_id: &str, stuck: &Stuck) {
    let Some(agent) = crate::identity::resolver::block_agent_id(mstore, block_id) else { return };
    let name = mstore.agent_def_get(&agent).ok().flatten().map(|d| d.name).unwrap_or_else(|| agent.clone());
    let body = format!("{name}'s turn was ended: {}. It can be sent a new message and goes on.", stuck.describe());
    if !crate::backend::worker_notes::tell_leader(mstore, &agent, &body) {
        tracing::info!(agent = %agent, "loop guard: no leader to tell that the turn was ended");
    }
}

/// The turn of `block_id` was ended for `stuck`: hold the agent's note and tell its leader.
pub(crate) fn after_ending(block_id: &str, mstore: Option<&Arc<Store>>, stuck: &Stuck) {
    tracing::warn!(block_id = %block_id, what = %stuck.describe(), "loop guard: ending the turn");
    crate::backend::reactive::held_notes::hold(block_id, NOTE_KEY, &agent_note(stuck));
    if let Some(mstore) = mstore.cloned() {
        let (block_id, stuck) = (block_id.to_string(), stuck.clone());
        // Reads the store and takes the handler's lock: not on the reader's task.
        tokio::task::spawn_blocking(move || tell_leader(&mstore, &block_id, &stuck));
    }
}
