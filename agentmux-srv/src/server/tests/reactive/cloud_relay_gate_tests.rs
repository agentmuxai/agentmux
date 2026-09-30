// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `cloud_relay_gate_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use crate::server::tests::test_state;

fn req(source: Option<&str>, delivery_tier: Option<&str>) -> InjectionRequest {
    InjectionRequest {
        target_agent: "nobody-anywhere".to_string(),
        message: "hello".to_string(),
        source_agent: source.map(str::to_string),
        delivery_tier: delivery_tier.map(str::to_string),
        ..Default::default()
    }
}

/// **The loop guard.** A message that arrived over the cloud must never be
/// pushed back to it: an agent unknown to every subscribed instance would
/// otherwise bounce between the relay and each sidecar forever, re-queueing
/// on every hop. `forward_hops` cannot catch this — the cloud delivers each
/// message as a fresh inbound request with the count reset — so tier 4
/// carries its own guard, and this is the test that pins it.
#[tokio::test]
async fn a_message_that_arrived_over_wan_is_never_re_relayed() {
    let state = test_state();
    let out = try_cloud_relay(&state, &req(Some("agent2"), Some("wan"))).await;
    assert!(out.is_none(), "a wan-delivered message must not re-enter tier 4");
}

/// The cloud route derives the sender from `X-Agent-ID` and 400s without
/// it, so a caller with no source agent (cron, an external bridge) has no
/// tier 4 at all — better to skip than to burn a round trip on a request
/// that cannot succeed.
#[tokio::test]
async fn a_caller_with_no_source_agent_has_no_tier_4() {
    let state = test_state();
    assert!(try_cloud_relay(&state, &req(None, Some("host"))).await.is_none());
    assert!(
        try_cloud_relay(&state, &req(Some(""), Some("host")))
            .await
            .is_none(),
        "an empty source_agent is as unusable as a missing one"
    );
}

/// Host- and LAN-tier messages are eligible; with no muxbus credential
/// present this still ends in `None`, but via token resolution rather than
/// an early guard — i.e. these are not accidentally excluded.
#[tokio::test]
async fn host_and_lan_messages_are_eligible_but_need_a_credential() {
    let state = test_state();
    assert!(try_cloud_relay(&state, &req(Some("agent2"), Some("host"))).await.is_none());
    assert!(try_cloud_relay(&state, &req(Some("agent2"), Some("lan"))).await.is_none());
    assert!(try_cloud_relay(&state, &req(Some("agent2"), None)).await.is_none());
}
