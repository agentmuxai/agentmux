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

// SPEC_KANBAN_AGENTS_HOLD_AND_UNHOLD_2026_10_22.md: agents hold and unhold.

/// A router and the state behind it, to read a task's history.
fn app_and_state() -> (axum::Router, crate::server::AppState) {
    let state = crate::server::tests::test_state();
    (crate::server::build_router(state.clone()), state)
}

fn history(state: &crate::server::AppState, num: i64) -> Vec<(String, String, String)> {
    let id = state.identity_store.work_board_get(&num.to_string()).unwrap().unwrap().item.id;
    state.identity_store.work_events(&id, 100).unwrap().into_iter().map(|e| (e.kind, e.actor, e.body)).collect()
}

#[tokio::test]
async fn any_agent_holds_and_unholds_a_waiting_task_and_the_history_says_who() {
    let (app, state) = app_and_state();
    let (_, v) = post(&app, "/agentmux/work", json!({"title": "t", "payload": "p", "target_agent": "a1"})).await;
    let num = v["num"].as_i64().unwrap();

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a2", "reason": "waiting on a reply"})).await;
    assert_eq!((s, v["state"].as_str(), v["changed"].as_bool()), (StatusCode::OK, Some("held"), Some(true)), "another agent's task: {v}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["claimed"], false, "held: nobody takes it");
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a2"})).await;
    assert_eq!((s, v["changed"].as_bool()), (StatusCode::OK, Some(false)), "a repeat is not an error");

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/unhold"), json!({"agent_id": "a3", "reason": "it came"})).await;
    assert_eq!((s, v["state"].as_str(), v["for"].as_str()), (StatusCode::OK, Some("open"), Some("a1")));
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/unhold"), json!({"agent_id": "a3"})).await;
    assert_eq!((s, v["changed"].as_bool()), (StatusCode::OK, Some(false)));

    let rows = history(&state, num);
    let moves: Vec<_> = rows.iter().filter(|(k, _, _)| k == "held" || k == "unheld").collect();
    assert_eq!(moves.len(), 2, "one row per move, none for a repeat: {rows:?}");
    assert!(moves.iter().any(|(k, a, b)| k == "held" && a == "a2" && b == "waiting on a reply"));
    assert!(moves.iter().any(|(k, a, b)| k == "unheld" && a == "a3" && b == "it came"));
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["num"].as_i64(), Some(num));
}

#[tokio::test]
async fn only_its_holder_can_put_a_claimed_task_on_hold_and_doing_so_keeps_the_attempt() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "t", "payload": "p"})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (num, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a2"})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("a1"), "names who is working on it: {v}");
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "its holder, without the attempt");

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1", "attempt": attempt, "reason": "blocked on X"})).await;
    assert_eq!((s, v["state"].as_str(), v["for"].as_str()), (StatusCode::OK, Some("held"), Some("a1")), "{v}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": num.to_string()})).await;
    assert_eq!((c["claimed"].as_bool(), c["attempt"].as_i64()), (Some(true), Some(attempt)), "the pause gave the attempt back");
}

#[tokio::test]
async fn a_finished_or_questioned_task_cant_be_held_and_an_open_one_cant_be_unheld() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "t", "payload": "p", "review": true})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (num, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());
    post(&app, &format!("/agentmux/work/{num}/ask"), json!({"agent_id": "a1", "attempt": attempt, "text": "which?"})).await;
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1", "attempt": attempt})).await;
    assert_eq!(s, StatusCode::CONFLICT, "blocked: its question is the pause");
    post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": attempt, "text": "done"})).await;
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("review"), "handed in: {v}");
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/unhold"), json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "not on hold");
    let (s, _) = post(&app, "/agentmux/work/9999/hold", json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_named_claim_takes_a_held_task_and_says_why_when_it_cant() {
    let (app, state) = app_and_state();
    let (_, mine) = post(&app, "/agentmux/work", json!({"title": "mine", "payload": "p", "target_agent": "a1", "hold": true})).await;
    let (_, other) = post(&app, "/agentmux/work", json!({"title": "next", "payload": "p", "target_agent": "a1"})).await;
    let (_, theirs) = post(&app, "/agentmux/work", json!({"title": "theirs", "payload": "p", "target_agent": "a2"})).await;
    let num = mine["num"].as_i64().unwrap();

    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": format!("#{num}")})).await;
    assert_eq!((c["claimed"].as_bool(), c["num"].as_i64()), (Some(true), Some(num)), "that task, not the open one: {c}");
    let kinds: Vec<_> = history(&state, num).into_iter().map(|(k, _, _)| k).collect();
    assert!(kinds.contains(&"unheld".to_string()) && kinds.contains(&"claimed".to_string()), "{kinds:?}");

    let theirs = theirs["num"].as_i64().unwrap();
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": theirs.to_string()})).await;
    assert_eq!(c["claimed"], false);
    assert!(c["why"].as_str().unwrap().contains("a2"), "{c}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": num.to_string()})).await;
    assert!(c["why"].as_str().unwrap().contains("claimed"), "already taken: {c}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["num"], other["num"], "without id, claim is unchanged");
}

#[tokio::test]
async fn a_group_member_refused_by_name_is_told_the_real_reason() {
    let app = app();
    let (_, first) = post(&app, "/agentmux/work", json!({"title": "first", "payload": "p"})).await;
    let first = first["num"].as_i64().unwrap();
    let (_, t) = post(
        &app,
        "/agentmux/work",
        json!({"title": "group work", "payload": "p", "target_group": "g1", "after": [first.to_string()]}),
    )
    .await;
    let num = t["num"].as_i64().unwrap();
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": num.to_string(), "groups": ["g1"]})).await;
    assert_eq!(c["claimed"], false);
    assert!(c["why"].as_str().unwrap().contains(&format!("starts after #{first}")), "a member: {c}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a2", "id": num.to_string()})).await;
    assert!(c["why"].as_str().unwrap().contains("group g1"), "not a member: {c}");
}
