// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
use super::*;

fn peer(host: &str, channel: &str, agents: &[&str]) -> LanInstance {
    serde_json::from_value(json!({
        "instance_id": "v0", "hostname": host, "version": "0.60.40", "channel": channel, "os": "macos",
        "address": "192.0.2.10", "port": 29700, "auth_key": "k", "agents": agents, "first_seen": 0, "last_seen": 0,
        "other_ttl_secs": 4500
    }))
    .unwrap()
}

fn sent_by(host: &str, channel: &str) -> String {
    lan::instance_sender_id(host, channel)
}

fn text(agent: &str) -> String {
    Note::new(agent, false, 35, &[48288], "waiting on the build").wire()
}

fn starpower() -> Vec<LanInstance> {
    vec![peer("starpower", "stable", &["Wren", "Pulp"]), peer("charlie", "stable", &["Maricon"])]
}

/// Guarantee c: a note is accepted only from the machine that hosts the named worker, proven and approved.
#[test]
fn a_note_is_accepted_only_from_the_machine_that_hosts_the_worker_when_proven_and_approved() {
    let peers = starpower();
    let from = sent_by("starpower", "stable");
    match judge(&text("Wren"), &from, Some(true), true, &peers) {
        Verdict::Accept { note, machine } => assert_eq!((note.agent.as_str(), machine.as_str(), note.minutes), ("Wren", "starpower", 35)),
        other => panic!("{other:?}"),
    }
    assert!(matches!(judge(&text("Wren"), &from, None, true, &peers), Verdict::Refuse(_)), "nothing proven");
    assert!(matches!(judge(&text("Wren"), &from, Some(false), true, &peers), Verdict::Refuse(_)), "a signature that failed");
    assert!(matches!(judge(&text("Wren"), &from, Some(true), false, &peers), Verdict::Refuse(_)), "a new or revoked install");
    // The worker is on another machine: charlie's sender key cannot speak for starpower's agent.
    let other = sent_by("charlie", "stable");
    assert!(matches!(judge(&text("Wren"), &other, Some(true), true, &peers), Verdict::Refuse(_)), "a note about another machine's agent");
    assert!(matches!(judge(&text("Maricon"), &other, Some(true), true, &peers), Verdict::Accept { .. }), "its own agent is fine");
    // A sender that is no listed peer.
    let unknown = sent_by("nowhere", "stable");
    assert!(matches!(judge(&text("Wren"), &unknown, Some(true), true, &peers), Verdict::Refuse(_)));
    // A body that is not exactly a note.
    assert!(matches!(judge("QUIET-NOTE {\"agent\":\"Wren\"}", &from, Some(true), true, &peers), Verdict::Refuse(_)));
}

/// Only another machine's own sender key is held to this: every other jekt, and any text that does not start like a note,
/// is delivered as it was.
#[test]
fn every_other_jekt_goes_on_as_it_did() {
    let peers = starpower();
    assert_eq!(judge("hello", &sent_by("starpower", "stable"), Some(true), true, &peers), Verdict::Ordinary);
    assert_eq!(judge("Swarm broadcast from starpower, sent by its operator to 1 agent:\n\nhi", &sent_by("starpower", "stable"), Some(true), true, &peers), Verdict::Ordinary);
    // An agent that writes the words is delivered, under its own marker and trust line, as a message from that agent.
    assert_eq!(judge(&text("Wren"), "wren", Some(true), true, &peers), Verdict::Ordinary);
    assert_eq!(judge(&text("Wren"), "swarm", Some(true), true, &peers), Verdict::Ordinary, "not the shape of a machine's sender name");
    assert_eq!(judge(&text("Wren"), "", Some(true), true, &peers), Verdict::Ordinary);
}

/// Guarantee e: the sending side picks exactly one listed peer, never guesses.
#[test]
fn a_lead_is_sent_to_only_when_one_peer_of_that_name_lists_it() {
    let peers = vec![peer("narko", "stable", &["AgentY", "AgentX"]), peer("charlie", "stable", &["Maricon"]), peer("charlie", "dev", &["Maricon"])];
    assert_eq!(peer_for(&peers, "agenty", "NARKO").unwrap().channel, "stable", "any case");
    assert!(peer_for(&peers, "AgentY", "starpower").is_err(), "a machine this one has not seen");
    assert!(peer_for(&peers, "Clare", "narko").is_err(), "a machine that does not list the lead");
    assert!(peer_for(&peers, "Maricon", "charlie").is_err(), "two peers list it: the seat has to say which");
    assert!(peer_for(&[], "AgentY", "narko").is_err());
}

#[test]
fn the_answer_to_the_sender_says_whether_it_was_taken() {
    assert_eq!(answer("m1", true, "")["success"], true);
    let refused = answer("m1", false, "the sender is not proven");
    assert_eq!((refused["success"].as_bool(), refused["refused"].as_bool()), (Some(false), Some(true)));
}
