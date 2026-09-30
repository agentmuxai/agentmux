// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `resolve_delivery_tier_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;

#[test]
fn full_auth_key_honours_a_channel_claim() {
    // §D4: the forwarding instance labels the hop; the receiving
    // instance (authenticated with its own full key) keeps the label.
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, Some("channel")), "channel");
    // A lan_key holder still can't claim it — LAN wins, as for every tier.
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::LanKey, Some("channel")), "lan");
}

#[test]
fn lan_key_forces_lan_regardless_of_claim() {
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::LanKey, Some("host")), "lan");
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::LanKey, Some("wan")), "lan");
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::LanKey, None), "lan");
}

#[test]
fn full_auth_key_trusts_the_body_claim_including_lan() {
    // reagentx P0 regression: same-host Tier 2a/2b forwarding
    // re-authenticates an already-lan-tagged jekt with a sibling
    // instance's own full auth_key. If this ever downgrades "lan" to
    // "host" again, an already-detected LAN signature failure's
    // forced-sensitive escalation silently disappears on the second
    // hop (lan_verified resets to None, and verify_lan_signature only
    // runs when delivery_tier == "lan").
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, Some("lan")), "lan");
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, Some("wan")), "wan");
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, Some("host")), "host");
}

#[test]
fn full_auth_key_defaults_to_host_when_body_omits_the_field() {
    assert_eq!(resolve_delivery_tier(super::super::ReactiveAuthVia::FullAuthKey, None), "host");
}
