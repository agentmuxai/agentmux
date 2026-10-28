// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::server::tests::test_state;

const FAST: Timing = Timing { poll: Duration::from_millis(5), absent_grace: Duration::from_millis(40), ask_wait: Duration::from_secs(30) };

fn run_nothing() {
    window::register_runner("sender_key_repin", Arc::new(|_, _| Box::pin(async { Ok(()) })));
}

/// A request for the operation these tests share, with facts no other test's request shares.
fn asked() -> String {
    let u = uuid::Uuid::new_v4().simple().to_string();
    let facts = [("agent", format!("a-{u}")), ("old_fingerprint", "aa".to_string()), ("new_fingerprint", "bb".to_string())];
    requests::registry().request("u-asker", "asker", "sender_key_repin", &facts.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), "because", now_ms()).unwrap().id
}

fn turn() -> std::sync::MutexGuard<'static, ()> {
    window::TEST_RUNNER_TURN.lock().unwrap_or_else(|e| e.into_inner())
}

#[tokio::test]
async fn with_no_super_agent_the_request_goes_ahead_with_a_longer_countdown_and_no_approver() {
    let _turn = turn();
    run_nothing();
    let state = test_state();
    let id = asked();
    let before = now_ms();
    watch_with(&state, &id, FAST, |_| false).await;
    let r = requests::registry().get(&id).unwrap();
    assert_eq!((r.status, r.decided_by), (requests::Status::Pending, None));
    let view = window::status(&id).expect("a countdown");
    assert_eq!((view.status, view.approver), ("pending", None), "the banner says no super agent reviewed it");
    let secs = (view.deadline_ms - before) / 1000;
    assert!((59..=60).contains(&secs), "twice the usual 30 s, got {secs}");
    assert_eq!(window::veto(&state, &id), "vetoed", "the person's veto still stops it");
}

#[tokio::test]
async fn a_super_agent_that_does_not_answer_in_time_is_gone_around() {
    let _turn = turn();
    run_nothing();
    let state = test_state();
    let id = asked();
    let late = Timing { ask_wait: Duration::from_millis(40), ..FAST };
    watch_with(&state, &id, late, |_| true).await;
    assert_eq!(requests::registry().get(&id).unwrap().status, requests::Status::Pending);
    assert!(window::status(&id).is_some());
    window::veto(&state, &id);
}

#[tokio::test]
async fn a_super_agent_that_is_there_and_answers_is_not_gone_around() {
    let _turn = turn();
    run_nothing();
    let state = test_state();
    let id = asked();
    let watcher = {
        let (state, id) = (state.clone(), id.clone());
        tokio::spawn(async move { watch_with(&state, &id, FAST, |_| true).await })
    };
    requests::registry().deny(&id, "boss", "no", now_ms()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), watcher).await.expect("the watcher ends once it is answered").unwrap();
    let r = requests::registry().get(&id).unwrap();
    assert_eq!((r.status, r.decided_by.as_deref()), (requests::Status::Denied, Some("boss")));
    assert!(window::status(&id).is_none(), "no countdown was opened");
}

#[tokio::test]
async fn a_request_already_approved_is_left_alone() {
    let _turn = turn();
    run_nothing();
    let state = test_state();
    let id = asked();
    requests::registry().approve(&id, "boss", now_ms()).unwrap();
    watch_with(&state, &id, FAST, |_| false).await;
    assert_eq!(requests::registry().get(&id).unwrap().decided_by.as_deref(), Some("boss"));
    assert!(window::status(&id).is_none());
}

#[tokio::test]
async fn a_brief_absence_is_waited_out() {
    let _turn = turn();
    run_nothing();
    let state = test_state();
    let id = asked();
    let started = std::time::Instant::now();
    // There at first, gone for less than the grace, back again: nobody is gone around.
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let watcher = {
        let (state, id) = (state.clone(), id.clone());
        let grace = Timing { absent_grace: Duration::from_secs(5), ..FAST };
        tokio::spawn(async move {
            watch_with(&state, &id, grace, move |_| calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) % 4 != 1).await;
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(requests::registry().get(&id).unwrap().status, requests::Status::Asked, "{:?}", started.elapsed());
    requests::registry().cancel(&id, "u-asker", now_ms()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), watcher).await.unwrap().unwrap();
}

#[tokio::test]
async fn nothing_is_counted_down_that_nothing_can_run() {
    let _turn = turn();
    window::unregister_runner("sender_key_repin");
    let state = test_state();
    let id = asked();
    watch_with(&state, &id, FAST, |_| false).await;
    assert_eq!(requests::registry().get(&id).unwrap().status, requests::Status::Expired);
    assert!(window::status(&id).is_none());
}

#[tokio::test]
async fn the_longer_countdown_never_passes_the_operators_cap_of_two_minutes() {
    let _turn = turn();
    window::register_runner("quit_agentmux", Arc::new(|_, _| Box::pin(async { Ok(()) })));
    let state = test_state();
    let id = requests::registry().request("u-asker", "asker", "quit_agentmux", &Default::default(), "", now_ms()).unwrap().id;
    let before = now_ms();
    watch_with(&state, &id, FAST, |_| false).await;
    let view = window::status(&id).expect("a countdown");
    let secs = (view.deadline_ms - before) / 1000;
    assert!((119..=120).contains(&secs), "a quit is already 2 min, so it stays 2 min, got {secs}");
    assert_eq!(window::veto(&state, &id), "vetoed");
    // And an operation of 30 s doubles, as the other test checks, and nothing goes above the cap.
    for op in sensitive_ops::OPS.iter() {
        assert!(op.fallback_window_secs() <= sensitive_ops::MAX_WINDOW_SECS, "{}", op.op);
    }
}
