// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/skill`: the token is required, only catalog skills can be changed, an edit
//! keeps what it leaves out, and a new skill reaches the agents created after it. The per-site
//! `m4.actor_*` counters move with a token, so these tests take the lock the counter test takes.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::storage::Skill;
use crate::server::routes::build_router;
use crate::server::tests::{test_state, token_for, M4_ACTOR_COUNTERS};
use crate::server::AppState;

const AGENT: &str = "sk-alpha";

async fn call(state: &AppState, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/agent/skill")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = build_router(state.clone()).oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn error_of(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

fn skill(id: &str, name: &str, is_global: bool) -> Skill {
    Skill {
        id: id.to_string(),
        name: name.to_string(),
        trigger: String::new(),
        skill_type: "agent-skill".to_string(),
        description: "when to use".to_string(),
        content: "the text".to_string(),
        is_global,
        created_at: 1,
        updated_at: 1,
    }
}

#[tokio::test]
async fn a_request_without_the_agents_token_is_refused() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let (_, v) = call(&state, None, json!({ "action": "list", "agent_id": AGENT })).await;
    assert!(error_of(&v).starts_with("FORBIDDEN"), "{v}");
}

#[tokio::test]
async fn an_edit_keeps_what_it_leaves_out_and_a_new_skill_reaches_later_agents() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);

    let (_, made) = call(
        &state,
        Some(&token),
        json!({ "action": "upsert", "agent_id": AGENT, "name": "tidy", "description": "when tidy", "content": "v1" }),
    )
    .await;
    let id = made["id"].as_str().expect(&made.to_string()).to_string();

    let (_, listed) = call(&state, Some(&token), json!({ "action": "list", "agent_id": AGENT })).await;
    let row = listed.as_array().unwrap().iter().find(|r| r["id"] == id.as_str()).expect("listed");
    assert_eq!(row["all_agents"], false);
    assert_eq!(row["new_agents"], true);
    assert!(row.get("content").is_none(), "list carries no skill text");

    call(&state, Some(&token), json!({ "action": "upsert", "agent_id": AGENT, "skill_id": id, "content": "v2" })).await;
    let (_, got) = call(&state, Some(&token), json!({ "action": "get", "agent_id": AGENT, "skill_id": id })).await;
    assert_eq!(got["content"], "v2");
    assert_eq!(got["name"], "tidy");
    assert_eq!(got["description"], "when tidy");
}

#[tokio::test]
async fn a_private_skill_cannot_be_read_changed_or_given_away_here() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let uid = format!("uid-{AGENT}");
    state.mstore.skill_upsert_unique(&state.identity_store, &uid, &skill("sk-private", "mine", false), true).unwrap();

    for action in ["get", "delete", "bind", "unbind", "reach"] {
        let (_, v) = call(&state, Some(&token), json!({ "action": action, "agent_id": AGENT, "skill_id": "sk-private" })).await;
        assert!(error_of(&v).starts_with("FORBIDDEN"), "{action}: {v}");
    }
    let (_, v) = call(
        &state,
        Some(&token),
        json!({ "action": "upsert", "agent_id": AGENT, "skill_id": "sk-private", "content": "x" }),
    )
    .await;
    assert!(error_of(&v).starts_with("FORBIDDEN"), "{v}");
    assert_eq!(state.identity_store.skill_get("sk-private").unwrap().unwrap().content, "the text");
}

#[tokio::test]
async fn bind_with_no_target_binds_the_caller_and_reach_updates_only_what_is_given() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let (_, made) = call(&state, Some(&token), json!({ "action": "upsert", "agent_id": AGENT, "name": "bound" })).await;
    let id = made["id"].as_str().unwrap().to_string();

    call(&state, Some(&token), json!({ "action": "bind", "agent_id": AGENT, "skill_id": id })).await;
    let bound = state.mstore.effective_skills(&state.identity_store, &format!("uid-{AGENT}"));
    assert!(bound.iter().any(|s| s.name == "bound"));

    let (_, v) = call(&state, Some(&token), json!({ "action": "reach", "agent_id": AGENT, "skill_id": id, "all_agents": true })).await;
    assert_eq!(v["all_agents"], true);
    assert_eq!(v["new_agents"], true);
    let (_, v) = call(&state, Some(&token), json!({ "action": "reach", "agent_id": AGENT, "skill_id": id, "all_agents": false })).await;
    assert_eq!(v["all_agents"], false);
    assert_eq!(v["new_agents"], true);
}
