// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sharing a login with an agent (spec SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md §5, A1 to A10):
//! who makes a share, and what an agent is told about it.

use super::identity_tests::{agent, call, keyless, login, setup, update};
use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

/// A host op, with or without the host's token.
async fn host(app: &axum::Router, with_host_token: bool, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder().uri("/api/v1/host/vault").method("POST").header("X-AuthKey", "test-secret-key").header("Content-Type", "application/json");
    if with_host_token {
        req = req.header("X-Host-Token", "host-ipc-token");
    }
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn shared_of(state: &crate::server::AppState, id: &str) -> Option<String> {
    let row = state.identity_store.vault_get(id).unwrap().unwrap();
    vault::signins::fields_of(&row).get("shared").cloned()
}

#[tokio::test]
async fn a2_only_the_persons_answer_makes_a_share_and_it_leaves_the_entry_as_it_was() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-pulp");
    let id = login(&app).await;
    let before = state.identity_store.vault_get(&id).unwrap().unwrap();
    let (s, out) = host(&app, true, None, json!({ "op": "share_add", "id": id, "agent": "uid-vid-pulp" })).await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(shared_of(&state, &id).as_deref(), Some("uid-vid-pulp"));
    let after = state.identity_store.vault_get(&id).unwrap().unwrap();
    assert_eq!((after.vault_id, after.sealed), (before.vault_id, before.sealed), "the Vault and the secret are as they were");
    let line = history_of_entry(&state, &id).into_iter().find(|l| l.action == "shared").expect("the share is in the entry's history");
    assert_eq!((line.actor.as_str(), line.target.as_str()), (vault::PERSON, "an agent"));

    let (s, _) = host(&app, true, None, json!({ "op": "share_remove", "id": id, "agent": "uid-vid-pulp" })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(shared_of(&state, &id), None);
    // An agent that reads the history sees a share happened and no name.
    let (_, history) = call(&app, Some(&token), "/api/v1/vault/history", json!({ "id": id, "auth": keyless("vid-pulp") })).await;
    assert!(history.to_string().contains("shared") && !history.to_string().contains("uid-vid-pulp"), "{history}");
}

#[tokio::test]
async fn a2_an_agent_cannot_make_widen_or_remove_a_share_by_any_other_call() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-pulp");
    let id = login(&app).await;
    // Its own update, naming itself or every agent.
    for value in ["uid-vid-pulp", "*"] {
        let (s, out) = call(&app, Some(&token), "/api/v1/vault/update", update(&id, "vid-pulp", json!({ "shared": value }))).await;
        assert!(s.is_client_error(), "{out}");
        assert!(out["error"].as_str().unwrap().contains("the person sets it in the Vault pane"), "{out}");
    }
    assert_eq!(shared_of(&state, &id), None, "nothing was shared");
    // The person's host op, by an agent's token with or without the host's.
    let op = json!({ "op": "share_add", "id": id, "agent": "uid-vid-pulp" });
    assert_eq!(host(&app, false, Some(&token), op.clone()).await.0, StatusCode::FORBIDDEN);
    assert_eq!(host(&app, false, None, op).await.0, StatusCode::FORBIDDEN);
    assert_eq!(shared_of(&state, &id), None);
    // Once the person has shared it, the agent can't take the share away or widen it either.
    host(&app, true, None, json!({ "op": "share_add", "id": id, "agent": "uid-vid-pulp" })).await;
    let (s, _) = call(&app, Some(&token), "/api/v1/vault/update", update(&id, "vid-pulp", json!({ "shared": "" }))).await;
    assert!(s.is_client_error());
    assert_eq!(shared_of(&state, &id).as_deref(), Some("uid-vid-pulp"));
    assert_eq!(host(&app, false, Some(&token), json!({ "op": "share_remove", "id": id, "agent": "uid-vid-pulp" })).await.0, StatusCode::FORBIDDEN);
    assert_eq!(shared_of(&state, &id).as_deref(), Some("uid-vid-pulp"));
}

#[tokio::test]
async fn a5_a_share_to_an_agent_the_vault_is_closed_to_is_refused() {
    let (app, state) = setup().await;
    let _ = agent(&state, "vid-pulp");
    let id = login(&app).await;
    let store = &state.identity_store;
    let closed = vault::vaults::create(store, "Work", vault::vaults::VaultAgents::None).unwrap();
    vault::vaults::move_entry(store, &id, &closed.id).unwrap();
    let (s, out) = host(&app, true, None, json!({ "op": "share_add", "id": id, "agent": "uid-vid-pulp" })).await;
    assert!(s.is_client_error(), "{out}");
    assert!(out["error"].as_str().unwrap().contains("open the Vault to it first"), "{out}");
    assert_eq!(shared_of(&state, &id), None);
    // Open to that agent by name: the share is made.
    vault::vaults::update(store, &closed.id, "Work", vault::vaults::VaultAgents::Named(vec!["vid-pulp".into()])).unwrap();
    assert_eq!(host(&app, true, None, json!({ "op": "share_add", "id": id, "agent": "uid-vid-pulp" })).await.0, StatusCode::OK);
    // An agent that isn't on this computer can't be named.
    let (s, out) = host(&app, true, None, json!({ "op": "share_add", "id": id, "agent": "uid-nobody" })).await;
    assert!(s.is_client_error() && out["error"].as_str().unwrap().contains("no agent"), "{out}");
}

#[tokio::test]
async fn a8_list_and_show_say_shared_with_you_and_not_with_whom() {
    let (app, state) = setup().await;
    let mine = agent(&state, "vid-pulp");
    let theirs = agent(&state, "vid-other");
    let id = login(&app).await;
    for who in ["uid-vid-pulp", "uid-vid-other"] {
        assert_eq!(host(&app, true, None, json!({ "op": "share_add", "id": id, "agent": who })).await.0, StatusCode::OK);
    }
    let (_, list) = call(&app, Some(&mine), "/api/v1/vault/list", json!({ "auth": keyless("vid-pulp") })).await;
    let entry = list["entries"].as_array().unwrap().iter().find(|e| e["id"] == id.as_str()).expect("listed").clone();
    assert_eq!(entry["shared_with_you"], true, "{list}");
    let (_, shown) = call(&app, Some(&mine), "/api/v1/vault/show", json!({ "id": id, "auth": keyless("vid-pulp") })).await;
    assert_eq!(shown["shared_with_you"], true, "{shown}");
    for said in [list.to_string(), shown.to_string()] {
        assert!(!said.contains("uid-vid-other") && !said.contains("uid-vid-pulp") && !said.contains("\"shared\""), "who it is shared with is not told: {said}");
    }
    // An agent it is not shared with is told nothing of it.
    let third = agent(&state, "vid-third");
    let (_, theirs_list) = call(&app, Some(&third), "/api/v1/vault/list", json!({ "auth": keyless("vid-third") })).await;
    assert!(theirs_list["entries"].as_array().unwrap().iter().all(|e| e.get("shared_with_you").is_none()), "{theirs_list}");
    let _ = theirs;
}

fn history_of_entry(state: &crate::server::AppState, id: &str) -> Vec<crate::backend::vault::VaultActivity> {
    vault::history::history_for(&state.identity_store, id, vault::history::Asker::Person, 50).unwrap()
}
