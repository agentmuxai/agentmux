// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sending from a paired device (agentmux-mobile's
//! SPEC_SEND_FROM_THE_PHONE_2026_10_13 §4, §5.2.1, §9): who may send, the
//! signature, the body, the agent, and what a retry gets. Delivery itself is
//! the Swarm broadcast's path, tested there; here the agents have no
//! controller, so a send that passes every check is refused as not running.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use base64::Engine as _;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use tower::ServiceExt;

use super::*;
use crate::backend::viewer::{pairing, signing};
use crate::server::http_viewer::build_viewer_router;
use crate::server::tests::test_state;

struct Phone {
    device_id: String,
    token: String,
    key: EcdsaKeyPair,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A device paired with a P-256 key; `can_send` is the owner's grant.
fn phone(state: &AppState, can_send: bool) -> Phone {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let token = pairing::new_token();
    let device_id = uuid::Uuid::new_v4().to_string();
    let public = b64(key.public_key().as_ref());
    state.mstore.viewer_device_insert(&device_id, &pairing::token_hash(&token), "Pixel 9", &public, 1).unwrap();
    state.mstore.viewer_device_set_can_send(&device_id, can_send).unwrap();
    Phone { device_id, token, key }
}

fn agent(state: &AppState) -> String {
    let name = format!("send-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&name, &uuid::Uuid::new_v4().to_string(), None).unwrap();
    name
}

fn path_of(name: &str) -> String {
    format!("/agentmux/viewer/agents/{name}/input")
}

fn body_of(message: &str, message_id: &str) -> Vec<u8> {
    serde_json::json!({ "message": message, "message_id": message_id }).to_string().into_bytes()
}

/// A request signed the way the phone signs one, at `ts`.
fn signed(phone: &Phone, path: &str, body: Vec<u8>, ts: i64) -> Request<Body> {
    let text = signing::signed_text("POST", path, ts, &body);
    let sig = phone.key.sign(&SystemRandom::new(), text.as_bytes()).unwrap();
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("Authorization", format!("Bearer {}", phone.token))
        .header("Content-Type", "application/json")
        .header("X-Amxv-Timestamp", ts.to_string())
        .header("X-Amxv-Signature", b64(sig.as_ref()))
        .body(Body::from(body))
        .unwrap()
}

fn now() -> i64 {
    agentmux_common::time::now_ms()
}

async fn call(state: &AppState, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = build_viewer_router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tokio::test]
async fn sending_needs_the_owners_grant_and_a_key() {
    let state = test_state();
    let name = agent(&state);
    let off = phone(&state, false);
    let (status, json) = call(&state, signed(&off, &path_of(&name), body_of("hi", &id()), now())).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::FORBIDDEN, Some("send_off")));

    // Granted, but paired without a key (before devices had keys).
    let token = pairing::new_token();
    state.mstore.viewer_device_insert("keyless", &pairing::token_hash(&token), "Old", "", 1).unwrap();
    state.mstore.viewer_device_set_can_send("keyless", true).unwrap();
    let req = Request::builder()
        .method(Method::POST)
        .uri(path_of(&name))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(body_of("hi", &id())))
        .unwrap();
    let (status, json) = call(&state, req).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::FORBIDDEN, Some("pair_again")));

    // No token, or the lan_key's header: the auth middleware refuses first.
    let req = Request::builder().method(Method::POST).uri(path_of(&name)).body(Body::from(body_of("hi", &id()))).unwrap();
    assert_eq!(call(&state, req).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_signature_must_cover_this_request_and_be_fresh() {
    let state = test_state();
    let name = agent(&state);
    let phone_a = phone(&state, true);
    let path = path_of(&name);

    // Unsigned.
    let req = Request::builder()
        .method(Method::POST)
        .uri(&path)
        .header("Authorization", format!("Bearer {}", phone_a.token))
        .body(Body::from(body_of("hi", &id())))
        .unwrap();
    let (status, json) = call(&state, req).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::FORBIDDEN, Some("bad_signature")));

    // Signed over another body than the one sent.
    let mut req = signed(&phone_a, &path, body_of("hi", &id()), now());
    *req.body_mut() = Body::from(body_of("rm -r", &id()));
    let (status, json) = call(&state, req).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::FORBIDDEN, Some("bad_signature")));

    // Signed for another agent's path.
    let other = agent(&state);
    let mut req = signed(&phone_a, &path_of(&other), body_of("hi", &id()), now());
    *req.uri_mut() = path.parse().unwrap();
    let (status, json) = call(&state, req).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::FORBIDDEN, Some("bad_signature")));

    // Signed by another device's key with this device's token.
    let phone_b = phone(&state, true);
    let borrowed = Phone { device_id: phone_a.device_id.clone(), token: phone_a.token.clone(), key: phone_b.key };
    let (status, json) = call(&state, signed(&borrowed, &path, body_of("hi", &id()), now())).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::FORBIDDEN, Some("bad_signature")));

    // Signed two minutes ago, or ahead.
    for ts in [now() - 120_000, now() + 120_000] {
        let (status, json) = call(&state, signed(&phone_a, &path, body_of("hi", &id()), ts)).await;
        assert_eq!((status, json["reason"].as_str()), (StatusCode::FORBIDDEN, Some("stale_timestamp")));
    }
}

#[tokio::test]
async fn the_body_and_the_agent_are_checked_after_the_signature() {
    let state = test_state();
    let name = agent(&state);
    let phone = phone(&state, true);
    let path = path_of(&name);

    for body in [
        b"not json".to_vec(),
        body_of("   \n", &id()),
        body_of("hi", "not-a-uuid"),
        serde_json::json!({ "message": "hi" }).to_string().into_bytes(),
    ] {
        let (status, _) = call(&state, signed(&phone, &path, body.clone(), now())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&body));
    }
    let long = "x".repeat(crate::backend::reactive::MAX_MESSAGE_LENGTH + 1);
    assert_eq!(call(&state, signed(&phone, &path, body_of(&long, &id()), now())).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    let huge = "x".repeat(super::MAX_INPUT_BODY + 1);
    assert_eq!(call(&state, signed(&phone, &path, body_of(&huge, &id()), now())).await.0, StatusCode::PAYLOAD_TOO_LARGE);

    let (status, _) = call(&state, signed(&phone, &path_of("nobody"), body_of("hi", &id()), now())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_hidden_agent_is_not_found() {
    let state = test_state();
    let name = agent(&state);
    let block_id = state.reactive_handler.get_agent(&name).unwrap().block_id;
    let def_id = uuid::Uuid::new_v4().to_string();
    let mut def = crate::backend::storage::agents::test_agent_def(&def_id, &name, "claude", "standalone", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut block = crate::backend::obj::Block { oid: block_id, ..Default::default() };
    block.meta.insert("agentId".into(), serde_json::json!(def_id));
    state.mstore.insert(&mut block).unwrap();
    state.mstore.agent_hide_from_devices_set(&def_id, true).unwrap();
    let phone = phone(&state, true);
    let (status, _) = call(&state, signed(&phone, &path_of(&name), body_of("hi", &id()), now())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_agent_nothing_can_deliver_to_is_refused_and_the_message_can_be_retried() {
    let state = test_state();
    let name = agent(&state);
    let phone = phone(&state, true);
    let path = path_of(&name);
    let message_id = id();
    let (status, json) = call(&state, signed(&phone, &path, body_of("hi", &message_id), now())).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::CONFLICT, Some("not_running")), "{json}");
    // Not remembered as sent: the same message_id is tried again, not answered as a duplicate.
    let (status, json) = call(&state, signed(&phone, &path, body_of("hi", &message_id), now())).await;
    assert_eq!((status, json["reason"].as_str()), (StatusCode::CONFLICT, Some("not_running")));
    assert!(json.get("duplicate").is_none());
}

#[tokio::test]
async fn a_delivered_message_id_is_answered_from_memory_and_sends_are_limited() {
    use crate::backend::viewer::sends::{Admit, Sent, RATE_LIMIT};
    let state = test_state();
    let name = agent(&state);
    let phone = phone(&state, true);
    let path = path_of(&name);

    // A message the agent took earlier (the ledger as the route leaves it).
    let message_id = id();
    let t = std::time::Instant::now();
    assert_eq!(state.viewer.sends.lock().admit(&phone.device_id, &message_id, t), Admit::New);
    state.viewer.sends.lock().finish(&phone.device_id, &message_id, Some(Sent::Queued));
    let (status, json) = call(&state, signed(&phone, &path, body_of("hi", &message_id), now())).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(json, serde_json::json!({ "message_id": message_id, "state": "queued", "duplicate": true }));

    for _ in 1..RATE_LIMIT {
        state.viewer.sends.lock().admit(&phone.device_id, &id(), t);
    }
    let req = signed(&phone, &path, body_of("hi", &id()), now());
    let resp = build_viewer_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().get("retry-after").is_some());
}
