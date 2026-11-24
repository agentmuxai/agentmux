// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents mute_quiet` through its handler: only the agent's lead, never itself, recorded as the caller.

use super::*;
use crate::sagas::park_agents_tests::{seed_agent_block, seed_workspace};
use crate::server::tests::test_state;

async fn call(state: &AppState, caller: Option<&str>, body: Value) -> (StatusCode, Value) {
    let caller = caller.map(|uid| Extension(Caller::Agent { uid: uid.to_string() }));
    let req: MuteRequest = serde_json::from_value(body).unwrap();
    let resp = handle_agent_mute_quiet(State(state.clone()), caller, Json(req)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// `worker` is a spawned worker of `lead`.
fn make_worker(state: &AppState, worker: &str, lead: &str) {
    state
        .mstore
        .agent_content_set(&crate::backend::storage::store::AgentContent {
            agent_id: worker.to_string(),
            content_type: crate::backend::agent_seat::WORKER_OF_CONTENT.into(),
            content: json!({ "agent": lead, "id": lead }).to_string(),
            updated_at: 1,
        })
        .unwrap();
}

#[tokio::test]
async fn a_request_with_no_agent_token_is_refused() {
    let state = test_state();
    let (status, body) = call(&state, None, json!({ "agent": "anyone", "minutes": 10 })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn the_agents_lead_mutes_it_for_a_stated_time_and_zero_lifts_it() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "mq-lead-a").await;
    seed_agent_block(&state, &tabs[0], "mq-worker-a").await;
    make_worker(&state, "mq-worker-a", "mq-lead-a");

    let before = agentmux_common::time::now_ms_u64();
    let (status, body) = call(&state, Some("mq-lead-a"), json!({ "agent": "mq-worker-a", "minutes": 40, "reason": "building the release" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["muted"].as_bool(), body["reason"].as_str()), (Some(true), Some("building the release")));
    let until = body["until_ms"].as_u64().unwrap();
    assert!(until >= before + 40 * 60_000 && until <= before + 41 * 60_000, "{until}");
    assert!(crate::backend::quiet_watch::mute_of("mq-worker-a", before + MIN).is_some());
    assert!(
        crate::backend::reactive::handler::get_global_handler().get_audit_log(usize::MAX).iter().any(|e| e.source_agent.as_deref() == Some("mq-lead-a") && e.success),
        "recorded under the lead, never as the operator"
    );

    let (status, body) = call(&state, Some("mq-lead-a"), json!({ "agent": "mq-worker-a", "minutes": 0 })).await;
    assert_eq!((status, body["lifted"].as_bool()), (StatusCode::OK, Some(true)), "{body}");
    assert!(crate::backend::quiet_watch::mute_of("mq-worker-a", before + MIN).is_none());
}

const MIN: u64 = 60_000;

#[tokio::test]
async fn only_the_leader_may_mute_and_never_the_agent_itself() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "mq-lead-b").await;
    seed_agent_block(&state, &tabs[0], "mq-worker-b").await;
    seed_agent_block(&state, &tabs[0], "mq-other-b").await;
    make_worker(&state, "mq-worker-b", "mq-lead-b");

    let (status, body) = call(&state, Some("mq-other-b"), json!({ "agent": "mq-worker-b", "minutes": 30 })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().contains("NOT_ITS_LEAD"), "{body}");
    let (status, body) = call(&state, Some("mq-worker-b"), json!({ "agent": "mq-worker-b", "minutes": 30 })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("SELF_MUTE"), "{body}");
    assert!(crate::backend::quiet_watch::mute_of("mq-worker-b", agentmux_common::time::now_ms_u64()).is_none(), "nothing was muted");
}

#[tokio::test]
async fn the_time_is_one_to_240_minutes() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "mq-lead-c").await;
    seed_agent_block(&state, &tabs[0], "mq-worker-c").await;
    make_worker(&state, "mq-worker-c", "mq-lead-c");
    let (status, body) = call(&state, Some("mq-lead-c"), json!({ "agent": "mq-worker-c", "minutes": 241 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("240"), "{body}");
}
