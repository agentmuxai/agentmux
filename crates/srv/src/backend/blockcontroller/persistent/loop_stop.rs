// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! What the stdout reader does when the loop guard (`agents::loop_guard`) says the agent is stuck: end
//! the turn, hold a note for the agent's next turn, and tell its leader
//! (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 5). A brake, not a stop: the process and
//! the session stay, and the next message goes on from there.

use super::*;
use crate::agents::loop_guard::Stuck;

/// The key the agent's note is held under; a second one replaces the first.
const NOTE_KEY: &str = "loop-guard";

/// Ask the harness to end its turn (the control-protocol interrupt). `false` when its stdin is gone or full.
pub(super) fn interrupt_turn(inner: &Mutex<PersistentInner>) -> bool {
    let tx = inner.lock().unwrap().stdin_tx.clone();
    tx.is_some_and(|tx| tx.try_send(interrupt_control_request_line()).is_ok())
}

/// The note the agent finds at the start of its next turn.
pub(super) fn agent_note(stuck: &Stuck) -> String {
    let what = crate::backend::reactive::sanitize::quote_in_system_note(&stuck.describe());
    format!("[AgentMux] Your last turn was ended: {what}. Do not make that call again as it was; read what it answered, change what you pass, or ask your leader.")
}

/// The note to the agent's leader, if it has one on this machine with a pane open.
pub(super) fn tell_leader(mstore: &Arc<Store>, block_id: &str, stuck: &Stuck) {
    let Some(agent) = crate::identity::resolver::block_agent_id(mstore, block_id) else { return };
    let name = mstore.agent_def_get(&agent).ok().flatten().map(|d| d.name).unwrap_or_else(|| agent.clone());
    let body = format!("{name}'s turn was ended: {}. It can be sent a new message and goes on.", stuck.describe());
    if !crate::backend::worker_notes::tell_leader(mstore, &agent, &body) {
        tracing::info!(agent = %agent, "loop guard: no leader to tell that the turn was ended");
    }
}

/// End the turn for `stuck`. The interrupt's `error_during_execution` result is the end we asked for, which
/// the reader must not read as a failure. `false` when the interrupt could not be sent (the process is gone or its
/// stdin is full), in which case nobody is told: the turn did not end.
pub(super) fn end_the_turn(inner: &Mutex<PersistentInner>, block_id: &str, mstore: Option<&Arc<Store>>, stuck: &Stuck) -> bool {
    if !interrupt_turn(inner) {
        return false;
    }
    tracing::warn!(block_id = %block_id, what = %stuck.describe(), "loop guard: ending the turn");
    crate::backend::reactive::held_notes::hold(block_id, NOTE_KEY, &agent_note(stuck));
    if let Some(mstore) = mstore.cloned() {
        let (block_id, stuck) = (block_id.to_string(), stuck.clone());
        // Reads the store and takes the handler's lock: not on the reader's task.
        tokio::task::spawn_blocking(move || tell_leader(&mstore, &block_id, &stuck));
    }
    true
}
