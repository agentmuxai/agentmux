// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! What the stdout reader does when the loop guard (`agents::loop_guard`) says the agent is stuck: end
//! the turn with the control-protocol interrupt, then hold a note and tell the leader
//! (`blockcontroller/loop_stop.rs`). A brake, not a stop: the process and the session stay, and the next
//! message goes on from there.

use super::*;
use crate::agents::loop_guard::Stuck;

/// Ask the harness to end its turn (the control-protocol interrupt). `false` when its stdin is gone or full.
pub(super) fn interrupt_turn(inner: &Mutex<PersistentInner>) -> bool {
    let tx = inner.lock().unwrap().stdin_tx.clone();
    tx.is_some_and(|tx| tx.try_send(interrupt_control_request_line()).is_ok())
}

/// End the turn for `stuck`. The interrupt's `error_during_execution` result is the end we asked for, which
/// the reader must not read as a failure. `false` when the interrupt could not be sent (the process is gone or its
/// stdin is full), in which case nobody is told: the turn did not end.
pub(super) fn end_the_turn(inner: &Mutex<PersistentInner>, block_id: &str, mstore: Option<&Arc<Store>>, stuck: &Stuck) -> bool {
    if !interrupt_turn(inner) {
        return false;
    }
    crate::backend::blockcontroller::loop_stop::after_ending(block_id, mstore, stuck);
    true
}
