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

/// A note that was taken is counted against its message id, so a captured copy is not proven a second time; a refused one
/// releases the id as any refused jekt does.
#[test]
fn a_taken_note_cannot_be_replayed_and_a_refused_one_leaves_its_id_free() {
    use crate::backend::reactive::types::InjectionRequest;
    use super::super::reactive::replay;
    let signed = |id: &str| InjectionRequest {
        source_agent: Some("swarm-starpower-00000001".into()),
        request_id: Some(id.into()),
        lan_sig: Some("sig".into()),
        lan_verified: Some(true),
        ts_secs: Some(1_000),
        ..Default::default()
    };
    let now = agentmux_common::time::now_secs();
    let mut first = signed("quiet-note-taken");
    let mut claim = replay::reserve(&mut first, now);
    assert_eq!(first.lan_verified, Some(true));
    let kept = finish(&mut claim, answer("quiet-note-taken", true, ""));
    assert_eq!(kept["success"], true, "the answer is passed on unchanged");
    drop(claim);
    let mut copy = signed("quiet-note-taken");
    let _second = replay::reserve(&mut copy, now);
    assert_eq!(copy.lan_verified, Some(false), "a copy of a taken note no longer verifies");
    assert!(matches!(judge(&text("Wren"), "swarm-starpower-00000001", copy.lan_verified, true, &starpower()), Verdict::Refuse(_)));

    let mut once = signed("quiet-note-refused");
    let mut claim = replay::reserve(&mut once, now);
    finish(&mut claim, answer("quiet-note-refused", false, "no"));
    drop(claim);
    let mut again = signed("quiet-note-refused");
    let _third = replay::reserve(&mut again, now);
    assert_eq!(again.lan_verified, Some(true), "a refused note does not use up its id");
}

/// Notes past the limit are one held line that the next replaces, even while the lead's turn is running: they are never
/// written into the turn one by one.
#[test]
fn notes_past_the_limit_are_one_held_line_that_each_next_one_replaces() {
    let n = Note::new("Wren", false, 35, &[1], "");
    let (block, sender) = ("pane-quiet-fold-test", "swarm-starpower-00000001");
    place(block, sender, "starpower", 5, &n, Admit::Fold { more: 1 });
    place(block, sender, "starpower", 6, &n, Admit::Fold { more: 2 });
    let (turn, _) = crate::backend::reactive::held_notes::attach(block, "hello");
    assert_eq!(turn.matches("more quiet note").count(), 1, "{turn}");
    assert!(turn.contains("2 more quiet notes from starpower in the last hour"), "{turn}");
    assert!(!turn.contains("1 more quiet note "), "{turn}");
}

fn host(label: &str) -> (tempfile::TempDir, std::sync::Arc<crate::backend::storage::wan_identity::WanIdentityStore>) {
    let dir = tempfile::tempdir().unwrap();
    let wan = std::sync::Arc::new(crate::backend::storage::wan_identity::WanIdentityStore::open(&dir.path().join("wan.db")).unwrap());
    wan.instance_ensure(label).unwrap();
    (dir, wan)
}

/// The receiving half end to end, in one process: a note signed by a machine that was never heard from, whose sender key
/// is vouched for by an install this machine trusts, is pinned at first contact, taken, and held for the lead as a block
/// with its own marker. A copy of it is refused, and an ordinary jekt from the same machine is not touched.
#[tokio::test]
async fn a_signed_note_from_a_new_machine_is_taken_through_first_contact_with_its_certificate() {
    use agentmux_common::jekt_sign::{generate_lan_keypair, WanKeyRecord};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use super::super::reactive::replay;
    let state = crate::server::tests::test_state();
    let (_sdir, sender_wan) = host("starpower");
    let (_rdir, receiver) = host("narko");
    state.mstore.set_wan_identity(receiver.clone());
    let installed = sender_wan.instance_ensure("starpower").unwrap();
    receiver.agent_instance_pin_get_or_set("clare", "stable", &installed.instance_id, &installed.host_hint, agentmux_common::time::now_secs()).unwrap();
    let sender = lan::instance_sender_id("starpower", "stable");
    let (public, private) = generate_lan_keypair([5u8; 32]);
    let now = agentmux_common::time::now_secs();
    let cert = WanKeyRecord::certify(&installed.private_key, &sender, "lan", &public, &installed.host_hint, now).unwrap();
    let binding = agentmux_common::lan_install_binding::sign_lan_install_binding(&private, &cert.agent_id, &cert.instance_id, cert.issued_at).unwrap();
    state.lan_discovery.seed_lan_pubkey_with_cert_for_test(&sender, public.to_vec(), Some(cert));
    state.lan_discovery.seed_lan_install_sig_for_test(&sender, binding);
    assert!(state.mstore.lan_peer_pubkey_pin_since(&sender).unwrap().is_none(), "never heard from");
    let (lead, block) = ("quiet-lead-e2e", "block-quiet-lead-e2e");
    state.reactive_handler.register_agent(lead, block, None).unwrap();
    let mut peers = vec![peer("starpower", "stable", &["Wren"])];
    peers[0].address = "192.0.2.77".into();

    let signed = |id: &str, text: &str| {
        let mut req = lan::signed_request(&sender, &BASE64.encode(private), lead, text, id, now).unwrap();
        req.delivery_tier = Some("lan".into());
        req
    };
    let mut req = signed("e2e-note-1", &Note::new("Wren", false, 35, &[48288], "waiting on the build").wire());
    super::super::reactive::verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(true), "first contact: the certificate of a trusted install vouches for the sender key");
    assert!(state.mstore.lan_peer_pubkey_pin_since(&sender).unwrap().is_some(), "and the key is pinned");
    let mut claim = replay::reserve(&mut req, now);
    let answer = intercept_among(&state, &req, &mut claim, peers.clone()).await.expect("a quiet note is answered here");
    assert_eq!(answer["success"], true, "{answer}");
    drop(claim);
    let (turn, _) = crate::backend::reactive::held_notes::attach(block, "hello");
    assert!(turn.contains("[BULLETIN:FROM=") && turn.contains("TRUST=lan-verified INSTANCE_STATUS=approved"), "{turn}");
    assert!(turn.contains("Your agent Wren (on starpower) has been quiet for 35 min on #48288. It is idle."), "{turn}");
    assert!(!turn.contains("[AgentMux]") && !turn.contains("AGENTMUX-QUOTED"), "{turn}");

    // The same signed note again is a copy: it no longer verifies, and nothing is held.
    let mut copy = signed("e2e-note-1", &Note::new("Wren", false, 35, &[48288], "waiting on the build").wire());
    super::super::reactive::verify_lan_signature(&state, &mut copy).await;
    let mut claim = replay::reserve(&mut copy, now);
    let refused = intercept_among(&state, &copy, &mut claim, peers.clone()).await.expect("answered");
    assert_eq!(refused["success"], false, "{refused}");
    assert_eq!(crate::backend::reactive::held_notes::attach(block, "again").0.matches("[BULLETIN:").count(), 1, "still the one note, not two");

    // An ordinary jekt from the same machine's key is not a quiet note: it goes on as any jekt does.
    let mut plain = signed("e2e-plain-1", "hello from the machine");
    super::super::reactive::verify_lan_signature(&state, &mut plain).await;
    let mut claim = replay::reserve(&mut plain, now);
    assert!(intercept_among(&state, &plain, &mut claim, peers).await.is_none());
}

/// A worker whose definition name is not a route name is found among the agents its machine lists.
#[test]
fn a_worker_with_a_display_name_is_found_by_its_route_name() {
    let peers = vec![peer("starpower", "stable", &["Loap-3"])];
    let from = sent_by("starpower", "stable");
    let note = Note::new("Loap #3", false, 35, &[48288], "").wire();
    assert!(matches!(judge(&note, &from, Some(true), true, &peers), Verdict::Accept { .. }));
}

/// A worker whose route name is 64 characters long is found among the agents its machine lists.
#[test]
fn a_worker_with_a_64_character_route_name_is_accepted() {
    let long = "w".repeat(64);
    let peers = vec![peer("starpower", "stable", &[long.as_str()])];
    let note = Note::new(&long, false, 35, &[48288], "").wire();
    assert!(matches!(judge(&note, &sent_by("starpower", "stable"), Some(true), true, &peers), Verdict::Accept { .. }));
}
