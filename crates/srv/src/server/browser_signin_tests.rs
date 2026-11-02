// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `signin`: which sign-in is meant, and the whole flow against a
//! stand-in for the host.

use std::sync::{Arc, Mutex as StdMutex};

use axum::extract::{Path, State as AxState};
use axum::routing::post;
use axum::Router;

use super::*;
use crate::backend::obj::Block;
use crate::backend::vault::seal;
use crate::backend::vault::signins::Space;
use crate::backend::vault::uses::UseAnswer;

fn found(rows: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    rows.iter().map(|(e, o, u)| (e.to_string(), o.to_string(), u.to_string())).collect()
}

#[test]
fn the_sign_in_meant_is_the_one_named_or_the_only_one() {
    let two = found(&[("e1", "https://accounts.example.com", "alex@example.com"), ("e2", "https://accounts.example.com", "sam")]);
    assert_eq!(choose(two.clone(), Some("e2")).unwrap().0, "e2");
    assert!(choose(two.clone(), Some("e3")).is_err(), "not one the page can take");
    let many = choose(two, None).unwrap_err();
    assert!(many.contains("e1 (al***") && many.contains("e2 (sa***"), "{many}");
    assert!(!many.contains("alex@"), "only masked user names: {many}");
    assert_eq!(choose(found(&[("e1", "https://a.example.com", "alex")]), None).unwrap().0, "e1");
    assert!(choose(Vec::new(), None).is_err());
}

#[test]
fn a_grant_is_kept_under_a_key_no_site_can_have() {
    assert_eq!(grant_key("Agent1"), "signin:Agent1");
    assert!(!crate::server::browser_signin_fill::fillable(&grant_key("Agent1")));
}

#[test]
fn only_the_notice_of_that_pane_is_stopped() {
    let (tx, mut rx) = oneshot::channel();
    PENDING.lock().insert("n1".into(), ("pane-a".into(), tx));
    assert!(stop("pane-b", "n1").is_err());
    assert!(rx.try_recv().is_err(), "still running");
    stop("pane-a", "n1").unwrap();
    assert!(rx.try_recv().is_ok());
    assert!(stop("pane-a", "n1").is_err(), "stopped once");
}

/// What the stand-in host was asked to type.
#[derive(Default)]
struct Host {
    seen: StdMutex<Vec<(String, Value)>>,
}

async fn host_route(AxState(h): AxState<Arc<Host>>, Path(route): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    h.seen.lock().unwrap().push((route.clone(), body));
    match route.as_str() {
        "signin_fill" => Json(json!({ "ok": true, "data": { "filled": true, "username_filled": true } })),
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

/// An agent with its own browser pane of `identity`, a sign-in form of a site
/// of its own on the page, and the host's stand-in.
async fn setup(identity: Option<&str>) -> Setup {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let host = Arc::new(Host::default());
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
    let agent = format!("signer-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&agent, &block.oid, None).unwrap();
    let key = state.mstore.agent_jekt_key_ensure(&agent).unwrap();
    let ts = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &agent, "__srv__", ts, "");
    let auth = UiAutomationAuth { agent_id: agent, ts_secs: ts, sig };
    let site = format!("https://s{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    crate::server::browser_signin_fill::form(&state, &block.oid, &site);
    Setup { state, host, auth, block: block.oid, site }
}

fn save(s: &Setup, user: &str) -> String {
    let space = Space { profile: PERSONAL.into(), origin: s.site.clone(), realm: String::new(), proxy: false };
    signins::save(&s.state.identity_store, &space, user, "correct horse").unwrap()
}

async fn signin(s: &Setup, entry: Option<&str>) -> (StatusCode, Value) {
    let req = SigninRequest { auth: s.auth.clone(), pane: None, entry: entry.map(String::from) };
    let resp = handle_signin(State(s.state.clone()), None, Json(req)).await;
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
    panic!("the sign-in was never asked about");
}

fn typed(s: &Setup) -> Option<Value> {
    s.host.seen.lock().unwrap().iter().find(|(r, _)| r == "signin_fill").map(|(_, b)| b.clone())
}

#[tokio::test]
async fn a_first_sign_in_asks_the_person_and_the_agent_gets_no_value() {
    let s = setup(None).await;
    let entry = save(&s, "alex@example.com");
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let (status, body) = signin(&s, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let asked = ask.await.unwrap();
    assert!(asked.signin && asked.offer_always);
    assert_eq!((asked.origin.as_str(), asked.field.as_str(), asked.entry_id.as_str()), (s.site.as_str(), "al***", entry.as_str()));
    let sent = typed(&s).expect("the host was asked to type it");
    assert_eq!((sent["origin"].as_str(), sent["username"].as_str(), sent["password"].as_str()), (Some(s.site.as_str()), Some("alex@example.com"), Some("correct horse")));
    assert_eq!(body["data"]["entry"].as_str(), Some(entry.as_str()));
    assert!(!body.to_string().contains("correct horse") && !body.to_string().contains("alex@example.com"), "{body}");
    let activity = serde_json::to_string(&crate::backend::vault::activity(&s.state.identity_store, 50).unwrap()).unwrap();
    assert!(activity.contains(&s.auth.agent_id) && activity.contains(signins::SIGNED_IN), "{activity}");
}

#[tokio::test]
async fn always_is_kept_for_this_agent_and_then_the_notice_can_stop_it() {
    let s = setup(None).await;
    let entry = save(&s, "alex");
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Always));
    assert_eq!(signin(&s, None).await.0, StatusCode::OK);
    ask.await.unwrap();
    assert!(s.state.identity_store.vault_grant_check(&entry, &grant_key(&s.auth.agent_id)).unwrap());
    assert!(!s.state.identity_store.vault_grant_check(&entry, &grant_key("another-agent")).unwrap(), "for this agent only");

    s.host.seen.lock().unwrap().clear();
    let block = s.block.clone();
    let stopper = tokio::spawn(async move {
        for _ in 0..200 {
            let id = PENDING.lock().iter().find(|(_, (b, _))| *b == block).map(|(id, _)| id.clone());
            if let Some(id) = id {
                return stop(&block, &id);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err("no notice showed".into())
    });
    let (status, body) = signin(&s, None).await;
    stopper.await.unwrap().unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(typed(&s).is_none(), "nothing typed after Stop");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id), "a granted sign-in doesn't ask");
}

#[tokio::test]
async fn a_no_types_nothing_and_a_page_with_no_form_asks_nobody() {
    let s = setup(None).await;
    save(&s, "alex");
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Deny));
    assert_eq!(signin(&s, None).await.0, StatusCode::FORBIDDEN);
    ask.await.unwrap();
    assert!(typed(&s).is_none());

    crate::server::browser_signin_fill::navigated(&s.state, &s.block);
    let (status, body) = signin(&s, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id));
}

#[tokio::test]
async fn an_incognito_tab_uses_personals_and_asks_each_time() {
    let s = setup(Some("incognito:aaaaaaaa-2222")).await;
    save(&s, "alex");
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    assert_eq!(signin(&s, None).await.0, StatusCode::OK);
    assert!(!ask.await.unwrap().offer_always, "no Always in Incognito");
}

#[tokio::test]
async fn a_notice_saved_with_a_pane_from_before_a_restart_is_cleared_when_the_host_registers() {
    let s = setup(None).await;
    set_notice(&s.state, &s.block, json!({ "id": "gone", "agent": "a", "origin": s.site, "user": "al***", "deadline_ms": 0 }));
    let showing = || s.state.mstore.get::<Block>(&s.block).unwrap().unwrap().meta.get(PENDING_META_KEY).cloned().unwrap_or(Value::Null);
    assert!(!showing().is_null());
    reset_notices(&s.state);
    assert!(showing().is_null(), "no sign-in waits behind it: its Stop would stop nothing");
}
