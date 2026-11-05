// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents migrate_ids` (`POST /api/v1/agent/migrate-ids`), through the handler: who may ask and
//! for what. The migration itself is `backend::slug_migration`'s, tested there.

use super::*;
use crate::server::tests::test_state;
use serde_json::Value;

async fn call(caller: Option<&str>, body: Value) -> (StatusCode, Value) {
    let state = test_state();
    let caller = caller.map(|uid| Extension(Caller::Agent { uid: uid.to_string() }));
    let req: MigrateIdsRequest = serde_json::from_value(body).unwrap();
    let resp = handle_agent_migrate_ids(State(state), caller, Json(req)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[test]
fn an_agent_may_ask_for_four_steps_and_no_others() {
    assert_eq!(mode_of("plan"), Some(ForkSlugMigrationMode::Plan));
    assert_eq!(mode_of("stop"), Some(ForkSlugMigrationMode::Stop));
    assert_eq!(mode_of("apply"), Some(ForkSlugMigrationMode::Apply));
    assert_eq!(mode_of("rollback"), Some(ForkSlugMigrationMode::Rollback));
    // `restore` puts a whole backup back; it stays the person's, in Settings.
    assert_eq!(mode_of("restore"), None);
    assert_eq!(mode_of(""), None);
}

#[tokio::test]
async fn a_request_without_an_agent_token_is_refused_and_nothing_runs() {
    let (status, body) = call(None, json!({ "step": "apply" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().unwrap().contains("no agent token"), "{body}");
}

#[tokio::test]
async fn an_unknown_step_is_refused_before_anything_is_read() {
    let (status, body) = call(Some("some-agent"), json!({ "step": "restore" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("INVALID_ARG"), "{body}");
}

#[tokio::test]
async fn a_token_naming_no_agent_here_is_refused() {
    let (status, body) = call(Some("not-an-agent-on-this-machine"), json!({ "step": "plan" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().unwrap().contains("no agent on this machine"), "{body}");
}
