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
    let _turn = TEST_RUNNER_TURN.lock().unwrap_or_else(|e| e.into_inner());
    unregister_runner("sender_key_repin");
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

#[tokio::test]
async fn a_request_withdrawn_during_the_countdown_does_not_run() {
    let state = test_state();
    let ran = Arc::new(AtomicUsize::new(0));
    register_runner("widget_install", counting_runner(&ran, Ok(())));
    let id = approved("widget_install", &[("widget", "w"), ("publisher", "p"), ("permissions", "storage")]);
    open(&state, &id, Duration::from_secs(600)).unwrap();
    assert_eq!(requests::registry().cancel(&id, "u-asker", now_ms()).unwrap().status, Status::Cancelled);
    assert_eq!(cancelled(&state, &id), "cancelled");
    assert_eq!(status(&id).unwrap().status, "cancelled", "the banner stops counting");
    assert_eq!(cancelled(&state, &id), "too_late");
    assert_eq!(veto(&state, &id), "too_late");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert_eq!(requests::registry().get(&id).unwrap().status, Status::Cancelled);
}

#[tokio::test]
async fn a_withdrawal_in_the_last_moment_is_still_caught_before_the_run() {
    let state = test_state();
    let ran = Arc::new(AtomicUsize::new(0));
    register_runner("vault_use", counting_runner(&ran, Ok(())));
    let id = approved("vault_use", &[("entry", "e"), ("kind", "login")]);
    open(&state, &id, Duration::from_millis(40)).unwrap();
    // Withdrawn in the registry, with nobody telling the window.
    requests::registry().cancel(&id, "u-asker", now_ms()).unwrap();
    assert_eq!(settled(&id).await.status, "cancelled");
    assert_eq!(ran.load(Ordering::SeqCst), 0, "the run is claimed in the registry first");
    assert_eq!(requests::registry().get(&id).unwrap().status, Status::Cancelled);
}

#[tokio::test]
async fn a_runner_that_panics_before_returning_its_future_is_settled_as_failed() {
    let _turn = TEST_RUNNER_TURN.lock().unwrap_or_else(|e| e.into_inner());
    let state = test_state();
    register_runner("quit_agentmux", Arc::new(|_, _| panic!("boom while building")));
    let reg = requests::registry();
    let id = reg.request("u-asker", "asker", "quit_agentmux", &BTreeMap::new(), "", now_ms()).unwrap().id;
    reg.approve(&id, "super", now_ms()).unwrap();
    open(&state, &id, Duration::from_millis(10)).unwrap();
    let done = settled(&id).await;
    assert_eq!(done.status, "failed");
    assert!(done.error.unwrap().contains("stopped unexpectedly"));
    assert_eq!(reg.get(&id).unwrap().status, Status::Failed, "it does not stay under way");
}

#[tokio::test]
async fn an_operation_that_panics_is_settled_as_failed() {
    let state = test_state();
    register_runner("upgrade", Arc::new(|_, _| Box::pin(async { panic!("boom") })));
    let id = approved("upgrade", &[("version", "9")]);
    open(&state, &id, Duration::from_millis(10)).unwrap();
    let done = settled(&id).await;
    assert_eq!(done.status, "failed");
    assert!(done.error.unwrap().contains("stopped unexpectedly"));
    let r = requests::registry().get(&id).unwrap();
    assert_eq!(r.status, Status::Failed, "it no longer counts as open");
    assert!(!r.status.is_open());
}

mod decide {
    use super::*;
    use crate::backend::rpc_types::{RpcContext, RpcMessage};

    struct Harness {
        engine: Arc<WshRpcEngine>,
        rx: tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        state: AppState,
    }

    fn harness(ctx: RpcContext) -> Harness {
        let state = test_state();
        let (engine, rx) = WshRpcEngine::new();
        register(&engine, &state);
        engine.set_rpc_context(ctx);
        Harness { engine, rx, state }
    }

    impl Harness {
        async fn decide(&mut self, id: &str, decision: &str) -> Result<serde_json::Value, String> {
            self.engine.handle_message(RpcMessage {
                command: COMMAND_SENSITIVE_DECIDE.to_string(),
                reqid: "req-1".to_string(),
                data: Some(serde_json::json!({ "request_id": id, "decision": decision })),
                ..Default::default()
            });
            let resp = tokio::time::timeout(Duration::from_secs(10), self.rx.recv()).await.unwrap().unwrap();
            if resp.error.is_empty() {
                Ok(resp.data.unwrap_or_default())
            } else {
                Err(resp.error)
            }
        }
    }

    #[tokio::test]
    async fn the_proven_window_vetoes_and_approves_now() {
        let mut h = harness(RpcContext::operator_for_tests());
        // No runner is registered for this operation by any test, so what runs
        // is not what is checked here: only that the decision is taken and settles.
        let make = || approved("sender_key_repin", &[("agent", "a"), ("old_fingerprint", "aa"), ("new_fingerprint", "bb")]);
        let id = make();
        open(&h.state, &id, Duration::from_secs(600)).unwrap();
        assert_eq!(h.decide(&id, "veto").await.unwrap()["outcome"], "vetoed");
        assert_eq!(status(&id).unwrap().status, "vetoed");
        assert_eq!(h.decide(&id, "veto").await.unwrap()["outcome"], "too_late");

        let id = make();
        open(&h.state, &id, Duration::from_secs(600)).unwrap();
        assert_eq!(h.decide(&id, "approve_now").await.unwrap()["outcome"], "approved_now");
        assert!(matches!(settled(&id).await.status, "done" | "failed"), "it ran without waiting 10 minutes");
    }

    #[tokio::test]
    async fn an_agent_connection_is_refused_and_the_countdown_is_untouched() {
        let mut h = harness(RpcContext { agent_id: "super".to_string(), ..Default::default() });
        let id = approved("job_template_install", &[("template", "t"), ("publisher", "p")]);
        open(&h.state, &id, Duration::from_secs(600)).unwrap();
        for decision in ["veto", "approve_now"] {
            let err = h.decide(&id, decision).await.unwrap_err();
            assert!(err.contains("FORBIDDEN"), "{err}");
        }
        assert_eq!(status(&id).unwrap().status, "pending", "an agent can neither stop nor hurry it");
        veto(&h.state, &id);
    }

    #[tokio::test]
    async fn a_connection_the_host_never_proved_is_refused() {
        let mut h = harness(RpcContext::default());
        let id = approved("tower_pair", &[("device", "watch")]);
        open(&h.state, &id, Duration::from_secs(600)).unwrap();
        let err = h.decide(&id, "veto").await.unwrap_err();
        assert!(err.contains("FORBIDDEN") && err.contains("isn't the AgentMux window"), "{err}");
        assert_eq!(status(&id).unwrap().status, "pending");
        veto(&h.state, &id);
    }

    #[tokio::test]
    async fn an_unknown_decision_is_refused() {
        let mut h = harness(RpcContext::operator_for_tests());
        let err = h.decide("whatever", "maybe").await.unwrap_err();
        assert!(err.contains("INVALID_ARG"), "{err}");
    }
}
