// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The model gateway end to end: a real request through the real router to a stand-in upstream on
//! loopback, with the key in a Vault entry. Nothing here reaches a network outside the machine.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

use crate::backend::model_gateway::{catalog, launch, spend, token};
use crate::backend::storage::agents::test_agent_def;
use crate::backend::vault::{self, NewEntry};
use crate::server::routes::build_router;
use crate::server::tests::test_state;
use crate::server::AppState;

const KEY: &str = "sk-or-test-key-0000";
const SSE: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"model\":\"deepseek/deepseek-v4.1-flash-1017\",\"usage\":{\"input_tokens\":1000,\"output_tokens\":1}}}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2000}}\n\n",
);

#[derive(Default)]
struct Seen {
    authorization: Option<String>,
    x_api_key: Option<String>,
    path: String,
    body: Value,
}

/// A stand-in for the route's upstream: records what it was sent and answers with a stream.
async fn upstream(seen: Arc<Mutex<Seen>>) -> String {
    async fn messages(State(seen): State<Arc<Mutex<Seen>>>, headers: HeaderMap, body: String) -> impl IntoResponse {
        let text = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
        *seen.lock().unwrap() = Seen { authorization: text("authorization"), x_api_key: text("x-api-key"), path: "/v1/messages".into(), body: serde_json::from_str(&body).unwrap_or_default() };
        ([(header::CONTENT_TYPE, "text/event-stream")], SSE)
    }
    let app = axum::Router::new().route("/v1/messages", post(messages)).with_state(seen);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// An agent on the `openrouter` route with a key in the Vault; returns its gateway token.
fn agent_on_route(state: &AppState, name: &str, with_key: bool) -> (String, String) {
    crate::backend::vault::seal::use_test_key();
    let uid = format!("uid-{name}");
    let mut def = test_agent_def(&uid, name, "claude", "agent", 1, "");
    def.slug = name.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    launch::set_choice(&state.mstore, &uid, "claude", "openrouter", "").unwrap();
    if with_key {
        let entry: NewEntry = serde_json::from_value(json!({ "kind": "api-key", "label": "OpenRouter API key", "parts": { "value": KEY } })).unwrap();
        vault::create(&state.identity_store, entry).unwrap();
    }
    (token::token_for(name, &uid, "openrouter"), uid)
}

async fn call(state: &AppState, token: Option<&str>, uri: &str, body: Value) -> (StatusCode, String) {
    let mut req = Request::builder().method(Method::POST).uri(uri).header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let resp = build_router(state.clone()).oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn a_request_reaches_the_upstream_with_the_key_and_the_agents_model_and_is_counted() {
    let state = test_state();
    let seen = Arc::new(Mutex::new(Seen::default()));
    *catalog::TEST_BASE_URL.lock().unwrap() = Some(upstream(seen.clone()).await);
    let (token, _) = agent_on_route(&state, "gw-e2e", true);

    let body = json!({ "model": "claude-haiku-4-5-20251001", "max_tokens": 16, "stream": true, "messages": [{ "role": "user", "content": "hi" }] });
    let (status, answer) = call(&state, Some(&token), "/api/v1/model/openrouter/v1/messages", body).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer, SSE, "the stream passes through unchanged");
    assert!(!answer.contains(KEY) && !answer.contains(&token));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.authorization.as_deref(), Some(&format!("Bearer {KEY}")[..]), "the upstream gets the route's key");
    assert_eq!(seen.x_api_key, None, "and not the harness's token");
    assert_eq!(seen.body["model"], "deepseek/deepseek-v4.1-flash-1017", "a Claude name is mapped to the agent's model");
    assert_eq!(seen.body["max_tokens"], 16);

    let used = spend::today("gw-e2e", agentmux_common::time::now_ms());
    assert_eq!((used.requests, used.tokens.input, used.tokens.output), (1, 1000, 2000));
    let price = catalog::route("openrouter").unwrap().model("deepseek/deepseek-v4.1-flash-1017").unwrap();
    assert!((used.cost_usd - spend::cost(price, &used.tokens)).abs() < 1e-12);

    let log = vault::activity(&state.identity_store, 10).unwrap();
    assert!(log.iter().any(|a| a.actor == "gw-e2e" && a.action == vault::route_key::USED), "the use is in the Vault's log under the agent");
    assert!(!serde_json::to_string(&log).unwrap().contains(KEY));
}

#[tokio::test]
async fn a_request_the_gateway_should_not_forward_is_refused_before_anything_leaves() {
    let state = test_state();
    let (token, _) = agent_on_route(&state, "gw-refused", true);
    let body = json!({ "model": "x" });

    let (status, why) = call(&state, None, "/api/v1/model/openrouter/v1/messages", body.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{why}");
    let (status, _) = call(&state, Some("amxg_not-minted"), "/api/v1/model/openrouter/v1/messages", body.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&state, Some(&token), "/api/v1/model/nowhere/v1/messages", body.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a token is for its own route");
    let (status, _) = call(&state, Some(&token), "/api/v1/model/openrouter/v1/credits", body.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "only the harness's own paths");

    token::revoke("uid-gw-refused", "openrouter");
    let (status, _) = call(&state, Some(&token), "/api/v1/model/openrouter/v1/messages", body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a revoked token stops at once");
}

#[tokio::test]
async fn a_route_without_a_key_says_so_and_sends_nothing() {
    let state = test_state();
    let (token, _) = agent_on_route(&state, "gw-nokey", false);
    let (status, why) = call(&state, Some(&token), "/api/v1/model/openrouter/v1/messages", json!({ "model": "x" })).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(why.contains("OpenRouter API key"), "{why}");
}

#[tokio::test]
async fn an_agent_not_set_to_the_route_is_refused_even_with_a_token() {
    let state = test_state();
    let uid = "uid-gw-other";
    let mut def = test_agent_def(uid, "gw-other", "claude", "agent", 1, "");
    def.slug = "gw-other".to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let token = token::token_for("gw-other", uid, "openrouter");
    let (status, _) = call(&state, Some(&token), "/api/v1/model/openrouter/v1/messages", json!({ "model": "x" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
