// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use crate::server::ReactiveAuthVia;

const NOW: u64 = 1_000_000;

fn peer(hostname: &str, last_polled_ok: u64) -> LanInstance {
    LanInstance {
        instance_id: format!("peer-{hostname}"),
        hostname: hostname.into(),
        version: "0".into(),
        channel: String::new(),
        os: String::new(),
        install: String::new(),
        build: String::new(),
        address: "192.0.2.10".into(),
        port: 1,
        auth_key: String::new(),
        agents: vec![],
        agent_status: Default::default(),
        agent_lines: Default::default(),
        agent_providers: Default::default(),
        agent_accounts: Default::default(),
        agent_resets: Default::default(),
        agent_seats: Default::default(),
        first_seen: 0,
        last_seen: NOW,
        last_polled_ok,
        other_ttl_secs: 4500,
    }
}

fn seen(hostname: &str) -> Option<SeenAgent> {
    Some(SeenAgent { hostname: hostname.into(), seen_secs: NOW })
}

#[test]
fn an_agent_no_peer_ever_listed_is_nobodys() {
    assert_eq!(down_host(None, &[], NOW), None);
}

#[test]
fn a_known_agent_whose_machine_is_not_in_the_peer_list_is_down() {
    assert_eq!(down_host(seen("charlie"), &[], NOW), Some("charlie".to_string()));
    assert_eq!(down_host(seen("charlie"), &[peer("starpower", NOW)], NOW), Some("charlie".to_string()), "another machine answering says nothing");
}

#[test]
fn a_machine_that_answers_its_poll_is_up_and_one_that_stopped_answering_is_down() {
    assert_eq!(down_host(seen("charlie"), &[peer("Charlie", NOW - POLL_FRESH_SECS)], NOW), None, "answered within the window, whatever the case");
    assert_eq!(down_host(seen("charlie"), &[peer("charlie", NOW - POLL_FRESH_SECS - 1)], NOW), Some("charlie".to_string()));
    // mDNS can keep a dead peer in the list for minutes; it never answered a poll, or stopped.
    assert_eq!(down_host(seen("charlie"), &[peer("charlie", 0)], NOW), Some("charlie".to_string()));
}

fn request(target: &str, id: &str) -> (InjectionRequest, InjectionResponse) {
    let req = InjectionRequest {
        target_agent: target.into(),
        message: "are you there?".into(),
        source_agent: Some("tester".into()),
        delivery_tier: Some("host".into()),
        request_id: Some(id.into()),
        audit_source_uid: "uid-tester".into(),
        ..Default::default()
    };
    let resp = InjectionResponse {
        deferred: None,
        success: false,
        request_id: id.into(),
        block_id: None,
        error: Some(format!("agent not found: {target}")),
        timestamp: 0,
        effective_tier: None,
        requires_stop: None,
        channel_verified: None,
    };
    (req, resp)
}

fn listed(name: &str, host: &str) {
    seen_agents::remember(&[name.to_string()], host, agentmux_common::time::now_secs_u64());
}

#[tokio::test]
async fn a_message_to_an_agent_of_a_machine_that_is_down_is_held_for_24_hours_under_the_peer_key() {
    let state = test_state();
    listed("Pholdone", "charlie");
    let (req, resp) = request("Pholdone", "req-peer-1");
    let answer = hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await.expect("held");
    assert_eq!(answer["held"], true, "{answer}");
    assert_eq!(answer["held_reason"], "peer_offline", "{answer}");
    assert_eq!(answer["retry_when"], "peer_online");
    assert_eq!(answer["held_host"], "charlie");
    assert_eq!(answer["request_id"], "req-peer-1");
    assert!(answer["error"].as_str().unwrap().contains("24 h"), "{answer}");

    let rows = state.mstore.jekt_held_for_target("peer:pholdone", 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].target_agent.as_str(), rows[0].message.as_str(), rows[0].audit_source_uid.as_str()), ("Pholdone", "are you there?", "uid-tester"));
    assert!(rows[0].expires_at_ms - rows[0].sent_at_ms >= 24 * 3600 * 1000 - 1, "kept for the full 24 h");

    // The same message again is not kept twice.
    let again = hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await.expect("still answered as held");
    assert_eq!(again["held"], true);
    assert_eq!(state.mstore.jekt_held_for_target("peer:pholdone", 10).unwrap().len(), 1);
}

#[tokio::test]
async fn the_replay_for_local_agents_never_sees_a_peer_row() {
    let state = test_state();
    listed("Pholdtwo", "charlie");
    let (req, resp) = request("Pholdtwo", "req-peer-2");
    hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await.unwrap();
    let outcomes = crate::server::jekt_held::replay_pass(&state).await;
    assert!(outcomes.is_empty(), "{outcomes:?}");
    assert_eq!(state.mstore.jekt_held_for_target("peer:pholdtwo", 10).unwrap().len(), 1);
}

#[tokio::test]
async fn nothing_is_held_for_a_name_no_peer_listed() {
    let state = test_state();
    let (req, resp) = request("Pnobody-listed-this", "req-peer-3");
    assert_eq!(hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await, None);
    assert!(state.mstore.jekt_held_targets().unwrap().is_empty());
}

#[tokio::test]
async fn the_hold_follows_the_rules_of_any_hold() {
    let state = test_state();
    listed("Pholdthree", "charlie");
    let (req, resp) = request("Pholdthree", "req-peer-4");
    // A LAN caller never fills the hold.
    assert_eq!(hold_for_down_peer(&state, ReactiveAuthVia::LanKey, &req, &resp).await, None);
    // Nor does a periodic cron fire.
    let cron = InjectionRequest { source_agent: Some("cron".into()), ..req.clone() };
    assert_eq!(hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &cron, &resp).await, None);
    // Nor a message with no id to key the row by.
    let no_id = InjectionResponse { request_id: String::new(), ..request("Pholdthree", "").1 };
    assert_eq!(hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &no_id).await, None);
    assert!(state.mstore.jekt_held_targets().unwrap().is_empty());
}

#[tokio::test]
async fn an_agent_defined_here_is_held_the_old_way_even_when_a_peer_once_listed_its_name() {
    let state = test_state();
    listed("Pholdfour", "charlie");
    let mut def = test_agent_def("uid-pholdfour", "Pholdfour", "claude", "agent", 1, "");
    def.slug = "pholdfour".into();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let (req, resp) = request("Pholdfour", "req-peer-5");
    assert_eq!(hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await, None);
}

#[tokio::test]
async fn a_replay_with_the_machine_still_down_keeps_the_message() {
    let state = test_state();
    listed("Pholdfive", "charlie");
    let (req, resp) = request("Pholdfive", "req-peer-6");
    hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await.unwrap();
    replay_pass(&state).await;
    assert_eq!(state.mstore.jekt_held_for_target("peer:pholdfive", 10).unwrap().len(), 1, "no peer, no relay: it waits");
}

#[tokio::test]
async fn a_refused_message_counts_and_a_taken_one_goes() {
    let state = test_state();
    listed("Pholdsix", "charlie");
    let (req, resp) = request("Pholdsix", "req-peer-7");
    hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await.unwrap();
    let row = state.mstore.jekt_held_for_target("peer:pholdsix", 10).unwrap().remove(0);

    settle(&state, &row, Sent::Later).await;
    assert_eq!(state.mstore.jekt_held_for_target("peer:pholdsix", 10).unwrap().len(), 1, "not now: it stays, uncounted");
    assert_eq!(state.mstore.jekt_held_for_target("peer:pholdsix", 10).unwrap()[0].attempts, 0);

    settle(&state, &row, Sent::Refused).await;
    assert_eq!(state.mstore.jekt_held_for_target("peer:pholdsix", 10).unwrap()[0].attempts, 1, "a refusal is counted");

    settle(&state, &row, Sent::Gone).await;
    assert!(state.mstore.jekt_held_for_target("peer:pholdsix", 10).unwrap().is_empty());
}

#[tokio::test]
async fn a_message_refused_too_often_is_dropped() {
    let state = test_state();
    listed("Pholdseven", "charlie");
    let (req, resp) = request("Pholdseven", "req-peer-8");
    hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await.unwrap();
    let row = state.mstore.jekt_held_for_target("peer:pholdseven", 10).unwrap().remove(0);
    for _ in 0..MAX_REFUSALS {
        settle(&state, &row, Sent::Refused).await;
    }
    assert!(state.mstore.jekt_held_for_target("peer:pholdseven", 10).unwrap().is_empty());
}

#[test]
fn a_replayed_message_is_addressed_by_name_and_a_late_one_says_so() {
    let (req, _) = request("Pholdeight", "req-peer-9");
    let mut row = crate::server::jekt_hold::held_row(&req, "req-peer-9".into(), 1_700_000_000_000);
    row.target_uid = peer_key("pholdeight");

    let replayed = request_for(&row);
    assert_eq!(replayed.target_agent, "Pholdeight", "the name the sender used, not the key");
    assert_eq!((replayed.delivery_tier.as_deref(), replayed.marker_target.as_deref()), (Some("host"), None));
    assert_eq!(replayed.request_id.as_deref(), Some("req-peer-9"), "the same id, so the receiver can tell a repeat");

    assert!(late_message(&row, 1_700_000_000_000 + 60_000).starts_with("are you there?"), "a minute late is not marked");
    let late = late_message(&row, 1_700_000_000_000 + 3 * 3600 * 1000);
    assert!(late.starts_with("(sent ") && late.ends_with("are you there?"), "{late}");
}

#[test]
fn every_agent_gets_to_the_front_of_the_replay_in_turn() {
    let keys: Vec<String> = (0..10).map(|i| format!("peer:a{i}")).collect();
    let mut firsts: Vec<String> = (0..keys.len()).map(|pass| rotated(keys.clone(), pass)[0].clone()).collect();
    firsts.sort();
    let mut all = keys.clone();
    all.sort();
    assert_eq!(firsts, all, "after one lap each agent has led a pass, so a machine that stays down cannot hold the front");
    assert_eq!(rotated(keys.clone(), keys.len() + 3)[0], "peer:a3", "past a lap it wraps");
    assert_eq!(rotated(Vec::new(), 5), Vec::<String>::new(), "nothing held is nothing to rotate");
    assert_eq!(rotated(keys.clone(), 4).len(), keys.len(), "none is lost");
}

#[test]
fn a_row_is_spent_at_its_hold_or_its_senders_deadline() {
    let (req, _) = request("Pspent", "req-spent");
    let mut row = crate::server::jekt_hold::held_row(&req, "req-spent".into(), 1_000);
    row.expires_at_ms = 5_000;
    assert!(!spent(&row, 4_999));
    assert!(spent(&row, 5_000), "the hold ran out");
    row.sender_expires_at_ms = Some(3_000);
    assert!(spent(&row, 3_000), "the sender's deadline came first");
    assert!(!spent(&row, 2_999));
}

#[tokio::test]
async fn a_row_that_ran_out_is_dropped_not_sent_and_a_live_one_waits() {
    let state = test_state();
    listed("Pspenttwo", "charlie");
    let (req, resp) = request("Pspenttwo", "req-spent-2");
    hold_for_down_peer(&state, ReactiveAuthVia::FullAuthKey, &req, &resp).await.unwrap();
    let mut row = state.mstore.jekt_held_for_target("peer:pspenttwo", 10).unwrap().remove(0);

    // No peer and no relay login here, so a live row is not sent: it waits.
    assert_eq!(send_row(&state, &row, &None).await, Sent::Later);
    // The same row once its 24 h ran out is settled as gone without any send being tried.
    row.expires_at_ms = agentmux_common::time::now_ms() - 1;
    let sent = send_row(&state, &row, &None).await;
    assert_eq!(sent, Sent::Gone);
    settle(&state, &row, sent).await;
    assert!(state.mstore.jekt_held_for_target("peer:pspenttwo", 10).unwrap().is_empty());
}
