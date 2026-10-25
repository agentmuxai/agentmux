// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A request whose token names an agent may claim only that agent as its
//! sender, and only the operator's window broadcasts as the user. The
//! per-site `m4.actor_*` counters still move with a token, so these tests take
//! the lock the counter test takes.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::backend::storage::agents::test_agent_def;
use crate::server::routes::build_router;
use crate::server::tests::{test_state, M4_ACTOR_COUNTERS};
use crate::server::AppState;

/// A state with agents "sc-alpha" and "sc-beta" and alpha's token.
fn state_with_alpha() -> (AppState, String) {
    let state = test_state();
    for (id, name, slug) in [("uid-sc-alpha", "Alpha Agent", "sc-alpha"), ("uid-sc-beta", "Beta Agent", "sc-beta")] {
        let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-sc-alpha").unwrap();
    (state, token)
}

async fn post(state: &AppState, token: Option<&str>, uri: &str, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = build_router(state.clone()).oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn refused(v: &Value) -> bool {
    v["error"].as_str().is_some_and(|e| e.starts_with("FORBIDDEN"))
}

/// The four sender-claiming routes, each with `from` as the claimed sender.
fn sender_requests(from: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("/agentmux/reactive/inject", json!({ "target_agent": "sc-nobody", "message": "hi", "source_agent": from })),
        ("/api/bus/send", json!({ "from": from, "to": "sc-nobody", "payload": "hi" })),
        ("/api/bus/inject", json!({ "from": from, "target": "sc-nobody", "message": "hi" })),
        ("/api/bus/broadcast", json!({ "from": from, "payload": "hi" })),
    ]
}

#[tokio::test]
async fn a_token_cannot_send_as_another_agent() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    for (uri, body) in sender_requests("sc-beta") {
        let (status, v) = post(&state, Some(&token), uri, body).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(refused(&v), "{uri}: {v}");
    }
}

#[tokio::test]
async fn a_token_may_send_as_itself_by_any_name_it_answers_to() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    for name in ["sc-alpha", "Alpha Agent", "alpha agent"] {
        for (uri, body) in sender_requests(name) {
            let (_, v) = post(&state, Some(&token), uri, body).await;
            assert!(!refused(&v), "{uri} as {name}: {v}");
        }
    }
}

/// An agent renamed while it runs keeps sending under the slug its environment held at launch.
#[tokio::test]
async fn a_token_may_still_send_under_a_slug_its_agent_was_renamed_from() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    state.mstore.test_rename_slug("uid-sc-alpha", "sc-alpha-renamed", 7);
    for (uri, body) in sender_requests("sc-alpha") {
        let (_, v) = post(&state, Some(&token), uri, body).await;
        assert!(!refused(&v), "{uri} under the old slug: {v}");
    }
    for (uri, body) in sender_requests("sc-beta") {
        let (_, v) = post(&state, Some(&token), uri, body).await;
        assert!(refused(&v), "{uri} still refuses another agent: {v}");
    }
}

/// With no token there is nothing to compare the claim with: the UI, `muxsh` and a peer srv send as before.
#[tokio::test]
async fn an_unattributed_request_may_name_any_sender() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, _token) = state_with_alpha();
    for (uri, body) in sender_requests("sc-beta") {
        let (status, v) = post(&state, None, uri, body).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(!refused(&v), "{uri}: {v}");
    }
}

#[tokio::test]
async fn a_token_that_names_no_sender_is_not_refused() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    let (_, v) = post(&state, Some(&token), "/agentmux/reactive/inject", json!({ "target_agent": "sc-nobody", "message": "hi" })).await;
    assert!(!refused(&v), "{v}");
}

/// The forward a Swarm broadcast takes to another channel comes from a server, never an agent.
#[tokio::test]
async fn the_cross_channel_broadcast_forward_refuses_a_request_that_carries_an_agent_token() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    let body = json!({ "block_id": "sc-no-such-block", "body": "x", "msg_id": "m", "recipients": 1 });
    let (status, v) = post(&state, Some(&token), "/agentmux/agent/broadcast", body.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert!(refused(&v), "{v}");
    let (status, v) = post(&state, None, "/agentmux/agent/broadcast", body).await;
    assert_eq!(status, StatusCode::OK, "a server's forward is served as before: {v}");
    assert_eq!(v["success"], false);
    assert!(!refused(&v), "{v}");
}

async fn rpc(state: &AppState, ctx: RpcContext, command: &str, data: Value) -> Result<Value, String> {
    let (engine, mut rx) = WshRpcEngine::new();
    crate::server::app_api::fleet::register(&engine, state);
    engine.set_rpc_context(ctx);
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() {
        Ok(resp.data.unwrap_or_default())
    } else {
        Err(resp.error)
    }
}

/// Only the window the host proved sends a broadcast: not a connection registered as an agent (whatever
/// it registered as), and not one that never registered but was never proven either.
#[tokio::test]
async fn only_the_operators_window_may_broadcast_as_the_user() {
    let state = test_state();
    let data = json!({ "targets": ["sc-no-such-block"], "message": "x" });
    for ctx in [
        RpcContext { agent_id: "sc-alpha".to_string(), ..Default::default() },
        RpcContext::default(),
    ] {
        let e = rpc(&state, ctx, "fleet.broadcast", data.clone()).await.unwrap_err();
        assert!(e.starts_with("FORBIDDEN: only the operator broadcasts as the user"), "{e}");
    }
    let sent = rpc(&state, RpcContext::operator_for_tests(), "fleet.broadcast", data).await.unwrap();
    assert_eq!(sent["failed"].as_array().map(Vec::len), Some(1), "the operator's broadcast ran, and its one target was unknown: {sent}");
}
