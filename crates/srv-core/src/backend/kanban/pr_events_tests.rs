// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_queue::WorkItem;
use rusqlite::params;
use serde_json::json;

const NOW: i64 = 1_000_000_000;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

/// A task held by `holder`, numbered `num` on the default board.
fn held(s: &Store, id: &str, holder: &str) {
    s.work_queue_enqueue(&WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
        kind: "build".into(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: String::new(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: "tester".into(),
        created_by_uid: String::new(),
        created_at: 1,
        updated_at: 1,
        not_before: None,
        result: String::new(),
    })
    .unwrap();
    s.work_card_ensure(id, 1).unwrap();
    s.conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE db_work_queue SET state = 'claimed', claimed_by = ?2, claimed_by_uid = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, holder, format!("uid-{holder}"), NOW],
        )
        .unwrap();
}

fn num_of(s: &Store, id: &str) -> i64 {
    s.work_board_get(id).unwrap().unwrap().card.num
}

fn agents(id: &str) -> Option<AgentRef> {
    (id == "pell" || id == "naki-2").then(|| AgentRef { id: id.into(), uid: format!("uid-{id}") })
}

fn event(number: u64, action: PrAction, task_ref: &str, agent: &str, by_agentmux: bool, delivery: &str, at: i64) -> PrEvent {
    PrEvent {
        repo: "agentmuxai/agentmux".into(),
        number,
        action,
        head: Some("abc".into()),
        updated_at: json!(at),
        delivery: delivery.into(),
        task_ref: Some(task_ref.into()),
        agent: Some(agent.into()),
        by_agentmux,
    }
}

fn owners() -> Vec<String> {
    vec!["agentmuxai".into(), "a5af".into()]
}

fn trust(o: &[String]) -> Trust<'_> {
    Trust { from_integration: true, trusted_owners: o }
}

#[test]
fn a_signed_text_is_an_event_only_when_it_says_so_itself() {
    let text = json!({ "kind": "pr_event", "repo": "o/r", "number": 4, "action": "merged", "updated_at": "2026-11-21T10:00:00Z", "agent": "pell" });
    let ev = parse_pr_event(&text.to_string()).unwrap();
    assert_eq!((ev.number, ev.action, ev.agent.as_deref()), (4, PrAction::Merged, Some("pell")));
    assert_eq!(time_ms(&ev.updated_at), chrono::DateTime::parse_from_rfc3339("2026-11-21T10:00:00Z").ok().map(|d| d.timestamp_millis()));
    assert_eq!(time_ms(&json!(1234)), Some(1234), "or epoch milliseconds");
    assert_eq!(time_ms(&json!("soon")), None);
    // The same fields with another kind, or none, or not JSON, are not an event.
    let mut other = text.clone();
    other["kind"] = json!("message");
    assert_eq!(parse_pr_event(&other.to_string()), None);
    other.as_object_mut().unwrap().remove("kind");
    assert_eq!(parse_pr_event(&other.to_string()), None);
    assert_eq!(parse_pr_event("not json"), None);
    assert_eq!(parse_pr_event(r#"{"kind":"pr_event","repo":"o/r","action":"exploded"}"#), None);
}

#[test]
fn the_other_field_names_are_read_as_ours() {
    // `id`, `by_app` and `task` (a board and a number) mean `delivery`, `by_agentmux` and `task_ref`; fields we do not use are ignored.
    let text = json!({
        "kind": "pr_event", "id": "dlv-9", "repo": "o/r", "number": 12, "action": "opened", "state": "open",
        "head": "abc1234", "updated_at": "2026-11-21T10:00:00Z", "by_app": true, "task": { "board": "work", "n": 7 }, "agent": "pell"
    });
    let ev = parse_pr_event(&text.to_string()).unwrap();
    assert_eq!((ev.delivery.as_str(), ev.by_agentmux, ev.agent.as_deref()), ("dlv-9", true, Some("pell")));
    assert_eq!(ev.task_ref.as_deref(), Some("work#7"));
    assert_eq!(TaskRef::parse("work#7"), Some(TaskRef::Board { board: "work".into(), num: 7 }));
    let read = |extra: serde_json::Value| {
        let mut e = json!({ "kind": "pr_event", "repo": "o/r", "number": 1, "action": "opened" });
        e.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        parse_pr_event(&e.to_string())
    };
    // A number with no board is a bare one, whether the board is missing, null or empty; no task is none.
    assert_eq!(read(json!({ "task": { "n": 3 } })).unwrap().task_ref.as_deref(), Some("3"));
    assert_eq!(read(json!({ "task": { "board": null, "n": 3 } })).unwrap().task_ref.as_deref(), Some("3"));
    assert_eq!(read(json!({ "task": { "board": "", "n": 3 } })).unwrap().task_ref.as_deref(), Some("3"));
    assert_eq!(read(json!({})).unwrap().task_ref, None);
    assert_eq!(read(json!({ "task": null })).unwrap().task_ref, None);
    assert_eq!(read(json!({ "task": { "board": "work", "n": null } })).unwrap().task_ref, None, "a null number is no task, and not a dropped event");
    // Ours wins when both names are there, and an event that carries both is still read.
    assert_eq!(read(json!({ "task_ref": "#5", "task": { "n": 3 } })).unwrap().task_ref.as_deref(), Some("#5"));
    let both = read(json!({ "delivery": "ours", "id": "theirs", "by_agentmux": false, "by_app": true })).unwrap();
    assert_eq!((both.delivery.as_str(), both.by_agentmux), ("ours", false));
}

#[test]
fn an_event_from_anyone_but_the_integration_is_dropped_and_counted() {
    let (s, _d) = store();
    held(&s, "a", "pell");
    let intake = Intake::default();
    let o = owners();
    let n = num_of(&s, "a");
    let ev = event(1, PrAction::Opened, &format!("#{n}"), "pell", true, "d1", 10);
    let not_integration = Trust { from_integration: false, trusted_owners: &o };
    assert_eq!(intake.apply(&s, &ev, &not_integration, &agents, NOW), Outcome::Dropped("not from the integration"));
    assert!(s.work_card_prs("a").unwrap().is_empty());
    assert_eq!(intake.dropped()["not from the integration"], 1);
    // The integration's own is applied: both directions of the sender rule.
    assert!(matches!(intake.apply(&s, &ev, &trust(&o), &agents, NOW), Outcome::Applied { .. }));
}

#[test]
fn an_untrusted_owner_and_a_replayed_delivery_are_dropped() {
    let (s, _d) = store();
    held(&s, "a", "pell");
    let intake = Intake::default();
    let o = owners();
    let n = format!("#{}", num_of(&s, "a"));
    let mut outsider = event(1, PrAction::Opened, &n, "pell", true, "d1", 10);
    outsider.repo = "evil/agentmux".into();
    assert_eq!(intake.apply(&s, &outsider, &trust(&o), &agents, NOW), Outcome::Dropped("untrusted owner"));

    let ev = event(1, PrAction::Opened, &n, "pell", true, "d2", 10);
    assert!(matches!(intake.apply(&s, &ev, &trust(&o), &agents, NOW), Outcome::Applied { changed: true, .. }));
    assert_eq!(intake.apply(&s, &ev, &trust(&o), &agents, NOW + 1), Outcome::Dropped("replayed delivery"));
    // After the hour the id is forgotten; the report is still not news, so it changes nothing.
    assert!(matches!(intake.apply(&s, &ev, &trust(&o), &agents, NOW + SEEN_FOR_MS + 1), Outcome::Applied { changed: false, .. }));
}

#[test]
fn an_event_for_our_own_pull_request_reaches_its_card_and_a_merge_is_news_once() {
    let (s, _d) = store();
    held(&s, "a", "pell");
    let intake = Intake::default();
    let o = owners();
    let n = format!("#{}", num_of(&s, "a"));
    assert!(matches!(intake.apply(&s, &event(7, PrAction::Opened, &n, "pell", true, "d1", 10), &trust(&o), &agents, NOW), Outcome::Applied { changed: true, newly_merged: false, .. }));
    let row = &s.work_card_prs("a").unwrap()[0];
    assert_eq!((row.repo.as_str(), row.number, row.state.as_str(), row.source.as_str()), ("agentmuxai/agentmux", 7, "open", "event"));
    assert!(!row.may_close(), "a tag alone never closes a task");

    let merged = event(7, PrAction::Merged, &n, "pell", true, "d2", 20);
    assert_eq!(
        intake.apply(&s, &merged, &trust(&o), &agents, NOW),
        Outcome::Applied { work_id: "a".into(), changed: true, newly_merged: true }
    );
    // A late "opened" and the merge again change nothing.
    assert!(matches!(intake.apply(&s, &event(7, PrAction::Opened, &n, "pell", true, "d3", 15), &trust(&o), &agents, NOW), Outcome::Applied { changed: false, .. }));
    assert!(matches!(intake.apply(&s, &event(7, PrAction::Merged, &n, "pell", true, "d4", 20), &trust(&o), &agents, NOW), Outcome::Applied { changed: false, newly_merged: false, .. }));
    assert_eq!(s.work_card_prs("a").unwrap()[0].state, "merged");
}

#[test]
fn a_pull_request_from_outside_that_names_our_task_number_is_shown_as_unmatched_and_never_linked() {
    let (s, _d) = store();
    held(&s, "a", "pell");
    let intake = Intake::default();
    let o = owners();
    let n = format!("#{}", num_of(&s, "a"));
    let outsider = event(9, PrAction::Opened, &n, "pell", false, "d1", 10);
    assert_eq!(intake.apply(&s, &outsider, &trust(&o), &agents, NOW), Outcome::Unmatched(Why::Outsider));
    assert!(s.work_card_prs("a").unwrap().is_empty(), "never linked");
    let list = s.work_pr_unmatched_list().unwrap();
    assert_eq!((list.len(), list[0].number, list[0].agent_id.as_str()), (1, 9, "pell"));

    // Once a person put it on the card, its events reach it whoever opened it.
    s.work_card_pr_link("a", "agentmuxai/agentmux", 9, PrSource::Person, 1).unwrap();
    assert!(matches!(intake.apply(&s, &event(9, PrAction::Merged, &n, "pell", false, "d2", 20), &trust(&o), &agents, NOW), Outcome::Applied { newly_merged: true, .. }));
}

#[test]
fn an_ambiguous_or_unknown_match_never_guesses() {
    let (s, _d) = store();
    held(&s, "a", "pell");
    let intake = Intake::default();
    let o = owners();
    // A number the agent does not hold.
    let ev = event(3, PrAction::Opened, "#9999", "pell", true, "d1", 10);
    assert_eq!(intake.apply(&s, &ev, &trust(&o), &agents, NOW), Outcome::Unmatched(Why::NoTask));
    // No reference at all.
    let mut bare = event(4, PrAction::Opened, "#1", "pell", true, "d2", 10);
    bare.task_ref = None;
    assert_eq!(intake.apply(&s, &bare, &trust(&o), &agents, NOW), Outcome::Unmatched(Why::NoReference));
    assert!(s.work_card_prs("a").unwrap().is_empty());
    assert_eq!(s.work_pr_unmatched_list().unwrap().len(), 2);
}

#[test]
fn an_agent_this_computer_does_not_have_is_dropped_not_listed() {
    let (s, _d) = store();
    held(&s, "a", "pell");
    let intake = Intake::default();
    let o = owners();
    let n = format!("#{}", num_of(&s, "a"));
    let elsewhere = event(5, PrAction::Opened, &n, "someone-on-another-computer", true, "d1", 10);
    assert_eq!(intake.apply(&s, &elsewhere, &trust(&o), &agents, NOW), Outcome::Dropped("no card of this computer"));
    let mut tagless = event(6, PrAction::Opened, &n, "x", true, "d2", 10);
    tagless.agent = None;
    assert_eq!(intake.apply(&s, &tagless, &trust(&o), &agents, NOW), Outcome::Dropped("no card of this computer"));
    assert!(s.work_pr_unmatched_list().unwrap().is_empty() && s.work_card_prs("a").unwrap().is_empty());
    assert_eq!(intake.dropped()["no card of this computer"], 2);
}

#[test]
fn review_checks_and_release_events_are_read_and_set_aside_for_now() {
    let (s, _d) = store();
    held(&s, "a", "pell");
    let intake = Intake::default();
    let o = owners();
    let n = format!("#{}", num_of(&s, "a"));
    for action in [PrAction::Review, PrAction::Checks, PrAction::Release] {
        let ev = event(1, action, &n, "pell", true, &format!("{action:?}"), 10);
        assert_eq!(intake.apply(&s, &ev, &trust(&o), &agents, NOW), Outcome::Ignored("not shown on the card yet"));
    }
    assert!(s.work_card_prs("a").unwrap().is_empty());
}
