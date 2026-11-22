// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who a Vault call is from (spec SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md §7, C1 to C6): a
//! container agent holds a token and no signing key, and is proven by its token as `Browser` proves it, on every
//! route that records or asks on an agent's behalf.

use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

const HOST: (&str, &str) = ("X-Host-Token", "host-ipc-token");

/// A state with agent `slug` (uid `uid-<slug>`) running in pane `b-<slug>`, and its token.
pub(super) fn agent(state: &crate::server::AppState, slug: &str) -> String {
    let uid = format!("uid-{slug}");
    let mut def = crate::backend::storage::agents::test_agent_def(&uid, slug, "claude", "agent", 1, "");
    def.slug = slug.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.attach_token_index().unwrap();
    state.reactive_handler.register_agent_full(slug, &format!("b-{slug}"), None, 0, None, Some(&uid), "test.vault_identity").unwrap();
    state.mstore.agent_token_ensure(&uid).unwrap()
}

pub(super) async fn call(app: &axum::Router, token: Option<&str>, uri: &str, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder().uri(uri).method("POST").header("X-AuthKey", "test-secret-key").header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

/// The identity a container agent sends: its name, and a signature that verifies nowhere (it has no key).
pub(super) fn keyless(slug: &str) -> Value {
    json!({ "agent_id": slug, "ts_secs": agentmux_common::time::now_ms() / 1000, "sig": "no-key" })
}

pub(super) async fn setup() -> (axum::Router, crate::server::AppState) {
    crate::backend::vault::seal::use_test_key();
    let state = crate::server::tests::test_state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
    (crate::server::build_router(state.clone()), state)
}

pub(super) async fn login(app: &axum::Router) -> String {
    let body = json!({ "op": "create", "kind": "login", "label": "Dashboard", "fields": { "site": "https://dash.example.com", "username": "alice" }, "parts": { "password": "pw-one-of-a-kind" } });
    let req = Request::builder().uri("/api/v1/host/vault").method("POST").header("X-AuthKey", "test-secret-key").header("X-Host-Token", HOST.1).header("Content-Type", "application/json").body(Body::from(body.to_string())).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    v["data"]["id"].as_str().unwrap().to_string()
}

pub(super) fn update(id: &str, slug: &str, fields: Value) -> Value {
    json!({ "id": id, "fields": fields, "tags": ["dashboard"], "reason": "tag the sign-in for the dashboard task", "auth": keyless(slug) })
}

fn history_of(state: &crate::server::AppState, id: &str) -> Vec<crate::backend::vault::VaultActivity> {
    vault::history::history_for(&state.identity_store, id, vault::history::Asker::Person, 50).unwrap()
}

// C1, C2: a container agent's update is recorded under its own identity, like a native agent's.
#[tokio::test]
async fn c2_a_token_only_agents_update_is_recorded_under_its_identity() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-pulp");
    let id = login(&app).await;
    let (s, body) = call(&app, Some(&token), "/api/v1/vault/update", update(&id, "vid-pulp", json!({ "notes": "the dashboard" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let line = history_of(&state, &id).into_iter().find(|l| l.action == "changed details").expect("the change is in the history");
    assert_eq!(line.actor, "agent:vid-pulp");
    assert!(!body.to_string().contains("pw-one-of-a-kind") && !format!("{line:?}").contains("pw-one-of-a-kind"));
}

// C4: a call that can't be proven changes nothing and is recorded as nothing.
#[tokio::test]
async fn c4_an_unproven_update_is_refused_and_records_nothing() {
    let (app, state) = setup().await;
    let id = login(&app).await;
    let (s, body) = call(&app, None, "/api/v1/vault/update", update(&id, "vid-nobody", json!({ "notes": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().contains("which agent asks"), "{body}");
    assert!(history_of(&state, &id).iter().all(|l| l.action != "changed details"));
}

// C1: a token proves the agent it was issued to and no other.
#[tokio::test]
async fn c1_a_token_for_one_agent_does_not_verify_another() {
    let (app, state) = setup().await;
    let alpha = agent(&state, "vid-alpha");
    agent(&state, "vid-beta");
    let id = login(&app).await;
    let (s, body) = call(&app, Some(&alpha), "/api/v1/vault/update", update(&id, "vid-beta", json!({ "notes": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert!(history_of(&state, &id).iter().all(|l| l.action != "changed details"));
}

// C1: the token of an agent that is not running (stopped, deleted) proves nothing.
#[tokio::test]
async fn c1_the_token_of_an_agent_that_is_not_running_is_refused() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-gone");
    state.reactive_handler.unregister_agent("vid-gone");
    let id = login(&app).await;
    let (s, body) = call(&app, Some(&token), "/api/v1/vault/update", update(&id, "vid-gone", json!({ "notes": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let (s, _) = call(&app, Some("amx_not-a-token"), "/api/v1/vault/update", update(&id, "vid-gone", json!({ "notes": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// C6: where a login may be used is not an agent's to change, a verified one included.
#[tokio::test]
async fn c6_an_agents_update_of_the_site_changes_nothing() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-site");
    let id = login(&app).await;
    for field in ["site", "realm", "profile", "proxy"] {
        let (s, body) = call(&app, Some(&token), "/api/v1/vault/update", update(&id, "vid-site", json!({ field: "https://elsewhere.example.net" }))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{field}: {body}");
        assert!(body["error"].as_str().unwrap().contains("the person sets it"), "{body}");
    }
    let row = state.identity_store.vault_get(&id).unwrap().unwrap();
    assert!(row.fields.contains("https://dash.example.com") && !row.fields.contains("elsewhere"), "{}", row.fields);
}

// C5: a Vault closed to all but one named agent is that agent's by its token, as by its signature.
#[tokio::test]
async fn c5_a_vault_open_to_a_named_agent_is_seen_by_its_token() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-named");
    let other = agent(&state, "vid-other");
    let host = |op: Value| {
        Request::builder().uri("/api/v1/host/vault").method("POST").header("X-AuthKey", "test-secret-key").header("X-Host-Token", HOST.1).header("Content-Type", "application/json").body(Body::from(op.to_string())).unwrap()
    };
    let resp = app.clone().oneshot(host(json!({ "op": "vault_create", "name": "Named", "agents": "named", "agent_ids": ["vid-named"] }))).await.unwrap();
    let v: Value = serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    let vault = v["data"]["vault"]["id"].as_str().unwrap().to_string();
    let resp = app
        .clone()
        .oneshot(host(json!({ "op": "create", "kind": "api-key", "label": "Named key", "vault": vault, "fields": {}, "parts": { "value": "k-123" } })))
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    let id = v["data"]["id"].as_str().unwrap().to_string();

    let (_, mine) = call(&app, Some(&token), "/api/v1/vault/list", json!({ "auth": keyless("vid-named") })).await;
    assert!(mine.to_string().contains(&id), "the named agent sees it by its token: {mine}");
    let (_, theirs) = call(&app, Some(&other), "/api/v1/vault/list", json!({ "auth": keyless("vid-other") })).await;
    assert!(!theirs.to_string().contains(&id), "another agent does not: {theirs}");
}

/// A container agent: its pane's meta says it runs in a container, as an agent block's does.
fn container_agent(state: &crate::server::AppState, slug: &str) -> String {
    let uid = format!("uid-{slug}");
    let mut def = crate::backend::storage::agents::test_agent_def(&uid, slug, "claude", "agent", 1, "");
    def.slug = slug.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.attach_token_index().unwrap();
    let meta = [("agentMode", "container"), ("agentId", slug)].iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    let mut block = crate::backend::obj::Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() };
    state.mstore.insert(&mut block).unwrap();
    state.reactive_handler.register_agent_full(slug, &block.oid, None, 0, None, Some(&uid), "test.vault_identity").unwrap();
    state.mstore.agent_token_ensure(&uid).unwrap()
}

// A rotation runs its commands on the host: a container agent, proven by its token, is refused before anyone is asked.
#[tokio::test]
async fn c1_a_container_agents_rotation_is_refused_before_the_person_is_asked() {
    let (app, state) = setup().await;
    let token = container_agent(&state, "vid-box");
    let id = login(&app).await;
    let body = json!({ "id": id, "set": "echo set", "reason": "rotate the dashboard login", "auth": keyless("vid-box") });
    let (s, out) = call(&app, Some(&token), "/api/v1/vault/rotate", body).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{out}");
    assert!(out["error"].as_str().unwrap().contains("container agent can't"), "{out}");
    assert!(vault::uses::uses().list().iter().all(|u| u.entry_id != id), "nobody was asked");
    assert!(history_of(&state, &id).iter().all(|l| l.actor != "agent:vid-box"));
}
