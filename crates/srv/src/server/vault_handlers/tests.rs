// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

const HOST: (&str, &str) = ("X-Host-Token", "host-ipc-token");

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

fn post(uri: &str, headers: &[(&str, &str)], body: Value) -> Request<Body> {
    let mut b = Request::builder().uri(uri).method("POST").header("X-AuthKey", "test-secret-key").header("Content-Type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).header("X-AuthKey", "test-secret-key").body(Body::empty()).unwrap()
}

async fn app() -> axum::Router {
    crate::backend::vault::seal::use_test_key();
    let state = crate::server::tests::test_state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
    crate::server::build_router(state)
}

fn new_ssn() -> Value {
    json!({ "op": "create", "kind": "ssn", "label": "Alex", "fields": { "owner": "Alex" }, "parts": { "value": "123-45-6789" } })
}

// §5.4: the instance key every agent holds can't write or read the Vault
// through the pane's route; only the host's token can.
#[tokio::test]
async fn only_the_host_reaches_the_panes_route() {
    let app = app().await;
    for op in [new_ssn(), json!({ "op": "list" }), json!({ "op": "delete", "id": "ssn-alex" })] {
        let (s, body) = send(&app, post("/api/v1/host/vault", &[], op.clone())).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{op}");
        assert!(body.to_string().contains("only the AgentMux host"));
        let (s, _) = send(&app, post("/api/v1/host/vault", &[("X-Host-Token", "a-guess")], op)).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn the_pane_saves_and_lists_and_agents_see_only_names() {
    let app = app().await;
    let (s, body) = send(&app, post("/api/v1/host/vault", &[HOST], new_ssn())).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let id = body["data"]["id"].as_str().unwrap().to_string();

    let (_, body) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "list" }))).await;
    let entry = body["data"]["entries"].as_array().unwrap().iter().find(|e| e["id"] == id.as_str()).unwrap().clone();
    assert_eq!(entry["tail"], "6789");
    assert!(!body.to_string().contains("123-45"), "the pane never gets a whole value: {body}");

    let (s, body) = send(&app, get("/api/v1/vault")).await;
    assert_eq!(s, StatusCode::OK);
    let text = body.to_string();
    assert!(text.contains(&id) && text.contains("\"label\":\"Alex\""), "{text}");
    for leak in ["6789", "tail", "owner", "fields", "sealed"] {
        assert!(!text.contains(leak), "an agent's list has no {leak}: {text}");
    }

    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "delete", "id": id }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = send(&app, get("/api/v1/vault")).await;
    assert!(!body.to_string().contains(&id));
}

#[tokio::test]
async fn an_agents_request_is_answered_with_the_new_entrys_id_only() {
    let app = app().await;
    let ask = {
        let app = app.clone();
        tokio::spawn(async move {
            send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "api-key", "label": "Stripe test key", "why": "run the payment tests", "wait_secs": 20 }))).await
        })
    };
    let mut req = None;
    for _ in 0..100 {
        req = requests::requests().list().into_iter().find(|r| r.label == "Stripe test key");
        if req.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let req = req.expect("the request waits for the person");
    assert_eq!(req.agent, "Something on this computer (not a verified agent)", "an unsigned request names no agent");
    assert_eq!(req.why, "run the payment tests");

    let fill = json!({ "op": "create", "request_id": req.id, "kind": "api-key", "label": "Stripe test key", "parts": { "value": "sk_test_abcdef123456" } });
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], fill.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't answer for the person");
    assert!(!ask.is_finished());
    let (s, body) = send(&app, post("/api/v1/host/vault", &[HOST], fill)).await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let (s, answer) = ask.await.unwrap();
    assert_eq!(s, StatusCode::OK);
    assert_eq!(answer["status"], "created");
    assert_eq!(answer["id"], "api-key-stripe-test-key");
    assert!(!answer.to_string().contains("sk_test"), "{answer}");
}

#[tokio::test]
async fn a_declined_request_says_so() {
    let app = app().await;
    let ask = {
        let app = app.clone();
        tokio::spawn(async move { send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "ssn", "label": "Declined one", "wait_secs": 20 }))).await })
    };
    let mut id = None;
    for _ in 0..100 {
        id = requests::requests().list().into_iter().find(|r| r.label == "Declined one").map(|r| r.id);
        if id.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "decline_request", "request_id": id.unwrap() }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, answer) = ask.await.unwrap();
    assert_eq!(answer["status"], "declined");
}

// §5.3: only the host says a Vault pane is showing; agents only ask.
#[tokio::test]
async fn only_the_host_says_a_vault_pane_is_showing() {
    let app = app().await;
    let on_screen = |app: axum::Router| async move { send(&app, get("/api/v1/vault/on_screen")).await.1["on_screen"].clone() };
    let note = |pane: &str, on: bool| json!({ "op": "on_screen", "pane": pane, "on": on });
    assert_eq!(on_screen(app.clone()).await, json!(false));
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], note("p1", true))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], note("p1", true))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(on_screen(app.clone()).await, json!(true));
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], note("p1", false))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't clear it");
    assert_eq!(on_screen(app.clone()).await, json!(true));
    send(&app, post("/api/v1/host/vault", &[HOST], note("p1", false))).await;
    assert_eq!(on_screen(app).await, json!(false));
}

// Two Vault panes: one hiding leaves the other's note in place.
#[tokio::test]
async fn one_pane_hiding_doesnt_clear_anothers_note() {
    let app = app().await;
    let on_screen = |app: axum::Router| async move { send(&app, get("/api/v1/vault/on_screen")).await.1["on_screen"].clone() };
    let note = |pane: &str, on: bool| json!({ "op": "on_screen", "pane": pane, "on": on });
    send(&app, post("/api/v1/host/vault", &[HOST], note("p1", true))).await;
    send(&app, post("/api/v1/host/vault", &[HOST], note("p2", true))).await;
    send(&app, post("/api/v1/host/vault", &[HOST], note("p1", false))).await;
    assert_eq!(on_screen(app.clone()).await, json!(true), "p2 still shows");
    send(&app, post("/api/v1/host/vault", &[HOST], note("p2", false))).await;
    assert_eq!(on_screen(app.clone()).await, json!(false));
    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "on_screen", "on": true }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a note names its pane");
}

#[tokio::test]
async fn a_request_for_an_unknown_kind_or_without_a_label_is_refused() {
    let app = app().await;
    let (s, body) = send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "rocket", "label": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("api-key"), "it lists the kinds");
    let (s, _) = send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "ssn", "label": "  " }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
