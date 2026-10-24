// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Schedule run routes through the real router: history, Run now and
//! report, and who may call them (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3,
//! §7).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::jobs::RunLedger;
use crate::backend::storage::agents::test_agent_def;
use crate::server::AppState;

/// A state whose cron store and run ledger are its (identity-schema) test
/// store, with two agents and their tokens: the job's creator and another.
fn state() -> (AppState, String, String) {
    let mut state = crate::server::tests::test_state();
    state.shared_store = Some(state.mstore.clone());
    for (uid, slug) in [("uid-jobs-owner", "jobs-owner"), ("uid-jobs-other", "jobs-other")] {
        let mut def = test_agent_def(uid, slug, "claude", "agent", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }
    state.mstore.attach_token_index().unwrap();
    let owner = state.mstore.agent_token_ensure("uid-jobs-owner").unwrap();
    let other = state.mstore.agent_token_ensure("uid-jobs-other").unwrap();
    let ledger = RunLedger::new(state.identity_store.clone(), Some(state.mstore.clone()), state.broker.clone());
    state.cron_scheduler.install_runs(ledger);
    let sched = state.cron_scheduler.clone();
    sched.install_delivery(Arc::new(|_req, _uid| Box::pin(async { json!({"success": true, "block_id": "b-target"}) })));
    (state, owner, other)
}

async fn send(state: &AppState, method: Method, token: Option<&str>, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("X-AuthKey", "test-secret-key");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let body = match body {
        Some(b) => {
            req = req.header("Content-Type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = crate::server::build_router(state.clone()).oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The owner creates a job; returns its id.
async fn create(state: &AppState, owner: &str, extra: Value) -> String {
    let mut body = json!({
        "name": "jobs-routes", "expression": "0 0 1 1 *", "prompt": "hi",
        "target": "jobs-target", "target_uid": "uid-jobs-target", "created_by": "jobs-owner",
    });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let (status, v) = send(state, Method::POST, Some(owner), "/agentmux/cron", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_job_defaults_to_catch_up_skip_and_refuses_another_value() {
    let (state, owner, _) = state();
    let id = create(&state, &owner, json!({})).await;
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().catch_up, "skip");
    let id = create(&state, &owner, json!({"catch_up": "latest"})).await;
    assert_eq!(state.mstore.cron_get(&id).unwrap().unwrap().catch_up, "latest");
    let body = json!({"name": "n", "expression": "0 0 1 1 *", "prompt": "p", "target": "t", "catch_up": "all"});
    let (status, v) = send(&state, Method::POST, Some(&owner), "/agentmux/cron", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
}

/// Run now: the owner and the operator may; another agent may not. The run
/// shows in the history and in list's last run.
#[tokio::test(flavor = "multi_thread")]
async fn run_now_is_for_the_owner_or_the_operator_and_shows_in_history() {
    let (state, owner, other) = state();
    let id = create(&state, &owner, json!({})).await;
    let uri = format!("/agentmux/cron/{id}/run");

    let (status, v) = send(&state, Method::POST, Some(&other), &uri, Some(json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert!(state.identity_store.job_runs_for_job(&id, 10, None).unwrap().is_empty(), "nothing ran");

    let (status, v) = send(&state, Method::POST, Some(&owner), &uri, Some(json!({}))).await;
    assert_eq!((status, v["outcome"].as_str()), (StatusCode::OK, Some("delivered")), "{v}");
    let (status, v) = send(&state, Method::POST, None, &uri, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "the operator: {v}");

    let (status, v) = send(&state, Method::GET, None, &format!("/agentmux/cron/{id}/runs?limit=5"), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["triggered_by"], "operator");
    assert_eq!(runs[1]["triggered_by"], "uid-jobs-owner");
    assert_eq!(runs[0]["trigger"], "manual");

    let (_, v) = send(&state, Method::GET, None, "/agentmux/cron", None).await;
    let job = v["jobs"].as_array().unwrap().iter().find(|j| j["id"] == id.as_str()).unwrap().clone();
    assert_eq!(job["last_run"]["id"], runs[0]["id"]);
    assert_eq!(job["catch_up"], "skip");
    assert!(job.get("next_fire").is_some());
}

/// The agent that ran a run reports its outcome; another agent can't; a
/// bad outcome is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_reported_by_the_agent_that_ran_it() {
    let (state, owner, other) = state();
    let id = create(&state, &owner, json!({})).await;
    let (_, v) = send(&state, Method::POST, None, &format!("/agentmux/cron/{id}/run"), Some(json!({}))).await;
    let run_id = v["run_id"].as_str().unwrap().to_string();
    let uri = format!("/agentmux/cron/runs/{run_id}/report");

    let (status, _) = send(&state, Method::POST, Some(&other), &uri, Some(json!({"outcome": "ok"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "neither the runner nor the owner");
    let (status, _) = send(&state, Method::POST, None, &uri, Some(json!({"outcome": "unknown"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The target agent itself (here: the owner may too).
    state.mstore.agent_def_insert(&mut {
        let mut def = test_agent_def("uid-jobs-target", "jobs-target", "claude", "agent", 1, "");
        def.slug = "jobs-target".into();
        def
    })
    .unwrap();
    let target = state.mstore.agent_token_ensure("uid-jobs-target").unwrap();
    let body = json!({"outcome": "findings", "summary": "2 stale branches", "links": ["https://example.com/r"]});
    let (status, v) = send(&state, Method::POST, Some(&target), &uri, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let run = state.identity_store.job_run_get(&run_id).unwrap().unwrap();
    assert_eq!((run.outcome.as_str(), run.summary.as_str()), ("findings", "2 stale branches"));
    assert_eq!(run.links["reported"][0], "https://example.com/r");

    let (status, _) = send(&state, Method::POST, None, "/agentmux/cron/runs/no-such-run/report", Some(json!({"outcome": "ok"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
