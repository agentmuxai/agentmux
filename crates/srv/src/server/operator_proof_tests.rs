// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Operator-only actions need the host's proof: a connection proves it is the
//! AgentMux window through the CEF host, once per nonce.

use axum::body::Body;
use axum::http::Request;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::backend::rpc_types::RpcMessage;

const PROBE: &str = "test.operator_probe";

/// An engine with `operator.challenge` and a probe answering whether the
/// connection passes `operator_only`.
fn engine() -> (Arc<WshRpcEngine>, tokio::sync::mpsc::UnboundedReceiver<RpcMessage>) {
    let (engine, rx) = WshRpcEngine::new();
    register_challenge(&engine);
    engine.register_handler(
        PROBE,
        Box::new(|_data, ctx| Box::pin(async move { ctx.operator_only("probe").map(|()| Some(json!(true))) })),
    );
    (engine, rx)
}

async fn call(
    engine: &Arc<WshRpcEngine>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    command: &str,
) -> Result<Value, String> {
    engine.handle_message(RpcMessage {
        command: command.to_string(),
        reqid: format!("req-{}", uuid::Uuid::new_v4()),
        data: Some(Value::Null),
        ..Default::default()
    });
    let resp = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() {
        Ok(resp.data.unwrap_or_default())
    } else {
        Err(resp.error)
    }
}

async fn challenge(engine: &Arc<WshRpcEngine>, rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>) -> String {
    let v = call(engine, rx, COMMAND_OPERATOR_CHALLENGE).await.expect("challenge");
    v["nonce"].as_str().expect("nonce").to_string()
}

fn as_agent(engine: &WshRpcEngine, agent: &str) {
    engine.set_rpc_context(RpcContext { agent_id: agent.to_string(), ..Default::default() });
}

#[test]
fn operator_only_needs_the_proof_and_no_agent_registration() {
    let unproven = RpcContext::default();
    let err = unproven.operator_only("only the operator does this").unwrap_err();
    assert_eq!(err, "FORBIDDEN: only the operator does this (this connection isn't the AgentMux window)");

    let agent = RpcContext { agent_id: "AgentX".into(), ..Default::default() };
    assert_eq!(agent.operator_only("only the operator does this").unwrap_err(), "FORBIDDEN: only the operator does this");
    let agent_claiming = RpcContext { agent_id: "AgentX".into(), operator_proven: true, ..Default::default() };
    assert!(agent_claiming.operator_only("x").is_err());

    assert!(RpcContext::operator_for_tests().operator_only("x").is_ok());
}

#[tokio::test]
async fn an_unproven_connection_is_refused_until_the_host_attests_it() {
    let (engine, mut rx) = engine();
    let err = call(&engine, &mut rx, PROBE).await.unwrap_err();
    assert!(err.contains("isn't the AgentMux window"), "{err}");

    let nonce = challenge(&engine, &mut rx).await;
    // Asking for a challenge proves nothing by itself.
    assert!(call(&engine, &mut rx, PROBE).await.is_err());
    assert!(attest(&nonce));
    assert_eq!(call(&engine, &mut rx, PROBE).await.unwrap(), json!(true));
}

#[tokio::test]
async fn a_nonce_is_spent_once_and_proves_only_its_own_connection() {
    let (window, mut window_rx) = engine();
    let (other, mut other_rx) = engine();
    let nonce = challenge(&window, &mut window_rx).await;
    assert!(attest(&nonce));
    assert!(!attest(&nonce), "a nonce is single-use");
    assert!(call(&other, &mut other_rx, PROBE).await.is_err(), "the proof belongs to the connection it was issued to");
    assert!(!attest("not-a-nonce"));
    assert!(!attest(""));
}

#[tokio::test]
async fn registering_as_an_agent_drops_the_proof() {
    let (engine, mut rx) = engine();
    let nonce = challenge(&engine, &mut rx).await;
    assert!(attest(&nonce));
    assert!(call(&engine, &mut rx, PROBE).await.is_ok());

    as_agent(&engine, "AgentX");
    let err = call(&engine, &mut rx, PROBE).await.unwrap_err();
    assert_eq!(err, "FORBIDDEN: probe");
}

#[tokio::test]
async fn a_connection_registered_as_an_agent_gets_no_challenge_and_no_proof() {
    let (engine, mut rx) = engine();
    let nonce = challenge(&engine, &mut rx).await;
    as_agent(&engine, "AgentX");
    let err = call(&engine, &mut rx, COMMAND_OPERATOR_CHALLENGE).await.unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");
    // A nonce issued before the registration doesn't prove it either.
    assert!(!attest(&nonce));
    assert!(!engine.mark_operator_proven());
    assert!(call(&engine, &mut rx, PROBE).await.is_err());
}

#[test]
fn a_nonce_expires() {
    let (engine, _rx) = WshRpcEngine::new();
    let mut c = Challenges::default();
    let t0 = Instant::now();
    let nonce = c.issue(&engine, t0);
    assert!(c.take(&nonce, t0 + TTL).is_none(), "expired at the TTL");
    assert!(c.take(&nonce, t0).is_none(), "and spent by the failed attempt");

    let fresh = c.issue(&engine, t0);
    assert!(c.take(&fresh, t0 + TTL - Duration::from_secs(1)).is_some());
}

#[test]
fn a_closed_connection_cannot_be_proven() {
    let mut c = Challenges::default();
    let t0 = Instant::now();
    let nonce = {
        let (engine, _rx) = WshRpcEngine::new();
        c.issue(&engine, t0)
    };
    assert!(c.take(&nonce, t0).is_none());
}

#[test]
fn waiting_challenges_are_bounded_and_the_oldest_go_first() {
    let (engine, _rx) = WshRpcEngine::new();
    let mut c = Challenges::default();
    let t0 = Instant::now();
    let first = c.issue(&engine, t0);
    for i in 1..(MAX_PENDING as u64 + 20) {
        c.issue(&engine, t0 + Duration::from_millis(i));
    }
    assert!(c.pending.len() <= MAX_PENDING, "{}", c.pending.len());
    assert!(c.take(&first, t0 + Duration::from_secs(1)).is_none(), "the oldest was dropped");

    // Expired ones are cleared before anything is evicted.
    let later = t0 + TTL + Duration::from_secs(5);
    c.issue(&engine, later);
    assert_eq!(c.pending.len(), 1);
}

async fn post(app: &axum::Router, host_token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .uri("/api/v1/host/operator_attest")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    if let Some(token) = host_token {
        b = b.header("X-Host-Token", token);
    }
    let resp = app.clone().oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
async fn only_the_host_can_attest_and_it_proves_the_connection() {
    let state = crate::server::tests::test_state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
    let app = crate::server::build_router(state);
    let (engine, mut rx) = engine();
    let nonce = challenge(&engine, &mut rx).await;

    // The instance auth key alone, or a wrong host token: refused, and the
    // nonce is not spent.
    for token in [None, Some("not-the-token")] {
        let (s, out) = post(&app, token, json!({ "nonce": nonce })).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{out}");
        assert_eq!(out["ok"], false);
    }
    assert!(call(&engine, &mut rx, PROBE).await.is_err());

    let (s, out) = post(&app, Some("host-ipc-token"), json!({ "nonce": nonce })).await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["ok"], true);
    assert!(call(&engine, &mut rx, PROBE).await.is_ok());

    let (s, out) = post(&app, Some("host-ipc-token"), json!({ "nonce": nonce })).await;
    assert_eq!(s, StatusCode::CONFLICT, "a spent nonce: {out}");
    let (s, _) = post(&app, Some("host-ipc-token"), json!({ "nonce": "unknown" })).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn no_host_registered_means_no_attestation() {
    let state = crate::server::tests::test_state();
    let app = crate::server::build_router(state);
    let (engine, mut rx) = engine();
    let nonce = challenge(&engine, &mut rx).await;
    let (s, _) = post(&app, Some(""), json!({ "nonce": nonce })).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
