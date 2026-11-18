// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent asks to be given a login (`Vault` `share`), and a login it generates
//! (spec SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md §5, A2 and A8):
//! what the person is shown, that only their yes makes the share, and that an agent asks for itself only.

use std::time::Duration;

use super::identity_tests::{agent, call, keyless, login, setup};
use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;
use crate::backend::vault::uses::{self, Decision, UseAnswer, UseRequest};

const REASON: &str = "sign in to the dashboard to read the build status";

fn shared_of(state: &crate::server::AppState, id: &str) -> Option<String> {
    let row = state.identity_store.vault_get(id).unwrap().unwrap();
    vault::signins::fields_of(&row).get("shared").cloned()
}

fn lines_of(state: &crate::server::AppState, id: &str) -> Vec<crate::backend::vault::VaultActivity> {
    vault::history::history_for(&state.identity_store, id, vault::history::Asker::Person, 50).unwrap()
}

/// The agent's `Vault share` request, made in the background: the answer comes once the person has.
fn ask(app: &axum::Router, token: &str, slug: &str, body: Value) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let (app, token) = (app.clone(), token.to_string());
    let mut body = body;
    body["auth"] = keyless(slug);
    tokio::spawn(async move { call(&app, Some(&token), "/api/v1/vault/share", body).await })
}

/// The card the agent's request puts up for the person.
async fn card_of(id: &str, slug: &str) -> UseRequest {
    for _ in 0..200 {
        if let Some(card) = uses::uses().list().into_iter().find(|u| u.share && u.entry_id == id && u.agent == slug) {
            return card;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no share card for {slug} on {id}");
}

fn no_card(id: &str, slug: &str) -> bool {
    !uses::uses().list().iter().any(|u| u.share && u.entry_id == id && u.agent == slug)
}

async fn host(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().uri("/api/v1/host/vault").method("POST").header("X-AuthKey", "test-secret-key").header("X-Host-Token", "host-ipc-token").header("Content-Type", "application/json").body(Body::from(body.to_string())).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
async fn a2_an_agents_request_shows_the_person_the_login_and_only_their_yes_makes_the_share() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-ask-a");
    let id = login(&app).await;
    let asked = ask(&app, &token, "vid-ask-a", json!({ "id": id, "reason": REASON }));
    let card = card_of(&id, "vid-ask-a").await;
    // What the person is shown: the login, its site, the user name masked, and what the agent says it is for.
    assert_eq!((card.entry_label.as_str(), card.origin.as_str(), card.agent_reason.as_str()), ("Dashboard", "https://dash.example.com", REASON));
    assert!(!card.field.contains("alice"), "the user name is masked: {}", card.field);
    assert!(!card.offer_always && !uses::goes_ahead(&card), "no Always, and silence is a no");
    assert_eq!(shared_of(&state, &id), None, "nothing is shared while the person is asked");
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Once));
    let (s, out) = asked.await.unwrap();
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["status"], "shared", "{out}");
    assert_eq!(shared_of(&state, &id).as_deref(), Some("uid-vid-ask-a"));
    // The agent is told the login and its site, never the user name or the password.
    assert!(!out.to_string().contains("alice") && !out.to_string().contains("pw-one-of-a-kind"), "{out}");
    let lines = lines_of(&state, &id);
    let asked_line = lines.iter().find(|l| l.action == vault::shares::ASKED_TO_BE_SHARED).expect("the request is in the history");
    assert_eq!((asked_line.actor.as_str(), asked_line.outcome.as_str(), asked_line.reason.as_str()), ("vid-ask-a", "done", REASON));
    assert!(lines.iter().any(|l| l.action == "shared" && l.actor == vault::PERSON), "the share is the person's line");
}

#[tokio::test]
async fn a2_a_no_or_silence_shares_nothing_and_says_so() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-ask-b");
    let id = login(&app).await;
    let asked = ask(&app, &token, "vid-ask-b", json!({ "id": id, "reason": REASON }));
    let card = card_of(&id, "vid-ask-b").await;
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Deny));
    let (s, out) = asked.await.unwrap();
    assert_eq!(s, StatusCode::FORBIDDEN, "{out}");
    assert!(out["error"].as_str().unwrap().contains("nothing was shared"), "{out}");
    assert_eq!(shared_of(&state, &id), None);
    let line = lines_of(&state, &id).into_iter().find(|l| l.action == vault::shares::ASKED_TO_BE_SHARED).expect("the refusal is in the history");
    assert_eq!(line.outcome, "denied");
    // Nobody answering is a no too: a wait that runs out shares nothing.
    let silent = ask(&app, &token, "vid-ask-b", json!({ "id": id, "reason": REASON, "wait_secs": 1 }));
    let (s, out) = silent.await.unwrap();
    assert_eq!(s, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(shared_of(&state, &id), None);
}

#[tokio::test]
async fn a2_an_agent_asks_for_itself_only() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-ask-c");
    let _other = agent(&state, "vid-ask-other");
    let id = login(&app).await;
    for named in ["vid-ask-other", "uid-vid-ask-other", "*", "everyone"] {
        let (s, out) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": id, "reason": REASON, "agent": named, "auth": keyless("vid-ask-c") })).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{named}: {out}");
        assert!(out["error"].as_str().unwrap().contains("for itself only"), "{out}");
    }
    assert!(no_card(&id, "vid-ask-c") && no_card(&id, "vid-ask-other"), "nobody was asked");
    assert_eq!(shared_of(&state, &id), None);
    // Naming itself is no different from leaving it out.
    let asked = ask(&app, &token, "vid-ask-c", json!({ "id": id, "reason": REASON, "agent": "vid-ask-c" }));
    let card = card_of(&id, "vid-ask-c").await;
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Deny));
    assert_eq!(asked.await.unwrap().0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a2_a_caller_that_cannot_be_proven_asks_nobody() {
    let (app, state) = setup().await;
    let _ = agent(&state, "vid-ask-d");
    let id = login(&app).await;
    let (s, out) = call(&app, None, "/api/v1/vault/share", json!({ "id": id, "reason": REASON })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{out}");
    let (s, _) = call(&app, Some("amx_not-a-token"), "/api/v1/vault/share", json!({ "id": id, "reason": REASON, "auth": keyless("vid-ask-d") })).await;
    assert!(s.is_client_error());
    assert!(no_card(&id, "vid-ask-d"));
    assert_eq!(shared_of(&state, &id), None);
}

#[tokio::test]
async fn a2_a_login_already_shared_with_the_agent_asks_nobody_and_one_that_cannot_be_shared_is_refused() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-ask-e");
    let id = login(&app).await;
    assert_eq!(host(&app, json!({ "op": "share_add", "id": id, "agent": "uid-vid-ask-e" })).await.0, StatusCode::OK);
    let (s, out) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": id, "reason": REASON, "auth": keyless("vid-ask-e") })).await;
    assert_eq!((s, out["status"].as_str(), out["already"].as_bool()), (StatusCode::OK, Some("shared"), Some(true)), "{out}");
    assert!(no_card(&id, "vid-ask-e"), "no card for what is shared already");
    // A sign-in for a site's HTTP prompt is typed by the browser itself: no share, and no card.
    let (s, made) = host(&app, json!({ "op": "create", "kind": "login", "label": "Intranet", "fields": { "site": "https://intranet.example.com", "realm": "Staff" }, "parts": { "password": "pw-two" } })).await;
    assert_eq!(s, StatusCode::OK, "{made}");
    let realm_id = made["data"]["id"].as_str().unwrap().to_string();
    let (s, out) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": realm_id, "reason": REASON, "auth": keyless("vid-ask-e") })).await;
    assert!(s.is_client_error() && out["error"].as_str().unwrap().contains("can't be shared"), "{out}");
    assert!(no_card(&realm_id, "vid-ask-e"));
    // An entry the agent may not use is "no entry", and a reason is needed.
    let (s, out) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": "no-such-entry", "reason": REASON, "auth": keyless("vid-ask-e") })).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{out}");
    let (s, _) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": id, "auth": keyless("vid-ask-e") })).await;
    assert!(s.is_client_error());
}

#[tokio::test]
async fn a7_a_login_changed_while_the_person_was_asked_is_not_shared() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-ask-f");
    let id = login(&app).await;
    let asked = ask(&app, &token, "vid-ask-f", json!({ "id": id, "reason": REASON }));
    let card = card_of(&id, "vid-ask-f").await;
    // The site is edited after the card was shown: the person said yes to another site.
    let (s, out) = host(&app, json!({ "op": "update", "id": id, "label": "Dashboard", "fields": { "site": "https://elsewhere.example.net", "username": "alice" }, "tags": [], "parts": {}, "clear": [] })).await;
    assert_eq!(s, StatusCode::OK, "{out}");
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Once));
    let (s, out) = asked.await.unwrap();
    assert!(s.is_client_error() && out["error"].as_str().unwrap().contains("changed while the person was asked"), "{out}");
    assert_eq!(shared_of(&state, &id), None, "nothing was shared");
}

#[tokio::test]
async fn generate_makes_a_login_for_the_agents_own_profile_and_leaves_where_it_is_offered_to_the_person() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-gen-a");
    let body = |fields: Value| json!({ "kind": "login", "label": "CI bot", "why": "an account for the CI dashboard", "fields": fields, "auth": keyless("vid-gen-a") });
    let (s, made) = call(&app, Some(&token), "/api/v1/vault/generate", body(json!({ "site": "https://ci.example.com", "username": "bot" }))).await;
    assert_eq!(s, StatusCode::OK, "{made}");
    let row = state.identity_store.vault_get(made["id"].as_str().unwrap()).unwrap().unwrap();
    let fields = vault::signins::fields_of(&row);
    let own = format!("profile:{}", crate::backend::browser_profiles_store::agent_profile_id("uid-vid-gen-a"));
    assert_eq!((fields.get("profile"), fields.get("site"), fields.get("username")), (Some(&own), Some(&"https://ci.example.com".to_string()), Some(&"bot".to_string())));
    // Where a sign-in is offered, and who it is shared with, are the person's.
    for field in ["realm", "proxy", "profile", "shared"] {
        let (s, out) = call(&app, Some(&token), "/api/v1/vault/generate", body(json!({ "site": "https://ci.example.com", field: "personal" }))).await;
        assert!(s.is_client_error(), "{field}: {out}");
        assert!(out["error"].as_str().unwrap().contains("the person sets it in the Vault pane"), "{field}: {out}");
    }
    // A key or a note has no profile to belong to.
    let (s, key) = call(&app, Some(&token), "/api/v1/vault/generate", json!({ "kind": "api-key", "label": "Signing key", "why": "sign the test webhooks", "auth": keyless("vid-gen-a") })).await;
    assert_eq!(s, StatusCode::OK, "{key}");
    let row = state.identity_store.vault_get(key["id"].as_str().unwrap()).unwrap().unwrap();
    assert!(!vault::signins::fields_of(&row).contains_key("profile"));
}
