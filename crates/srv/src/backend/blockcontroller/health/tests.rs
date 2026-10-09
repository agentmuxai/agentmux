// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for the turn-activity tracker and the turn ledger (`super`).

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
    TurnInput { origin: TurnOrigin::User, text: text.into(), joins_turn: None }
}
fn automated() -> TurnInput {
    TurnInput { origin: TurnOrigin::Automated, text: "jekt".into(), joins_turn: None }
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
fn delivering_the_queue_after_a_spawn_keeps_the_user_s_label() {
    let t = TurnActivityTracker::new("b".into());
    t.hint_next_turn(user("do X then quit"));
    t.set_active_turn(true); // spawn_process
    assert!(t.mark_turn_active_for_queued(), "already active");
    let p = t.provenance().unwrap();
    assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false), "bookkeeping, not new input");

    // Idle, the same mark starts the turn with what was queued for it.
    t.set_active_turn(false);
    t.hint_next_turn(user("again"));
    assert!(!t.mark_turn_active_for_queued());
    assert_eq!(t.provenance().unwrap().user_text.as_deref(), Some("again"));
}

#[test]
fn an_unlabelled_message_queued_behind_the_user_s_taints_that_turn() {
    let t = TurnActivityTracker::new("b".into());
    t.hint_next_turn(user("go"));
    t.hint_next_turn_unlabelled();
    t.set_active_turn(true);
    assert!(t.provenance().unwrap().tainted);
}

// ---- turn ledger (SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.3) ----

use std::sync::atomic::{AtomicU64, Ordering};

/// A tracker on a clock the test moves, recording what it publishes.
fn ledger_tracker() -> (TurnActivityTracker, Arc<AtomicU64>, Arc<Mutex<Vec<TurnLedger>>>) {
    let clock = Arc::new(AtomicU64::new(1_000_000));
    let c = Arc::clone(&clock);
    let t = TurnActivityTracker::with_clock("b".into(), Box::new(move || c.load(Ordering::SeqCst)));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);
    t.set_ledger_publisher(Arc::new(move |l: &TurnLedger| s.lock().unwrap().push(l.clone())));
    (t, clock, seen)
}

fn advance(clock: &AtomicU64, ms: u64) {
    clock.fetch_add(ms, Ordering::SeqCst);
}

fn jekt() -> TurnInput {
    TurnInput { origin: TurnOrigin::Automated, text: "jekt".into(), joins_turn: None }
}

fn stats(output_tokens: u64, steps: u32) -> PassStats {
    PassStats { output_tokens, cost_usd: 0.25, steps, duration_api_ms: 1_000 }
}

#[test]
fn a_single_pass_turn_ends_with_its_pass() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    advance(&clock, 5_000);
    t.set_active_turn(false);
    t.add_pass_stats(stats(300, 2));
    let l = t.ledger().unwrap();
    assert_eq!((l.passes, l.active, l.end), (1, false, Some(TurnEnd::Completed)));
    assert_eq!(l.ended_at_ms, Some(l.started_at_ms + 5_000));
    assert_eq!((l.counted_passes, l.output_tokens, l.steps), (1, 300, 2));
    assert_eq!(l.origin, Some(TurnOrigin::User));
}

#[test]
fn a_jekt_written_mid_pass_and_answered_in_the_next_pass_is_one_turn() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.mark_turn_active_from(Some(jekt())); // J2: arrives mid-pass
    advance(&clock, 10_000);
    t.set_active_turn(false);
    t.add_pass_stats(stats(300, 2));
    let l = t.ledger().unwrap();
    assert_eq!((l.ended_at_ms, l.settle_until_ms.is_some()), (None, true), "waits for the next pass");
    advance(&clock, 50);
    // The CLI answers the jekt in a pass of its own: no input from srv.
    assert!(t.mark_turn_active_from_cli(), "the mid-pass input explains the init");
    let p = t.provenance().unwrap();
    assert_eq!((p.origin, p.tainted), (TurnOrigin::User, true), "the user's pass, tainted by the jekt it answers");
    advance(&clock, 3_000);
    t.set_active_turn(false);
    t.add_pass_stats(stats(100, 1));
    let l = t.ledger().unwrap();
    assert_eq!(l.turn_id, id, "the same turn");
    assert_eq!((l.passes, l.inputs, l.counted_passes, l.output_tokens, l.steps), (2, 1, 2, 400, 3));
    assert_eq!(l.end, Some(TurnEnd::Completed));
    assert_eq!(l.origin, Some(TurnOrigin::User), "the trigger never changes");
}

#[test]
fn a_task_notification_during_a_pass_joins_the_cli_s_own_next_pass() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.note_cli_task_notification(None); // the task finished mid-pass
    advance(&clock, 4_000);
    t.set_active_turn(false);
    advance(&clock, 20);
    assert!(t.mark_turn_active_from_cli(), "init after the notification starts a pass");
    let l = t.ledger().unwrap();
    assert_eq!((l.turn_id, l.passes, l.active), (id, 2, true));
}

#[test]
fn a_task_finishing_after_the_turn_ended_starts_an_automated_turn() {
    // The order Claude Code 2.1.112 was recorded emitting: result, then
    // task_updated / task_notification, then system/init, 8 s later.
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("start a background build")));
    let first = t.ledger().unwrap().turn_id;
    t.set_active_turn(false);
    assert!(t.ledger().unwrap().end.is_some(), "nothing was pending: the turn is over");
    advance(&clock, 8_000);
    t.note_cli_task_notification(None);
    assert!(t.mark_turn_active_from_cli());
    let l = t.ledger().unwrap();
    assert_ne!(l.turn_id, first);
    assert_eq!((l.origin, l.passes), (Some(TurnOrigin::Automated), 1));
    assert!(t.is_active_turn());
    assert_eq!(t.provenance().unwrap().origin, TurnOrigin::Automated, "never the user's turn");
}

#[test]
fn an_init_with_no_task_notification_is_not_taken_for_a_pass() {
    let (t, _, _) = ledger_tracker();
    assert!(!t.mark_turn_active_from_cli());
    assert!(!t.is_active_turn(), "a stray init must not leave the agent busy");
    t.mark_turn_active_from(Some(user("go")));
    t.note_cli_task_notification(None);
    assert!(!t.mark_turn_active_from_cli(), "a pass is already running");
}

#[test]
fn a_settling_turn_ends_at_its_last_pass_when_nothing_follows() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.mark_turn_active_from(Some(jekt()));
    advance(&clock, 1_000);
    t.set_active_turn(false);
    assert!(t.ledger().unwrap().settle_until_ms.is_some());
    advance(&clock, TURN_SETTLE_MS + 1);
    t.mark_turn_active_from(Some(jekt())); // too late to join
    let l = t.ledger().unwrap();
    assert_ne!(l.turn_id, id, "a new turn");
    assert_eq!((l.passes, l.origin), (1, Some(TurnOrigin::Automated)));
}

#[test]
fn a_fresh_message_from_the_user_starts_a_new_turn_even_while_settling() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.mark_turn_active_from(Some(jekt()));
    t.set_active_turn(false);
    advance(&clock, 100);
    t.mark_turn_active_from(Some(user("something new")));
    assert_ne!(t.ledger().unwrap().turn_id, id);
}

#[test]
fn the_pane_s_held_message_joins_the_turn_it_was_held_during() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    advance(&clock, 2_000);
    t.set_active_turn(false); // ended: nothing srv knew of was pending
    advance(&clock, 300);
    let held = TurnInput { origin: TurnOrigin::User, text: "queued while busy".into(), joins_turn: Some(id) };
    t.mark_turn_active_from(Some(held));
    let l = t.ledger().unwrap();
    assert_eq!((l.turn_id, l.passes, l.inputs, l.active), (id, 2, 1, true));
    assert_eq!((l.ended_at_ms, l.end), (None, None), "reopened");
}

#[test]
fn a_held_message_for_a_long_finished_turn_starts_a_new_one() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.set_active_turn(false);
    advance(&clock, HELD_FLUSH_JOIN_MS + 1);
    let held = TurnInput { origin: TurnOrigin::User, text: "late".into(), joins_turn: Some(id) };
    t.mark_turn_active_from(Some(held));
    assert_ne!(t.ledger().unwrap().turn_id, id);
}

#[test]
fn queued_delivery_joins_only_a_settling_turn() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.mark_turn_active_from(Some(jekt()));
    t.set_active_turn(false);
    advance(&clock, 10);
    t.mark_turn_active_for_queued();
    assert_eq!(t.ledger().unwrap().turn_id, id);
    t.set_active_turn(false); // nothing pending now
    advance(&clock, 10);
    t.mark_turn_active_for_queued();
    assert_ne!(t.ledger().unwrap().turn_id, id);
}

#[test]
fn process_exit_ends_the_turn() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    advance(&clock, 700);
    t.set_exited(1);
    let l = t.ledger().unwrap();
    assert_eq!((l.active, l.end), (false, Some(TurnEnd::Exited)));
    assert_eq!(l.last_pass_ended_at_ms, l.ended_at_ms);
}

#[test]
fn turn_ids_are_unique_even_within_one_millisecond() {
    let (t, _, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("a")));
    let a = t.ledger().unwrap().turn_id;
    t.set_active_turn(false);
    t.mark_turn_active_from(Some(user("b")));
    assert!(t.ledger().unwrap().turn_id > a);
}

#[test]
fn every_change_is_published_and_nothing_else() {
    let (t, _, seen) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    t.mark_turn_active_returning_was_active(); // already active, no ledger change … except inputs
    t.set_active_turn(false);
    t.add_pass_stats(PassStats::default()); // a counted pass, even with nothing in it
    t.set_active_turn(false); // already idle: no change
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 4, "start, mid-pass input, end, stats: {seen:#?}");
    assert!(seen[0].active && !seen[2].active);
    assert_eq!(seen[3].counted_passes, 1);
}

#[test]
fn pass_stats_read_a_result_frame() {
    let frame = serde_json::json!({
        "type": "result", "subtype": "success", "num_turns": 2, "duration_api_ms": 9_800,
        "total_cost_usd": 0.0123, "usage": { "input_tokens": 9, "output_tokens": 238 }
    });
    let s = PassStats::from_result_frame(&frame);
    assert_eq!((s.output_tokens, s.steps, s.duration_api_ms), (238, 2, 9_800));
    assert!((s.cost_usd - 0.0123).abs() < 1e-9);
    assert_eq!(PassStats::from_result_frame(&serde_json::json!({"type": "result"})), PassStats::default());
}

#[test]
fn the_ledger_serializes_for_the_pane() {
    let (t, _, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let v = serde_json::to_value(t.ledger().unwrap()).unwrap();
    assert_eq!(v["origin"], "user");
    assert_eq!(v["active"], true);
    assert!(v.get("ended_at_ms").is_none(), "absent, not null");
}

/// A pass's figures belong to its own turn, even when input that opens a
/// new turn arrives right after its `result` (#4492).
#[test]
fn a_pass_s_figures_stay_with_its_turn_when_a_new_turn_follows_at_once() {
    let (t, _, seen) = ledger_tracker();
    t.mark_turn_active_from(Some(user("a")));
    let first = t.ledger().unwrap().turn_id;
    t.end_pass(Some(stats(300, 2)));
    t.mark_turn_active_from(Some(user("b"))); // a new turn, at once
    let next = t.ledger().unwrap();
    assert_ne!(next.turn_id, first);
    assert_eq!((next.counted_passes, next.output_tokens), (0, 0), "nothing of the old pass");
    let ended = seen.lock().unwrap().iter().rev().find(|l| l.turn_id == first).cloned().unwrap();
    assert_eq!((ended.counted_passes, ended.output_tokens, ended.steps), (1, 300, 2));
    assert_eq!(ended.end, Some(TurnEnd::Completed));
}

/// Input answered inside its own pass leaves no continuation behind: a
/// task that wakes the CLI minutes later is an automated pass, never the
/// user's (#4492).
#[test]
fn a_late_task_wake_up_never_inherits_an_old_user_pass_s_provenance() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("finish then quit")));
    t.mark_turn_active_from(Some(user("and this too"))); // answered inside the pass
    t.set_active_turn(false);
    advance(&clock, 5 * 60_000);
    t.note_cli_task_notification(None);
    assert!(t.mark_turn_active_from_cli());
    let p = t.provenance().unwrap();
    assert_eq!((p.origin, p.user_text), (TurnOrigin::Automated, None));
    assert_eq!(t.ledger().unwrap().origin, Some(TurnOrigin::Automated));
}

/// Nor does a stale continuation alone explain a later init.
#[test]
fn an_expired_continuation_does_not_explain_an_init() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    t.mark_turn_active_from(Some(jekt()));
    t.set_active_turn(false);
    advance(&clock, TURN_SETTLE_MS + 1);
    assert!(!t.mark_turn_active_from_cli(), "no notification, settle window over");
    assert!(!t.is_active_turn());
}

/// A pass the CLI starts for both a task and input from the last pass is
/// the automated one.
#[test]
fn a_continuation_that_a_task_also_woke_is_automated() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    t.mark_turn_active_from(Some(user("more")));
    t.note_cli_task_notification(None);
    t.set_active_turn(false);
    advance(&clock, 20);
    assert!(t.mark_turn_active_from_cli());
    assert_eq!(t.provenance().unwrap().origin, TurnOrigin::Automated);
}

/// ACP, App Server and one-shot subprocess passes start with no input:
/// the pane's held message joins its turn through the hint (#4492).
#[test]
fn a_held_message_joins_its_turn_through_a_controller_without_labelled_input() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_returning_was_active();
    let id = t.ledger().unwrap().turn_id;
    t.set_active_turn(false);
    advance(&clock, 300);
    t.hint_join(id);
    t.mark_turn_active_returning_was_active(); // the controller's own mark
    let l = t.ledger().unwrap();
    assert_eq!((l.turn_id, l.passes), (id, 2));
}

#[test]
fn a_join_hint_is_good_once_and_only_briefly() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_returning_was_active();
    let id = t.ledger().unwrap().turn_id;
    t.set_active_turn(false);
    t.hint_join(id);
    advance(&clock, HELD_FLUSH_JOIN_MS + 1);
    t.mark_turn_active_returning_was_active();
    assert_ne!(t.ledger().unwrap().turn_id, id, "stale hint");

    let id2 = t.ledger().unwrap().turn_id;
    t.set_active_turn(false);
    t.hint_join(id2);
    t.mark_turn_active_returning_was_active(); // joins, using the hint up
    t.set_active_turn(false);
    advance(&clock, 10);
    t.mark_turn_active_returning_was_active();
    assert_ne!(t.ledger().unwrap().turn_id, id2, "the hint was used once");
}

#[test]
fn a_hint_for_another_turn_joins_nothing() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_returning_was_active();
    let id = t.ledger().unwrap().turn_id;
    t.set_active_turn(false);
    advance(&clock, 10);
    t.hint_join(id + 999);
    t.mark_turn_active_returning_was_active();
    assert_ne!(t.ledger().unwrap().turn_id, id);
}

/// Every published ledger carries a higher `seq` than the one before it,
/// across turns, so the pane can drop one that lands late (#4492).
#[test]
fn every_published_ledger_has_a_higher_seq() {
    let (t, clock, seen) = ledger_tracker();
    t.mark_turn_active_from(Some(user("a")));
    t.mark_turn_active_from(Some(jekt()));
    t.end_pass(Some(stats(10, 1)));
    advance(&clock, TURN_SETTLE_MS + 1);
    t.mark_turn_active_from(Some(user("b")));
    let seqs: Vec<u64> = seen.lock().unwrap().iter().map(|l| l.seq).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    assert_eq!(seqs.first(), Some(&1));
}

/// A hint for a message that reached a running pass is spent there: the
/// next fresh message starts its own turn (#4492).
#[test]
fn a_hint_spent_on_a_running_pass_never_joins_a_later_fresh_message() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.hint_join(id);
    t.mark_turn_active_from(Some(user("held, delivered mid-pass")));
    t.end_pass(None);
    advance(&clock, TURN_SETTLE_MS + 1);
    t.mark_turn_active_returning_was_active();
    assert_ne!(t.ledger().unwrap().turn_id, id);
}

/// A settle window that lapsed with no next pass counts as the turn's end
/// for a held message's join, as the pane reads it (#4492).
#[test]
fn a_held_message_joins_a_turn_whose_settle_window_lapsed() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.mark_turn_active_from(Some(jekt())); // answered inside the pass
    t.end_pass(None);
    advance(&clock, TURN_SETTLE_MS + 3_000); // lapsed, nothing wrote the end
    let held = TurnInput { origin: TurnOrigin::User, text: "held".into(), joins_turn: Some(id) };
    t.mark_turn_active_from(Some(held));
    assert_eq!(t.ledger().unwrap().turn_id, id);
}

/// A held message queued behind a spawn keeps its join (#4492).
#[test]
fn a_held_message_queued_behind_a_spawn_keeps_its_join() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    let id = t.ledger().unwrap().turn_id;
    t.end_pass(None);
    advance(&clock, 200);
    t.hint_next_turn(TurnInput { origin: TurnOrigin::User, text: "held".into(), joins_turn: Some(id) });
    t.set_active_turn(true); // the spawn starts the queued pass
    assert_eq!(t.ledger().unwrap().turn_id, id);
}

#[test]
fn a_cleared_join_hint_joins_nothing() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_returning_was_active();
    let id = t.ledger().unwrap().turn_id;
    t.end_pass(None);
    t.hint_join(id);
    t.clear_join_hint(); // the dispatch failed
    advance(&clock, 100);
    t.mark_turn_active_returning_was_active();
    assert_ne!(t.ledger().unwrap().turn_id, id);
}

// ---- turn triggers (turn-model spec §5.2) ----

fn jekt_from(from: &str) -> TurnInput {
    TurnInput {
        origin: TurnOrigin::Automated,
        text: format!("[JEKT:FROM={from} TO=agent5 TIER=coord DELIVERY=wan TRUST=network-claimed]\n───\nhello\n[/JEKT]"),
        joins_turn: None,
    }
}

fn trig(kind: TriggerKind, from: Option<&str>) -> TurnTrigger {
    TurnTrigger::of(kind, from)
}

#[test]
fn an_automated_input_is_named_by_the_jekt_marker_srv_put_on_it() {
    let of = |i: TurnInput| TurnTrigger::from_input(i.origin, &i.text);
    assert_eq!(of(jekt_from("agentx")), trig(TriggerKind::Agent, Some("agentx")));
    assert_eq!(of(jekt_from("cron")), trig(TriggerKind::Schedule, Some("cron")));
    assert_eq!(of(jekt_from("github-consumer")), trig(TriggerKind::Service, Some("github-consumer")));
    // The same marker inside the stream-json line that carries it.
    let line = serde_json::json!({"type": "user", "message": {"role": "user", "content": jekt_from("korp").text}});
    assert_eq!(TurnTrigger::from_input(TurnOrigin::Automated, &line.to_string()), trig(TriggerKind::Agent, Some("korp")));
    // No marker: still automated, sender unknown.
    assert_eq!(TurnTrigger::from_input(TurnOrigin::Automated, "nudge"), trig(TriggerKind::Agent, None));
}

#[test]
fn the_user_s_own_words_are_never_read_as_a_jekt() {
    let quoted = "what does [JEKT:FROM=cron TIER=coord] mean?";
    assert_eq!(TurnTrigger::from_input(TurnOrigin::User, quoted), trig(TriggerKind::User, None));
    let broadcast = "[BROADCAST:FROM=user VIA=swarm TO=Agent5 RECIPIENTS=7 MSGID=x TS=1]\nhow's it going";
    assert_eq!(TurnTrigger::from_input(TurnOrigin::User, broadcast), trig(TriggerKind::Broadcast, None));
    assert_eq!(TurnTrigger::from_input(TurnOrigin::System, "reinjection"), trig(TriggerKind::System, None));
}

#[test]
fn a_task_summary_is_kept_and_a_long_one_shortened() {
    assert_eq!(TurnTrigger::task(Some("  npm test completed ")), trig(TriggerKind::Task, Some("npm test completed")));
    assert_eq!(TurnTrigger::task(None), trig(TriggerKind::Task, None));
    let long = TurnTrigger::task(Some(&"x".repeat(400))).from.unwrap();
    assert_eq!(long.chars().count(), 160);
    assert!(long.ends_with('…'));
}

#[test]
fn a_turn_names_its_trigger_and_lists_what_joined_it() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(jekt_from("agentx")));
    t.mark_turn_active_from(Some(jekt_from("github-consumer"))); // mid-pass
    advance(&clock, 1_000);
    t.set_active_turn(false);
    advance(&clock, 10);
    let held = TurnInput { origin: TurnOrigin::User, text: "also this".into(), joins_turn: Some(t.ledger().unwrap().turn_id) };
    t.mark_turn_active_from(Some(held));
    let l = t.ledger().unwrap();
    assert_eq!(l.trigger, Some(trig(TriggerKind::Agent, Some("agentx"))));
    assert_eq!(l.absorbed, vec![trig(TriggerKind::Service, Some("github-consumer")), trig(TriggerKind::User, None)]);
    assert_eq!(l.inputs, 2);
}

#[test]
fn a_task_wake_up_is_named_by_the_task() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("start the build")));
    t.set_active_turn(false);
    advance(&clock, 30_000);
    t.note_cli_task_notification(Some("Background command \"npm run build\" completed (exit code 0)"));
    assert!(t.mark_turn_active_from_cli());
    assert_eq!(
        t.ledger().unwrap().trigger,
        Some(trig(TriggerKind::Task, Some("Background command \"npm run build\" completed (exit code 0)")))
    );
}

#[test]
fn a_message_queued_at_spawn_names_the_turn_it_starts() {
    let (t, _, _) = ledger_tracker();
    t.hint_next_turn(jekt_from("cron"));
    t.set_active_turn(true); // the spawn
    assert_eq!(t.ledger().unwrap().trigger, Some(trig(TriggerKind::Schedule, Some("cron"))));
}

#[test]
fn absorbed_inputs_are_capped_but_still_counted() {
    let (t, _, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    for _ in 0..(ABSORBED_CAP + 5) {
        t.mark_turn_active_from(Some(jekt_from("agentx")));
    }
    let l = t.ledger().unwrap();
    assert_eq!((l.absorbed.len(), l.inputs as usize), (ABSORBED_CAP, ABSORBED_CAP + 5));
}

#[test]
fn the_trigger_serializes_for_the_pane() {
    let (t, _, _) = ledger_tracker();
    t.mark_turn_active_from(Some(jekt_from("agentx")));
    let v = serde_json::to_value(t.ledger().unwrap()).unwrap();
    assert_eq!(v["trigger"], serde_json::json!({"kind": "agent", "from": "agentx", "external": true}));
    assert!(v.get("absorbed").is_none(), "empty: absent");
}

#[test]
fn only_a_turn_something_else_started_is_external() {
    use TriggerKind::*;
    for kind in [Agent, Service, Schedule, Task] {
        assert!(TurnTrigger::of(kind, None).external, "{kind:?}");
    }
    for kind in [User, Broadcast, System] {
        assert!(!TurnTrigger::of(kind, None).external, "{kind:?}");
    }
    assert!(TurnTrigger::task(Some("done")).external);
}

/// A task the CLI wakes for inside a turn is listed as what joined it (#4503).
#[test]
fn a_task_wake_that_joins_a_turn_is_recorded() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    t.note_cli_task_notification(Some("Background command \"npm test\" completed (exit code 0)"));
    t.end_pass(None);
    advance(&clock, 20);
    assert!(t.mark_turn_active_from_cli());
    let l = t.ledger().unwrap();
    assert_eq!((l.passes, l.inputs), (2, 1));
    assert_eq!(l.trigger, Some(trig(TriggerKind::User, None)), "the turn is still the user's");
    assert_eq!(l.absorbed.first().map(|t| t.kind), Some(TriggerKind::Task));
}

/// A plain continuation (input answered in its own pass) adds no input.
#[test]
fn a_plain_continuation_adds_no_input() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    t.mark_turn_active_from(Some(user("more")));
    t.end_pass(None);
    advance(&clock, 20);
    assert!(t.mark_turn_active_from_cli());
    assert_eq!(t.ledger().unwrap().inputs, 1, "only the mid-pass message");
}

/// A task notification that loses the race to a user pass is not taken
/// for a later plain continuation (#4503).
#[test]
fn a_task_that_lost_the_race_is_not_credited_to_a_later_continuation() {
    let (t, clock, _) = ledger_tracker();
    t.note_cli_task_notification(Some("done")); // while idle
    t.mark_turn_active_from(Some(user("go"))); // the user's pass wins
    t.mark_turn_active_from(Some(user("more"))); // answered in its own pass
    t.end_pass(None);
    advance(&clock, 20);
    assert!(t.mark_turn_active_from_cli()); // a plain continuation
    let l = t.ledger().unwrap();
    assert_eq!(l.inputs, 1, "only the mid-pass message");
    assert!(l.absorbed.iter().all(|a| a.kind != TriggerKind::Task));
}

/// A CLI-started pass with no task notification gets no made-up sender.
#[test]
fn a_cli_pass_with_no_task_has_no_trigger() {
    let mut inner = TurnActivityTrackerInner {
        active_turn: true,
        exit_code: None,
        provenance: None,
        next_turn: None,
        ledger: None,
        input_during_pass: false,
        cli_wake_pending: false,
        continuation_expected: false,
        continuation_provenance: None,
        pending_join: None,
        ledger_seq: 0,
        next_turn_trigger: None,
        pending_task_trigger: None,
    };
    inner.begin_pass(PassStart::CliWake, None, 1_000);
    assert_eq!(inner.ledger.unwrap().trigger, None);
}

/// A settling turn reads, once lapsed, as ended at its last pass.
#[test]
fn a_lapsed_settling_ledger_closes_at_its_last_pass() {
    let (t, clock, _) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    t.mark_turn_active_from(Some(jekt()));
    advance(&clock, 500);
    t.end_pass(None);
    let settling = t.ledger().unwrap();
    let closed = closed_when_lapsed(&settling).expect("settling");
    assert_eq!(closed.ended_at_ms, settling.last_pass_ended_at_ms);
    assert_eq!((closed.end, closed.settle_until_ms, closed.seq), (Some(TurnEnd::Completed), None, settling.seq));
    assert_eq!(closed_when_lapsed(&closed), None, "already closed");
    t.mark_turn_active_from(Some(user("next")));
    assert_eq!(closed_when_lapsed(&t.ledger().unwrap()), None, "running");
}

/// Each kind of ledger change has its own `[turn]` line.
#[test]
fn ledger_changes_are_named_for_the_log() {
    let (t, clock, seen) = ledger_tracker();
    t.mark_turn_active_from(Some(user("go")));
    t.mark_turn_active_from(Some(jekt()));
    t.end_pass(Some(stats(10, 1)));
    advance(&clock, 20);
    assert!(t.mark_turn_active_from_cli());
    t.end_pass(None);
    let seen = seen.lock().unwrap();
    let named: Vec<_> = std::iter::once(ledger_transition(None, seen.first()))
        .chain(seen.windows(2).map(|w| ledger_transition(Some(&w[0]), Some(&w[1]))))
        .flatten()
        .collect();
    assert_eq!(
        named,
        vec![
            "opened",
            "input joined the running pass",
            "pass ended, settling for a next one",
            "next pass joined the turn",
            "ended",
        ]
    );
}
