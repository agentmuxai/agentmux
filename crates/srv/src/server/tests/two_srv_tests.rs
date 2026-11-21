// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The two-srv harness (`two_srv`) against the viewer listener as it is: pairing over TLS pinned to the host's
//! certificate, a feed read off a real socket, a wrong pin, a revoked token, and a signed or unsigned send.
//! Names that start with `gap_` pin what the listener does not do yet, so the change that adds it has a test to turn.

use super::two_srv::{start_host, start_viewer, start_viewer_pinned_to};

fn user_line(text: &str) -> String {
    serde_json::json!({ "type": "user", "message": { "role": "user", "content": text } }).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_viewer_reads_the_feed_snapshot_and_a_later_append_over_the_pinned_listener() {
    let host = start_host();
    let viewer = start_viewer(&host);
    let agent = host.seed_agent("two-srv-feed");
    host.append(&agent, &user_line("first"));
    host.append(&agent, &user_line("second"));

    let device = viewer.pair(&host, "laptop").await.unwrap();
    assert!(device.token.starts_with("amxv_"));
    let mut feed = viewer.open_feed(&device, &agent.name).await.unwrap();
    let (event, _, snapshot) = feed.next_event().await.expect("the snapshot");
    assert_eq!(event, "snapshot");
    let lines: Vec<&str> = snapshot["lines"].as_array().unwrap().iter().filter_map(|l| l.as_str()).collect();
    assert_eq!(lines, vec![user_line("first"), user_line("second")]);

    host.append(&agent, &user_line("third"));
    let (event, _, appended) = feed.next_event().await.expect("the append");
    assert_eq!(event, "append");
    assert_eq!(appended["lines"][0].as_str(), Some(user_line("third").as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_two_instances_keep_their_own_state() {
    let host = start_host();
    let viewer = start_viewer(&host);
    let device = viewer.pair(&host, "laptop").await.unwrap();
    assert!(host.state.mstore.viewer_device_list().unwrap().iter().any(|d| d.device_id == device.device_id));
    assert!(viewer.state.mstore.viewer_device_list().unwrap().is_empty(), "the viewer's own instance holds no device of the host's");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_pinned_to_another_certificate_is_refused_at_the_handshake_and_pairs_nothing() {
    let host = start_host();
    let viewer = start_viewer_pinned_to(&host, &"ab".repeat(32));
    let err = viewer.pair(&host, "laptop").await.err().expect("a wrong pin is refused");
    assert!(!err.is_empty());
    assert!(host.state.mstore.viewer_device_list().unwrap().is_empty(), "nothing was paired");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_devices_token_is_refused_and_its_open_feed_ends() {
    let host = start_host();
    let viewer = start_viewer(&host);
    let agent = host.seed_agent("two-srv-revoke");
    host.append(&agent, &user_line("hello"));
    let device = viewer.pair(&host, "laptop").await.unwrap();
    let mut feed = viewer.open_feed(&device, &agent.name).await.unwrap();
    assert_eq!(feed.next_event().await.expect("the snapshot").0, "snapshot");
    assert_eq!(viewer.get(&device, "/agentmux/viewer/hello").await.unwrap().status(), 200);

    host.revoke(&device);
    assert!(feed.next_event().await.is_none(), "the feed ends");
    assert_eq!(viewer.get(&device, "/agentmux/viewer/hello").await.unwrap().status(), 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_needs_the_owners_grant_and_the_devices_signature() {
    let host = start_host();
    let viewer = start_viewer(&host);
    let agent = host.seed_agent("two-srv-send");
    let device = viewer.pair(&host, "laptop").await.unwrap();

    // Not granted: refused before anything else.
    let (status, json) = viewer.send(&device, &agent.name, "hi", true).await.unwrap();
    assert_eq!((status.as_u16(), json["reason"].as_str()), (403, Some("send_off")));

    host.allow_send(&device);
    // Unsigned: the token alone sends nothing.
    let (status, json) = viewer.send(&device, &agent.name, "hi", false).await.unwrap();
    assert_eq!((status.as_u16(), json["reason"].as_str()), (403, Some("bad_signature")), "{json}");
}

/// A signed send from the viewer passes every check the listener makes. What the host does with it after that is
/// the Swarm broadcast's delivery, which needs a running agent controller; this harness has none, so the answer is
/// the delivery step's own "not running".
#[tokio::test(flavor = "multi_thread")]
async fn gap_a_signed_send_reaches_the_delivery_step_but_the_harness_has_no_running_agent_to_deliver_to() {
    let host = start_host();
    let viewer = start_viewer(&host);
    let agent = host.seed_agent("two-srv-signed");
    let device = viewer.pair(&host, "laptop").await.unwrap();
    host.allow_send(&device);
    let (status, json) = viewer.send(&device, &agent.name, "hi", true).await.unwrap();
    assert_eq!((status.as_u16(), json["reason"].as_str()), (409, Some("not_running")), "{json}");
}
