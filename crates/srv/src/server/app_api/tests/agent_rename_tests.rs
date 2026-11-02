// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents rename` (`POST /api/v1/agent/rename`), through the handler.

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::AgentDefinition;
use crate::server::tests::test_state;

fn agent(state: &AppState, name: &str) -> AgentDefinition {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, name, "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.agent_def_get(&id).unwrap().unwrap()
}

async fn call(state: &AppState, caller: Option<&AgentDefinition>, agent: &str, name: &str) -> (StatusCode, Value) {
    let caller = caller.map(|d| Extension(Caller::Agent { uid: d.id.clone() }));
    let req: RenameRequest = serde_json::from_value(json!({ "agent": agent, "name": name })).unwrap();
    let resp = handle_agent_rename(State(state.clone()), caller, Json(req)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn name_of(state: &AppState, id: &str) -> String {
    state.mstore.agent_def_get(id).unwrap().unwrap().name
}

fn audited(by: &AgentDefinition) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler()
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.source_agent.as_deref() == Some(by.slug.as_str()))
        .collect()
}

#[tokio::test]
async fn an_agent_renames_another_by_slug_or_id_and_the_old_name_still_reaches_it() {
    let state = test_state();
    let (me, pulse) = (agent(&state, "Renamer"), agent(&state, "Pulse"));
    let (status, body) = call(&state, Some(&me), &pulse.slug, "Pulp").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["name"].as_str(), body["was"].as_str()), (Some("Pulp"), Some("Pulse")));
    assert_eq!(body["slug"], json!(pulse.slug), "the slug is fixed when the agent launches");
    assert_eq!(body["agent_id"], json!(pulse.id));
    assert_eq!(name_of(&state, &pulse.id), "Pulp");
    assert_eq!(state.mstore.agent_def_get(&pulse.id).unwrap().unwrap().slug, pulse.slug);
    // The name it had still addresses it, so a message to the old name arrives.
    assert!(state.mstore.former_name_listers("Pulse").unwrap().contains(&pulse.id));
    // By id as well, and recorded under the caller, never the operator.
    let (status, body) = call(&state, Some(&me), &pulse.id, "Pulp 2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(name_of(&state, &pulse.id), "Pulp 2");
    let log = audited(&me);
    assert_eq!(log.iter().filter(|e| e.success).count(), 2);
    assert!(log.iter().any(|e| e.reason.as_deref().is_some_and(|r| r.contains("renamed Pulse"))), "{log:?}");
}

#[tokio::test]
async fn a_name_another_agent_has_or_had_is_refused_and_nothing_changes() {
    let state = test_state();
    let (me, a, b) = (agent(&state, "Renamer"), agent(&state, "Winget"), agent(&state, "Camper"));
    // The name another agent has now.
    let (status, body) = call(&state, Some(&me), &b.slug, "winget").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("already the name of another agent"), "{body}");
    assert_eq!(name_of(&state, &b.id), "Camper");
    // The name another agent was renamed from still addresses that agent.
    let (status, _) = call(&state, Some(&me), &a.slug, "Winny").await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&state, Some(&me), &b.slug, "Winget").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("still addresses another agent"), "{body}");
    assert_eq!(name_of(&state, &b.id), "Camper");
    assert!(audited(&me).iter().any(|e| !e.success), "a refusal is recorded too");
}

#[tokio::test]
async fn a_rename_needs_an_agent_token_a_real_agent_and_a_plain_name() {
    let state = test_state();
    let (me, pulse) = (agent(&state, "Renamer"), agent(&state, "Pulse"));
    let (status, body) = call(&state, None, &pulse.slug, "Pulp").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(name_of(&state, &pulse.id), "Pulse");
    let (status, _) = call(&state, Some(&me), "nobody-here", "Pulp").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for bad in ["", "   ", "two\nlines", &"x".repeat(65)] {
        let (status, body) = call(&state, Some(&me), &pulse.slug, bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}: {body}");
    }
    assert_eq!(name_of(&state, &pulse.id), "Pulse");
}

#[tokio::test]
async fn an_agent_may_rename_itself_to_a_new_spelling_of_its_own_name() {
    let state = test_state();
    let pulse = agent(&state, "Pulse");
    let (status, body) = call(&state, Some(&pulse), &pulse.slug, "PULSE").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(name_of(&state, &pulse.id), "PULSE");
}

#[tokio::test]
async fn every_name_another_agent_answers_to_is_taken_not_only_its_display_name() {
    let state = test_state();
    let (me, a, b) = (agent(&state, "Renamer"), agent(&state, "Not Found"), agent(&state, "Camper"));
    // Another agent's slug: a message addressed to it would reach two agents.
    let (status, body) = call(&state, Some(&me), &b.slug, &a.slug).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(name_of(&state, &b.id), "Camper");
    // The error's status comes from the kind of refusal, not from the words of the name it quotes.
    let (status, body) = call(&state, Some(&me), &b.slug, "not found").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // An agent's own slug is its own name to take.
    let (status, body) = call(&state, Some(&me), &b.slug, &b.slug).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
