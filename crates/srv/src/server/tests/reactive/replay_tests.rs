// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn signed(source: &str, target: &str, msgid: &str) -> InjectionRequest {
    InjectionRequest {
        source_agent: Some(source.to_string()),
        target_agent: target.to_string(),
        request_id: Some(msgid.to_string()),
        message: "hello".to_string(),
        sig_verified: Some(true),
        ..Default::default()
    }
}

#[test]
fn a_signed_message_delivered_once_fails_the_second_time() {
    let mut seen = Seen::default();
    let mut first = signed("agentx", "agenty", "m-1");
    assert!(!seen.check(&mut first, 1_000));
    seen.record(&first, 1_000);

    let mut again = signed("AgentX", "AgentY", "m-1");
    assert!(seen.check(&mut again, 1_100));
    assert_eq!(again.sig_verified, Some(false));
}

#[test]
fn every_verified_verdict_fails_and_others_are_left_alone() {
    let mut seen = Seen::default();
    let mut first = signed("agentx", "agenty", "m-1");
    first.lan_verified = Some(true);
    seen.record(&first, 1_000);

    let mut again = signed("agentx", "agenty", "m-1");
    again.lan_verified = Some(true);
    again.channel_verified = None;
    seen.check(&mut again, 1_001);
    assert_eq!(again.sig_verified, Some(false));
    assert_eq!(again.lan_verified, Some(false));
    assert_eq!(again.channel_verified, None);
}

#[test]
fn another_message_id_sender_or_target_is_a_different_message() {
    let mut seen = Seen::default();
    seen.record(&signed("agentx", "agenty", "m-1"), 1_000);
    for mut other in [signed("agentx", "agenty", "m-2"), signed("korp", "agenty", "m-1"), signed("agentx", "korp", "m-1")] {
        assert!(!seen.check(&mut other, 1_001));
        assert_eq!(other.sig_verified, Some(true));
    }
}

#[test]
fn unsigned_messages_are_neither_checked_nor_recorded() {
    let mut seen = Seen::default();
    let mut unsigned = signed("agentx", "agenty", "m-1");
    unsigned.sig_verified = None;
    seen.record(&unsigned, 1_000);
    assert!(seen.entries.is_empty());
    seen.record(&signed("agentx", "agenty", "m-1"), 1_000);
    let mut again = signed("agentx", "agenty", "m-1");
    again.sig_verified = None;
    assert!(!seen.check(&mut again, 1_001));
    assert_eq!(again.sig_verified, None);
}

#[test]
fn a_message_without_an_id_is_never_matched() {
    let mut seen = Seen::default();
    let mut first = signed("agentx", "agenty", "");
    seen.record(&first, 1_000);
    assert!(seen.entries.is_empty());
    assert!(!seen.check(&mut first, 1_001));
}

#[test]
fn an_entry_ends_after_the_window() {
    let mut seen = Seen::default();
    seen.record(&signed("agentx", "agenty", "m-1"), 1_000);
    let mut later = signed("agentx", "agenty", "m-1");
    assert!(!seen.check(&mut later, 1_000 + WINDOW_SECS + 1));
    assert_eq!(later.sig_verified, Some(true));
}

#[test]
fn the_memory_stays_bounded() {
    let mut seen = Seen::default();
    for i in 0..(MAX_ENTRIES + 10) {
        seen.record(&signed("agentx", "agenty", &format!("m-{i}")), 1_000 + i as i64);
    }
    assert!(seen.entries.len() <= MAX_ENTRIES);
    let mut newest = signed("agentx", "agenty", &format!("m-{}", MAX_ENTRIES + 9));
    assert!(seen.check(&mut newest, 1_000 + MAX_ENTRIES as i64 + 9));
}
