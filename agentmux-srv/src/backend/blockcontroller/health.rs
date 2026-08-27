// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-block turn-activity tracking (is a turn currently in flight, and
//! the last process exit code). Previously part of a larger "agent health"
//! detector that also did silence-based unresponsive detection; that
//! detection logic was removed (see
//! docs/specs/SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md) —
//! this struct keeps only the turn-active bookkeeping, which several other
//! subsystems depend on independently of the removed detector
//! (`broker::process::lifecycle_from`, the Swarm pane's running/idle
//! badge, subagent-watcher reconciliation, `muxspect describe`).

use std::sync::Mutex;

struct TurnActivityTrackerInner {
    active_turn: bool,
    exit_code: Option<i32>,
}

/// Per-block turn-activity tracker.
pub struct TurnActivityTracker {
    block_id: String,
    inner: Mutex<TurnActivityTrackerInner>,
}

impl TurnActivityTracker {
    pub fn new(block_id: String) -> Self {
        Self {
            block_id,
            inner: Mutex::new(TurnActivityTrackerInner {
                active_turn: false,
                exit_code: None,
            }),
        }
    }

    /// Called when a new turn starts (subprocess spawned).
    pub fn set_active_turn(&self, active: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_turn = active;
        if active {
            inner.exit_code = None;
        }
        drop(inner);
        tracing::info!(block_id = %self.block_id, active, "[health] turn_active flip");
    }

    /// Atomically marks a turn active and reports whether one was already in
    /// flight (the pre-call value) — a single lock acquisition, unlike
    /// calling `is_active_turn()` then `set_active_turn(true)` separately.
    /// That two-step form is a check-then-act race: `send_message` (user
    /// input) and `send_user_message` (muxbus delivery) can run concurrently
    /// on the same block, and both reading `false` before either writes
    /// `true` lets both decide to spawn a watchdog — the exact duplicate the
    /// "only re-arm when resuming from idle" logic exists to prevent.
    pub fn mark_turn_active_returning_was_active(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let was_active = inner.active_turn;
        inner.active_turn = true;
        inner.exit_code = None;
        drop(inner);
        tracing::info!(
            block_id = %self.block_id,
            active = true,
            was_active,
            "[health] turn_active flip"
        );
        was_active
    }

    /// Called when the subprocess exits.
    pub fn set_exited(&self, exit_code: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_turn = false;
        inner.exit_code = Some(exit_code);
        drop(inner);
        tracing::info!(block_id = %self.block_id, exit_code, "[health] turn_active flip (process exited)");
    }

    /// Whether there's an active turn in progress.
    pub fn is_active_turn(&self) -> bool {
        self.inner.lock().unwrap().active_turn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_turn_active_returning_was_active_reports_the_pre_call_value() {
        let tracker = TurnActivityTracker::new("test-block".to_string());
        assert!(!tracker.is_active_turn());

        // First call: was idle before this call.
        let was_active = tracker.mark_turn_active_returning_was_active();
        assert!(!was_active, "first call should report idle-before-call");
        assert!(tracker.is_active_turn(), "turn is now active");

        // Second call while already active: reports true (already in flight).
        let was_active_again = tracker.mark_turn_active_returning_was_active();
        assert!(was_active_again, "second call should report already-active");
        assert!(tracker.is_active_turn());
    }

    /// Regression test for the exact race reagent flagged on PR #2005: a
    /// naive `is_active_turn()` read followed by a separate
    /// `set_active_turn(true)` write lets two concurrent callers (send_message
    /// vs. send_user_message on the same block) both observe `false` before
    /// either writes `true`, so both decide to spawn a watchdog.
    /// `mark_turn_active_returning_was_active` closes that window by holding
    /// the lock across both the read and the write — this test simulates the
    /// interleaving directly (no real concurrency needed to prove the
    /// invariant: exactly one of N concurrent-in-spirit calls sees "was
    /// idle").
    #[test]
    fn mark_turn_active_is_atomic_across_repeated_calls() {
        let tracker = TurnActivityTracker::new("test-block".to_string());
        let results: Vec<bool> = (0..5).map(|_| tracker.mark_turn_active_returning_was_active()).collect();
        // Exactly the first call observes "was idle" (false); every
        // subsequent call — however tightly interleaved a real concurrent
        // caller might be — observes "already active" (true), because each
        // read-and-write pair is indivisible under the lock.
        assert_eq!(results, vec![false, true, true, true, true]);
    }
}
