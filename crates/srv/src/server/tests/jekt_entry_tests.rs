// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `jekt_entry` tests: every entry point that hands a request to the handler
// itself resolves the transcript-request fields the way `deliver()` does.
// They test the prepared request, not a delivery: `test_state()`'s
// `reactive_handler` is the global singleton every test in the binary
// shares (see `verify_jekt_signature_tests`).

use super::*;
use crate::server::jekt_echo::EchoTrust;
use crate::server::reactive::resolve_transcript_request_tier_fields;
use crate::server::tests::test_state;

fn transcript_request_message() -> String {
    r#"{"type":"transcript_request","request_id":"r1","max_lines":50}"#.to_string()
}

#[tokio::test]
async fn bus_inject_marks_a_transcript_request_as_deliver_does() {
    let state = test_state();
    let req = bus_inject_request(&state, "requester", "agent1", &transcript_request_message(), None, String::new());
    assert!(req.is_transcript_request, "a transcript_request body reads TIER=sensitive");

    let mut via_deliver = InjectionRequest {
        target_agent: "agent1".to_string(),
        message: transcript_request_message(),
        source_agent: Some("requester".to_string()),
        delivery_tier: Some("host".to_string()),
        ..Default::default()
    };
    resolve_transcript_request_tier_fields(&state.mstore, &mut via_deliver);
    assert_eq!(req.transcript_request_escalate_forced, via_deliver.transcript_request_escalate_forced);
}

#[tokio::test]
async fn bus_inject_leaves_an_ordinary_message_alone() {
    let state = test_state();
    let req = bus_inject_request(&state, "requester", "agent1", "just chatting", None, String::new());
    assert!(!req.is_transcript_request);
    assert!(!req.transcript_request_escalate_forced);
}

#[tokio::test]
async fn bus_inject_keeps_its_request_fields() {
    let state = test_state();
    let req = bus_inject_request(&state, "a", "b", "hi", Some("high".to_string()), "uid-1".to_string());
    assert_eq!(req.source_agent.as_deref(), Some("a"));
    assert_eq!(req.target_agent, "b");
    assert_eq!(req.priority.as_deref(), Some("high"));
    assert_eq!(req.delivery_tier.as_deref(), Some("host"));
    assert_eq!(req.audit_source_uid, "uid-1");
}

#[tokio::test]
async fn a_widget_send_marks_a_transcript_request() {
    let state = test_state();
    let mut req = InjectionRequest {
        target_agent: "agent1".to_string(),
        message: transcript_request_message(),
        source_agent: Some("widget:acme.board".to_string()),
        delivery_tier: Some("host".to_string()),
        ..Default::default()
    };
    prepare_host_entry(&state, &mut req);
    assert!(req.is_transcript_request);
    assert_eq!(req.sig_verified, None, "a widget has no key on file");
}

fn bridge_request(message: String) -> InjectionRequest {
    InjectionRequest {
        target_agent: "agent1".to_string(),
        message,
        source_agent: Some("slack".to_string()),
        delivery_tier: Some("wan".to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_bridge_request_is_resolved_against_the_store() {
    let state = test_state();
    let mut req = bridge_request(transcript_request_message());
    resolve_with_store(Some(&state.mstore), &mut req);
    assert!(req.is_transcript_request);

    let mut direct = bridge_request(transcript_request_message());
    resolve_transcript_request_tier_fields(&state.mstore, &mut direct);
    assert_eq!(req.transcript_request_escalate_forced, direct.transcript_request_escalate_forced);
}

#[test]
fn without_a_store_a_transcript_request_reads_as_under_ask() {
    let mut req = bridge_request(transcript_request_message());
    resolve_with_store(None, &mut req);
    assert!(req.is_transcript_request);
    assert!(req.transcript_request_escalate_forced, "ESCALATE=required, as for `ask`");

    let mut plain = bridge_request("hello".to_string());
    resolve_with_store(None, &mut plain);
    assert!(!plain.is_transcript_request);
    assert!(!plain.transcript_request_escalate_forced);
}

/// The `bus:inject` sender echo renders the verdicts the delivery was
/// decided on: a sender with a key on file and no signature echoes as
/// `sig_verified: Some(false)` (TRUST=unverified), as its recipient reads it.
#[tokio::test]
async fn the_bus_inject_echo_carries_the_signature_verdict() {
    let state = test_state();
    state.mstore.agent_jekt_key_ensure("echo-sender").unwrap();
    let req = bus_inject_request(&state, "echo-sender", "agent1", "hello", None, String::new());
    assert_eq!(req.sig_verified, Some(false));

    let trust = EchoTrust::of(&req);
    assert_eq!(trust.delivery_tier, "host");
    assert_eq!(trust.sig_verified, Some(false));
}

#[test]
fn echo_trust_takes_every_verdict_from_the_request() {
    let req = InjectionRequest {
        delivery_tier: Some("lan".to_string()),
        sig_verified: Some(true),
        reagent_verified: Some(false),
        lan_verified: Some(true),
        channel_verified: Some(false),
        ..Default::default()
    };
    let trust = EchoTrust::of(&req);
    assert_eq!(
        trust,
        EchoTrust {
            delivery_tier: "lan",
            sig_verified: Some(true),
            reagent_verified: Some(false),
            lan_verified: Some(true),
            channel_verified: Some(false),
        }
    );
    assert_eq!(EchoTrust::of(&InjectionRequest::default()).delivery_tier, "host");
}
