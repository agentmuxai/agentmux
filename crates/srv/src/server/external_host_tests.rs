// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A registered external host vouches for a connection: what it may do, and what nothing else may.

use ed25519_dalek::{Signer, SigningKey};
use serde_json::Value;

use super::*;
use crate::backend::rpc::actor::{actor_of, Actor};
use crate::backend::rpc_types::RpcContext;

const WORKSPACE: &str = "ws-test-1";
const OTHER_CONN: &str = "connection-id-other-0001";
const CONN: &str = "connection-id-0000001";
const PERSON: &str = "4f2c1e90-aaaa-bbbb-cccc-0123456789ab";

fn signer() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn host_for(key: &SigningKey, workspace: &str) -> Host {
    let text = json!({ "v": 1, "workspace": workspace, "public_key": URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes()) });
    Host::from_file_text(&text.to_string()).expect("a valid key file")
}

fn host() -> Host {
    host_for(&signer(), WORKSPACE)
}

fn sign(key: &SigningKey, text: &str) -> String {
    URL_SAFE_NO_PAD.encode(key.sign(text.as_bytes()).to_bytes())
}

fn conn_header(key: &SigningKey, workspace: &str, id: &str) -> String {
    format!("{id}.{}", sign(key, &format!("amx-conn/v1|{workspace}|{id}")))
}

/// A connection the host proxied: it holds the id and has asked for a challenge.
fn proxied(host: &Host, id: &str) -> (Arc<WshRpcEngine>, String) {
    let (engine, _rx) = WshRpcEngine::new();
    // The receiver must outlive the engine's use; a leaked channel end keeps the sender open.
    std::mem::forget(_rx);
    assert!(host.attach(id, &engine));
    engine.set_gateway_conn(id.to_string());
    let nonce = super::super::operator_proof::issue(&engine);
    (engine, nonce)
}

fn attest_req(key: &SigningKey, conn: &str, nonce: &str, person: &str, valid_until: u64) -> AttestRequest {
    let text = format!("amx-attest/v1|{WORKSPACE}|{conn}|{nonce}|{person}|{valid_until}");
    AttestRequest {
        v: 1,
        nonce: nonce.to_string(),
        conn: conn.to_string(),
        workspace: WORKSPACE.to_string(),
        person: person.to_string(),
        valid_until,
        sig: sign(key, &text),
    }
}

fn status(r: Result<String, Refusal>) -> u16 {
    r.expect_err("it was refused").0.as_u16()
}

#[test]
fn the_key_file_is_checked() {
    let key = signer();
    let good = |v: Value| Host::from_file_text(&v.to_string());
    let public = URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes());
    assert!(good(json!({ "v": 1, "workspace": "ws-1", "public_key": public })).is_ok());
    assert!(good(json!({ "v": 2, "workspace": "ws-1", "public_key": public })).is_err(), "another version");
    assert!(good(json!({ "v": 1, "workspace": "ws 1|x", "public_key": public })).is_err(), "a workspace with a separator");
    assert!(good(json!({ "v": 1, "workspace": "ws-1", "public_key": "AAAA" })).is_err(), "a short key");
    assert!(good(json!({ "v": 1, "workspace": "ws-1", "public_key": "!!!" })).is_err(), "not base64url");
    assert!(Host::from_file_text("not json").is_err());
}

#[test]
fn the_fingerprint_names_the_public_key_and_nothing_else() {
    // The key made from the seed [7; 32]; the same value the host's own tests check, so the two sides name a key alike.
    let fp = host().fingerprint();
    assert_eq!(fp, "fe812c12f3ab4ce6");
    assert_eq!(fp.len(), 16);
    let other = host_for(&SigningKey::from_bytes(&[8u8; 32]), WORKSPACE).fingerprint();
    assert_ne!(fp, other, "another key has another fingerprint");
    // Nothing of the private key or the whole public key is in it.
    assert!(!fp.contains(&hex::encode(signer().to_bytes())[..8]));
    assert!(!hex::encode(signer().verifying_key().to_bytes()).starts_with(&fp));
}

#[test]
fn a_second_registration_is_refused() {
    let cell = OnceLock::new();
    assert!(set_once(&cell, host()).is_ok());
    let other = SigningKey::from_bytes(&[9u8; 32]);
    assert!(set_once(&cell, host_for(&other, WORKSPACE)).is_err(), "an agent registering its own key");
    // The first key is still the one in force.
    let held: &Host = cell.get().unwrap();
    assert!(held.conn_from_header(&conn_header(&signer(), WORKSPACE, CONN)).is_some());
    assert!(held.conn_from_header(&conn_header(&other, WORKSPACE, CONN)).is_none());
}

#[test]
fn only_the_hosts_signature_gives_a_connection_an_id() {
    let h = host();
    assert_eq!(h.conn_from_header(&conn_header(&signer(), WORKSPACE, CONN)).as_deref(), Some(CONN));
    let stranger = SigningKey::from_bytes(&[1u8; 32]);
    assert!(h.conn_from_header(&conn_header(&stranger, WORKSPACE, CONN)).is_none(), "another key");
    assert!(h.conn_from_header(&conn_header(&signer(), "ws-other", CONN)).is_none(), "another workspace");
    let signed = conn_header(&signer(), WORKSPACE, CONN);
    let (_, sig) = signed.split_once('.').unwrap();
    assert!(h.conn_from_header(&format!("{OTHER_CONN}.{sig}")).is_none(), "another id under the same signature");
    assert!(h.conn_from_header(CONN).is_none(), "an unsigned id");
    assert!(h.conn_from_header("short.AAAA").is_none());
    assert!(h.conn_from_header("").is_none());
}

#[test]
fn a_connection_the_host_proxied_is_attested_for_the_person() {
    let h = host();
    let (engine, nonce) = proxied(&h, CONN);
    let now = agentmux_common::time::now_secs_u64();
    let until = now + 300;
    let person = h.attest(&attest_req(&signer(), CONN, &nonce, PERSON, until), now).expect("attested");
    assert_eq!(person, PERSON);
    let ctx = engine.rpc_context();
    assert!(ctx.operator_proven_now());
    assert_eq!(ctx.operator_person.as_deref(), Some(PERSON));
    assert_eq!(actor_of(&ctx), Actor::Person(PERSON.to_string()));
    assert_eq!(actor_of(&ctx).label(), format!("person:{PERSON} (AgentMux Web)"));
    assert!(ctx.operator_only("probe").is_ok());
}

#[test]
fn a_connection_opened_on_loopback_is_never_attested() {
    let h = host();
    // An agent opens its own connection straight to srv: no id from the host, but it can ask for a challenge.
    let (engine, _rx) = WshRpcEngine::new();
    std::mem::forget(_rx);
    let nonce = super::super::operator_proof::issue(&engine);
    // Even a correctly signed attestation, naming an id the host really issued to someone else, is refused.
    let (_person_engine, _their_nonce) = proxied(&h, CONN);
    let now = agentmux_common::time::now_secs_u64();
    assert_eq!(status(h.attest(&attest_req(&signer(), CONN, &nonce, PERSON, now + 300), now)), 403);
    assert!(!engine.rpc_context().operator_proven_now());
    // The lookup the host makes before it signs says the connection has no id, so it never signs.
    assert!(super::super::operator_proof::peek_engine(&nonce).and_then(|e| e.gateway_conn()).is_none());
}

#[test]
fn a_page_tricked_into_posting_another_connections_nonce_does_not_attest_it() {
    let h = host();
    let (mine, my_nonce) = proxied(&h, CONN);
    let (theirs, their_nonce) = proxied(&h, OTHER_CONN);
    let now = agentmux_common::time::now_secs_u64();
    // The host signed for my connection but with the other connection's nonce: the nonce's connection is not CONN.
    assert_eq!(status(h.attest(&attest_req(&signer(), CONN, &their_nonce, PERSON, now + 300), now)), 403);
    assert!(!theirs.rpc_context().operator_proven_now());
    assert!(!mine.rpc_context().operator_proven_now());
    // My own nonce, unaffected, still attests my connection.
    assert!(h.attest(&attest_req(&signer(), CONN, &my_nonce, PERSON, now + 300), now).is_ok());
}

#[test]
fn a_nonce_works_once() {
    let h = host();
    let (_engine, nonce) = proxied(&h, CONN);
    let now = agentmux_common::time::now_secs_u64();
    let req = attest_req(&signer(), CONN, &nonce, PERSON, now + 300);
    assert!(h.attest(&req, now).is_ok());
    assert_eq!(status(h.attest(&req, now)), 409, "the same signed message again");
}

#[test]
fn a_bad_attestation_is_refused_and_leaves_the_nonce_for_the_real_one() {
    let h = host();
    let (engine, nonce) = proxied(&h, CONN);
    let now = agentmux_common::time::now_secs_u64();
    let stranger = SigningKey::from_bytes(&[2u8; 32]);
    assert_eq!(status(h.attest(&attest_req(&stranger, CONN, &nonce, PERSON, now + 300), now)), 403, "another key");
    let mut other_workspace = attest_req(&signer(), CONN, &nonce, PERSON, now + 300);
    other_workspace.workspace = "ws-other".to_string();
    assert_eq!(status(h.attest(&other_workspace, now)), 403, "another workspace");
    let mut changed_person = attest_req(&signer(), CONN, &nonce, PERSON, now + 300);
    changed_person.person = "someone-else".to_string();
    assert_eq!(status(h.attest(&changed_person, now)), 403, "a changed person");
    let mut changed_until = attest_req(&signer(), CONN, &nonce, PERSON, now + 300);
    changed_until.valid_until += 60;
    assert_eq!(status(h.attest(&changed_until, now)), 403, "a changed lease");
    let mut bad_field = attest_req(&signer(), CONN, &nonce, PERSON, now + 300);
    bad_field.person = "a|b".to_string();
    assert_eq!(status(h.attest(&bad_field, now)), 400, "a separator in a field");
    assert!(!engine.rpc_context().operator_proven_now());
    // None of that spent the real nonce.
    assert!(h.attest(&attest_req(&signer(), CONN, &nonce, PERSON, now + 300), now).is_ok());
}

#[test]
fn the_lease_is_short_and_lapses() {
    let h = host();
    let now = agentmux_common::time::now_secs_u64();
    let (_e1, n1) = proxied(&h, CONN);
    assert_eq!(status(h.attest(&attest_req(&signer(), CONN, &n1, PERSON, now), now)), 403, "already over");
    let (_e2, n2) = proxied(&h, OTHER_CONN);
    assert_eq!(status(h.attest(&attest_req(&signer(), OTHER_CONN, &n2, PERSON, now + MAX_LEASE_SECS + 60), now)), 403, "too long");
    // A lease that has run out is no proof.
    let lapsed = RpcContext {
        operator_proven: true,
        operator_person: Some(PERSON.to_string()),
        operator_proof_until: Some(agentmux_common::time::now_secs_u64().saturating_sub(1)),
        ..Default::default()
    };
    assert!(!lapsed.operator_proven_now());
    // The refusal says to reconnect, and keeps the marker the app matches to prove the connection again.
    let refusal = lapsed.operator_only("probe").unwrap_err();
    assert!(refusal.starts_with("FORBIDDEN: probe") && refusal.contains("isn't the AgentMux window") && refusal.contains("reconnect"), "{refusal}");
    assert_eq!(actor_of(&lapsed), Actor::Unproven);
    let live = RpcContext { operator_proof_until: Some(agentmux_common::time::now_secs_u64() + 60), ..lapsed };
    assert!(live.operator_only("probe").is_ok());
}

#[test]
fn a_connection_that_registers_as_an_agent_is_not_attested_and_loses_the_proof() {
    let h = host();
    let now = agentmux_common::time::now_secs_u64();
    let (engine, nonce) = proxied(&h, CONN);
    engine.set_rpc_context(RpcContext { agent_id: "worker".to_string(), ..Default::default() });
    assert_eq!(status(h.attest(&attest_req(&signer(), CONN, &nonce, PERSON, now + 300), now)), 409);
    // And one attested first drops the proof when it registers afterwards.
    let (engine2, nonce2) = proxied(&h, OTHER_CONN);
    assert!(h.attest(&attest_req(&signer(), OTHER_CONN, &nonce2, PERSON, now + 300), now).is_ok());
    engine2.set_rpc_context(RpcContext { agent_id: "worker".to_string(), ..Default::default() });
    assert!(engine2.rpc_context().operator_only("probe").is_err());
}

#[test]
fn an_id_held_by_an_open_connection_cannot_be_taken() {
    let h = host();
    let (first, _nonce) = proxied(&h, CONN);
    let (second, _rx) = WshRpcEngine::new();
    std::mem::forget(_rx);
    assert!(!h.attach(CONN, &second), "the first holder keeps the id");
    h.detach(CONN);
    assert!(h.attach(CONN, &second), "free again once it closed");
    drop(first);
}

#[test]
fn a_signed_revocation_ends_the_proof_and_only_a_fresh_one_does() {
    let h = host();
    let now = agentmux_common::time::now_secs_u64();
    let (engine, nonce) = proxied(&h, CONN);
    assert!(h.attest(&attest_req(&signer(), CONN, &nonce, PERSON, now + 300), now).is_ok());
    let revoke = |key: &SigningKey, issued_at: u64| RevokeRequest {
        v: 1,
        conn: CONN.to_string(),
        workspace: WORKSPACE.to_string(),
        issued_at,
        sig: sign(key, &format!("amx-revoke/v1|{WORKSPACE}|{CONN}|{issued_at}")),
    };
    let stranger = SigningKey::from_bytes(&[3u8; 32]);
    assert!(h.revoke(&revoke(&stranger, now), now).is_err(), "not the host's signature");
    assert!(h.revoke(&revoke(&signer(), now - 600), now).is_err(), "too old");
    assert!(engine.rpc_context().operator_proven_now(), "refused revocations changed nothing");
    assert!(h.revoke(&revoke(&signer(), now), now).is_ok());
    assert!(!engine.rpc_context().operator_proven_now());
}

#[tokio::test]
async fn the_routes_answer_404_when_no_host_is_registered() {
    // This test binary never registers the global host, so a srv started without the flag has no such routes' effect.
    assert!(host_registered_globally().not());
    let r = handle_attest(Json(attest_req(&signer(), CONN, "00000000-0000-0000-0000-000000000000", PERSON, agentmux_common::time::now_secs_u64() + 60))).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = handle_nonce(Json(NonceRequest { nonce: "00000000-0000-0000-0000-000000000000".to_string() })).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

fn host_registered_globally() -> bool {
    super::host().is_some()
}

trait Not {
    fn not(self) -> bool;
}

impl Not for bool {
    fn not(self) -> bool {
        !self
    }
}
