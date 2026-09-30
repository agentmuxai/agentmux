// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `verify_lan_signature_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use crate::server::tests::test_state;

// test_state() has no real LAN peers discovered, so
// find_agent_lan_pubkey always returns None here — these tests exercise
// the paths reachable without one (tier scoping, no-signature-attempted,
// no-pubkey-found). The actual verify_lan_jekt crypto — correct sig
// verifies, wrong sig fails, tampered content/sender fails — is
// exhaustively covered in agentmux-common/src/jekt_sign.rs; the tier
// escalation this feeds into (is_lan_sig_invalid forcing sensitive,
// TRUST=lan-verified rendering) is covered end-to-end via
// Handler::inject_message in backend/reactive/tests.rs, driven directly
// off req.lan_verified rather than through this HTTP-round-trip lookup.

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn lan_req(source_agent: &str, target_agent: &str, message: &str) -> InjectionRequest {
    InjectionRequest {
        target_agent: target_agent.to_string(),
        message: message.to_string(),
        source_agent: Some(source_agent.to_string()),
        delivery_tier: Some("lan".to_string()),
        request_id: Some("req-lan-1".to_string()),
        ts_secs: Some(now()),
        ..Default::default()
    }
}

#[tokio::test]
async fn lan_signature_verification_is_skipped_off_the_lan_tier() {
    let state = test_state();
    let mut req = lan_req("agentx", "agenty", "hello");
    req.delivery_tier = Some("host".to_string());
    req.lan_sig = Some("anything".to_string());
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, None, "lan signing only applies to the LAN tier");
}

#[tokio::test]
async fn no_lan_sig_attempted_leaves_lan_verified_unset() {
    let state = test_state();
    let mut req = lan_req("agentx", "agenty", "hello");
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, None, "nothing to check when no signature was attempted");
}

// reagentx P0 follow-up regression test: a rate-limited pubkey lookup
// must NOT be treated the same as "no key found." Without the fix, an
// attacker could exhaust the limiter with junk lookups, then slip a
// forged signature for a real agent's identity through as
// unverified/benign instead of forced-sensitive.
#[tokio::test]
async fn rate_limited_pubkey_lookup_forces_failed_not_unset() {
    let state = test_state();
    // Burn through the fan-out rate limiter with distinct agent_ids —
    // each is a genuine cache miss (test_state() has zero real LAN
    // peers, so every one of these negatively caches after consuming
    // one token). LAN_PUBKEY_LOOKUP_RATE_LIMIT is 10/sec.
    for i in 0..10 {
        let _ = state
            .lan_discovery
            .find_agent_lan_pubkey(&format!("burn-{i}"), &state.http_client)
            .await;
    }
    // The 11th distinct lookup this second must be rate-limited.
    let mut req = lan_req("korp", "agenty", "hello");
    req.lan_sig = Some("forged-or-real-doesnt-matter".to_string());
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(
        req.lan_verified,
        Some(false),
        "a rate-limited lookup for a claimed sender with a real signature attempt must be \
         treated as a verification FAILURE, never silently left unset like a genuinely \
         unknown/unsigned sender"
    );
}

#[tokio::test]
async fn a_claimed_sender_with_no_discoverable_pubkey_leaves_lan_verified_unset() {
    // A lan_sig IS present, but with zero LAN peers discovered
    // (test_state()'s default), find_agent_lan_pubkey can't find
    // anyone's public key — "nothing to check against" must not be
    // conflated with "the signature is invalid."
    let state = test_state();
    let mut req = lan_req("agentx", "agenty", "hello");
    req.lan_sig = Some("some-signature".to_string());
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(
        req.lan_verified, None,
        "an unfindable public key must not be conflated with a failed verification"
    );
}
