// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The loop's decisions without a network: which members are on the LAN, when a round is due, what is remembered of
// one, and how a peer's answer is read.

use std::time::{Duration, Instant};

use super::*;

fn plan(collection: &str, members: &[&str], fingerprint: &str) -> Plan {
    Plan { collection: collection.into(), kind: "kanban".into(), members: members.iter().map(|m| m.to_string()).collect(), fingerprint: fingerprint.into() }
}

fn seen(install_id: &str, host: &str) -> Seen {
    Seen { install_id: install_id.into(), host: host.into() }
}

fn round(ended: Ended, pulled: usize) -> Round {
    Round { ended, entries_taken: 0, entries_given: 0, keys_taken: 0, pulled, kept_local: 0, conflicts: 0, pushed: 0, refused: Vec::new() }
}

#[test]
fn only_members_that_are_on_the_lan_are_asked() {
    let plans = [plan("kanban.main", &["m-a", "m-b"], "f1"), plan("kanban.other", &["m-c"], "f2")];
    let lan = [seen("m-a", "Area54"), seen("", "no-id"), seen("m-z", "stranger")];
    let t = targets(&plans, &lan);
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].collection.as_str(), t[0].machine.as_str(), t[0].host.as_str()), ("kanban.main", "m-a", "Area54"));
}

#[test]
fn a_machine_that_does_not_say_which_install_it_is_matches_no_member() {
    let plans = [plan("kanban.main", &[""], "f1")];
    assert!(targets(&plans, &[seen("", "x")]).is_empty());
}

#[test]
fn a_round_is_due_when_never_run_changed_or_old_and_not_otherwise() {
    let now = Instant::now();
    assert!(due(None, "f", now));
    let l = remember(&round(Ended::Synced, 0), "f", now);
    assert!(!due(Some(&l), "f", now + Duration::from_secs(30)), "nothing changed and it is recent");
    assert!(due(Some(&l), "g", now + Duration::from_secs(30)), "what this machine holds changed");
    assert!(due(Some(&l), "f", now + RESYNC), "after a while, to heal a lost push");
}

#[test]
fn a_round_that_took_something_is_looked_at_again_so_it_goes_on_to_the_others() {
    let now = Instant::now();
    let l = remember(&round(Ended::Synced, 3), "f", now);
    assert!(due(Some(&l), "f", now + Duration::from_secs(15)));
}

#[test]
fn a_peer_that_did_not_finish_is_left_alone_until_its_retry_time() {
    let now = Instant::now();
    for ended in [Ended::Unreachable("no route".into()), Ended::PeerIdle, Ended::NotAMemberThere, Ended::Distrusted("x".into()), Ended::Failed("x".into())] {
        let l = remember(&round(ended, 0), "f", now);
        assert!(!due(Some(&l), "g", now + RETRY - Duration::from_secs(1)), "even if this machine changed");
        assert!(due(Some(&l), "g", now + RETRY));
    }
}

#[test]
fn a_peer_answer_is_read_as_what_it_says_a_refusal_with_its_code_or_a_machine_that_did_not_answer() {
    let ok: Result<Vec<u32>, CallError> = answer_of(RemoteOpResult::done(json!([1, 2])));
    assert_eq!(ok, Ok(vec![1, 2]));
    let no: Result<Vec<u32>, CallError> = answer_of(RemoteOpResult::refused("not_active"));
    assert_eq!(no, Err(CallError::Refused("not_active".into())));
    let down: Result<Vec<u32>, CallError> = answer_of(RemoteOpResult::failed("couldn't reach that machine: timed out"));
    assert_eq!(down, Err(CallError::Unreachable("couldn't reach that machine: timed out".into())));
    let odd: Result<Vec<u32>, CallError> = answer_of(RemoteOpResult::done(json!({"nope": 1})));
    assert!(matches!(odd, Err(CallError::Refused(code)) if code.starts_with("bad_answer")));
}
