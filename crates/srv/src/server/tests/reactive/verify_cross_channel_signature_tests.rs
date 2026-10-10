// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `verify_cross_channel_signature_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use crate::backend::reactive::registry::{write_shared_entry_for_test, AgentEntry};
use crate::server::tests::test_state;
use agentmux_common::jekt_sign::{generate_lan_keypair, sign_channel_jekt, sign_lan_jekt};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

const NOW: i64 = 1_800_000_000;

/// This instance's own channel in these tests: no sender below uses it
/// unless the test is about a same-instance sender.
const LOCAL: &str = "chan-local";

/// `(private, public)` — `generate_lan_keypair` itself returns
/// `(public, private)`; flipped here so call sites read naturally.
fn keypair(seed_byte: u8) -> ([u8; 32], [u8; 32]) {
    let (public, private) = generate_lan_keypair([seed_byte; 32]);
    (private, public)
}

fn publish(shared_dir: &std::path::Path, agent: &str, channel: &str, pubkey: Option<&[u8; 32]>) {
    write_shared_entry_for_test(
        shared_dir,
        &AgentEntry {
            agent_id: agent.to_string(),
            local_url: "http://127.0.0.1:9001".to_string(),
            block_id: "block-1".to_string(),
            pid: 1,
            updated_at: 1,
            auth_key: String::new(),
            channel: channel.to_string(),
            registration_nonce: 0,
            jekt_public_key: pubkey.map(|k| BASE64.encode(k)).unwrap_or_default(),
        },
    );
}

fn channel_req(source: &str, source_channel: &str, ts: i64) -> InjectionRequest {
    InjectionRequest {
        target_agent: "lark".to_string(),
        message: "here is the brief".to_string(),
        source_agent: Some(source.to_string()),
        delivery_tier: Some("channel".to_string()),
        request_id: Some("msg-xc-1".to_string()),
        ts_secs: Some(ts),
        source_channel: Some(source_channel.to_string()),
        ..Default::default()
    }
}

fn sign(req: &InjectionRequest, private: &[u8; 32]) -> String {
    sign_channel_jekt(
        private,
        req.request_id.as_deref().unwrap(),
        req.source_agent.as_deref().unwrap(),
        req.source_channel.as_deref().unwrap(),
        &req.target_agent,
        req.ts_secs.unwrap(),
        &req.message,
    )
    .unwrap()
}

#[tokio::test]
async fn a_valid_signature_against_the_published_key_verifies() {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let mut req = channel_req("agent4", "chan-a", NOW);
    req.channel_sig = Some(sign(&req, &private));
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(true));
}

#[tokio::test]
async fn a_resolvable_sender_with_no_signature_is_an_active_failure() {
    // §D3's load-bearing row: entry + key found, nothing signed.
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (_, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let mut req = channel_req("agent4", "chan-a", NOW);
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(false));
}

#[tokio::test]
async fn a_signature_from_the_wrong_key_fails() {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (_, public) = keypair(1);
    let (imposter_private, _) = keypair(2);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let mut req = channel_req("agent4", "chan-a", NOW);
    req.channel_sig = Some(sign(&req, &imposter_private));
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(false));
}

#[tokio::test]
async fn a_pre_upgrade_entry_with_an_empty_key_cannot_be_checked() {
    // §6: "cannot check," never "failed the check" — otherwise every
    // sender on a mixed-version machine escalates.
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    publish(dir.path(), "agent4", "chan-a", None);
    let mut req = channel_req("agent4", "chan-a", NOW);
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, None, "an empty published key must yield None, not Some(false)");
}

#[tokio::test]
async fn an_unknown_sender_with_no_entry_anywhere_is_left_unset() {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let mut req = channel_req("slack-bridge", "chan-a", NOW);
    req.channel_sig = Some("irrelevant".to_string());
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, None);
}

#[tokio::test]
async fn a_same_instance_sender_is_left_to_the_hmac_path() {
    // §D2 step 2: this instance holds an HMAC key for the claimed sender,
    // so verify_jekt_signature owns the verdict — even though a shared
    // entry with a key exists and nothing was signed.
    let state = test_state();
    state.mstore.agent_jekt_key_ensure("agent4").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (_, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let mut req = channel_req("agent4", LOCAL, NOW);
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, None, "must not shadow the HMAC verifier for a local agent");
}

#[tokio::test]
async fn an_agent_that_moved_to_another_instance_verifies_despite_a_stale_hmac_key() {
    // This instance spawned agent4 once and still holds its HMAC key; agent4
    // now runs in chan-a and signs with the key it published there. Its
    // message must verify, and the stale key's failed HMAC check must not
    // mark it a forgery (the 2026-10-10 TRUST=unverified incident).
    let state = test_state();
    state.mstore.agent_jekt_key_ensure("agent4").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let mut req = channel_req("agent4", "chan-a", NOW);
    req.channel_sig = Some(sign(&req, &private));
    req.sig_verified = Some(false); // what the HMAC check against the stale key found
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(true));
    assert_eq!(req.sig_verified, None, "the stale key's verdict is not a forgery once the sender is proven");
}

#[tokio::test]
async fn a_forger_claiming_an_agent_this_instance_knows_still_fails_both_checks() {
    // Naming another channel doesn't buy a pass: without the agent's private
    // key the channel check fails, and the HMAC verdict stays a forgery.
    let state = test_state();
    state.mstore.agent_jekt_key_ensure("agent4").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (_, public) = keypair(1);
    let (wrong_private, _) = keypair(2);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let mut req = channel_req("agent4", "chan-a", NOW);
    req.channel_sig = Some(sign(&req, &wrong_private));
    req.sig_verified = Some(false);
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(false));
    assert_eq!(req.sig_verified, Some(false));
}

#[tokio::test]
async fn a_valid_signature_outside_the_freshness_window_fails() {
    // §D6: host-tier's 300s window, not LAN/WAN's 600s.
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let stale = NOW - CHANNEL_SIG_MAX_AGE_SECS - 1;
    let mut req = channel_req("agent4", "chan-a", stale);
    req.channel_sig = Some(sign(&req, &private));
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(false));

    // ...and just inside it still verifies, so the boundary is the
    // window and not something else.
    let fresh = NOW - CHANNEL_SIG_MAX_AGE_SECS + 5;
    let mut req = channel_req("agent4", "chan-a", fresh);
    req.channel_sig = Some(sign(&req, &private));
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(true));
    assert_eq!(CHANNEL_SIG_MAX_AGE_SECS, 300, "spec §D6 pins the host-tier window");
}

#[tokio::test]
async fn an_agent_live_in_two_channels_verifies_against_either_published_key() {
    // §D2 step 5: same name, two channels, two keypairs; a signature
    // from the second channel must verify even though the first
    // channel's entry sorts ahead of it.
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (_, public_a) = keypair(1);
    let (private_b, public_b) = keypair(2);
    publish(dir.path(), "agent4", "chan-a", Some(&public_a));
    publish(dir.path(), "agent4", "chan-b", Some(&public_b));
    let mut req = channel_req("agent4", "chan-b", NOW);
    req.channel_sig = Some(sign(&req, &private_b));
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(true));
}

#[tokio::test]
async fn a_lan_signature_replayed_as_a_cross_channel_one_fails() {
    // §D5 domain separator — same key, same (msgid, sender, target, ts,
    // message), but the LAN payload must not verify on this tier.
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    let mut req = channel_req("agent4", "chan-a", NOW);
    req.channel_sig = sign_lan_jekt(&private, "msg-xc-1", "agent4", "lark", NOW, "here is the brief");
    assert!(req.channel_sig.is_some());
    verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
    assert_eq!(req.channel_verified, Some(false));
}

#[tokio::test]
async fn a_signature_minted_for_one_channel_replayed_as_another_fails() {
    // §D5 channel binding — the same agent's genuine signature from
    // chan-a, presented as if it came from chan-b.
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    publish(dir.path(), "agent4", "chan-b", Some(&public));
    let genuine = channel_req("agent4", "chan-a", NOW);
    let sig = sign(&genuine, &private);
    let mut replayed = channel_req("agent4", "chan-b", NOW);
    replayed.channel_sig = Some(sig);
    verify_cross_channel_signature_in(&state, &mut replayed, dir.path(), LOCAL, NOW);
    assert_eq!(replayed.channel_verified, Some(false));
}

#[tokio::test]
async fn verification_is_scoped_to_the_channel_tier() {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = keypair(1);
    publish(dir.path(), "agent4", "chan-a", Some(&public));
    for tier in ["host", "lan", "wan"] {
        let mut req = channel_req("agent4", "chan-a", NOW);
        req.delivery_tier = Some(tier.to_string());
        req.channel_sig = Some(sign(&req, &private));
        verify_cross_channel_signature_in(&state, &mut req, dir.path(), LOCAL, NOW);
        assert_eq!(req.channel_verified, None, "tier {tier} must leave channel_verified unset");
    }
}

#[test]
fn same_host_forward_relabels_host_as_channel_and_keeps_network_tiers() {
    // §D4: only a host-tier (or unlabelled) request becomes `channel`
    // on the hop; a lan/wan/channel label is preserved so the peer
    // re-derives that tier's own verification.
    assert_eq!(same_host_forward_tier(None), "channel");
    assert_eq!(same_host_forward_tier(Some("host")), "channel");
    assert_eq!(same_host_forward_tier(Some("channel")), "channel");
    assert_eq!(same_host_forward_tier(Some("lan")), "lan");
    assert_eq!(same_host_forward_tier(Some("wan")), "wan");
}
