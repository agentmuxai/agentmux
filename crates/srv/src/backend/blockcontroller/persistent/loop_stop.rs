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

/// The same, for the turn of one process: the interrupt goes out only if that process is still the current one. A
/// replaced process's reader still sees frames for a moment, and `stdin_tx` already belongs to its successor.
fn interrupt_turn_of(inner: &Mutex<PersistentInner>, generation: u64) -> bool {
    let tx = {
        let g = inner.lock().unwrap();
        if g.spawn_generation != generation {
            return false;
        }
        g.stdin_tx.clone()
    };
    tx.is_some_and(|tx| tx.try_send(interrupt_control_request_line()).is_ok())
}

/// The turns the spend cap ended, by block, and when: the interrupted turn's `is_error` result is the end that was asked
/// for, not a failure, and the reader must not read it as one.
fn cap_ends() -> &'static Mutex<std::collections::HashMap<String, std::time::Instant>> {
    static ENDS: std::sync::OnceLock<Mutex<std::collections::HashMap<String, std::time::Instant>>> = std::sync::OnceLock::new();
    ENDS.get_or_init(Default::default)
}

/// How long after the interrupt its result still counts as the cap's: a turn that never answers must not make the next
/// turn's real failure look like it.
const CAP_END_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);

/// End the turn in progress for the spend cap, with the control-protocol interrupt: the process and the session stay.
/// Nothing to end (no turn in flight, or no process) is not an error.
pub(in super::super) fn interrupt_for_cap(c: &PersistentSubprocessController) -> Result<(), String> {
    if !c.health_monitor.is_active_turn() {
        return Ok(());
    }
    cap_ends().lock().unwrap().insert(c.block_id.clone(), std::time::Instant::now());
    if interrupt_turn(&c.inner) {
        return Ok(());
    }
    cap_ends().lock().unwrap().remove(&c.block_id);
    Err("the harness's input is closed or full".to_string())
}

/// Whether the result frame the reader is looking at ends a turn the spend cap interrupted. Takes the mark.
pub(in super::super) fn take_cap_end(block_id: &str) -> bool {
    cap_ends().lock().unwrap().remove(block_id).is_some_and(|at| at.elapsed() < CAP_END_WINDOW)
}

/// End the turn for `stuck`. The interrupt's `error_during_execution` result is the end we asked for, which
/// the reader must not read as a failure. `false` when the interrupt could not be sent (the process is gone or its
/// stdin is full, or `generation` is no longer the current process), in which case nobody is told: the turn did not end.
pub(in super::super) fn end_the_turn(inner: &Mutex<PersistentInner>, generation: u64, block_id: &str, mstore: Option<&Arc<Store>>, stuck: &Stuck) -> bool {
    if !interrupt_turn_of(inner, generation) {
        return false;
    }
    crate::backend::blockcontroller::loop_stop::after_ending(block_id, mstore, stuck);
    true
}
