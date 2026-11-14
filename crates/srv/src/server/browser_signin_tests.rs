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
use crate::backend::vault::uses::{Decision, UseAnswer};

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
    let key = grant_key("Agent1", "personal", "https://a.example.com");
    assert_eq!(key, "signin:Agent1|personal|https://a.example.com");
    assert!(!crate::server::browser_signin_fill::fillable(&key));
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
    let req = SigninRequest { auth: s.auth.clone(), pane: None, entry: entry.map(String::from), reason: Some("sign in to the intranet for the test".into()) };
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
    assert_eq!(asked.agent_reason, "sign in to the intranet for the test", "the card says what the agent said");
    assert!(activity.contains("sign in to the intranet for the test"), "and so does the history: {activity}");
}

#[tokio::test]
async fn a_sign_in_without_a_reason_is_refused_before_anyone_is_asked() {
    let s = setup(None).await;
    save(&s, "alex@example.com");
    let req = SigninRequest { auth: s.auth.clone(), pane: None, entry: None, reason: None };
    let resp = handle_signin(State(s.state.clone()), None, Json(req)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("a reason is required"), "{body:?}");
    assert!(typed(&s).is_none(), "nothing was typed");
}

#[tokio::test]
async fn always_is_kept_for_this_agent_and_then_the_notice_can_stop_it() {
    let s = setup(None).await;
    let entry = save(&s, "alex");
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Always));
    assert_eq!(signin(&s, None).await.0, StatusCode::OK);
    ask.await.unwrap();
    let check = |agent: &str, profile: &str, site: &str| s.state.identity_store.vault_grant_check(&entry, &grant_key(agent, profile, site)).unwrap();
    assert!(check(&s.auth.agent_id, PERSONAL, &s.site));
    assert!(!check("another-agent", PERSONAL, &s.site), "for this agent only");
    assert!(!check(&s.auth.agent_id, PERSONAL, "https://elsewhere.example.com"), "for the site approved only");
    assert!(!check(&s.auth.agent_id, "profile:work", &s.site), "for the profile approved only");

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

#[tokio::test]
async fn the_notice_goes_when_the_agents_call_is_cancelled_while_it_shows() {
    let s = setup(None).await;
    let showing = || s.state.mstore.get::<Block>(&s.block).unwrap().unwrap().meta.get(PENDING_META_KEY).cloned().unwrap_or(Value::Null);
    let block = s.block.clone();
    let shown = tokio::time::timeout(Duration::from_millis(300), async {
        let seen = tokio::spawn(async move {
            for _ in 0..40 {
                if PENDING.lock().values().any(|(b, _)| *b == block) {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            false
        });
        let n = notice(&s.state, &s.block, "a", &s.site, "alex", "signin", NOTICE);
        tokio::pin!(n);
        tokio::select! { _ = &mut n => false, up = seen => up.unwrap() && !showing().is_null() }
    })
    .await;
    assert_eq!(shown, Ok(true), "the notice showed before the call was cancelled");
    assert!(PENDING.lock().values().all(|(b, _)| *b != s.block), "no Stop is left waiting");
    assert!(showing().is_null(), "the pane shows no notice for a sign-in that will never be typed");
}

fn grant(s: &Setup, entry: &str) {
    let created = s.state.identity_store.vault_get(entry).unwrap().unwrap().created_at;
    s.state.identity_store.vault_grant_add(entry, &grant_key(&s.auth.agent_id, PERSONAL, &s.site), agentmux_common::time::now_ms(), created).unwrap();
}

#[tokio::test]
async fn a_second_sign_in_while_a_notice_shows_on_the_pane_is_refused() {
    let s = setup(None).await;
    let entry = save(&s, "alex");
    grant(&s, &entry);
    let other = uuid::Uuid::new_v4().to_string();
    let (tx, _rx) = oneshot::channel();
    PENDING.lock().insert(other.clone(), (s.block.clone(), tx));
    let (status, body) = signin(&s, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(typed(&s).is_none(), "nothing typed behind the notice the pane shows");
    assert!(PENDING.lock().contains_key(&other), "the notice showing keeps its Stop");
    PENDING.lock().remove(&other);
}

#[tokio::test]
async fn a_pane_paused_while_the_person_is_asked_gets_nothing_typed() {
    let s = setup(None).await;
    save(&s, "alex");
    let (state, block, agent) = (s.state.clone(), s.block.clone(), s.auth.agent_id.clone());
    let ask = tokio::spawn(async move {
        for _ in 0..200 {
            if let Some(u) = uses::uses().list().into_iter().find(|u| u.agent == agent) {
                crate::server::browser_actions::set_paused(&state, &block, true);
                uses::uses().answer(&u.id, UseAnswer::of(Decision::Once));
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the sign-in was never asked about");
    });
    let (status, body) = signin(&s, None).await;
    ask.await.unwrap();
    crate::server::browser_actions::set_paused(&s.state, &s.block, false);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("nothing was typed"), "{body}");
    assert!(typed(&s).is_none(), "a yes given before the pause doesn't type into a paused pane");
}

#[tokio::test]
async fn a_sign_in_deleted_and_saved_again_while_the_person_is_asked_is_not_typed() {
    let s = setup(None).await;
    let entry = save(&s, "alex");
    let (state, agent, site) = (s.state.clone(), s.auth.agent_id.clone(), s.site.clone());
    let ask = tokio::spawn(async move {
        for _ in 0..200 {
            if let Some(u) = uses::uses().list().into_iter().find(|u| u.agent == agent) {
                crate::backend::vault::delete(&state.identity_store, &u.entry_id).unwrap();
                // A different sign-in under the same id: the person was never shown it.
                std::thread::sleep(Duration::from_millis(5));
                let space = Space { profile: PERSONAL.into(), origin: site, realm: String::new(), proxy: false };
                let again = signins::save(&state.identity_store, &space, "mallory", "another password").unwrap();
                uses::uses().answer(&u.id, UseAnswer::of(Decision::Always));
                return again;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the sign-in was never asked about");
    });
    let (status, body) = signin(&s, None).await;
    let again = ask.await.unwrap();
    assert_ne!(status, StatusCode::OK, "{body}");
    assert!(body.to_string().contains("nothing was typed"), "{body}");
    assert!(typed(&s).is_none(), "the replacement's user name and password aren't typed");
    if again == entry {
        let key = grant_key(&s.auth.agent_id, PERSONAL, &s.site);
        assert!(!s.state.identity_store.vault_grant_check(&again, &key).unwrap(), "the Always isn't given to the replacement");
    }
}

#[tokio::test]
async fn a_sign_in_whose_site_is_edited_while_the_person_is_asked_is_not_typed() {
    let s = setup(None).await;
    let entry = save(&s, "alex");
    // The page shows a second site's form too, which the edit moves the entry to.
    let other = format!("https://o{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    crate::server::browser_signin_fill::form(&s.state, &s.block, &other);
    let (state, agent, id) = (s.state.clone(), s.auth.agent_id.clone(), entry.clone());
    let ask = tokio::spawn(async move {
        for _ in 0..200 {
            if let Some(u) = uses::uses().list().into_iter().find(|u| u.agent == agent) {
                let row = state.identity_store.vault_get(&id).unwrap().unwrap();
                let mut fields: std::collections::BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap();
                fields.insert("site".into(), other.clone());
                let tags: Vec<String> = serde_json::from_str(&row.tags).unwrap_or_default();
                let change = crate::backend::vault::EntryChange { id: id.clone(), label: row.label, fields, tags, parts: Default::default(), clear: vec![] };
                crate::backend::vault::update(&state.identity_store, change).unwrap();
                uses::uses().answer(&u.id, UseAnswer::of(Decision::Once));
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the sign-in was never asked about");
    });
    let (status, body) = signin(&s, Some(&entry)).await;
    ask.await.unwrap();
    assert_ne!(status, StatusCode::OK, "{body}");
    assert!(body.to_string().contains("nothing was typed"), "{body}");
    assert!(typed(&s).is_none(), "nothing typed into a site the person wasn't asked about");
}
