// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's own notes about the work at hand (`note_into_turn_or_hold`): into
//! the running turn, or held for the next one, never a turn of their own.

use super::super::*;
use crate::backend::reactive::held_notes;

fn live_controller(block_id: &str) -> (Arc<PersistentSubprocessController>, mpsc::Receiver<String>) {
    let c = Arc::new(PersistentSubprocessController::new("tab".to_string(), block_id.to_string(), None, None, None, None));
    let (tx, rx) = mpsc::channel::<String>(8);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    (c, rx)
}

fn take_held(block_id: &str) -> String {
    let (message, notes) = held_notes::attach(block_id, "");
    notes.delivered();
    message
}

#[test]
fn a_note_joins_the_running_turn() {
    let (c, mut rx) = live_controller("block-note-joins");
    c.health_monitor.set_active_turn(true);
    assert!(c.note_into_turn_or_hold("k", "[AgentMux] overlap"));
    assert!(rx.try_recv().unwrap().contains("[AgentMux] overlap"));
    assert_eq!(take_held("block-note-joins"), "", "not held as well");
}

#[test]
fn with_no_turn_running_a_note_is_held_and_nothing_is_written() {
    let (c, mut rx) = live_controller("block-note-idle");
    assert!(!c.note_into_turn_or_hold("k", "[AgentMux] overlap"));
    assert!(rx.try_recv().is_err(), "a write would start a turn");
    assert!(!c.health_monitor.is_active_turn());
    assert_eq!(take_held("block-note-idle"), "[AgentMux] overlap");
}

/// The race: the note's check and the turn's end land together. The turn-end
/// handler holds `inner` while it idles the turn, so the note waits for it,
/// then finds no turn and is held: it can't write into a turn that just
/// ended, which would start a new one.
#[test]
fn a_turn_ending_while_the_note_is_on_its_way_holds_the_note() {
    let block = "block-note-race";
    let (c, mut rx) = live_controller(block);
    c.health_monitor.set_active_turn(true);
    let generation = c.inner.lock().unwrap().spawn_generation;
    let mut ending = c.inner.lock().unwrap();
    let note = {
        let c = c.clone();
        std::thread::spawn(move || c.note_into_turn_or_hold("k", "[AgentMux] overlap"))
    };
    // Give the note time to reach the lock the turn-end handler holds.
    std::thread::sleep(std::time::Duration::from_millis(50));
    PersistentSubprocessController::turn_boundary_locked(&mut ending, &c.health_monitor, block, generation, None);
    drop(ending);
    assert!(!note.join().unwrap(), "held, not written");
    assert!(rx.try_recv().is_err(), "no new turn");
    assert_eq!(take_held(block), "[AgentMux] overlap");
}

#[test]
fn a_process_that_cannot_take_a_write_gets_the_note_held() {
    let (c, mut rx) = live_controller("block-note-restarting");
    c.health_monitor.set_active_turn(true);
    c.inner.lock().unwrap().restart_pending = true;
    assert!(!c.note_into_turn_or_hold("k", "[AgentMux] overlap"));
    assert!(rx.try_recv().is_err());
    assert_eq!(take_held("block-note-restarting"), "[AgentMux] overlap");
}
