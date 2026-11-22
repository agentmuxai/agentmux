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
use crate::backend::vault::signins::Space;
use crate::backend::vault::uses::{self, Decision, UseAnswer, UseHttpPrompt, UseRequest};
use crate::server::vault_prompt_share as prompt_share;

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
async fn b3_an_unticked_share_makes_none_and_a_ticked_one_shares_with_the_agent() {
    use crate::server::vault_import::{share_made, Who};
    let (app, state) = setup().await;
    let _token = agent(&state, "vid-imp-a");
    let id = login(&app).await;
    let who = Who { agent: "vid-imp-a".into(), pane: String::new(), slug: "vid-imp-a".into(), uid: "uid-vid-imp-a".into(), container: false };
    let (made, said) = share_made(&state, &who, &id, false).await;
    assert!(!made && said.contains("unticked"), "{said}");
    assert_eq!(shared_of(&state, &id), None, "an unticked share makes none");
    let (made, said) = share_made(&state, &who, &id, true).await;
    assert!(made, "{said}");
    assert_eq!(shared_of(&state, &id).as_deref(), Some("uid-vid-imp-a"));
    assert!(lines_of(&state, &id).iter().any(|l| l.action == "shared"), "the share is in the history");
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
    assert_eq!((asked_line.actor.as_str(), asked_line.outcome.as_str(), asked_line.reason.as_str()), ("agent:vid-ask-a", "done", REASON));
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
    // A login saved for no site has nothing to sign in to: no share, and no card.
    let (s, made) = host(&app, json!({ "op": "create", "kind": "login", "label": "Spare", "fields": { "username": "spare" }, "parts": { "password": "pw-two" } })).await;
    assert_eq!(s, StatusCode::OK, "{made}");
    let spare_id = made["data"]["id"].as_str().unwrap().to_string();
    let (s, out) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": spare_id, "reason": REASON, "auth": keyless("vid-ask-e") })).await;
    assert!(s.is_client_error() && out["error"].as_str().unwrap().contains("saved for a site"), "{out}");
    assert!(no_card(&spare_id, "vid-ask-e"));
    // An entry the agent may not use is "no entry", and a reason is needed.
    let (s, out) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": "no-such-entry", "reason": REASON, "auth": keyless("vid-ask-e") })).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{out}");
    let (s, _) = call(&app, Some(&token), "/api/v1/vault/share", json!({ "id": id, "auth": keyless("vid-ask-e") })).await;
    assert!(s.is_client_error());
}

/// A sign-in for the prompt of realm "Staff" on the intranet, saved for Personal.
async fn prompt_login(app: &axum::Router) -> String {
    let fields = json!({ "site": "https://intranet.example.com", "realm": "Staff", "username": "alice" });
    let (s, made) = host(app, json!({ "op": "create", "kind": "login", "label": "Intranet", "fields": fields, "parts": { "password": "pw-two-of-a-kind" } })).await;
    assert_eq!(s, StatusCode::OK, "{made}");
    made["data"]["id"].as_str().unwrap().to_string()
}

/// That prompt, met in a tab of agent `slug`'s own browser profile: what the host's credential broker asks about.
fn prompt_space(slug: &str) -> Space {
    let profile = format!("profile:{}", crate::backend::browser_profiles_store::agent_profile_id(&format!("uid-{slug}")));
    Space { profile, origin: "https://intranet.example.com".into(), realm: "Staff".into(), proxy: false }
}

#[tokio::test]
async fn a11_a_yes_on_the_card_shares_a_prompt_sign_in_and_covers_the_prompt_that_waited() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-pr-a");
    let _other = agent(&state, "vid-pr-other");
    let id = prompt_login(&app).await;
    let space = prompt_space("vid-pr-a");
    assert!(prompt_share::lookup(&state, &space).unwrap().is_none(), "nothing answers the prompt before the share");
    let asked = ask(&app, &token, "vid-pr-a", json!({ "id": id, "reason": REASON }));
    let card = card_of(&id, "vid-pr-a").await;
    // What the person is shown: the prompt by its realm, the user name masked, an Always, and silence is a no.
    assert_eq!(card.prompt, Some(UseHttpPrompt { realm: "Staff".into(), proxy: false }));
    assert_eq!((card.entry_label.as_str(), card.origin.as_str(), card.agent_reason.as_str()), ("Intranet", "https://intranet.example.com", REASON));
    assert!(card.offer_always && !uses::goes_ahead(&card) && !card.field.contains("alice"), "{card:?}");
    assert!(prompt_share::lookup(&state, &space).unwrap().is_none(), "nothing answers while the person is asked");
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Once));
    let (s, out) = asked.await.unwrap();
    assert_eq!((s, out["status"].as_str(), out["prompt"].as_bool(), out["always"].as_bool()), (StatusCode::OK, Some("shared"), Some(true), Some(false)), "{out}");
    assert!(out["next"].as_str().unwrap().contains("load the page again"), "{out}");
    assert!(!out.to_string().contains("alice") && !out.to_string().contains("pw-two-of-a-kind"), "{out}");
    assert_eq!(shared_of(&state, &id).as_deref(), Some("uid-vid-pr-a"));

    // The host is told there is one, shared, with the user name masked; the yes just given covers this prompt.
    let found = prompt_share::lookup(&state, &space).unwrap().expect("the shared sign-in answers in the agent's own profile");
    assert!(found.granted && !found.username.contains("alice") && !found.agent.is_empty(), "{} {}", found.username, found.agent);
    let (user, password) = prompt_share::fill(&state, &space).unwrap().expect("the browser is given it");
    assert_eq!((user.as_str(), password.as_str()), ("alice", "pw-two-of-a-kind"));
    // One use: the next prompt asks the person again.
    assert!(!prompt_share::lookup(&state, &space).unwrap().unwrap().granted);
    let used = lines_of(&state, &id).into_iter().find(|l| l.action == vault::signins::SIGNED_IN).expect("the use is in the history");
    assert_eq!((used.actor.as_str(), used.outcome.as_str()), ("agent:vid-pr-a", "done"), "the use is the agent's line");

    // Another agent's profile, and Personal, are given nothing through this share.
    for other in [prompt_space("vid-pr-other"), Space { profile: "personal".into(), ..space.clone() }, Space { realm: "Admin".into(), ..space.clone() }] {
        assert!(prompt_share::lookup(&state, &other).unwrap().is_none(), "{other:?}");
        assert!(prompt_share::fill(&state, &other).unwrap().is_none(), "{other:?}");
    }
}

#[tokio::test]
async fn a11_always_on_the_card_answers_each_prompt_until_the_share_is_removed() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-pr-b");
    let id = prompt_login(&app).await;
    let space = prompt_space("vid-pr-b");
    let asked = ask(&app, &token, "vid-pr-b", json!({ "id": id, "reason": REASON }));
    let card = card_of(&id, "vid-pr-b").await;
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Always));
    let (s, out) = asked.await.unwrap();
    assert_eq!((s, out["always"].as_bool()), (StatusCode::OK, Some(true)), "{out}");
    for _ in 0..2 {
        assert!(prompt_share::lookup(&state, &space).unwrap().unwrap().granted, "an Always is not used up");
        assert!(prompt_share::fill(&state, &space).unwrap().is_some());
    }
    let (s, list) = host(&app, json!({ "op": "share_list", "id": id })).await;
    assert_eq!((s, list["data"]["shares"][0]["always"].as_bool()), (StatusCode::OK, Some(true)), "the person sees it on the login's row: {list}");
    // The person stops sharing: the prompt is answered by nothing, and sharing again starts by asking.
    assert_eq!(host(&app, json!({ "op": "share_remove", "id": id, "agent": "uid-vid-pr-b" })).await.0, StatusCode::OK);
    assert!(prompt_share::lookup(&state, &space).unwrap().is_none());
    assert!(prompt_share::fill(&state, &space).unwrap().is_none());
    assert_eq!(host(&app, json!({ "op": "share_add", "id": id, "agent": "uid-vid-pr-b" })).await.0, StatusCode::OK);
    assert!(!prompt_share::lookup(&state, &space).unwrap().unwrap().granted, "a share made again asks again");
}

#[tokio::test]
async fn a11_removing_the_share_ends_a_yes_nobody_used() {
    let (app, state) = setup().await;
    let token = agent(&state, "vid-pr-c");
    let id = prompt_login(&app).await;
    let space = prompt_space("vid-pr-c");
    let asked = ask(&app, &token, "vid-pr-c", json!({ "id": id, "reason": REASON }));
    let card = card_of(&id, "vid-pr-c").await;
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Once));
    assert_eq!(asked.await.unwrap().0, StatusCode::OK);
    assert!(prompt_share::lookup(&state, &space).unwrap().unwrap().granted);
    assert_eq!(host(&app, json!({ "op": "share_remove", "id": id, "agent": "uid-vid-pr-c" })).await.0, StatusCode::OK);
    assert_eq!(host(&app, json!({ "op": "share_add", "id": id, "agent": "uid-vid-pr-c" })).await.0, StatusCode::OK);
    assert!(!prompt_share::lookup(&state, &space).unwrap().unwrap().granted, "the yes went with the share it was given for");
    // A form login's card is as it was: no Always, and its yes covers no prompt.
    let form = login(&app).await;
    let asked = ask(&app, &token, "vid-pr-c", json!({ "id": form, "reason": REASON }));
    let card = card_of(&form, "vid-pr-c").await;
    assert!(card.prompt.is_none() && !card.offer_always);
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Once));
    let (s, out) = asked.await.unwrap();
    assert_eq!((s, out.get("prompt"), out.get("next")), (StatusCode::OK, None, None), "{out}");
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

/// A call the browser's own process makes to srv about a prompt met in `space` (`credential.Lookup`, `credential.Fill`),
/// with the secret only the paired host holds when `secret` is given.
async fn broker(state: &crate::server::AppState, method: &str, space: &Space, secret: Option<&str>) -> crate::backend::service::WebReturnType {
    let args = json!({ "identity_id": space.profile, "origin": space.origin, "realm": space.realm, "is_proxy": space.proxy });
    let call = crate::backend::service::WebCallType { service: "credential".into(), method: method.into(), uicontext: None, args: vec![args, json!(secret.unwrap_or(""))] };
    crate::server::service::run_service_call(state, &call).await
}

const HOST_SECRET: &str = "test-host-reg-secret";

#[tokio::test]
async fn a11_the_browsers_credential_calls_answer_a_prompt_from_a_share_the_agent_asked_for() {
    // The whole chain through the real routes: the agent's `Vault share` request, the person's Always on its card, then
    // what the browser's process asks srv when the site's prompt appears in the agent's own profile.
    let (app, state) = setup().await;
    let token = agent(&state, "vid-pr-d");
    let _other = agent(&state, "vid-pr-e");
    let id = prompt_login(&app).await;
    let space = prompt_space("vid-pr-d");

    // Before any share: nothing answers, and Fill gives nothing.
    let before = broker(&state, "Lookup", &space, Some(HOST_SECRET)).await;
    assert_eq!((before.success, before.data.as_ref().and_then(|d| d["found"].as_bool())), (true, Some(false)), "{before:?}");
    assert!(!broker(&state, "Fill", &space, Some(HOST_SECRET)).await.success);

    // The agent asks, the person answers Always.
    let asked = ask(&app, &token, "vid-pr-d", json!({ "id": id, "reason": REASON }));
    let card = card_of(&id, "vid-pr-d").await;
    uses::uses().answer(&card.id, UseAnswer::of(Decision::Always));
    let (s, out) = asked.await.unwrap();
    assert_eq!((s, out["always"].as_bool()), (StatusCode::OK, Some(true)), "{out}");

    // Lookup: found, shared with that agent, already approved, the user name masked and no password.
    let looked = broker(&state, "Lookup", &space, Some(HOST_SECRET)).await;
    let data = looked.data.clone().unwrap_or_default();
    assert!(looked.success, "{looked:?}");
    assert_eq!((data["found"].as_bool(), data["shared"].as_bool(), data["approval"].as_str(), data["username"].as_str()), (Some(true), Some(true), Some("granted"), Some("al***")), "{data}");
    assert!(!data.to_string().contains("pw-two-of-a-kind") && !data.to_string().contains("alice"), "{data}");

    // Fill is the one call that returns the password, and an Always is not used up by it.
    for _ in 0..2 {
        let filled = broker(&state, "Fill", &space, Some(HOST_SECRET)).await;
        let data = filled.data.clone().unwrap_or_default();
        assert!(filled.success, "{filled:?}");
        assert_eq!((data["username"].as_str(), data["password"].as_str()), (Some("alice"), Some("pw-two-of-a-kind")));
    }
    let used = lines_of(&state, &id).into_iter().filter(|l| l.action == vault::signins::SIGNED_IN).count();
    assert_eq!(used, 2, "each use is in the history, as the agent's");

    // A caller without the host's secret (an agent riding the shared key) is given nothing; nor is another agent's
    // profile, nor Personal.
    for secret in [None, Some("not-the-secret")] {
        let refused = broker(&state, "Fill", &space, secret).await;
        assert!(!refused.success && refused.data.is_none(), "{refused:?}");
    }
    let other = prompt_space("vid-pr-e");
    assert_eq!(broker(&state, "Lookup", &other, Some(HOST_SECRET)).await.data.unwrap()["found"].as_bool(), Some(false), "another agent's profile");
    assert!(!broker(&state, "Fill", &other, Some(HOST_SECRET)).await.success, "another agent's profile");
    // Personal's own tab is offered its own saved login, as before, and it is no share's: nothing says it was shared.
    let personal = Space { profile: "personal".into(), ..space.clone() };
    let own = broker(&state, "Lookup", &personal, Some(HOST_SECRET)).await.data.unwrap();
    assert_eq!((own["found"].as_bool(), own.get("shared")), (Some(true), None), "{own}");
}
