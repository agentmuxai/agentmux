// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::google_grants::GrantLevel;

const HOST_TOKEN: &str = "host-ipc-token";

async fn state_with_agent() -> AppState {
    let state = crate::server::tests::test_state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: HOST_TOKEN.into() });
    let mut def = crate::backend::storage::agents::test_agent_def("def-1", "Naki", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut acct = crate::identity::oauth_tokens::tests::oauth_account("acc-1", "google", "cid");
    acct.context["email"] = json!("me@example.com");
    state.identity_store.identity_upsert(&acct).unwrap();
    state
}

async fn body_of(r: Response) -> (StatusCode, serde_json::Value) {
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn agent(uid: &str) -> Option<Extension<Caller>> {
    Some(Extension(Caller::Agent { uid: uid.into() }))
}

fn host_headers(token: &str) -> axum::http::HeaderMap {
    let mut h = axum::http::HeaderMap::new();
    h.insert("x-host-token", token.parse().unwrap());
    h
}

async fn host(state: &AppState, token: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    body_of(handle_host_google(State(state.clone()), host_headers(token), Json(body)).await).await
}

#[tokio::test]
async fn only_the_host_can_set_a_grant() {
    let state = state_with_agent().await;
    state.identity_store.agent_identity_link("def-1", "acc-1", "google").unwrap();
    let set = json!({ "op": "set", "account_id": "acc-1", "agent_id": "def-1", "service": "calendar", "level": "read" });
    let (status, _) = host(&state, "an-agents-guess", set.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-1", "calendar").unwrap(), None);

    let (status, _) = host(&state, HOST_TOKEN, set).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-1", "calendar").unwrap(), Some(GrantLevel::Read));

    let (status, body) = host(&state, HOST_TOKEN, json!({ "op": "account", "account_id": "acc-1" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["agents"], json!([{ "agentId": "def-1", "name": "Naki", "levels": { "calendar": "read" } }]));
}

#[tokio::test]
async fn a_grant_needs_the_agent_bound_and_a_built_service() {
    let state = state_with_agent().await;
    let set = |service: &str| json!({ "op": "set", "account_id": "acc-1", "agent_id": "def-1", "service": service, "level": "read" });
    let (status, body) = host(&state, HOST_TOKEN, set("calendar")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("bind it first"), "{body}");
    state.identity_store.agent_identity_link("def-1", "acc-1", "google").unwrap();
    let (_, body) = host(&state, HOST_TOKEN, set("slides")).await;
    assert!(body["error"].as_str().unwrap().contains("yet"), "{body}");
    let (status, body) = host(&state, HOST_TOKEN, set("gmail")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_agent_sees_its_own_services_and_nothing_without_a_token() {
    let state = state_with_agent().await;
    state.identity_store.agent_identity_link("def-1", "acc-1", "google").unwrap();
    state.identity_store.google_grant_set("def-1", "acc-1", "calendar", Some(GrantLevel::Write), 1).unwrap();
    let (status, body) = body_of(handle_google_self_services(State(state.clone()), agent("def-1")).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "account": "me@example.com", "services": [{ "service": "calendar", "level": "write" }] }));
    let (_, body) = body_of(handle_google_self_services(State(state.clone()), None).await).await;
    assert_eq!(body, json!({ "account": null, "services": [] }));
}

#[tokio::test]
async fn a_calendar_call_without_an_agent_token_is_refused() {
    let state = state_with_agent().await;
    let call = || Json(serde_json::from_value::<CalendarBody>(json!({ "action": "list_events" })).unwrap());
    let (status, body) = body_of(handle_google_calendar(State(state.clone()), None, call()).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["error"].as_str().unwrap().contains("no agent token"));
    // An agent with no account bound hears how to get one, and Google isn't called.
    let (status, body) = body_of(handle_google_calendar(State(state.clone()), agent("def-1"), call()).await).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("Bind to agent"), "{body}");
}

#[tokio::test]
async fn the_panel_lists_an_agent_whose_account_comes_from_its_template_parent() {
    let state = state_with_agent().await;
    let mut tpl = crate::backend::storage::agents::test_agent_def("tpl-1", "Template", "claude", "host", 1, "");
    tpl.is_seeded = 1;
    state.mstore.agent_def_insert(&mut tpl).unwrap();
    let mut child = crate::backend::storage::agents::test_agent_def("child-1", "Child", "claude", "host", 1, "");
    child.parent_id = "tpl-1".into();
    state.mstore.agent_def_insert(&mut child).unwrap();
    state.identity_store.agent_identity_link("tpl-1", "acc-1", "google").unwrap();
    let (status, body) = host(&state, HOST_TOKEN, json!({ "op": "set", "account_id": "acc-1", "agent_id": "child-1", "service": "calendar", "level": "read" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = host(&state, HOST_TOKEN, json!({ "op": "account", "account_id": "acc-1" })).await;
    let agents = body["data"]["agents"].as_array().unwrap();
    let child = agents.iter().find(|a| a["agentId"] == "child-1").expect("the child is listed");
    assert_eq!(child["levels"]["calendar"], "read");
}

#[test]
fn an_agent_can_open_the_picker_once_a_minute_per_account() {
    let t0 = std::time::Instant::now();
    assert!(pick_throttle("throttle-a", t0).is_ok());
    let e = pick_throttle("throttle-a", t0 + std::time::Duration::from_secs(10)).unwrap_err();
    assert!(e.contains("a moment ago"), "{e}");
    assert!(pick_throttle("throttle-b", t0 + std::time::Duration::from_secs(10)).is_ok(), "another account is its own");
    assert!(pick_throttle("throttle-a", t0 + std::time::Duration::from_secs(61)).is_ok());
}

#[test]
fn a_picker_that_did_not_open_does_not_use_up_the_slot() {
    let t0 = std::time::Instant::now();
    assert!(pick_throttle("throttle-c", t0).is_ok());
    pick_release("throttle-c");
    assert!(pick_throttle("throttle-c", t0 + std::time::Duration::from_secs(1)).is_ok(), "a retry right after a failed start is allowed");
}

#[tokio::test]
async fn pick_files_needs_a_drive_grant_and_the_drive_route_needs_an_agent_token() {
    let state = state_with_agent().await;
    state.identity_store.agent_identity_link("def-1", "acc-1", "google").unwrap();
    let body = || Json(serde_json::from_value::<DriveBody>(json!({ "action": "pick_files" })).unwrap());
    let (status, out) = body_of(handle_google_drive(State(state.clone()), agent("def-1"), body()).await).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(out["error"].as_str().unwrap().contains("Drive is off for you"), "{out}");
    let (status, _) = body_of(handle_google_drive(State(state.clone()), None, body()).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_grant_can_be_set_on_drive_and_the_panel_lists_the_built_services() {
    let state = state_with_agent().await;
    state.identity_store.agent_identity_link("def-1", "acc-1", "google").unwrap();
    let (status, body) = host(&state, HOST_TOKEN, json!({ "op": "set", "account_id": "acc-1", "agent_id": "def-1", "service": "drive", "level": "read" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = host(&state, HOST_TOKEN, json!({ "op": "account", "account_id": "acc-1" })).await;
    assert_eq!(body["data"]["services"], json!(["calendar", "drive", "gmail"]));
    assert_eq!(body["data"]["agents"][0]["levels"]["drive"], "read");
}
