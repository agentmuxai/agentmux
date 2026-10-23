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
fn a_signed_message_counts_once_and_fails_the_second_time() {
    let mut seen = Seen::default();
    let mut first = signed("agentx", "agenty", "m-1");
    assert!(seen.reserve(&mut first, 1_000).is_some());
    assert_eq!(first.sig_verified, Some(true));

    let mut again = signed("AgentX", "AgentY", "m-1");
    assert!(seen.reserve(&mut again, 1_100).is_none());
    assert_eq!(again.sig_verified, Some(false));
}

#[test]
fn two_copies_arriving_together_cannot_both_pass() {
    // Reserving is the check: the second copy fails before the first is delivered.
    let mut seen = Seen::default();
    let mut first = signed("agentx", "agenty", "m-1");
    let mut second = signed("agentx", "agenty", "m-1");
    let reserved = seen.reserve(&mut first, 1_000);
    assert!(reserved.is_some());
    assert!(seen.reserve(&mut second, 1_000).is_none());
    assert_eq!(second.sig_verified, Some(false));
}

#[test]
fn a_released_reservation_lets_the_message_through_again() {
    let mut seen = Seen::default();
    let mut forwarded = signed("agentx", "agenty", "m-1");
    let key = seen.reserve(&mut forwarded, 1_000).unwrap();
    seen.release(&key);
    let mut back = signed("agentx", "agenty", "m-1");
    assert!(seen.reserve(&mut back, 1_001).is_some());
    assert_eq!(back.sig_verified, Some(true));
}

#[test]
fn every_verified_verdict_fails_and_others_are_left_alone() {
    let mut seen = Seen::default();
    let mut first = signed("agentx", "agenty", "m-1");
    first.lan_verified = Some(true);
    seen.reserve(&mut first, 1_000);

    let mut again = signed("agentx", "agenty", "m-1");
    again.lan_verified = Some(true);
    seen.reserve(&mut again, 1_001);
    assert_eq!(again.sig_verified, Some(false));
    assert_eq!(again.lan_verified, Some(false));
    assert_eq!(again.channel_verified, None);
}

#[test]
fn another_message_id_sender_or_target_is_a_different_message() {
    let mut seen = Seen::default();
    seen.reserve(&mut signed("agentx", "agenty", "m-1"), 1_000);
    for mut other in [signed("agentx", "agenty", "m-2"), signed("korp", "agenty", "m-1"), signed("agentx", "korp", "m-1")] {
        assert!(seen.reserve(&mut other, 1_001).is_some());
        assert_eq!(other.sig_verified, Some(true));
    }
}

#[test]
fn unsigned_and_id_less_messages_are_ignored() {
    let mut seen = Seen::default();
    let mut unsigned = signed("agentx", "agenty", "m-1");
    unsigned.sig_verified = None;
    assert!(seen.reserve(&mut unsigned, 1_000).is_none());
    let mut no_id = signed("agentx", "agenty", "");
    assert!(seen.reserve(&mut no_id, 1_000).is_none());
    assert!(seen.entries.is_empty());
    assert_eq!(no_id.sig_verified, Some(true));
}

#[test]
fn an_entry_ends_after_the_window() {
    let mut seen = Seen::default();
    seen.reserve(&mut signed("agentx", "agenty", "m-1"), 1_000);
    let mut later = signed("agentx", "agenty", "m-1");
    assert!(seen.reserve(&mut later, 1_000 + WINDOW_SECS + 1).is_some());
    assert_eq!(later.sig_verified, Some(true));
}

#[test]
fn the_memory_stays_bounded() {
    let mut seen = Seen::default();
    for i in 0..(MAX_ENTRIES + 10) {
        seen.reserve(&mut signed("agentx", "agenty", &format!("m-{i}")), 1_000 + i as i64);
    }
    assert!(seen.entries.len() <= MAX_ENTRIES);
    let mut newest = signed("agentx", "agenty", &format!("m-{}", MAX_ENTRIES + 9));
    assert!(seen.reserve(&mut newest, 1_000 + MAX_ENTRIES as i64 + 9).is_none());
}
