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
async fn a_grant_needs_a_built_service_and_no_binding() {
    let state = state_with_agent().await;
    let set = |service: &str| json!({ "op": "set", "account_id": "acc-1", "agent_id": "def-1", "service": service, "level": "read" });
    // The agent is not bound to the account: a grant is all it needs (spec D7).
    let (status, body) = host(&state, HOST_TOKEN, set("calendar")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = host(&state, HOST_TOKEN, json!({ "op": "set", "account_id": "acc-1", "agent_id": "nobody", "service": "calendar", "level": "read" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
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
    assert_eq!(
        body,
        json!({ "account": "me@example.com", "accounts": ["me@example.com"], "services": [{ "service": "calendar", "level": "write", "account": "me@example.com" }] })
    );
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
    // An agent with no grant hears how to get one, and Google isn't called.
    let (status, body) = body_of(handle_google_calendar(State(state.clone()), agent("def-1"), call()).await).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("Google grant"), "{body}");
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
    assert!(out["error"].as_str().unwrap().contains("no Google account has Drive turned on for you"), "{out}");
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

// ── Several accounts per agent, and grants by agents (spec D7) ──────────────

fn scopes_of(service: crate::backend::google::Service, level: GrantLevel) -> serde_json::Value {
    json!(service.scopes(level))
}

/// `state_with_agent`, plus a peer agent and a second account, both connected with Calendar writes
/// and Gmail reads.
async fn state_with_two_accounts() -> AppState {
    use crate::backend::google::Service;
    let state = state_with_agent().await;
    let mut peer = crate::backend::storage::agents::test_agent_def("def-2", "Peer", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut peer).unwrap();
    let mut both = scopes_of(Service::Calendar, GrantLevel::Write).as_array().unwrap().clone();
    both.extend(scopes_of(Service::Gmail, GrantLevel::Read).as_array().unwrap().clone());
    for (id, email) in [("acc-1", "me@example.com"), ("acc-2", "work@example.org")] {
        let mut acct = crate::identity::oauth_tokens::tests::oauth_account(id, "google", "cid");
        acct.context["email"] = json!(email);
        acct.context["granted_scopes"] = json!(both);
        state.identity_store.identity_upsert(&acct).unwrap();
    }
    state
}

async fn grant(state: &AppState, as_agent: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let body = Json(serde_json::from_value::<GrantBody>(body).unwrap());
    body_of(handle_google_grant(State(state.clone()), agent(as_agent), body).await).await
}

#[tokio::test]
async fn an_agent_can_grant_another_and_the_account_log_names_who_did_it() {
    let state = state_with_two_accounts().await;
    let (status, body) =
        grant(&state, "def-1", json!({ "account": "WORK@example.org", "agent": "peer", "service": "calendar", "level": "read" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "account": "work@example.org", "agent": "Peer", "service": "calendar", "level": "read" }));
    assert_eq!(state.identity_store.google_grant_get("def-2", "acc-2", "calendar").unwrap(), Some(GrantLevel::Read));
    // Recorded under the agent that granted, not the one that was granted.
    let calls = state.identity_store.google_calls("acc-2", 10).unwrap();
    let line = calls.iter().find(|c| c.action == "grant").expect("the grant is logged");
    assert_eq!((line.agent_name.as_str(), line.ids.as_str(), line.service.as_str()), ("Naki", "Peer:read", "calendar"));
    // No agent named means the caller itself; ungrant takes it away again.
    let (status, _) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "gmail", "level": "read" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-1", "gmail").unwrap(), Some(GrantLevel::Read));
    let (status, _) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "gmail", "level": "off" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-1", "gmail").unwrap(), None);
}

#[tokio::test]
async fn a_grant_the_consent_cannot_back_is_refused_with_what_to_do() {
    let state = state_with_two_accounts().await;
    // Gmail was connected for reads only: a write grant would fail on every call.
    let (status, body) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "gmail", "level": "write" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let why = body["error"].as_str().unwrap();
    assert!(why.contains("reconnects it") && why.contains("supports: read"), "{why}");
    // Drive was never consented to at all.
    let (_, body) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "drive", "level": "read" })).await;
    assert!(body["error"].as_str().unwrap().contains("supports: none"), "{body}");
    // An unknown account lists the connected ones; an unknown agent and a service not built are named.
    let (_, body) = grant(&state, "def-1", json!({ "account": "x@example.com", "service": "gmail", "level": "read" })).await;
    let why = body["error"].as_str().unwrap();
    assert!(why.contains("me@example.com") && why.contains("work@example.org"), "{why}");
    let (_, body) = grant(&state, "def-1", json!({ "account": "me@example.com", "agent": "ghost", "service": "gmail", "level": "read" })).await;
    assert!(body["error"].as_str().unwrap().contains("no agent"), "{body}");
    let (_, body) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "slides", "level": "read" })).await;
    assert!(body["error"].as_str().unwrap().contains("calendar, drive or gmail"), "{body}");
    // Nothing was granted by any of them, and a call with no agent token cannot grant.
    assert!(state.identity_store.google_grants_for_agent("def-1").unwrap().is_empty());
    let body = Json(serde_json::from_value::<GrantBody>(json!({ "account": "me@example.com", "service": "gmail", "level": "read" })).unwrap());
    let (status, _) = body_of(handle_google_grant(State(state.clone()), None, body).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_agent_holding_two_accounts_names_one_and_the_services_list_both() {
    use crate::backend::google::{access, Service};
    let state = state_with_two_accounts().await;
    let def = state.mstore.agent_def_get("def-1").unwrap().unwrap();
    let pick = |asked: Option<&str>| access::access_for(&state.mstore, &state.id_store, &state.identity_store, &def, Service::Gmail, asked);
    // Nothing granted: told how to get it.
    assert!(pick(None).unwrap_err().contains("Google grant"));
    state.identity_store.google_grant_set("def-1", "acc-1", "gmail", Some(GrantLevel::Read), 1).unwrap();
    // One account: used without naming it. Naming another says what is held.
    assert_eq!(pick(None).unwrap().email(), "me@example.com");
    assert!(pick(Some("work@example.org")).unwrap_err().contains("You hold it on: me@example.com"));
    state.identity_store.google_grant_set("def-1", "acc-2", "gmail", Some(GrantLevel::Read), 1).unwrap();
    // Two: the agent must name one, in any case.
    let why = pick(None).unwrap_err();
    assert!(why.contains("several") && why.contains("me@example.com, work@example.org"), "{why}");
    assert_eq!(pick(Some("Work@Example.org")).unwrap().email(), "work@example.org");
    let (_, body) = body_of(handle_google_self_services(State(state.clone()), agent("def-1")).await).await;
    assert_eq!(body["accounts"], json!(["me@example.com", "work@example.org"]));
    assert_eq!(body["services"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn status_says_what_each_account_supports_who_holds_it_and_your_own_level() {
    let state = state_with_two_accounts().await;
    state.identity_store.google_grant_set("def-1", "acc-1", "calendar", Some(GrantLevel::Write), 1).unwrap();
    state.identity_store.google_grant_set("def-2", "acc-1", "calendar", Some(GrantLevel::Read), 1).unwrap();
    let (status, body) = body_of(handle_google_status(State(state.clone()), agent("def-1")).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["you"], "Naki");
    let me = &body["accounts"][0];
    assert_eq!(me["account"], "me@example.com");
    assert_eq!(me["services"]["calendar"]["connected_for"], "write");
    assert_eq!(me["services"]["calendar"]["yours"], "write");
    assert_eq!(me["services"]["calendar"]["holders"].as_array().unwrap().len(), 2);
    assert_eq!(me["services"]["gmail"]["connected_for"], "read");
    assert_eq!(me["services"]["gmail"]["yours"], "off");
    assert_eq!(me["services"]["drive"]["connected_for"], "none");
    assert_eq!(body["accounts"][1]["account"], "work@example.org");
    let (status, _) = body_of(handle_google_status(State(state.clone()), None).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn one_email_connected_twice_grants_on_the_connection_whose_consent_backs_it() {
    use crate::backend::google::Service;
    let state = state_with_two_accounts().await;
    // A second connection of me@example.com that only has Drive; "acc-1" (Calendar write, Gmail read) sorts first.
    let mut drive = crate::identity::oauth_tokens::tests::oauth_account("acc-0", "google", "cid");
    drive.context["email"] = json!("me@example.com");
    drive.context["granted_scopes"] = scopes_of(Service::Drive, GrantLevel::Read);
    state.identity_store.identity_upsert(&drive).unwrap();
    let (status, body) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "gmail", "level": "read" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-1", "gmail").unwrap(), Some(GrantLevel::Read));
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-0", "gmail").unwrap(), None);
    // Drive lands on the Drive-only connection; turning it off finds the same one.
    let (status, _) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "drive", "level": "read" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-0", "drive").unwrap(), Some(GrantLevel::Read));
    let (status, _) = grant(&state, "def-1", json!({ "account": "me@example.com", "service": "drive", "level": "off" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.identity_store.google_grant_get("def-1", "acc-0", "drive").unwrap(), None);
}
