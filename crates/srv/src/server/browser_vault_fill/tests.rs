// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `fill` with `vault`: the decision table, and the whole flow against a
//! stand-in for the host.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State as AxState};
use axum::routing::post;
use axum::Router;

use super::*;
use crate::backend::vault::seal;
use crate::backend::vault::{create, NewEntry};

const VALUE: &str = "123-45-6789";

#[test]
fn the_decision_table_asks_unless_the_site_is_granted_and_the_field_fits() {
    // (origin known, password-like, fillable, granted, fits)
    assert_eq!(plan(true, false, true, true, true), Plan::Go);
    assert_eq!(plan(true, false, true, false, true), Plan::Ask { offer_always: true, reason: "the first time this entry is used on this site" });
    assert!(matches!(plan(true, false, true, true, false), Plan::Ask { offer_always: false, .. }), "a granted site but a field that doesn't fit asks");
    assert!(matches!(plan(true, false, true, false, false), Plan::Ask { offer_always: false, .. }), "\"Always\" only when it fits");
    assert!(matches!(plan(false, false, true, true, true), Plan::Refuse(_)), "no known site");
    assert!(matches!(plan(true, true, true, true, true), Plan::Refuse(m) if m.contains("password")), "never a password, granted or not");
    assert!(matches!(plan(true, false, false, true, true), Plan::Refuse(m) if m.contains("takes typed text")));
}

/// What the stand-in host saw, and what it answers.
#[derive(Default)]
struct Host {
    seen: Mutex<Vec<(String, Value)>>,
    field: Mutex<Value>,
}

async fn host_route(AxState(h): AxState<Arc<Host>>, Path(route): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    h.seen.lock().unwrap().push((route.clone(), body));
    match route.as_str() {
        "field_info" => Json(json!({ "ok": true, "data": h.field.lock().unwrap().clone() })),
        "vault_fill" => Json(json!({ "ok": true, "data": { "filled": true, "validation": "", "changed_by_page": false } })),
        _ => Json(json!({ "ok": false, "error": "unknown" })),
    }
}

fn ssn_field(origin: &str) -> Value {
    json!({
        "origin": origin,
        "page": format!("{origin}/form"),
        "label": "Social Security Number",
        "fingerprint": "fp-1",
        "slots": ["tax-id"],
        "password_like": false,
        "fillable": true,
    })
}

struct Setup {
    state: AppState,
    host: Arc<Host>,
    auth: UiAutomationAuth,
    entry: String,
}

async fn setup(field: Value) -> Setup {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let host = Arc::new(Host::default());
    *host.field.lock().unwrap() = field;
    let app = Router::new().route("/agentmux/browser/:route", post(host_route)).with_state(host.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-token".into() });

    let agent = format!("filler-{}", uuid::Uuid::new_v4());
    let block = format!("block-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&agent, &block, None).unwrap();
    let key = state.mstore.agent_jekt_key_ensure(&agent).unwrap();
    let ts = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &agent, "__srv__", ts, "");
    let auth = UiAutomationAuth { agent_id: agent, ts_secs: ts, sig };
    let entry = create(
        &state.identity_store,
        NewEntry {
            kind: "ssn".into(),
            label: "Alex".into(),
            fields: Default::default(),
            tags: vec![],
            parts: crate::backend::vault::seal::Parts([("value".to_string(), VALUE.to_string())].into()),
        },
    )
    .unwrap();
    Setup { state, host, auth, entry }
}

async fn fill(s: &Setup, vault: &str) -> (StatusCode, Value) {
    let req = VaultFillRequest { auth: s.auth.clone(), pane: None, ref_: "e7".into(), vault: vault.into() };
    let resp = handle_vault_fill(State(s.state.clone()), None, Json(req)).await;
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

/// Answers `agent`'s waiting use as the person would (through the host). The
/// registry is shared by every test, so it looks only at this agent's.
async fn answer_when_asked(agent: String, decision: Decision) -> UseRequest {
    for _ in 0..200 {
        if let Some(u) = uses::uses().list().into_iter().find(|u| u.agent == agent) {
            uses::uses().answer(&u.id, decision);
            return u;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the use was never asked about");
}

fn routes(s: &Setup) -> Vec<String> {
    s.host.seen.lock().unwrap().iter().map(|(r, _)| r.clone()).collect()
}

fn typed(s: &Setup) -> Option<Value> {
    s.host.seen.lock().unwrap().iter().find(|(r, _)| r == "vault_fill").map(|(_, b)| b.clone())
}

fn activity_text(s: &Setup) -> String {
    serde_json::to_string(&crate::backend::vault::activity(&s.state.identity_store, 20).unwrap()).unwrap()
}

#[tokio::test]
async fn a_first_use_asks_the_person_and_the_value_never_reaches_the_agent() {
    let s = setup(ssn_field("https://irs.gov")).await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    let (status, body) = fill(&s, &s.entry).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let asked = ask.await.unwrap();
    assert_eq!((asked.origin.as_str(), asked.field.as_str(), asked.entry_label.as_str()), ("https://irs.gov", "Social Security Number", "Alex"));
    assert!(asked.offer_always, "the field looks like an SSN");
    assert_eq!(asked.what, "Social Security number");

    // What the agent gets back.
    assert_eq!(body["data"]["filled_from_vault"], s.entry.as_str());
    assert_eq!(body["data"]["value"], "[secret: hidden]");
    assert!(!body.to_string().contains("123-45"), "{body}");
    // What the host was asked to type, bound to the site and field the person saw.
    let sent = typed(&s).expect("the host was asked to type");
    assert_eq!(sent["text"], VALUE);
    assert_eq!((sent["expect_origin"].as_str(), sent["expect_fingerprint"].as_str()), (Some("https://irs.gov"), Some("fp-1")));
    assert_eq!(routes(&s), ["field_info", "vault_fill"]);
    // The trail says what, where and the outcome, and never the value.
    let log = activity_text(&s);
    assert!(log.contains("filled a web form") && log.contains("https://irs.gov") && log.contains("done"), "{log}");
    assert!(!log.contains("123-45"), "{log}");
    // Once is once: no grant.
    assert!(!s.state.identity_store.vault_grant_check(&s.entry, "https://irs.gov").unwrap());
}

#[tokio::test]
async fn always_grants_the_site_and_the_next_fill_doesnt_ask() {
    let s = setup(ssn_field("https://irs.gov")).await;
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Always));
    assert_eq!(fill(&s, &s.entry).await.0, StatusCode::OK);
    ask.await.unwrap();
    assert!(s.state.identity_store.vault_grant_check(&s.entry, "https://irs.gov").unwrap());

    // Granted site, field that fits: typed with no question.
    let (status, body) = fill(&s, &s.entry).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id), "it didn't ask");
    assert_eq!(routes(&s).iter().filter(|r| *r == "vault_fill").count(), 2);

    // The same entry on another site asks again.
    *s.host.field.lock().unwrap() = ssn_field("https://evil.example");
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Deny));
    let (status, body) = fill(&s, &s.entry).await;
    ask.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(routes(&s).iter().filter(|r| *r == "vault_fill").count(), 2, "nothing was typed");

    // A revoked grant asks again.
    s.state.identity_store.vault_grant_revoke(&s.entry, "https://irs.gov").unwrap();
    *s.host.field.lock().unwrap() = ssn_field("https://irs.gov");
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Once));
    assert_eq!(fill(&s, &s.entry).await.0, StatusCode::OK);
    ask.await.unwrap();
}

#[tokio::test]
async fn a_field_that_doesnt_look_like_an_ssn_asks_and_offers_no_always() {
    // A comment box: the case a tricked agent would use to put the SSN where it can read it back.
    let mut comment = ssn_field("https://irs.gov");
    comment["slots"] = json!([]);
    comment["label"] = json!("Leave a comment");
    let s = setup(comment).await;
    s.state.identity_store.vault_grant_add(&s.entry, "https://irs.gov", 1).unwrap();
    let ask = tokio::spawn(answer_when_asked(s.auth.agent_id.clone(), Decision::Deny));
    let (status, _) = fill(&s, &s.entry).await;
    let asked = ask.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!asked.offer_always);
    assert!(asked.reason.contains("doesn't look like"), "{}", asked.reason);
    assert!(typed(&s).is_none(), "Deny leaves the box empty");
    assert!(activity_text(&s).contains("denied"));
}

#[tokio::test]
async fn a_password_field_and_a_login_are_refused_without_asking() {
    let mut pw = ssn_field("https://irs.gov");
    pw["password_like"] = json!(true);
    let s = setup(pw).await;
    let (status, body) = fill(&s, &s.entry).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("password or one-time-code"), "{body}");
    assert!(uses::uses().list().iter().all(|u| u.agent != s.auth.agent_id) && typed(&s).is_none());

    let login = create(
        &s.state.identity_store,
        NewEntry {
            kind: "login".into(),
            label: "Example".into(),
            fields: [("site".to_string(), "https://example.com".to_string())].into(),
            tags: vec![],
            parts: crate::backend::vault::seal::Parts([("password".to_string(), "hunter2".to_string())].into()),
        },
    )
    .unwrap();
    let (status, body) = fill(&s, &login).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("password"), "{body}");
    assert!(routes(&s).iter().all(|r| r != "vault_fill"));
}

#[tokio::test]
async fn an_unknown_entry_or_part_is_refused_by_name() {
    let s = setup(ssn_field("https://irs.gov")).await;
    let (status, body) = fill(&s, "no-such-entry").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("no Vault entry"));
    let (status, body) = fill(&s, &format!("{}.cvc", s.entry)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("its parts are value"), "{body}");
    assert!(routes(&s).is_empty(), "the host wasn't asked about a field");
}

#[tokio::test]
async fn an_unverified_caller_is_refused_before_anything_is_read() {
    let s = setup(ssn_field("https://irs.gov")).await;
    let mut forged = s.auth.clone();
    forged.sig = "not-a-signature".into();
    let req = VaultFillRequest { auth: forged, pane: None, ref_: "e7".into(), vault: s.entry.clone() };
    let resp = handle_vault_fill(State(s.state.clone()), None, Json(req)).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(routes(&s).is_empty());
}

#[tokio::test]
async fn the_host_refusing_the_fill_is_reported_as_failed_and_leaks_nothing() {
    let s = setup(ssn_field("https://irs.gov")).await;
    s.state.identity_store.vault_grant_add(&s.entry, "https://irs.gov", 1).unwrap();
    // A host that answers the fill with an error.
    let app = Router::new().route(
        "/agentmux/browser/:route",
        post(|Path(route): Path<String>| async move {
            if route == "field_info" {
                Json(json!({ "ok": true, "data": ssn_field("https://irs.gov") }))
            } else {
                Json(json!({ "ok": false, "error": "the field changed while the person was reading the approval, so nothing was typed" }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    *s.state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-token".into() });
    let (status, body) = fill(&s, &s.entry).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("nothing was typed") && !body.to_string().contains("123-45"), "{body}");
    assert!(activity_text(&s).contains("failed"));
}
