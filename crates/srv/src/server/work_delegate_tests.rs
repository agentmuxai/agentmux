// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's accept and send back, through the real handler: any registered
//! agent may, and the history carries the agent, never the operator.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::registry::{AgentSeat, DefinitionStore};

struct World {
    state: AppState,
    app: axum::Router,
    leader_uid: String,
    leader_slug: String,
    leader_name: String,
    _defs: tempfile::TempDir,
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::new_v4().simple().to_string()[..8])
}

/// An agent registered on its own pane.
fn world(role: Option<&str>) -> World {
    let state = crate::server::tests::test_state();
    let defs = tempfile::tempdir().unwrap();
    state.mstore.set_def_registry(Arc::new(DefinitionStore::open(defs.path().join("definitions")).unwrap()));
    let leader_uid = unique("uid");
    let name = unique("Lead");
    let mut def = crate::backend::storage::agents::test_agent_def(&leader_uid, &name, "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    if let Some(role) = role {
        let seat = AgentSeat { role: role.to_string(), set_at_ms: 5, ..Default::default() };
        state.mstore.agent_seat_set(&leader_uid, Some(&seat)).unwrap();
    }
    let leader_slug = state.mstore.agent_def_get(&leader_uid).unwrap().unwrap().slug;
    let leader_block = unique("block");
    get_global_handler()
        .register_agent_full(&name, &leader_block, None, 0, Some(&leader_slug), Some(&leader_uid), "test.delegate")
        .unwrap();
    let app = crate::server::build_router(state.clone());
    World { state, app, leader_uid, leader_slug, leader_name: name, _defs: defs }
}

impl World {
    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        let token = crate::server::tests::token_for(&self.state, &crate::server::tests::actor_in(&body));
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header("X-AuthKey", "test-secret-key")
            .header("X-Agent-Token", token)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// A POST carrying the leader's own token.
    async fn post_as_leader(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.state.mstore.attach_token_index().unwrap();
        let token = self.state.mstore.agent_token_ensure(&self.leader_uid).unwrap();
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header("X-AuthKey", "test-secret-key")
            .header("X-Agent-Token", token)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// A task `writer` handed in, now in Review; its number.
    async fn in_review(&self) -> i64 {
        let writer = unique("writer");
        let (_, made) = self.post("/agentmux/work", json!({"title": "Spec: X", "payload": "p", "target_agent": writer, "review": true})).await;
        let num = made["num"].as_i64().unwrap();
        let (_, c) = self.post("/agentmux/work/claim", json!({"agent_id": writer})).await;
        assert_eq!(c["num"].as_i64(), Some(num), "{c}");
        let submit = json!({"agent_id": writer, "attempt": c["attempt"], "text": "done"});
        let (s, v) = self.post(&format!("/agentmux/work/{num}/submit"), submit).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(self.state_of(num), work_state::REVIEW);
        num
    }

    fn state_of(&self, num: i64) -> String {
        let id = resolve_id(&self.state, &num.to_string());
        self.state.identity_store.work_queue_get(&id).unwrap().unwrap().state
    }

    /// The agent decides `verb` on task `num`.
    async fn decide(&self, verb: Verb, num: i64, text: &str) -> (StatusCode, Value) {
        let req = DecisionRequest {
            auth: UiAutomationAuth { agent_id: self.leader_slug.clone(), ts_secs: 0, sig: String::new() },
            text: text.to_string(),
        };
        let caller = Caller::Agent { uid: self.leader_uid.clone() };
        let resp = decide(&self.state, Some(&caller), &num.to_string(), req, verb).await;
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    fn events(&self, num: i64) -> Vec<crate::backend::storage::work_board::WorkEvent> {
        let id = resolve_id(&self.state, &num.to_string());
        self.state.identity_store.work_events(&id, 50).unwrap()
    }
}

#[tokio::test]
async fn any_agent_accepts_and_the_history_says_which() {
    // A seated worker, a specialist and an agent with no seat: none needs to be a leader.
    for role in [Some("leader"), Some("worker"), None] {
        let w = world(role);
        let num = w.in_review().await;
        let (status, body) = w.decide(Verb::Accept, num, "").await;
        assert_eq!(status, StatusCode::OK, "{role:?}: {body}");
        assert_eq!(w.state_of(num), work_state::DONE);
        let e = w.events(num).into_iter().find(|e| e.kind == "accepted").expect("an accepted line");
        assert_eq!((e.actor.as_str(), e.actor_kind.as_str()), (w.leader_name.as_str(), "delegate"), "{role:?}");
        assert!(!e.body.contains("operator"), "never the operator's: {}", e.body);
    }
}

#[tokio::test]
async fn an_agent_may_accept_its_own_work_and_the_history_says_so() {
    let w = world(Some("leader"));
    let (_, made) = w.post("/agentmux/work", json!({"title": "Mine", "payload": "p", "target_agent_uid": w.leader_uid, "review": true})).await;
    let num = made["num"].as_i64().unwrap();
    let id = resolve_id(&w.state, &num.to_string());
    w.state.identity_store.work_start_for(&id, &w.leader_slug, &w.leader_uid, 1, 120_000).unwrap().expect("claimed");
    let item = w.state.identity_store.work_queue_get(&id).unwrap().unwrap();
    w.state.identity_store.work_submit(&id, &w.leader_slug, &w.leader_uid, item.attempts, "done", 2).unwrap().expect("submitted");
    let (status, body) = w.decide(Verb::Accept, num, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(w.state_of(num), work_state::DONE);
    let e = w.events(num).into_iter().find(|e| e.kind == "accepted").expect("an accepted line");
    assert!(e.body.contains("its own work"), "{}", e.body);
}

#[tokio::test]
async fn a_task_must_be_in_review() {
    let w = world(None);
    let (_, made) = w.post("/agentmux/work", json!({"title": "Waiting", "payload": "p"})).await;
    let waiting = made["num"].as_i64().unwrap();
    let (status, body) = w.decide(Verb::Accept, waiting, "").await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("not_in_review")), "{body}");
}

#[tokio::test]
async fn text_that_looks_like_a_credential_is_refused_not_stored() {
    let w = world(None);
    let num = w.in_review().await;
    let (_, body) = w.decide(Verb::Accept, num, "password: hunter2-correct-horse").await;
    assert_eq!(body["code"], "sensitive_text", "{body}");
    assert!(w.events(num).iter().all(|e| !e.body.contains("hunter2")));
}

#[tokio::test]
async fn an_agent_sends_back_with_a_reason() {
    let w = world(None);
    let num = w.in_review().await;
    // The reason is required: it goes to the holder.
    let (_, body) = w.decide(Verb::SendBack, num, "  ").await;
    assert_eq!(body["code"], "reason_required", "{body}");
    assert_eq!(w.state_of(num), work_state::REVIEW);
    // The holder isn't running here, so the task waits again for it.
    let (status, body) = w.decide(Verb::SendBack, num, "it misses the tests").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(w.state_of(num), work_state::OPEN);
    let e = w.events(num).into_iter().find(|e| e.kind == "sent_back").expect("a sent_back line");
    assert_eq!((e.actor.as_str(), e.actor_kind.as_str()), (w.leader_name.as_str(), "delegate"));
    assert!(e.body.contains("it misses the tests") && !e.body.contains("operator"), "{}", e.body);
}

#[tokio::test]
async fn an_agent_cancels_through_the_route_as_itself() {
    let w = world(None);
    let (_, made) = w.post("/agentmux/work", json!({"title": "Drop me", "payload": "p", "target_agent": unique("someone")})).await;
    let num = made["num"].as_i64().unwrap();
    let (status, body) = w
        .post_as_leader("/agentmux/work/act", json!({"agent_id": w.leader_slug, "action": "cancel", "id": num.to_string(), "text": "not needed"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(w.state_of(num), work_state::CANCELLED);
    let e = w.events(num).into_iter().find(|e| e.kind == "cancelled").expect("a cancelled line");
    assert_eq!((e.actor.as_str(), e.actor_kind.as_str()), (w.leader_slug.as_str(), "agent"), "never the operator");
    // Filing is `Tasks enqueue`, not the Kanban window's create.
    let (_, body) = w.post_as_leader("/agentmux/work/act", json!({"agent_id": w.leader_slug, "action": "create", "title": "x"})).await;
    assert!(body["error"].as_str().unwrap_or_default().contains("Tasks enqueue"), "{body}");
}

#[tokio::test]
async fn the_act_route_does_not_decide_in_the_operators_name() {
    let w = world(None);
    let num = w.in_review().await;
    for action in ["accept", "send_back"] {
        let (status, body) = w
            .post_as_leader("/agentmux/work/act", json!({"agent_id": w.leader_slug, "action": action, "id": num.to_string(), "text": "because"}))
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(w.state_of(num), work_state::REVIEW, "{action} must not move the task");
    }
    assert!(w.events(num).iter().all(|e| e.actor_kind != "operator" || e.kind == "created"), "nothing is recorded as the operator");
}
