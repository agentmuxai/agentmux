// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/aws/grants` and `POST /api/v1/aws/grants/decide`: who may read the list and who may answer.

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use serde_json::Value;

fn agent(state: &AppState, agent_type: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, &format!("Grants {}", &id[..8]), "claude", agent_type, 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    id
}

fn who(uid: &str) -> Option<Extension<Caller>> {
    Some(Extension(Caller::Agent { uid: uid.to_string() }))
}

fn decide_body(decision: &str) -> Json<DecideRequest> {
    Json(serde_json::from_value(json!({ "grant": "deploy", "change": "c1-1", "decision": decision, "reason": "because" })).unwrap())
}

async fn status_of(resp: Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn nobody_without_an_agent_token_reads_or_answers() {
    let state = test_state();
    let (status, _) = status_of(handle_aws_grants(State(state.clone()), None).await).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = status_of(handle_aws_grant_decide(State(state.clone()), None, decide_body("allow")).await).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = status_of(handle_aws_grant_decide(State(state), who("no-such-agent"), decide_body("allow")).await).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_container_agent_cannot_answer_for_the_grants_it_may_use() {
    let state = test_state();
    let container = agent(&state, "container");
    let (status, body) = status_of(handle_aws_grant_decide(State(state), who(&container), decide_body("allow")).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().contains("native agent"), "{body}");
}

#[tokio::test]
async fn a_native_agent_must_say_allow_or_remove() {
    let state = test_state();
    let native = agent(&state, "native");
    let (status, body) = status_of(handle_aws_grant_decide(State(state), who(&native), decide_body("maybe")).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("allow or remove"), "{body}");
}
