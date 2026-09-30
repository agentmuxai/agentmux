// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `verify_jekt_signature_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use crate::server::tests::test_state;

fn base_req(source_agent: &str, target_agent: &str, message: &str) -> InjectionRequest {
    InjectionRequest {
        target_agent: target_agent.to_string(),
        message: message.to_string(),
        source_agent: Some(source_agent.to_string()),
        delivery_tier: Some("host".to_string()),
        ..Default::default()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn a_correctly_signed_message_verifies_true() {
    let state = test_state();
    let key = state.mstore.agent_jekt_key_ensure("agentx").unwrap();

    let mut req = base_req("agentx", "agenty", "hello");
    req.request_id = Some("msg-1".to_string());
    req.ts_secs = Some(now());
    req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
        &key,
        req.request_id.as_deref().unwrap(),
        "agentx",
        "agenty",
        req.ts_secs.unwrap(),
        "hello",
    ));

    verify_jekt_signature(&state, &mut req);
    assert_eq!(req.sig_verified, Some(true));
}

/// The core P0 fix this whole file's `verify_jekt_signature` extraction
/// exists for: a claimed sender with a real key on file but NO
/// signature attached (exactly what `messagebus.rs::handle_inject` and
/// websocket.rs's `bus:inject` used to send, pre-fix) must render as a
/// real, escalating "unverified" — not silently pass through unchecked.
#[tokio::test]
async fn a_claimed_sender_with_a_key_but_no_signature_is_unverified() {
    let state = test_state();
    state.mstore.agent_jekt_key_ensure("agentx").unwrap();

    let mut req = base_req("agentx", "agenty", "hello");
    req.request_id = Some("msg-1".to_string());
    req.ts_secs = Some(now());
    // req.jekt_sig deliberately left None — the exact bypass shape.

    verify_jekt_signature(&state, &mut req);
    assert_eq!(
        req.sig_verified,
        Some(false),
        "a signable identity with no signature must be a real 'unverified,' not skipped"
    );
}

#[tokio::test]
async fn no_key_on_file_leaves_sig_verified_unset() {
    let state = test_state();
    // No agent_jekt_key_ensure call — "slack", or any non-agent caller.
    let mut req = base_req("slack", "agenty", "hello");
    verify_jekt_signature(&state, &mut req);
    assert_eq!(
        req.sig_verified, None,
        "no key on file means nothing to check — must not be escalated"
    );
}

// Was named "network_tier_is_never_checked_regardless_of_signature" and
// asserted the OPPOSITE of what's below — reagentx P0 (round 2) on the
// LAN signing PR: that WAS the bypass. Gating this check on
// delivery_tier meant a request could claim "wan"/"lan" for a
// source_agent this instance actually has a local key for, skip host
// verification entirely, and land unescalated. Flipped, not deleted, to
// document the fix rather than silently change it — see
// docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §3's second
// revision.
#[tokio::test]
async fn a_locally_known_sender_is_still_checked_even_under_a_claimed_network_tier() {
    let state = test_state();
    state.mstore.agent_jekt_key_ensure("agentx").unwrap();
    let mut req = base_req("agentx", "agenty", "hello");
    req.delivery_tier = Some("wan".to_string());
    // req.jekt_sig deliberately left None — claiming "wan" must not be a
    // way to dodge this check for an agent this instance actually knows.
    verify_jekt_signature(&state, &mut req);
    assert_eq!(
        req.sig_verified,
        Some(false),
        "a locally-known agent's identity, unsigned, must still be flagged regardless of \
         what delivery_tier the request claims — that claim is not a trust boundary"
    );
}

#[tokio::test]
async fn a_genuinely_unknown_remote_sender_is_unaffected_by_a_claimed_network_tier() {
    let state = test_state();
    // No agent_jekt_key_ensure call — this instance never spawned "korp,"
    // exactly the shape of a real remote LAN/WAN agent.
    let mut req = base_req("korp", "agenty", "hello");
    req.delivery_tier = Some("lan".to_string());
    verify_jekt_signature(&state, &mut req);
    assert_eq!(
        req.sig_verified, None,
        "no local key for the claimed sender means nothing to check — running this \
         unconditionally must not manufacture a finding for genuinely remote traffic"
    );
}

/// Anti-replay (reagentx P1 on PR #2565): a signature that was valid
/// once must stop verifying once its `ts_secs` falls outside the
/// freshness window — otherwise a captured signed jekt replays forever.
#[tokio::test]
async fn a_stale_timestamp_fails_verification_even_with_a_correct_signature() {
    let state = test_state();
    let key = state.mstore.agent_jekt_key_ensure("agentx").unwrap();

    let stale_ts = now() - JEKT_SIG_MAX_AGE_SECS - 60; // well outside the window
    let mut req = base_req("agentx", "agenty", "hello");
    req.request_id = Some("msg-1".to_string());
    req.ts_secs = Some(stale_ts);
    req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
        &key, "msg-1", "agentx", "agenty", stale_ts, "hello",
    ));

    verify_jekt_signature(&state, &mut req);
    assert_eq!(
        req.sig_verified,
        Some(false),
        "a mathematically correct signature must still fail outside the freshness window"
    );
}

#[tokio::test]
async fn a_timestamp_just_inside_the_window_still_verifies() {
    let state = test_state();
    let key = state.mstore.agent_jekt_key_ensure("agentx").unwrap();

    let recent_ts = now() - (JEKT_SIG_MAX_AGE_SECS - 10);
    let mut req = base_req("agentx", "agenty", "hello");
    req.request_id = Some("msg-1".to_string());
    req.ts_secs = Some(recent_ts);
    req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
        &key, "msg-1", "agentx", "agenty", recent_ts, "hello",
    ));

    verify_jekt_signature(&state, &mut req);
    assert_eq!(req.sig_verified, Some(true));
}

#[tokio::test]
async fn a_wrong_signature_is_unverified() {
    let state = test_state();
    state.mstore.agent_jekt_key_ensure("agentx").unwrap();

    let mut req = base_req("agentx", "agenty", "hello");
    req.request_id = Some("msg-1".to_string());
    req.ts_secs = Some(now());
    req.jekt_sig = Some("forged-not-a-real-signature".to_string());

    verify_jekt_signature(&state, &mut req);
    assert_eq!(req.sig_verified, Some(false));
}
