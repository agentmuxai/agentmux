// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The user's Stop on an agent's turn, and the grace window after it. The
//! turn end a Stop causes is the user's own action: the notification router
//! sends no "finished" for it, and the Swarm Supervisor neither judges nor
//! continues it (SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.4). Recorded by
//! `notify.emit`'s `TurnStopped` (`server/notify_handlers.rs`) at once, for the
//! Supervisor, and again by the notification router when it reaches that event
//! in its queue, so a turn start queued before the Stop can't erase it.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// How long after a user Stop the turn end it causes counts as the user's own
/// action.
pub const STOP_GRACE_MS: i64 = 15_000;

/// block_id → when the user last stopped its turn (epoch ms).
#[derive(Default)]
struct Stops(HashMap<String, i64>);

impl Stops {
    fn note(&mut self, block_id: &str, now_ms: i64) {
        // Bounded: an entry past the grace window can't matter any more.
        self.0.retain(|_, at| now_ms - *at < STOP_GRACE_MS);
        self.0.insert(block_id.to_string(), now_ms);
    }

    fn recent(&self, block_id: &str, now_ms: i64) -> bool {
        self.0.get(block_id).is_some_and(|at| now_ms - *at < STOP_GRACE_MS)
    }

    fn forget(&mut self, block_id: &str) {
        self.0.remove(block_id);
    }
}

fn stops() -> MutexGuard<'static, Stops> {
    static STOPS: OnceLock<Mutex<Stops>> = OnceLock::new();
    STOPS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The user pressed Stop on this block's turn.
pub fn note_user_stop(block_id: &str) {
    stops().note(block_id, agentmux_common::time::now_ms());
}

/// Whether the user stopped this block's turn within the grace window.
pub fn stopped_recently(block_id: &str) -> bool {
    stops().recent(block_id, agentmux_common::time::now_ms())
}

/// A new turn started on this block: an earlier Stop no longer applies to it.
pub fn forget(block_id: &str) {
    stops().forget(block_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_stop_holds_the_block_for_the_grace_window_only() {
        let mut stops = Stops::default();
        stops.note("b1", 1_000_000);
        assert!(stops.recent("b1", 1_000_000 + STOP_GRACE_MS - 1));
        assert!(!stops.recent("b1", 1_000_000 + STOP_GRACE_MS));
        assert!(!stops.recent("b2", 1_000_000));
    }

    #[test]
    fn a_new_turn_forgets_the_stop() {
        let mut stops = Stops::default();
        stops.note("b1", 1_000_000);
        stops.forget("b1");
        assert!(!stops.recent("b1", 1_000_001));
    }

    /// The order the router sees: `notify.emit` records the Stop, a turn start
    /// queued before it is processed (and forgets it), then the queued Stop
    /// records it again. The turn end that follows is still the user's.
    #[test]
    fn a_turn_start_queued_before_the_stop_does_not_erase_it() {
        let mut stops = Stops::default();
        stops.note("b1", 1_000_000);
        stops.forget("b1");
        stops.note("b1", 1_000_005);
        assert!(stops.recent("b1", 1_000_010));
    }

    #[test]
    fn stops_past_the_window_are_dropped_when_a_new_one_is_noted() {
        let mut stops = Stops::default();
        stops.note("old", 1_000_000);
        stops.note("new", 1_000_000 + STOP_GRACE_MS);
        assert!(!stops.0.contains_key("old"));
        assert!(stops.0.contains_key("new"));
    }

    #[test]
    fn the_shared_record_is_seen_by_every_reader() {
        let block = "user-stop-shared-record-test-block";
        note_user_stop(block);
        assert!(stopped_recently(block));
        forget(block);
        assert!(!stopped_recently(block));
    }
}
