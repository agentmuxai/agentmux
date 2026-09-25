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

/// Who delivered input to an agent (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOrigin {
    /// The human typing in the agent's own pane.
    User,
    /// A jekt, cron, nudge, broadcast, loop, another agent — any automated sender.
    Automated,
    /// srv or the frontend on its own behalf: hidden memory reinjection, a
    /// side question.
    System,
}

/// One input, as told to the tracker by the path that delivers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnInput {
    pub origin: TurnOrigin,
    /// The message text (kept only for `User`, for the §6.3 quote check).
    pub text: String,
}

/// What started the current turn, and whether anything else got in.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct TurnProvenance {
    pub origin: TurnOrigin,
    /// Non-user input was delivered into this turn after it started (§6.3:
    /// a taint, never undone within the turn).
    pub tainted: bool,
    /// The user's message that started the turn (origin `User` only).
    #[serde(skip)]
    pub user_text: Option<String>,
}

impl TurnProvenance {
    fn from_input(input: TurnInput) -> Self {
        let user_text = (input.origin == TurnOrigin::User).then_some(input.text);
        TurnProvenance { origin: input.origin, tainted: false, user_text }
    }
    fn absorb(&mut self, origin: TurnOrigin) {
        if origin != TurnOrigin::User {
            self.tainted = true;
        }
    }
}

struct TurnActivityTrackerInner {
    active_turn: bool,
    exit_code: Option<i32>,
    /// What started the CURRENT turn. Cleared on every idle→active flip that
    /// isn't labelled, so a turn only ever carries a provenance its own start
    /// reported — a stale label can never leak into a later turn.
    provenance: Option<TurnProvenance>,
    /// Told ahead of a turn that starts later — a message queued while the
    /// process spawns. Taken by the next idle→active flip.
    next_turn: Option<TurnProvenance>,
}

impl TurnActivityTrackerInner {
    /// idle→active with no label: take the queued hint, if any.
    fn start_unlabelled(&mut self) {
        self.provenance = self.next_turn.take();
    }
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
                provenance: None,
                next_turn: None,
            }),
        }
    }

    /// Called when a new turn starts (subprocess spawned).
    pub fn set_active_turn(&self, active: bool) {
        let mut inner = self.inner.lock().unwrap();
        let was_active = inner.active_turn;
        inner.active_turn = active;
        if active {
            inner.exit_code = None;
            if !was_active {
                inner.start_unlabelled();
            }
        } else {
            inner.provenance = None;
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
        self.mark_turn_active_from(None)
    }

    /// [`Self::mark_turn_active_returning_was_active`], saying who delivered
    /// the input (turn provenance, SPEC_AGENT_SELF_QUIT §6.3):
    /// - starting a turn records `input` as what started it — or, unlabelled,
    ///   whatever was queued for it (else unknown);
    /// - into a running turn, non-user input — or unlabelled input, which
    ///   can't be proven to be the user — taints it.
    pub fn mark_turn_active_from(&self, input: Option<TurnInput>) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let was_active = inner.active_turn;
        inner.active_turn = true;
        inner.exit_code = None;
        match (was_active, input) {
            (false, Some(i)) => inner.provenance = Some(TurnProvenance::from_input(i)),
            (false, None) => inner.start_unlabelled(),
            (true, Some(i)) => {
                if let Some(p) = inner.provenance.as_mut() {
                    p.absorb(i.origin);
                }
            }
            (true, None) => {
                if let Some(p) = inner.provenance.as_mut() {
                    p.tainted = true;
                }
            }
        }
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
        inner.provenance = None;
        inner.next_turn = None;
        drop(inner);
        tracing::info!(block_id = %self.block_id, exit_code, "[health] turn_active flip (process exited)");
    }

    /// Whether there's an active turn in progress.
    pub fn is_active_turn(&self) -> bool {
        self.inner.lock().unwrap().active_turn
    }

    /// A message queued now will start a later turn (it waits for the
    /// process to spawn). Two queued before that turn starts: the second, if
    /// not the user's, taints it.
    pub fn hint_next_turn(&self, input: TurnInput) {
        let mut inner = self.inner.lock().unwrap();
        match inner.next_turn.as_mut() {
            None => inner.next_turn = Some(TurnProvenance::from_input(input)),
            Some(p) => p.absorb(input.origin),
        }
    }

    /// The turn goes on but a new one logically begins — a deferred message
    /// released at a `result` boundary, where `turn_active` never dropped.
    pub fn begin_turn_from(&self, input: TurnInput) {
        let mut inner = self.inner.lock().unwrap();
        inner.provenance = Some(TurnProvenance::from_input(input));
    }

    /// Input delivered into the running turn without a turn flip (a deferred
    /// message released while the agent waits on a tool).
    pub fn note_input(&self, origin: TurnOrigin) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(p) = inner.provenance.as_mut() {
            p.absorb(origin);
        }
    }

    /// What started the turn in flight, if one is and it was reported.
    pub fn provenance(&self) -> Option<TurnProvenance> {
        let inner = self.inner.lock().unwrap();
        if inner.active_turn {
            inner.provenance.clone()
        } else {
            None
        }
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

    // ---- turn provenance (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.3) ----

    fn user(text: &str) -> TurnInput {
        TurnInput { origin: TurnOrigin::User, text: text.into() }
    }
    fn automated() -> TurnInput {
        TurnInput { origin: TurnOrigin::Automated, text: "jekt".into() }
    }

    #[test]
    fn a_turn_the_user_starts_carries_their_text() {
        let t = TurnActivityTracker::new("b".into());
        assert!(!t.mark_turn_active_from(Some(user("finish the PR then quit"))));
        let p = t.provenance().expect("active turn");
        assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false));
        assert_eq!(p.user_text.as_deref(), Some("finish the PR then quit"));
    }

    #[test]
    fn automated_input_into_a_user_turn_taints_it_for_good() {
        let t = TurnActivityTracker::new("b".into());
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_from(Some(user("also this"))); // the user steering their own turn
        assert!(!t.provenance().unwrap().tainted);
        t.mark_turn_active_from(Some(automated()));
        assert!(t.provenance().unwrap().tainted);
        t.mark_turn_active_from(Some(user("more")));
        assert!(t.provenance().unwrap().tainted, "never undone within the turn");
    }

    #[test]
    fn unlabelled_input_counts_as_unknown_never_as_the_user() {
        let t = TurnActivityTracker::new("b".into());
        t.mark_turn_active_returning_was_active();
        assert_eq!(t.provenance(), None, "an unlabelled start is unknown");
        t.set_active_turn(false);
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_returning_was_active(); // unlabelled input mid-turn
        assert!(t.provenance().unwrap().tainted);
    }

    #[test]
    fn a_label_never_outlives_its_turn() {
        let t = TurnActivityTracker::new("b".into());
        t.mark_turn_active_from(Some(user("go")));
        t.set_active_turn(false);
        assert_eq!(t.provenance(), None, "no turn, no provenance");
        t.set_active_turn(true); // the next turn starts unlabelled
        assert_eq!(t.provenance(), None, "the old user label must not leak into it");
        t.set_exited(0);
        assert_eq!(t.provenance(), None);
    }

    #[test]
    fn a_queued_message_labels_the_turn_it_starts_later() {
        let t = TurnActivityTracker::new("b".into());
        t.hint_next_turn(user("first message after spawn"));
        t.set_active_turn(true); // the spawn starts the turn
        let p = t.provenance().unwrap();
        assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false));
        t.set_active_turn(false);
        t.hint_next_turn(user("a"));
        t.hint_next_turn(automated());
        t.set_active_turn(true);
        assert!(t.provenance().unwrap().tainted, "a jekt queued alongside taints it");
    }

    #[test]
    fn a_boundary_release_or_tool_wait_release_is_automated() {
        let t = TurnActivityTracker::new("b".into());
        t.mark_turn_active_from(Some(user("go")));
        t.note_input(TurnOrigin::Automated);
        assert!(t.provenance().unwrap().tainted, "released while waiting on a tool");
        t.begin_turn_from(automated());
        assert_eq!(t.provenance().unwrap().origin, TurnOrigin::Automated, "released at the result boundary");
    }
}
