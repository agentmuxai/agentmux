// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The urgent-message watch's record: what it keeps, whose messages it keeps, and that it cannot touch a delivery.

use super::*;
use crate::backend::message_origin::OriginTrust;
use crate::backend::reactive::{InjectionRequest, InjectionResponse};

use super::TEST_TURN as TURN;

const PROVEN: [OriginTrust; 4] = [OriginTrust::HostVerified, OriginTrust::ChannelVerified, OriginTrust::LanVerified, OriginTrust::WanVerified];
const NOT_PROVEN: [OriginTrust; 3] = [OriginTrust::Unverified, OriginTrust::SelfDeclared, OriginTrust::NetworkClaimed];

fn ask(trust: OriginTrust) -> Option<Ask> {
    ask_from(Some("urgent"), trust, Some("AgentY"), "please look at #48288 now, the secret is hunter2", Some("block-1"), 5_000)
}

fn delivered(block: &str) -> InjectionResponse {
    serde_json::from_value(serde_json::json!({ "success": true, "request_id": "r1", "block_id": block, "timestamp": 1 })).expect("a response")
}

fn request(sender: &str, priority: &str, message: &str, tier: &str, sig: Option<bool>) -> InjectionRequest {
    InjectionRequest {
        source_agent: Some(sender.into()),
        priority: Some(priority.into()),
        message: message.into(),
        delivery_tier: Some(tier.into()),
        sig_verified: sig,
        ..Default::default()
    }
}

fn open_for(block: &str) -> Vec<Ask> {
    open_asks(5_000).into_iter().filter(|a| a.block_id == block).collect()
}

#[test]
fn an_urgent_message_from_a_proven_sender_that_names_a_task_is_an_ask() {
    for trust in PROVEN {
        let a = ask(trust).unwrap_or_else(|| panic!("no ask for {trust:?}"));
        assert_eq!(a, Ask { sender: "agenty".into(), block_id: "block-1".into(), tasks: vec![48288], at_ms: 5_000 });
    }
}

#[test]
fn a_sender_that_was_not_proven_for_the_message_makes_no_ask() {
    for trust in NOT_PROVEN {
        assert_eq!(ask(trust), None, "{trust:?}");
    }
}

#[test]
fn only_an_urgent_message_that_names_a_task_from_a_named_sender_to_a_pane_is_an_ask() {
    let t = OriginTrust::HostVerified;
    assert_eq!(ask_from(Some("normal"), t, Some("agenty"), "see #1", Some("b"), 1), None);
    assert_eq!(ask_from(None, t, Some("agenty"), "see #1", Some("b"), 1), None);
    assert_eq!(ask_from(Some("urgent"), t, Some("agenty"), "no task here", Some("b"), 1), None);
    assert_eq!(ask_from(Some("urgent"), t, None, "see #1", Some("b"), 1), None);
    assert_eq!(ask_from(Some("urgent"), t, Some("  "), "see #1", Some("b"), 1), None);
    assert_eq!(ask_from(Some("urgent"), t, Some("agenty"), "see #1", None, 1), None);
    assert_eq!(ask_from(Some("urgent"), t, Some("agenty"), "see #1", Some(""), 1), None);
}

#[test]
fn the_record_holds_the_sender_the_pane_the_task_numbers_and_the_time_and_not_the_words() {
    let a = ask(OriginTrust::HostVerified).expect("an ask");
    let shown = format!("{a:?}");
    assert!(!shown.contains("hunter2") && !shown.contains("please look"), "the text is kept: {shown}");
    // The record's whole shape: a change to it is a change to what is kept, and is made here on purpose.
    let Ask { sender, block_id, tasks, at_ms } = a;
    assert_eq!((sender.as_str(), block_id.as_str(), tasks, at_ms), ("agenty", "block-1", vec![48288], 5_000));
}

#[test]
fn a_delivered_urgent_message_is_kept_by_the_trust_the_marker_carries() {
    let _turn = TURN.lock();
    // A signed host message: host-verified.
    note_delivery(&request("AgentY", "urgent", "do #48288", "host", Some(true)), &delivered("keep-signed"));
    assert_eq!(open_for("keep-signed").len(), 1);
    // Unsigned (self-declared), signed wrongly (unverified) and a relayed claim (network-claimed): nothing kept.
    note_delivery(&request("AgentY", "urgent", "do #48288", "host", None), &delivered("keep-unsigned"));
    note_delivery(&request("AgentY", "urgent", "do #48288", "host", Some(false)), &delivered("keep-badsig"));
    note_delivery(&request("AgentY", "urgent", "do #48288", "wan", Some(true)), &delivered("keep-wan-claim"));
    for block in ["keep-unsigned", "keep-badsig", "keep-wan-claim"] {
        assert!(open_for(block).is_empty(), "{block} was kept");
    }
    settle(|a| a.block_id.starts_with("keep-"));
}

#[test]
fn a_message_that_was_not_delivered_or_is_not_urgent_is_not_kept() {
    let _turn = TURN.lock();
    let mut failed = delivered("not-delivered");
    failed.success = false;
    note_delivery(&request("AgentY", "urgent", "do #48288", "host", Some(true)), &failed);
    note_delivery(&request("AgentY", "normal", "do #48288", "host", Some(true)), &delivered("not-urgent"));
    assert!(open_for("not-delivered").is_empty() && open_for("not-urgent").is_empty());
}

#[test]
fn a_failure_in_the_record_does_not_reach_the_delivery() {
    // The record runs under `guarded`, which swallows a panic: the caller carries on exactly as before.
    guarded(|| panic!("the record failed"));
    let after = std::panic::catch_unwind(|| guarded(|| {}));
    assert!(after.is_ok());
}

#[test]
fn the_record_never_waits_for_the_list() {
    let _turn = TURN.lock();
    let held = ASKS.lock();
    let started = std::time::Instant::now();
    // While the watch has the list, the ask is dropped and the delivery goes on at once.
    note_delivery(&request("AgentY", "urgent", "do #48288", "host", Some(true)), &delivered("while-held"));
    assert!(started.elapsed() < std::time::Duration::from_millis(500), "it waited for the list");
    drop(held);
    assert!(open_for("while-held").is_empty());
}

#[test]
fn the_record_is_bounded_and_forgets_a_day_old_ask() {
    let _turn = TURN.lock();
    for i in 0..(MAX_ASKS + 20) {
        push(Ask { sender: "agenty".into(), block_id: format!("bound-{i}"), tasks: vec![1], at_ms: 5_000 });
    }
    let held = open_asks(5_000).into_iter().filter(|a| a.block_id.starts_with("bound-")).count();
    assert!(held <= MAX_ASKS, "{held} kept");
    assert!(open_for("bound-0").is_empty(), "the oldest goes first");
    assert!(open_asks(5_000 + MAX_AGE_MS).iter().all(|a| !a.block_id.starts_with("bound-")), "a day-old ask was kept");
}
