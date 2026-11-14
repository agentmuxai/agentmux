// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `generate_password`, against a stand-in for the host.

use std::sync::{Arc, Mutex as StdMutex};

use axum::extract::{Path, State as AxState};
use axum::routing::post;
use axum::Router;

use super::*;
use crate::backend::obj::Block;
use crate::backend::vault::seal;
use crate::backend::vault::uses::{self, Decision, UseAnswer};

/// What the stand-in host was asked, and what it says the field is.
struct Host {
    seen: StdMutex<Vec<(String, Value)>>,
    probe: StdMutex<Value>,
}

async fn host_route(AxState(h): AxState<Arc<Host>>, Path(route): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    h.seen.lock().unwrap().push((route.clone(), body));
    match route.as_str() {
        "signup_probe" => Json(json!({ "ok": true, "data": h.probe.lock().unwrap().clone() })),
        "signup_fill" => Json(json!({ "ok": true, "data": { "fields": 2 } })),
        _ => Json(json!({ "ok": false, "error": "unknown" })),
    }
}

struct Setup {
    state: AppState,
    host: Arc<Host>,
    auth: UiAutomationAuth,
    block: String,
    site: String,
}

fn site() -> String {
    format!("https://s{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// An agent with its own browser pane of `identity`, and a host that says a
/// sign-up form of a site of its own is under `e7`.
async fn setup(identity: Option<&str>) -> Setup {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let site = site();
    let host = Arc::new(Host { seen: StdMutex::new(Vec::new()), probe: StdMutex::new(json!({ "origin": site, "field": "t-1", "change": false, "minlength": 12 })) });
    let app = Router::new().route("/agentmux/browser/:route", post(host_route)).with_state(host.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-token".into() });

    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    block.meta.insert("view".into(), json!("browser"));
    if let Some(i) = identity {
        block.meta.insert(crate::server::browser_identity::IDENTITY_META_KEY.into(), json!(i));
    }
    state.mstore.insert(&mut block).unwrap();
    let agent = format!("maker-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&agent, &block.oid, None).unwrap();
    let key = state.mstore.agent_jekt_key_ensure(&agent).unwrap();
    let ts = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &agent, "__srv__", ts, "");
    let auth = UiAutomationAuth { agent_id: agent, ts_secs: ts, sig };
    Setup { state, host, auth, block: block.oid, site }
}

fn request(s: &Setup, ref_: Option<&str>, reason: Option<&str>) -> AccountRequest {
    AccountRequest { auth: s.auth.clone(), pane: None, ref_: ref_.map(String::from), username: Some("alex@example.com".into()), reason: reason.map(String::from) }
}

async fn generate(s: &Setup) -> (StatusCode, Value) {
    generate_with(s, request(s, Some("e12"), Some("sign up for the test shop's sandbox"))).await
}

async fn generate_with(s: &Setup, req: AccountRequest) -> (StatusCode, Value) {
    let resp = handle_generate_password(State(s.state.clone()), None, Json(req)).await;
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

/// Answers `agent`'s waiting use as the person would. The registry is shared by
/// every test, so it looks only at this agent's.
async fn answer_when_asked(agent: String, decision: Decision) -> UseRequest {
    for _ in 0..200 {
        if let Some(u) = uses::uses().list().into_iter().find(|u| u.agent == agent) {
            uses::uses().answer(&u.id, UseAnswer::of(decision));
            return u;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the account was never asked about");
}

fn asked_of(s: &Setup, route: &str) -> Vec<Value> {
    s.host.seen.lock().unwrap().iter().filter(|(r, _)| r == route).map(|(_, b)| b.clone()).collect()
}

/// The entries kept for the site: (id, tags, user name, password, profile).
fn kept(s: &Setup) -> Vec<(String, Vec<String>, String, String, String)> {
    s.state
        .identity_store
        .vault_list()
        .unwrap()
        .into_iter()
        .filter_map(|r| {
            let fields: std::collections::BTreeMap<String, String> = serde_json::from_str(&r.fields).unwrap_or_default();
            (fields.get("site").map(String::as_str) == Some(s.site.as_str())).then(|| {
                let tags: Vec<String> = serde_json::from_str(&r.tags).unwrap_or_default();
                let password = seal::open(&seal::key(false).unwrap(), &r.id, &r.sealed).unwrap().get("password").unwrap_or("").to_string();
                (r.id.clone(), tags, fields.get("username").cloned().unwrap_or_default(), password, fields.get("profile").cloned().unwrap_or_default())
            })
        })
        .collect()
}

#[tokio::test]
async fn the_person_is_asked_then_the_vault_keeps_the_password_before_the_site_sees_it() {
    let s = setup(None).await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let (status, body) = generate(&s).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let asked = ask.await.unwrap();
    assert!(asked.account && !asked.signin && !asked.offer_always, "no Always for an account");
    assert_eq!((asked.origin.as_str(), asked.entry_id.as_str(), asked.kind.as_str()), (s.site.as_str(), "", "login"));
    assert_eq!(asked.agent_reason, "sign up for the test shop's sandbox");

    let typed = asked_of(&s, "signup_fill");
    assert_eq!(typed.len(), 1, "the host was asked to type it once");
    let password = typed[0]["password"].as_str().unwrap().to_string();
    assert!(password.chars().count() >= 20, "a strong password, made under the field's own minimum");
    assert_eq!((typed[0]["origin"].as_str(), typed[0]["field"].as_str()), (Some(s.site.as_str()), Some("t-1")));

    let entries = kept(&s);
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].1.as_slice(), entries[0].2.as_str(), entries[0].3.as_str(), entries[0].4.as_str()), ([signups::UNCONFIRMED.to_string()].as_slice(), "alex@example.com", password.as_str(), "personal"));
    assert_eq!(body["data"]["entry"].as_str(), Some(entries[0].0.as_str()));
    assert!(!body.to_string().contains(&password), "the agent is never told the password: {body}");
    assert_eq!(body["data"]["password"], "[secret: hidden]");

    let activity = serde_json::to_string(&crate::backend::vault::activity(&s.state.identity_store, 50).unwrap()).unwrap();
    assert!(activity.contains(&s.auth.agent_id) && activity.contains("filled on sign-up") && activity.contains("sign up for the test shop's sandbox"), "{activity}");
    assert!(!activity.contains(&password), "{activity}");
}

#[tokio::test]
async fn a_call_without_a_reason_or_a_field_asks_nobody() {
    let s = setup(None).await;
    for (req, needle) in [(request(&s, Some("e12"), None), "a reason is required"), (request(&s, None, Some("sign up for the test shop's sandbox")), "give ref")] {
        let (status, body) = generate_with(&s, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains(needle), "{body}");
    }
    assert!(asked_of(&s, "signup_probe").is_empty() && asked_of(&s, "signup_fill").is_empty());
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id));
}

#[tokio::test]
async fn a_no_makes_no_password_and_types_nothing() {
    let s = setup(None).await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Deny));
    let (status, body) = generate(&s).await;
    ask.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(asked_of(&s, "signup_fill").is_empty(), "nothing typed");
    assert!(kept(&s).is_empty(), "no entry made");
    let activity = serde_json::to_string(&crate::backend::vault::activity(&s.state.identity_store, 50).unwrap()).unwrap();
    assert!(activity.contains(&s.auth.agent_id) && activity.contains("denied"), "the refusal is recorded as the agent's: {activity}");
}

#[tokio::test]
async fn a_form_that_asks_for_the_current_password_is_refused_before_anyone_is_asked() {
    let s = setup(None).await;
    *s.host.probe.lock().unwrap() = json!({ "origin": s.site, "change": true });
    let (status, body) = generate(&s).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("current password") && body["error"].as_str().unwrap().contains("ask the person"), "{body}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id), "nobody was asked");
    assert!(asked_of(&s, "signup_fill").is_empty() && kept(&s).is_empty());
}

#[tokio::test]
async fn a_site_that_isnt_https_gets_no_password() {
    let s = setup(None).await;
    *s.host.probe.lock().unwrap() = json!({ "origin": "http://shop.example.com", "field": "t-1", "change": false });
    let (status, body) = generate(&s).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("isn't https"), "{body}");
    assert!(asked_of(&s, "signup_fill").is_empty());
}

#[tokio::test]
async fn a_field_whose_rules_leave_no_strong_password_gets_none_and_nobody_is_asked() {
    let s = setup(None).await;
    *s.host.probe.lock().unwrap() = json!({ "origin": s.site, "field": "t-1", "change": false, "maxlength": 6 });
    let (status, body) = generate(&s).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id));
    assert!(kept(&s).is_empty());
}

#[tokio::test]
async fn stop_during_the_countdown_makes_no_password() {
    let s = setup(None).await;
    let block = s.block.clone();
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let stopper = tokio::spawn(async move {
        for _ in 0..400 {
            let id = crate::server::browser_signin::pending_id(&block);
            if let Some(id) = id {
                return crate::server::browser_signin::stop(&block, &id);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Err("no notice showed".into())
    });
    let (status, body) = generate(&s).await;
    ask.await.unwrap();
    stopper.await.unwrap().unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(asked_of(&s, "signup_fill").is_empty(), "nothing typed after Stop");
    assert!(kept(&s).is_empty(), "and no entry made, since the password is made after the countdown");
}

#[tokio::test]
async fn a_notice_names_the_agent_and_the_site_on_the_pane_before_the_password_is_typed() {
    let s = setup(None).await;
    let (state, block) = (s.state.clone(), s.block.clone());
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let watcher = tokio::spawn(async move {
        for _ in 0..400 {
            let shown = state.mstore.get::<Block>(&block).unwrap().unwrap().meta.get(super::super::browser_signin::PENDING_META_KEY).cloned().unwrap_or(Value::Null);
            if !shown.is_null() {
                return shown;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Value::Null
    });
    let (status, body) = generate(&s).await;
    ask.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let shown = watcher.await.unwrap();
    assert_eq!((shown["kind"].as_str(), shown["agent"].as_str(), shown["origin"].as_str()), (Some("account"), Some(s.auth.agent_id.as_str()), Some(s.site.as_str())), "{shown}");
    let after = s.state.mstore.get::<Block>(&s.block).unwrap().unwrap().meta.get(super::super::browser_signin::PENDING_META_KEY).cloned().unwrap_or(Value::Null);
    assert!(after.is_null(), "the notice is gone once the password is typed");
}

#[tokio::test]
async fn an_incognito_tab_keeps_the_password_in_personal_tagged() {
    let s = setup(Some("incognito:jar-1")).await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let (status, body) = generate(&s).await;
    let asked = ask.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(asked.reason.contains("Incognito"), "{}", asked.reason);
    let entries = kept(&s);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].4, "personal");
    assert!(entries[0].1.contains(&signups::FROM_INCOGNITO.to_string()) && entries[0].1.contains(&signups::UNCONFIRMED.to_string()), "{:?}", entries[0].1);
}

#[tokio::test]
async fn asking_again_after_the_site_refused_replaces_the_unconfirmed_password() {
    let s = setup(None).await;
    for _ in 0..2 {
        let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
        let (status, body) = generate(&s).await;
        ask.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let typed = asked_of(&s, "signup_fill");
    assert_eq!(typed.len(), 2);
    assert_ne!(typed[0]["password"], typed[1]["password"], "a new password each time");
    let entries = kept(&s);
    assert_eq!(entries.len(), 1, "the refused password isn't left as an entry of its own");
    assert_eq!(entries[0].3, typed[1]["password"].as_str().unwrap());
}
