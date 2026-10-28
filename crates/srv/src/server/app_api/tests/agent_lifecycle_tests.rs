// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents start`, `stop` and `detach` through their handlers.

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::sagas::park_agents_tests::{in_tab, seed_agent_block, seed_workspace, set_always_available};
use crate::server::tests::test_state;

/// Call a handler as the agent `caller` (none: a request with no agent token).
async fn call(state: &AppState, which: &str, caller: Option<&str>, agent: &str) -> (StatusCode, Value) {
    let caller = caller.map(|uid| Extension(Caller::Agent { uid: uid.to_string() }));
    let req: LifecycleRequest = serde_json::from_value(json!({ "agent": agent })).unwrap();
    let resp = match which {
        "start" => handle_agent_start(State(state.clone()), caller, Json(req)).await,
        "stop" => handle_agent_stop(State(state.clone()), caller, Json(req)).await,
        _ => handle_agent_detach(State(state.clone()), caller, Json(req)).await,
    };
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn audited(slug: &str) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler().get_audit_log(usize::MAX).into_iter().filter(|e| e.source_agent.as_deref() == Some(slug)).collect()
}

#[tokio::test]
async fn a_request_with_no_agent_token_is_refused_so_nothing_is_the_operators() {
    let state = test_state();
    for which in ["start", "stop", "detach"] {
        let (status, body) = call(&state, which, None, "anyone").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{which}");
        assert!(body["error"].as_str().unwrap().contains(&format!("Agents {which} acts as the calling agent")), "{body}");
    }
}

#[tokio::test]
async fn an_unknown_agent_and_an_empty_name_are_refused_and_audited() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "lc-caller-a").await;
    let (status, _) = call(&state, "stop", Some("lc-caller-a"), "no-such-agent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = call(&state, "detach", Some("lc-caller-a"), "  ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let refused = audited("lc-caller-a");
    assert!(refused.iter().any(|e| !e.success), "the refused attempt is recorded under the caller");
}

#[tokio::test]
async fn detach_parks_the_agents_block_whatever_the_setting_says_and_a_second_one_changes_nothing() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let (target, _caller) = (
        seed_agent_block(&state, &tabs[0], "lc-target-b").await,
        seed_agent_block(&state, &tabs[0], "lc-caller-b").await,
    );
    set_always_available(&state, false);
    assert_eq!(in_tab(&state, &target).await.as_deref(), Some(tabs[0].as_str()));

    let (status, body) = call(&state, "detach", Some("lc-caller-b"), "lc-target-b").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["detached"], true);
    assert_eq!(body["block_id"], target.as_str());
    assert_eq!(in_tab(&state, &target).await.as_deref(), Some(""), "parked: still a block, in no tab");
    assert!(state.mstore.get::<crate::backend::obj::Block>(&target).unwrap().is_some(), "the block was not deleted");
    let row = agent_runtime::list(&state, 1).into_iter().find(|r| r.agent_id == "lc-target-b").unwrap();
    assert_eq!(row.block_id.as_deref(), Some(target.as_str()), "a parked block is still the agent's block in the list");

    let (status, body) = call(&state, "detach", Some("lc-caller-b"), "lc-target-b").await;
    assert_eq!((status, body["detached"].clone()), (StatusCode::OK, json!(false)));
    assert!(body["note"].as_str().unwrap().contains("already detached"), "{body}");
    assert_eq!(audited("lc-caller-b").iter().filter(|e| e.success).count(), 2, "both calls are recorded");
}

#[tokio::test]
async fn detach_of_an_agent_with_no_block_says_how_to_run_it() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "lc-caller-c").await;
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(json!({
        "id": "lc-blockless", "slug": "lc-blockless", "name": "Blockless", "icon": "robot", "provider": "claude",
        "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let (status, body) = call(&state, "detach", Some("lc-caller-c"), "lc-blockless").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body["error"].as_str().unwrap().contains("Agents start"), "{body}");
}

#[tokio::test]
async fn stop_refuses_your_own_agent_and_leaves_a_stopped_one_wanted_stopped() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "lc-caller-d").await;
    let target = seed_agent_block(&state, &tabs[0], "lc-target-d").await;

    let (status, body) = call(&state, "stop", Some("lc-caller-d"), "lc-caller-d").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().starts_with("SELF_STOP"), "{body}");

    // No process runs in the target's block in a test: already stopped, and recorded as wanted stopped.
    let (status, body) = call(&state, "stop", Some("lc-caller-d"), "lc-target-d").await;
    assert_eq!((status, body["stopped"].clone()), (StatusCode::OK, json!(false)), "{body}");
    let block = state.mstore.get::<crate::backend::obj::Block>(&target).unwrap().unwrap();
    assert_eq!(super::super::agent_attach::desired_of(&block).as_deref(), Some("stopped"));
}

/// A sender's sixth start in a minute is refused with the limit named, whatever the five before did.
#[tokio::test]
async fn the_sixth_start_in_a_minute_is_refused_naming_the_limit() {
    let state = test_state();
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(json!({
        "id": "lc-sleeper", "slug": "lc-sleeper", "name": "Sleeper", "icon": "robot", "provider": "claude",
        "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut caller: crate::backend::storage::AgentDefinition = serde_json::from_value(json!({
        "id": "lc-caller-e", "slug": "lc-caller-e", "name": "Caller E", "icon": "robot", "provider": "claude",
        "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut caller).unwrap();
    // The workspace has no tab, so each start fails after it was counted.
    for i in 0..5 {
        let (status, body) = call(&state, "start", Some("lc-caller-e"), "lc-sleeper").await;
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS, "start {i}: {body}");
    }
    let (status, body) = call(&state, "start", Some("lc-caller-e"), "lc-sleeper").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body["error"].as_str().unwrap().contains("5 per sender per minute"), "{body}");
    // Another sender is not held back by it.
    let mut other: crate::backend::storage::AgentDefinition = serde_json::from_value(json!({
        "id": "lc-caller-f", "slug": "lc-caller-f", "name": "Caller F", "icon": "robot", "provider": "claude",
        "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut other).unwrap();
    let (status, _) = call(&state, "start", Some("lc-caller-f"), "lc-sleeper").await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[test]
fn a_process_runs_in_every_state_but_stopped_and_error() {
    let mut row = AgentRuntime {
        agent_id: "a".into(),
        state: AgentRuntimeState::Idle,
        detail: None,
        resets_at: None,
        block_id: None,
        desired: agent_runtime::desired_of(AgentRuntimeState::Idle),
        name: "a".into(),
        provider: "claude".into(),
        icon: String::new(),
        color: None,
    };
    for (state, alive) in [
        (AgentRuntimeState::Stopped, false),
        (AgentRuntimeState::Error, false),
        (AgentRuntimeState::Idle, true),
        (AgentRuntimeState::Working, true),
        (AgentRuntimeState::WaitingOnUser, true),
        (AgentRuntimeState::Limited, true),
        (AgentRuntimeState::SignedOut, true),
    ] {
        row.state = state;
        assert_eq!(is_alive(&row), alive, "{state:?}");
    }
}
