// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's agent routes end to end through the router (SPEC_WARDEN_KANBAN_2026_10_20.md).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

fn app() -> axum::Router {
    crate::server::build_router(crate::server::tests::test_state())
}

async fn post(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .uri(uri)
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn tasks_get_numbers_that_work_wherever_an_id_does() {
    let app = app();
    let (s, v) = post(&app, "/agentmux/work", json!({"title": "t", "payload": "p"})).await;
    assert_eq!(s, StatusCode::OK);
    let num = v["num"].as_i64().expect("a number");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["num"].as_i64(), Some(num));
    let attempt = c["attempt"].as_i64().unwrap();
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/heartbeat"), json!({"agent_id": "a1", "attempt": attempt})).await;
    assert_eq!(s, StatusCode::OK, "the number is accepted as the id");
}

#[tokio::test]
async fn finishing_a_task_names_the_next_one_waiting_for_the_agent() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "first", "payload": "p", "target_agent": "a1"})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (first, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());
    let (_, second) = post(&app, "/agentmux/work", json!({"title": "second", "payload": "p", "target_agent": "a1"})).await;
    post(&app, "/agentmux/work", json!({"title": "someone else's", "payload": "p", "target_agent": "a2"})).await;

    let (s, v) =
        post(&app, &format!("/agentmux/work/{first}/complete"), json!({"agent_id": "a1", "attempt": attempt, "result": "done"})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], "done", "no review asked for: done");
    assert_eq!(v["next"]["num"], second["num"]);
    assert_eq!(v["next"]["title"], "second");
    assert_eq!(v["waiting"], 1);
}

#[tokio::test]
async fn a_task_that_needs_review_is_handed_in_not_done_and_ask_blocks_it() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "t", "payload": "p", "review": true})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (num, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());

    let (s, _) = post(&app, &format!("/agentmux/work/{num}/update"), json!({"agent_id": "a1", "attempt": attempt + 1, "note": "x"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "a stale attempt");
    let (s, _) = post(
        &app,
        &format!("/agentmux/work/{num}/update"),
        json!({"agent_id": "a1", "attempt": attempt, "note": "half way", "checklist": [{"text": "a", "status": "completed"}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/ask"), json!({"agent_id": "a1", "attempt": attempt, "text": "which one?"})).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("blocked")));
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/ask"), json!({"agent_id": "a1", "attempt": attempt, "text": ""})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a question is required");

    let (s, v) =
        post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": attempt, "text": "PR #1"})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], "review", "the agent can't accept its own work");
    assert!(v["next"].is_null());
}

#[tokio::test]
async fn filing_for_later_can_hold_a_task_out_of_reach() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "later", "payload": "p", "hold": true})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["claimed"], false, "held: nobody takes it");
}
