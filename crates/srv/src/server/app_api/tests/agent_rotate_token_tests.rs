// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents rotate_token` (`POST /api/v1/agent/rotate-token`), through its handler.

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::sagas::park_agents_tests::{seed_agent_block, seed_workspace};
use crate::server::tests::test_state;

/// Call the handler as the agent `caller` (none: a request with no agent token).
async fn call(state: &AppState, caller: Option<&str>, agent: &str) -> (StatusCode, Value) {
    let caller = caller.map(|uid| Extension(Caller::Agent { uid: uid.to_string() }));
    let req: RotateRequest = serde_json::from_value(json!({ "agent": agent })).unwrap();
    let resp = handle_agent_rotate_token(State(state.clone()), caller, Json(req)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn audited(slug: &str) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler().get_audit_log(usize::MAX).into_iter().filter(|e| e.source_agent.as_deref() == Some(slug)).collect()
}

/// A stopped agent with no pane, as `Agents create` leaves it.
fn made(state: &AppState, id: &str) {
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(json!({
        "id": id, "slug": id, "name": id, "icon": "robot", "provider": "claude", "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
}

/// The agent has a token, and the state's index holds it.
fn token_of(state: &AppState, id: &str) -> String {
    let token = state.mstore.agent_token_ensure(id).unwrap();
    state.mstore.attach_token_index().unwrap();
    token
}

fn holder_of(state: &AppState, token: &str) -> Option<String> {
    state.mstore.token_index().unwrap().uid_for(token)
}

#[tokio::test]
async fn a_request_with_no_agent_token_is_refused_so_nothing_is_the_operators() {
    let state = test_state();
    let (status, body) = call(&state, None, "anyone").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().unwrap().contains("Agents rotate_token acts as the calling agent"), "{body}");
}

#[tokio::test]
async fn rotating_your_own_token_is_refused_saying_who_can_do_it_and_changes_nothing() {
    let state = test_state();
    made(&state, "rt-self");
    let before = token_of(&state, "rt-self");
    let (status, body) = call(&state, Some("rt-self"), "rt-self").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let error = body["error"].as_str().unwrap();
    assert!(error.starts_with("SELF_ROTATE") && error.contains("ask another agent"), "{body}");
    assert_eq!(holder_of(&state, &before).as_deref(), Some("rt-self"), "the token is untouched");
    assert!(audited("rt-self").iter().any(|e| !e.success), "the refusal is recorded under the caller");
}

#[tokio::test]
async fn an_unknown_agent_is_refused_and_recorded() {
    let state = test_state();
    made(&state, "rt-caller-u");
    let (status, _) = call(&state, Some("rt-caller-u"), "no-such-agent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(audited("rt-caller-u").iter().any(|e| !e.success));
}

/// A stopped agent needs no restart: its row is replaced at once, the old value is refused, the new one works,
/// the agent holds one token, and the rotation is recorded as the caller. No answer holds a token value.
#[tokio::test]
async fn a_stopped_agent_is_rotated_with_no_restart_and_the_old_token_stops_working() {
    let state = test_state();
    made(&state, "rt-caller-a");
    made(&state, "rt-target-a");
    made(&state, "rt-bystander-a");
    let old = token_of(&state, "rt-target-a");
    let bystander = token_of(&state, "rt-bystander-a");
    assert_eq!(holder_of(&state, &old).as_deref(), Some("rt-target-a"));

    let (status, body) = call(&state, Some("rt-caller-a"), "rt-target-a").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["rotated"].clone(), body["restarted"].clone()), (json!(true), json!(false)), "{body}");
    assert!(body["note"].as_str().unwrap().contains("next started"), "{body}");

    let new = state.mstore.agent_token_load("rt-target-a").unwrap().unwrap();
    assert_ne!(new, old);
    assert_eq!(holder_of(&state, &old), None, "the old token is refused at once");
    assert_eq!(holder_of(&state, &new).as_deref(), Some("rt-target-a"), "the new token works");
    assert_eq!(holder_of(&state, &bystander).as_deref(), Some("rt-bystander-a"), "another agent's token is untouched");
    assert_eq!(state.mstore.token_index().unwrap().len(), 2, "one token per agent");

    let shown = body.to_string();
    assert!(!shown.contains(&old) && !shown.contains(&new), "the answer holds no token value");
    let recorded: Vec<_> = audited("rt-caller-a").into_iter().filter(|e| e.success).collect();
    assert_eq!(recorded.len(), 1, "recorded once, as the caller");
    assert!(!format!("{:?}", recorded).contains(&new), "the record holds no token value");
}

/// An agent with a block but no process is also stopped: the same, with its block named.
#[tokio::test]
async fn an_agent_with_a_block_and_no_process_is_rotated_without_a_restart() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "rt-caller-b").await;
    let target = seed_agent_block(&state, &tabs[0], "rt-target-b").await;
    let old = token_of(&state, "rt-target-b");

    let (status, body) = call(&state, Some("rt-caller-b"), "rt-target-b").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["rotated"].clone(), body["restarted"].clone()), (json!(true), json!(false)), "{body}");
    assert_eq!(holder_of(&state, &old), None);
    assert!(state.mstore.get::<crate::backend::obj::Block>(&target).unwrap().is_some(), "its pane stays");
}

/// The window's outcome: the agent stopped, so its token is replaced; the agent has no block to restart in here,
/// which is said, and the caller is told and recorded.
#[tokio::test]
async fn after_the_window_a_stopped_agent_gets_the_new_token() {
    let state = test_state();
    made(&state, "rt-caller-c");
    made(&state, "rt-target-c");
    let old = token_of(&state, "rt-target-c");
    let actor = actor_of(&state, Some(&Caller::Agent { uid: "rt-caller-c".into() }), "rotate_token").ok().unwrap();

    settle(&state, &actor, "rt-target-c", "rt-target-c", "req-c", Ok(())).await;

    let new = state.mstore.agent_token_load("rt-target-c").unwrap().unwrap();
    assert_ne!(new, old);
    assert_eq!(holder_of(&state, &old), None);
    assert_eq!(holder_of(&state, &new).as_deref(), Some("rt-target-c"));
    assert!(audited("rt-caller-c").iter().any(|e| e.success), "recorded as the caller");
}

/// The person kept the agent running: nothing is replaced, so the running agent keeps a working token, and the
/// caller's record says it was not done.
#[tokio::test]
async fn when_the_user_keeps_it_running_the_token_is_unchanged() {
    let state = test_state();
    made(&state, "rt-caller-d");
    made(&state, "rt-target-d");
    let old = token_of(&state, "rt-target-d");
    let actor = actor_of(&state, Some(&Caller::Agent { uid: "rt-caller-d".into() }), "rotate_token").ok().unwrap();

    settle(&state, &actor, "rt-target-d", "rt-target-d", "req-d", Err("its user chose to keep it running".into())).await;

    assert_eq!(state.mstore.agent_token_load("rt-target-d").unwrap().as_deref(), Some(old.as_str()));
    assert_eq!(holder_of(&state, &old).as_deref(), Some("rt-target-d"));
    assert!(audited("rt-caller-d").iter().any(|e| !e.success));
}

#[tokio::test]
async fn the_wait_for_a_stop_ends_when_the_user_keeps_it_or_the_request_is_unknown() {
    let state = test_state();
    let pending = crate::sagas::pending_shutdown::request(
        &state,
        "blk-rt-keep",
        "rt-caller-e",
        "AgentStop",
        "",
        crate::sagas::pending_shutdown::Action::Stop { signal: None, stop_background: None },
    );
    assert_eq!(crate::sagas::pending_shutdown::keep(&state, "blk-rt-keep", &pending.request_id), "kept_by_user");
    let kept = wait_for_stop(&pending.request_id).await.unwrap_err();
    assert!(kept.contains("keep it running"), "{kept}");
    let lost = wait_for_stop("no-such-request").await.unwrap_err();
    assert!(lost.contains("lost"), "{lost}");
}
