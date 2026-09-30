// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `verify_reagent_signature_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;

// Reuses the exact fixture from crates/common/src/jekt_sign.rs's own
// `a_correctly_signed_reagent_message_verifies_under_the_production_key`
// test: a signature produced offline under the production `reagent-v1`
// key, over signed_material("msg-1", "github-consumer", "agentx", 1000,
// "hello"). The private key isn't in this repo (agentmux-cloud's Secrets
// Manager only) so a fresh signature can't be minted at test time —
// `now` is passed explicitly instead of wall-clock so this fixed
// ts_secs=1000 can be held inside the freshness window on demand.
const FIXTURE_SIG_B64: &str =
    "FCFjcvAzla329a39u8fFxOvRWaH1R2fUn8RsGtP9RaIbLbaS3aXgAQ7YB4ssWV5TDvAeGrwSkHfoeGi11iCPBg==";
// The same material signed under the `reagent-v1-dev` key.
const DEV_KEY_FIXTURE_SIG_B64: &str =
    "QehidZjJa2jYLPIPYSsVxUlm86W5Fdbr9PV3P4HJyZwJ68/HZR9EaAL0MpcVtTuZJW2+MMGebc0RH9HITNJGCw==";
const FIXTURE_TS_SECS: i64 = 1_000;

fn wan_req() -> InjectionRequest {
    InjectionRequest {
        target_agent: "agentx".to_string(),
        message: "hello".to_string(),
        source_agent: Some("github-consumer".to_string()),
        delivery_tier: Some("wan".to_string()),
        reagent_sig: Some(FIXTURE_SIG_B64.to_string()),
        reagent_key_id: Some("reagent-v1".to_string()),
        reagent_msg_id: Some("msg-1".to_string()),
        reagent_ts_secs: Some(FIXTURE_TS_SECS),
        ..Default::default()
    }
}

#[test]
fn a_correctly_signed_and_fresh_reagent_message_verifies() {
    let mut req = wan_req();
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, Some(true));
}

#[test]
fn a_valid_signature_under_the_dev_key_is_not_verified() {
    let mut req = wan_req();
    req.reagent_key_id = Some("reagent-v1-dev".to_string());
    req.reagent_sig = Some(DEV_KEY_FIXTURE_SIG_B64.to_string());
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, Some(false), "only the production key yields a verified sender");
}

#[test]
fn a_correct_signature_outside_the_freshness_window_fails() {
    let mut req = wan_req();
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS + REAGENT_SIG_MAX_AGE_SECS + 60);
    assert_eq!(
        req.reagent_verified,
        Some(false),
        "a mathematically correct signature must still fail outside the freshness window"
    );
}

#[test]
fn host_tier_is_never_checked_regardless_of_signature() {
    let mut req = wan_req();
    req.delivery_tier = Some("host".to_string());
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, None, "reagent signing only applies to the WAN tier");
}

#[test]
fn lan_tier_is_never_checked_regardless_of_signature() {
    let mut req = wan_req();
    req.delivery_tier = Some("lan".to_string());
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, None, "reagent signing only applies to the WAN tier");
}

#[test]
fn a_wrong_signature_is_unverified() {
    let mut req = wan_req();
    req.reagent_sig = Some("forged-not-a-real-signature".to_string());
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, Some(false));
}

#[test]
fn an_unknown_key_id_is_unverified() {
    let mut req = wan_req();
    req.reagent_key_id = Some("reagent-v2-does-not-exist".to_string());
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, Some(false));
}

// A partial set of the four fields is "not signed," not "signed but
// broken" — matches cloud_subscriber.rs's identical policy (see
// reagent_key_id's doc comment in types.rs).
#[test]
fn a_partial_signature_set_is_treated_as_unsigned_not_invalid() {
    let mut req = wan_req();
    req.reagent_key_id = None;
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, None);
}

#[test]
fn no_reagent_fields_at_all_is_left_unset() {
    let mut req = wan_req();
    req.reagent_sig = None;
    req.reagent_key_id = None;
    req.reagent_msg_id = None;
    req.reagent_ts_secs = None;
    verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
    assert_eq!(req.reagent_verified, None);
}
