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
        assert!(e.starts_with("FORBIDDEN: a Swarm broadcast arrives as the operator's own words"), "{e}");
    }
    let sent = rpc(&state, RpcContext::operator_for_tests(), "fleet.broadcast", data).await.unwrap();
    assert_eq!(sent["failed"].as_array().map(Vec::len), Some(1), "the operator's broadcast ran, and its one target was unknown: {sent}");
}

/// The bus's HTTP inbox routes act for the agent they name, so each needs that agent's token.
#[tokio::test]
async fn the_bus_inbox_routes_need_the_token_of_the_agent_they_name() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    let reads = |agent: &str| (json!({ "agent_id": agent }), json!({ "agent_id": agent, "message_ids": [] }));
    for (agent, token, allowed) in [
        ("sc-alpha", Some(token.as_str()), true),
        ("Alpha Agent", Some(token.as_str()), true),
        ("sc-beta", Some(token.as_str()), false),
        ("sc-alpha", None, false),
    ] {
        let (register, delete) = reads(agent);
        for (uri, body) in [("/api/bus/register", register), ("/api/bus/messages/delete", delete)] {
            let (_, v) = post(&state, token, uri, body).await;
            assert_eq!(!refused(&v), allowed, "{uri} for {agent} (token: {}): {v}", token.is_some());
        }
    }
}

async fn get(state: &AppState, token: Option<&str>, uri: &str) -> (StatusCode, Value) {
    let mut req = Request::builder().method(Method::GET).uri(uri).header("X-AuthKey", "test-secret-key");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = build_router(state.clone()).oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn no_token_refusal(v: &Value) -> bool {
    v["error"].as_str().is_some_and(|e| e.contains("must carry that agent's token"))
}

/// Memory, accounts and presets belong to the agent whose token the request carries; one that carries none
/// has no owner, and its `agent_id` is only a claim.
#[tokio::test]
async fn memory_and_identity_routes_refuse_a_request_with_no_token() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    let q = "agent_id=sc-beta";
    let gets = [
        format!("/api/v1/agent/memory/list?{q}"),
        format!("/api/v1/agent/memory/read?{q}&filename=MEMORY.md"),
        format!("/api/v1/agent/memory/history?{q}&filename=MEMORY.md"),
        format!("/api/v1/agent/memory/diff?{q}&filename=MEMORY.md&from_version_id=a&to_version_id=b"),
        format!("/api/v1/agent/identity/accounts?{q}"),
        format!("/api/v1/agent/preset/get?{q}"),
    ];
    for uri in &gets {
        let (status, v) = get(&state, None, uri).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {v}");
        let (_, v) = get(&state, Some(&token), uri).await;
        assert!(!no_token_refusal(&v), "{uri} with a token: {v}");
    }
    let posts = [
        ("/api/v1/agent/memory/write", json!({ "agent_id": "sc-beta", "filename": "MEMORY.md", "content": "x" })),
        ("/api/v1/agent/memory/revert", json!({ "agent_id": "sc-beta", "filename": "MEMORY.md", "target_version_id": "v" })),
        ("/api/v1/agent/identity/validate", json!({ "agent_id": "sc-beta", "account_id": "a" })),
    ];
    for (uri, body) in posts {
        let (status, v) = post(&state, None, uri, body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {v}");
        let (_, v) = post(&state, Some(&token), uri, body).await;
        assert!(!no_token_refusal(&v), "{uri} with a token: {v}");
    }
}

/// A Global Memory write over HTTP is recorded under the agent whose token made it; a request can't write as another.
#[tokio::test]
async fn a_token_cannot_write_or_revert_global_memory_as_another_agent() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    let (_, v) = post(&state, Some(&token), "/api/v1/agent/globalmemory/write", json!({ "agent_id": "sc-beta", "name": "Rules", "content": "x" })).await;
    assert!(refused(&v), "{v}");
    let (_, v) = post(&state, Some(&token), "/api/v1/agent/globalmemory/revert", json!({ "agent_id": "sc-beta", "id": "g", "version_id": "v" })).await;
    assert!(refused(&v), "{v}");
    let (_, v) = post(&state, Some(&token), "/api/v1/agent/globalmemory/write", json!({ "agent_id": "sc-alpha", "name": "Rules", "content": "x" })).await;
    assert!(!refused(&v), "an agent writes Global Memory as itself: {v}");
}

async fn rpc_bundle(state: &AppState, ctx: RpcContext, command: &str, data: Value) -> Result<Value, String> {
    let (engine, mut rx) = WshRpcEngine::new();
    crate::server::agent_handlers::bundle::register(&engine, state);
    engine.set_rpc_context(ctx);
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() {
        Ok(resp.data.unwrap_or_default())
    } else {
        Err(resp.error)
    }
}

/// An agent that saves Global Memory over the socket is recorded as that agent; the window is recorded as the window.
#[tokio::test]
async fn a_global_memory_save_over_the_socket_is_recorded_as_whoever_made_it() {
    let state = test_state();
    for (id, ctx, writer, source) in [
        ("sc-g-agent", RpcContext { agent_id: "sc-alpha".to_string(), ..Default::default() }, "sc-alpha", "agent"),
        ("sc-g-window", RpcContext::operator_for_tests(), "armory-ui", "human"),
    ] {
        let bundle = json!({ "id": id, "name": id, "instructions": "be kind", "is_global": true });
        rpc_bundle(&state, ctx, "upsertmemory", bundle).await.unwrap();
        let versions = state.id_store.bundle_version_list(id).unwrap();
        assert_eq!(versions.len(), 1, "{id}");
        assert_eq!((versions[0].written_by.as_str(), versions[0].source.as_str()), (writer, source), "{id}");
    }
}

/// The work queue and jobs act for an agent: each route refuses a request with no token, and a token is
/// enough to get past the guard (what the handler then says is its own business).
#[tokio::test]
async fn the_work_and_jobs_routes_refuse_a_request_with_no_token() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    let routes = [
        ("POST", "/agentmux/work"),
        ("POST", "/agentmux/work/claim"),
        ("POST", "/agentmux/work/w1/heartbeat"),
        ("POST", "/agentmux/work/w1/complete"),
        ("POST", "/agentmux/work/w1/release"),
        ("POST", "/agentmux/work/w1/update"),
        ("POST", "/agentmux/work/w1/ask"),
        ("POST", "/agentmux/work/w1/submit"),
        ("POST", "/agentmux/work/w1/hold"),
        ("POST", "/agentmux/work/w1/unhold"),
        ("DELETE", "/agentmux/work/w1"),
        ("POST", "/agentmux/jobs"),
        ("DELETE", "/agentmux/jobs/j1"),
        ("PATCH", "/agentmux/jobs/j1"),
        ("POST", "/agentmux/jobs/j1/run"),
        ("POST", "/agentmux/jobs/runs/r1/report"),
        ("POST", "/agentmux/cron"),
        ("DELETE", "/agentmux/cron/j1"),
        ("PATCH", "/agentmux/cron/j1"),
        ("POST", "/agentmux/cron/j1/run"),
        ("POST", "/agentmux/cron/runs/r1/report"),
    ];
    for (method, uri) in routes {
        for (token, guarded) in [(None, true), (Some(token.as_str()), false)] {
            let mut req = Request::builder().method(method).uri(uri).header("X-AuthKey", "test-secret-key").header("Content-Type", "application/json");
            if let Some(t) = token {
                req = req.header("X-Agent-Token", t);
            }
            let resp = build_router(state.clone()).oneshot(req.body(Body::from("{}")).unwrap()).await.unwrap();
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            let said_no_token = String::from_utf8_lossy(&bytes).contains("must carry that agent's token");
            assert_eq!(said_no_token, guarded, "{method} {uri} (token: {}): {status}", token.is_some());
        }
    }
}

/// A token acts as its own agent on the work and jobs routes: the agent a body names has to be it.
#[tokio::test]
async fn the_work_and_jobs_routes_refuse_a_body_that_names_another_agent() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let (state, token) = state_with_alpha();
    let names_another = |v: &Value| v["error"].as_str().is_some_and(|e| e.contains("the request names another"));
    for (uri, key) in [
        ("/agentmux/work/claim", "agent_id"),
        ("/agentmux/work/w1/complete", "agent_id"),
        ("/agentmux/work/w1/hold", "agent_id"),
        ("/agentmux/work", "created_by"),
        ("/agentmux/jobs", "created_by"),
    ] {
        let (_, v) = post(&state, Some(&token), uri, json!({ key: "sc-beta", "title": "t", "attempt": 1 })).await;
        assert!(names_another(&v), "{uri}: {v}");
        let (_, v) = post(&state, Some(&token), uri, json!({ key: "sc-alpha", "title": "t", "attempt": 1 })).await;
        assert!(!names_another(&v), "{uri} as itself: {v}");
    }
}
