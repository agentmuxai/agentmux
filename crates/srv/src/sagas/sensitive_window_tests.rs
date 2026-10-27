// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::server::tests::test_state;

fn facts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// An approved request for `op`, as the super agent would leave it. Each fact's
/// value is made unique, so no request joins another test's.
fn approved(op: &str, given: &[(&str, &str)]) -> String {
    let reg = requests::registry();
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let given: Vec<(String, String)> = given.iter().map(|(k, v)| (k.to_string(), format!("{v}-{unique}"))).collect();
    let id = reg.request("u-asker", "asker", op, &given.into_iter().collect(), "because", now_ms()).unwrap().id;
    reg.approve(&id, "super", now_ms()).unwrap();
    id
}

fn counting_runner(counter: &Arc<AtomicUsize>, result: Result<(), String>) -> Runner {
    let counter = counter.clone();
    Arc::new(move |_, _| {
        let (counter, result) = (counter.clone(), result.clone());
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            result
        })
    })
}

async fn settled(id: &str) -> SensitiveView {
    for _ in 0..400 {
        let v = status(id).expect("known countdown");
        if !matches!(v.status, "pending" | "proceeding") {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("countdown {id} never settled");
}

#[tokio::test]
async fn the_operation_runs_when_the_window_runs_out() {
    let state = test_state();
    let ran = Arc::new(AtomicUsize::new(0));
    register_runner("tower_pair", counting_runner(&ran, Ok(())));
    let id = approved("tower_pair", &[("device", "phone")]);
    let view = open(&state, &id, Duration::from_millis(40)).unwrap();
    assert_eq!((view.status, view.op.as_str(), view.requester.as_str(), view.approver.as_deref()), ("pending", "tower_pair", "asker", Some("super")));
    assert_eq!(ran.load(Ordering::SeqCst), 0, "not before the deadline");
    let done = settled(&id).await;
    assert_eq!((done.status, done.error), ("done", None));
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert_eq!(requests::registry().get(&id).unwrap().status, Status::Done);
}

#[tokio::test]
async fn a_veto_in_the_window_stops_it_and_runs_nothing() {
    let state = test_state();
    let ran = Arc::new(AtomicUsize::new(0));
    register_runner("job_template_install", counting_runner(&ran, Ok(())));
    let id = approved("job_template_install", &[("template", "t"), ("publisher", "p")]);
    open(&state, &id, Duration::from_secs(60)).unwrap();
    assert_eq!(veto(&state, &id), "vetoed");
    assert_eq!(status(&id).unwrap().status, "vetoed");
    assert_eq!(requests::registry().get(&id).unwrap().status, Status::Vetoed);
    assert_eq!(veto(&state, &id), "too_late", "a second veto finds it ended");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ran.load(Ordering::SeqCst), 0, "a vetoed operation never runs");
}

#[tokio::test]
async fn approving_now_skips_the_wait() {
    let state = test_state();
    let ran = Arc::new(AtomicUsize::new(0));
    register_runner("desktop_control_grant", counting_runner(&ran, Ok(())));
    let id = approved("desktop_control_grant", &[("scope", "screen")]);
    open(&state, &id, Duration::from_secs(600)).unwrap();
    assert_eq!(approve_now(&id), "approved_now");
    assert_eq!(approve_now(&id), "too_late", "once is enough");
    assert_eq!(settled(&id).await.status, "done");
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert_eq!(approve_now(&id), "too_late");
}

#[tokio::test]
async fn it_cannot_be_vetoed_once_it_is_under_way() {
    let state = test_state();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (s, r) = (started.clone(), release.clone());
    register_runner(
        "vault_always_allow",
        Arc::new(move |_, _| {
            let (s, r) = (s.clone(), r.clone());
            Box::pin(async move {
                s.notify_one();
                r.notified().await;
                Ok(())
            })
        }),
    );
    let reg = requests::registry();
    let unique = format!("e-{}", uuid::Uuid::new_v4().simple());
    let id = reg.request("u-asker", "asker", "vault_always_allow", &facts(&[("entry", &unique), ("kind", "login")]), "", now_ms()).unwrap().id;
    reg.approve(&id, "super", now_ms()).unwrap();
    open(&state, &id, Duration::from_millis(10)).unwrap();
    started.notified().await;
    assert_eq!(status(&id).unwrap().status, "proceeding");
    assert_eq!(veto(&state, &id), "too_late");
    release.notify_one();
    assert_eq!(settled(&id).await.status, "done");
}

#[tokio::test]
async fn a_failed_run_is_recorded_as_failed() {
    let state = test_state();
    register_runner("job_template_approve", counting_runner(&Arc::new(AtomicUsize::new(0)), Err("device went away".into())));
    let id = approved("job_template_approve", &[("template", "t"), ("publisher", "p")]);
    open(&state, &id, Duration::from_millis(10)).unwrap();
    let done = settled(&id).await;
    assert_eq!((done.status, done.error.as_deref()), ("failed", Some("device went away")));
    assert_eq!(requests::registry().get(&id).unwrap().status, Status::Failed);
}

#[tokio::test]
async fn an_operation_nothing_runs_yet_fails_instead_of_passing() {
    let state = test_state();
    let reg = requests::registry();
    let id = reg.request("u-asker", "asker", "sender_key_repin", &facts(&[("agent", &format!("a-{}", uuid::Uuid::new_v4().simple())), ("old_fingerprint", "aa"), ("new_fingerprint", "bb")]), "", now_ms()).unwrap().id;
    reg.approve(&id, "super", now_ms()).unwrap();
    open(&state, &id, Duration::from_millis(10)).unwrap();
    let done = settled(&id).await;
    assert_eq!(done.status, "failed");
    assert!(done.error.unwrap().contains("nothing runs sender_key_repin"));
}

#[tokio::test]
async fn only_an_approved_request_gets_a_countdown_and_the_clock_never_restarts() {
    let state = test_state();
    let reg = requests::registry();
    let asked = reg.request("u-asker", "asker", "tower_pair", &facts(&[("device", &format!("d-{}", uuid::Uuid::new_v4().simple()))]), "", now_ms()).unwrap().id;
    assert!(open(&state, &asked, Duration::from_secs(1)).unwrap_err().contains("has not been approved"));
    assert!(open(&state, "missing", Duration::from_secs(1)).unwrap_err().contains("no request"));

    let id = approved("desktop_control_grant", &[("scope", "window")]);
    let first = open(&state, &id, Duration::from_secs(600)).unwrap();
    let again = open(&state, &id, Duration::from_secs(1)).unwrap();
    assert_eq!(first.deadline_ms, again.deadline_ms);
    assert!(pending().iter().any(|v| v.request_id == id));
    veto(&state, &id);
    assert!(!pending().iter().any(|v| v.request_id == id), "a vetoed one is no longer counting down");
}
