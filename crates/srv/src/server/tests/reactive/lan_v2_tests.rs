// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity M4d-6, LAN half: the v2 signature is checked through
// `verify_lan_signature`, with the peer's answer seeded into the lookup cache.

use super::super::verify_lan_signature;
use crate::backend::reactive::{Handler, InjectionRequest};
use crate::server::tests::test_state;
use agentmux_common::jekt_sign::{generate_lan_keypair, sign_lan_jekt, sign_lan_jekt_v2};

fn now() -> i64 {
    agentmux_common::time::now_secs()
}

/// A LAN jekt from `korp`, v1-signed with `name_private` and, when given,
/// v2-signed as `uid` with `uid_private`.
fn signed(name_private: &[u8; 32], v2: Option<(&str, &[u8; 32])>) -> InjectionRequest {
    let ts = now();
    let mut req = InjectionRequest {
        target_agent: "lark".into(),
        message: "hello".into(),
        source_agent: Some("korp".into()),
        delivery_tier: Some("lan".into()),
        request_id: Some("req-lan-v2".into()),
        ts_secs: Some(ts),
        ..Default::default()
    };
    req.lan_sig = sign_lan_jekt(name_private, "req-lan-v2", "korp", "lark", ts, "hello");
    if let Some((uid, uid_private)) = v2 {
        req.source_uid = Some(uid.into());
        req.lan_sig_v2 = sign_lan_jekt_v2(uid_private, "req-lan-v2", "korp", uid, "lark", ts, "hello");
    }
    req
}

#[tokio::test]
async fn a_valid_v2_from_the_answering_peers_uid_is_recorded_as_claimed_only() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, uid_private) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut req = signed(&name_private, Some(("uid-k", &uid_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(true));
    assert_eq!(req.lan_claimed_uid, "uid-k");
    assert_eq!(req.audit_source_uid, "", "claimed, never attributed");
}

#[tokio::test]
async fn a_uid_the_answering_peer_did_not_give_is_not_claimed() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, uid_private) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut req = signed(&name_private, Some(("uid-other", &uid_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(true), "v1 stands on its own");
    assert_eq!(req.lan_claimed_uid, "");
}

#[tokio::test]
async fn a_changed_uid_key_under_the_same_name_is_not_claimed() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (first_public, _) = generate_lan_keypair([2u8; 32]);
    let (second_public, second_private) = generate_lan_keypair([3u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), first_public.to_vec())));
    let mut first = signed(&name_private, None);
    first.source_uid = Some("uid-k".into());
    first.lan_sig_v2 = Some("unchecked until the pin is set".into());
    verify_lan_signature(&state, &mut first).await; // pins uid-k's first key
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), second_public.to_vec())));
    let mut req = signed(&name_private, Some(("uid-k", &second_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_claimed_uid, "", "the pin holds the first key");
}

#[tokio::test]
async fn a_bad_v2_or_a_failed_v1_claims_nothing() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, _) = generate_lan_keypair([2u8; 32]);
    let (_, wrong_private) = generate_lan_keypair([4u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut bad_v2 = signed(&name_private, Some(("uid-k", &wrong_private)));
    verify_lan_signature(&state, &mut bad_v2).await;
    assert_eq!(bad_v2.lan_verified, Some(true));
    assert_eq!(bad_v2.lan_claimed_uid, "");

    let (_, uid_private) = generate_lan_keypair([2u8; 32]);
    let mut bad_v1 = signed(&wrong_private, Some(("uid-k", &uid_private)));
    verify_lan_signature(&state, &mut bad_v1).await;
    assert_eq!(bad_v1.lan_verified, Some(false));
    assert_eq!(bad_v1.lan_claimed_uid, "", "v2 is checked only after v1 verified");
}

#[tokio::test]
async fn the_claimed_uid_reaches_its_delivery_s_audit_entries_and_never_the_wire() {
    let mut handler = Handler::new();
    let claimed = InjectionRequest {
        target_agent: "m4d6-nobody".into(),
        message: "hi".into(),
        source_agent: Some("korp".into()),
        lan_claimed_uid: "uid-k".into(),
        ..Default::default()
    };
    handler.inject_message(claimed.clone());
    assert_eq!(handler.get_audit_log(1)[0].lan_claimed_uid, "uid-k");
    assert_eq!(handler.get_audit_log(1)[0].audit_source_uid, "");
    handler.inject_message(InjectionRequest { lan_claimed_uid: String::new(), ..claimed.clone() });
    assert_eq!(handler.get_audit_log(1)[0].lan_claimed_uid, "", "restored after the delivery");
    let wire = serde_json::to_value(&claimed).unwrap();
    assert!(wire.get("lan_claimed_uid").is_none(), "never forwarded: {wire}");
}
