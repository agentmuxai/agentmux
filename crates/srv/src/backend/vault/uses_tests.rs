// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const WAIT: Duration = Duration::from_secs(120);

fn req(id: &str) -> UseRequest {
    UseRequest {
        id: id.into(),
        agent: "Agent1".into(),
        block_id: "pane-1".into(),
        entry_id: "ssn-alex".into(),
        entry_label: "Alex".into(),
        what: "Social Security number".into(),
        origin: "https://irs.gov".into(),
        field: "SSN".into(),
        offer_always: true,
        reason: "a new site".into(),
        kind: "ssn".into(),
        ..Default::default()
    }
}

fn key(id: &str, agent: &str) -> UseRequest {
    UseRequest { agent: agent.into(), kind: "api-key".into(), entry_label: "GitHub token".into(), ..req(id) }
}

fn uses() -> &'static Uses {
    Box::leak(Box::default())
}

fn ended(rx: &mut oneshot::Receiver<UseAnswer>) -> Option<Ended> {
    rx.try_recv().ok().map(|a| Ended::of(Some(&a)))
}

#[tokio::test]
async fn an_answer_reaches_the_waiting_agent_once() {
    let u = uses();
    let rx = u.ask(req("u1"), WAIT);
    assert_eq!(u.list().len(), 1);
    assert!(u.answer("u1", UseAnswer { decision: Decision::Once, value: Some("typed".to_string().into()), save: true, by: By::Person }));
    assert!(!u.answer("u1", UseAnswer::of(Decision::Deny)), "answered already");
    let got = rx.await.unwrap();
    assert_eq!((got.decision, got.value.as_deref().map(String::as_str), got.save), (Decision::Once, Some("typed"), true));
    assert!(u.list().is_empty());
}

#[test]
fn a_request_nobody_waits_on_goes_and_cant_be_answered() {
    let u = uses();
    drop(u.ask(req("u1"), WAIT));
    assert!(u.list().is_empty());
    assert!(!u.answer("u1", UseAnswer::of(Decision::Once)));
}

#[test]
fn decisions_are_parsed_by_name_only() {
    assert_eq!(Decision::parse("once"), Some(Decision::Once));
    assert_eq!(Decision::parse("always"), Some(Decision::Always));
    assert_eq!(Decision::parse("deny"), Some(Decision::Deny));
    assert_eq!(Decision::parse("ALWAYS"), None);
    assert_eq!(Decision::parse(""), None);
}

#[test]
fn only_an_api_key_login_or_note_goes_ahead_on_silence() {
    for kind in ["api-key", "login", "note"] {
        assert!(goes_ahead_on_silence(kind, false), "{kind}");
        assert!(!goes_ahead_on_silence(kind, true), "{kind}: a value only the person can type never goes ahead");
    }
    for kind in ["ssn", "itin", "tin", "ein", "date-of-birth", "passport", "drivers-licence", "other-id", "card", "bank-account", ""] {
        assert!(!goes_ahead_on_silence(kind, false), "{kind:?}");
    }
}

#[test]
fn an_api_key_nobody_answers_goes_ahead_once_after_30_seconds() {
    let u = uses();
    let (mut rx, _) = u.enqueue(key("k", "Agent1"), WAIT, 1_000);
    let r = u.get("k").unwrap();
    assert_eq!((r.goes_ahead, r.deadline_ms, r.expires_ms), (true, 31_000, 121_000));
    u.decide_unanswered("k", 30_999);
    assert_eq!(ended(&mut rx), None, "not before the deadline");
    u.decide_unanswered("k", 31_000);
    assert_eq!(ended(&mut rx), Some(Ended::WentAhead), "once, never Always");
    assert!(u.get("k").is_none());
}

#[test]
fn an_id_entry_is_never_approved_by_time_and_expires_denied() {
    let u = uses();
    let (mut rx, _) = u.enqueue(req("id"), WAIT, 1_000);
    let r = u.get("id").unwrap();
    assert_eq!((r.goes_ahead, r.deadline_ms), (false, 121_000), "it waits the request's own wait");
    u.decide_unanswered("id", 31_000);
    assert_eq!(ended(&mut rx), None);
    assert!(u.hold_at("id", 115_000));
    assert_eq!(u.get("id").unwrap().deadline_ms, 121_000, "a hold doesn't move it");
    u.decide_unanswered("id", 121_000);
    assert_eq!(ended(&mut rx), Some(Ended::Expired));
}

#[test]
fn a_value_the_person_types_never_goes_ahead_on_silence() {
    let u = uses();
    let (mut rx, _) = u.enqueue(UseRequest { needs_value: true, ..key("typed", "Agent1") }, WAIT, 0);
    u.decide_unanswered("typed", 30_000);
    assert_eq!(ended(&mut rx), None);
    u.decide_unanswered("typed", 120_000);
    assert_eq!(ended(&mut rx), Some(Ended::Expired));
}

#[test]
fn a_hold_moves_the_deadline_but_never_past_the_requests_own_wait() {
    let u = uses();
    let (mut rx, _) = u.enqueue(key("k", "Agent1"), Duration::from_secs(40), 0);
    assert!(u.hold_at("k", 20_000));
    assert_eq!(u.get("k").unwrap().deadline_ms, 35_000, "15 s past the last activity");
    assert!(u.hold_at("k", 10_000));
    assert_eq!(u.get("k").unwrap().deadline_ms, 35_000, "never earlier");
    assert!(u.hold_at("k", 30_000));
    assert_eq!(u.get("k").unwrap().deadline_ms, 40_000, "never past the outer wait");
    u.decide_unanswered("k", 34_000);
    assert_eq!(ended(&mut rx), None);
    u.decide_unanswered("k", 40_000);
    assert_eq!(ended(&mut rx), Some(Ended::WentAhead));
    assert!(!u.hold("k"), "nothing waits now");
}

#[test]
fn one_agent_crowding_denies_its_own_oldest_only() {
    let u = uses();
    let (mut other, _) = u.enqueue(key("other", "Agent2"), WAIT, 0);
    let mut mine: Vec<_> = (0..MAX_PER_AGENT).map(|i| u.enqueue(key(&format!("m{i}"), "Agent1"), WAIT, 0).0).collect();
    let _newest = u.enqueue(key("m-new", "Agent1"), WAIT, 0);
    assert_eq!(ended(&mut mine[0]), Some(Ended::Crowded));
    assert_eq!(ended(&mut mine[1]), None);
    assert_eq!(ended(&mut other), None, "another agent's request stays");
    assert_eq!(u.list().iter().filter(|r| r.agent == "Agent1").count(), MAX_PER_AGENT);
}

#[test]
fn a_full_queue_never_pushes_out_another_agents_request() {
    let u = uses();
    let mut held = Vec::new();
    for a in 0..MAX_PENDING / MAX_PER_AGENT {
        for i in 0..MAX_PER_AGENT {
            held.push(u.enqueue(key(&format!("{a}-{i}"), &format!("Agent{a}")), WAIT, 0).0);
        }
    }
    let (mut newcomer, id) = u.enqueue(key("new", "Newcomer"), WAIT, 0);
    assert_eq!((ended(&mut newcomer), id), (Some(Ended::Crowded), None), "it is the one denied");
    assert!(held.iter_mut().all(|rx| rx.try_recv().is_err()));
    // An agent with requests of its own pushes out its own oldest.
    let (_again, id) = u.enqueue(key("again", "Agent0"), WAIT, 0);
    assert!(id.is_some());
    assert_eq!(ended(&mut held[0]), Some(Ended::Crowded));
    assert_eq!(u.list().len(), MAX_PENDING);
}

#[tokio::test]
async fn srv_decides_on_silence_with_no_window_answering() {
    let u = uses();
    let rx = u.ask(req("id"), Duration::from_millis(50));
    let got = tokio::time::timeout(Duration::from_secs(5), rx).await.expect("srv decided").unwrap();
    assert_eq!(Ended::of(Some(&got)), Ended::Expired);
    assert!(u.list().is_empty());
}

#[test]
fn each_ending_says_who_decided() {
    assert_eq!(Ended::WentAhead.outcome(), "done");
    assert_eq!(Ended::Expired.outcome(), "expired");
    assert_eq!(Ended::Denied.outcome(), "denied");
    assert!(Ended::Expired.refusal("nothing was typed").starts_with("an ID or card entry is used only on a yes"));
    assert!(Ended::Denied.refusal("nothing was typed").starts_with("the person said no"));
    assert_eq!(Ended::of(None), Ended::NoAnswer);
    assert!(Ended::WentAhead.allowed() && !Ended::Expired.allowed() && !Ended::Crowded.allowed());
}
