// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_queue::ClaimFilter;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

fn item(id: &str, created_at: i64) -> WorkItem {
    WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
        kind: String::new(),
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
        created_at,
        updated_at: created_at,
        not_before: None,
        result: String::new(),
    }
}

fn for_agent(id: &str, created_at: i64, agent: &str, uid: &str) -> WorkItem {
    WorkItem { target_agent: agent.into(), target_agent_uid: uid.into(), ..item(id, created_at) }
}

fn claim(s: &Store, agent: &str, uid: &str) -> Option<WorkItem> {
    let f = ClaimFilter { kind: None, agent_id: agent.into(), agent_uid: uid.into(), groups: vec![] };
    s.work_queue_claim(&f, 5000, 60_000).unwrap()
}

#[test]
fn cards_get_increasing_numbers_and_older_tasks_get_one_when_listed() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("a", 1000)).unwrap();
    let n = s.work_card_create("a", &NewCard { filed_by: "Loap-3".into(), ..Default::default() }, 1000).unwrap();
    // An older build's task: no card yet.
    s.work_queue_enqueue(&item("b", 1100)).unwrap();
    let all = s.work_board_list(0, 100, 2000).unwrap();
    assert_eq!(all.len(), 2);
    let b = all.iter().find(|t| t.item.id == "b").unwrap();
    assert!(b.card.num > n, "the older task is numbered after the first: {} vs {n}", b.card.num);
    assert_eq!(s.work_board_get(&format!("#{}", b.card.num)).unwrap().unwrap().item.id, "b");
    assert_eq!(s.work_board_get("a").unwrap().unwrap().card.filed_by, "Loap-3");
    assert_eq!(s.work_card_ensure("a", 3000).unwrap(), n, "ensure keeps the number");
}

#[test]
fn an_agent_takes_its_own_tasks_before_anyones_even_older_or_higher_priority_ones() {
    let (s, _d) = store();
    s.work_queue_enqueue(&WorkItem { priority: 5, ..item("anyone", 1000) }).unwrap();
    s.work_queue_enqueue(&for_agent("mine", 2000, "Loap-3", "uid-l")).unwrap();
    s.work_queue_enqueue(&for_agent("theirs", 500, "Naki", "uid-n")).unwrap();
    assert_eq!(claim(&s, "Loap-3", "uid-l").unwrap().id, "mine");
    assert_eq!(claim(&s, "Loap-3", "uid-l").unwrap().id, "anyone");
    assert!(claim(&s, "Loap-3", "uid-l").is_none(), "another agent's task is never taken");
}

#[test]
fn the_operators_order_in_the_backlog_decides_between_an_agents_own_tasks() {
    let (s, _d) = store();
    for (id, at) in [("first", 1000), ("second", 2000)] {
        s.work_queue_enqueue(&for_agent(id, at, "Loap-3", "uid-l")).unwrap();
        s.work_card_create(id, &NewCard::default(), at).unwrap();
    }
    // Dragged above "first".
    s.work_card_edit("second", Some(500.0), None, None, 3000).unwrap();
    let waiting = s.work_board_waiting_for("Loap-3", "uid-l", 10).unwrap();
    assert_eq!(waiting.iter().map(|t| t.item.id.as_str()).collect::<Vec<_>>(), ["second", "first"]);
    assert_eq!(claim(&s, "Loap-3", "uid-l").unwrap().id, "second");
}

#[test]
fn a_held_task_waits_until_released_and_nobody_claims_it_meanwhile() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("h", 1000)).unwrap();
    assert_eq!(s.work_hold("h", 2000).unwrap().unwrap().to, work_state::HELD);
    assert!(claim(&s, "Loap-3", "").is_none());
    assert!(s.work_hold("h", 2100).unwrap().is_none(), "already held");
    s.work_unhold("h", 2200).unwrap().unwrap();
    assert_eq!(claim(&s, "Loap-3", "").unwrap().id, "h");
}

#[test]
fn ask_answer_submit_and_accept_follow_the_holder_and_its_fence() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("t", 1000)).unwrap();
    let c = claim(&s, "Loap-3", "uid-l").unwrap();
    assert!(s.work_ask("t", "Naki", "uid-n", c.attempts, 6000).unwrap().is_none(), "not the holder");
    assert!(s.work_ask("t", "Loap-3", "uid-l", c.attempts + 1, 6000).unwrap().is_none(), "stale fence");
    assert!(s.work_ask("t", "Loap-3", "uid-l", c.attempts, 6000).unwrap().is_some());
    let t = s.work_queue_get("t").unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.claim_expires), (work_state::BLOCKED, None), "lease paused");
    // A blocked task is never reaped, however long the operator takes.
    s.work_queue_reap(i64::MAX / 2).unwrap();
    assert_eq!(s.work_queue_get("t").unwrap().unwrap().state, work_state::BLOCKED);

    s.work_answered("t", 7000, 60_000).unwrap().unwrap();
    let t = s.work_queue_get("t").unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.claim_expires), (work_state::CLAIMED, Some(67_000)));

    assert!(s.work_submit("t", "Loap-3", "uid-l", c.attempts, "PR #1", 8000).unwrap().is_some());
    let t = s.work_queue_get("t").unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.claimed_by.as_str(), t.result.as_str()), (work_state::REVIEW, "Loap-3", "PR #1"));
    assert!(s.work_accept("t", 9000).unwrap().is_some());
    assert_eq!(s.work_queue_get("t").unwrap().unwrap().state, work_state::DONE);
}

#[test]
fn a_send_back_returns_the_task_to_its_holder_under_the_same_attempt() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("t", 1000)).unwrap();
    let c = claim(&s, "Loap-3", "uid-l").unwrap();
    s.work_submit("t", "Loap-3", "uid-l", c.attempts, "done", 6000).unwrap().unwrap();
    s.work_send_back("t", 7000, 60_000).unwrap().unwrap();
    // The holder's attempt number still works.
    assert!(s.work_queue_complete("t", "Loap-3", "uid-l", c.attempts, "fixed", 8000).unwrap().is_some());
}

#[test]
fn a_send_back_with_the_holder_gone_waits_for_that_agent() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("t", 1000)).unwrap();
    let c = claim(&s, "Loap-3", "uid-l").unwrap();
    s.work_submit("t", "Loap-3", "uid-l", c.attempts, "done", 6000).unwrap().unwrap();
    s.work_send_back_waiting("t", 7000).unwrap().unwrap();
    let t = s.work_queue_get("t").unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.target_agent.as_str(), t.target_agent_uid.as_str()), (work_state::OPEN, "Loap-3", "uid-l"));
    assert_eq!(claim(&s, "Loap-3", "uid-l").unwrap().id, "t", "claimable again despite its attempts");
}

#[test]
fn starting_a_task_on_an_agent_claims_it_for_that_agent() {
    let (s, _d) = store();
    s.work_queue_enqueue(&for_agent("t", 1000, "Loap-3", "uid-l")).unwrap();
    s.work_hold("t", 1500).unwrap().unwrap();
    let started = s.work_start_for("t", "Naki", "uid-n", 2000, 60_000).unwrap().unwrap();
    assert_eq!((started.claimed_by.as_str(), started.target_agent.as_str()), ("Naki", "Naki"));
    assert_eq!(started.state, work_state::CLAIMED);
    assert!(s.work_start_for("t", "Loap-3", "uid-l", 2100, 60_000).unwrap().is_none(), "already started");
    assert!(s.work_queue_heartbeat("t", "Naki", "uid-n", started.attempts, 3000, 60_000).unwrap().is_some());
}

#[test]
fn assigning_moves_a_waiting_task_and_only_a_waiting_one() {
    let (s, _d) = store();
    s.work_queue_enqueue(&for_agent("t", 1000, "Loap-3", "uid-l")).unwrap();
    assert!(s.work_assign("t", "", "", 2000).unwrap(), "to anyone");
    assert_eq!(claim(&s, "Naki", "uid-n").unwrap().id, "t");
    assert!(!s.work_assign("t", "Loap-3", "uid-l", 3000).unwrap(), "claimed: not movable");
}

#[test]
fn cancel_reaches_the_new_states_and_retry_reopens_a_failed_task() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("h", 1000)).unwrap();
    s.work_hold("h", 1100).unwrap().unwrap();
    assert!(s.work_queue_cancel("h", "not needed", 1200).unwrap());

    s.work_queue_enqueue(&WorkItem { max_attempts: 1, ..item("f", 1000) }).unwrap();
    let c = claim(&s, "Loap-3", "").unwrap();
    s.work_queue_release("f", "Loap-3", "", c.attempts, "can't", 2000).unwrap().unwrap();
    assert_eq!(s.work_queue_get("f").unwrap().unwrap().state, work_state::FAILED);
    s.work_retry("f", 3000).unwrap().unwrap();
    assert_eq!(claim(&s, "Loap-3", "").unwrap().id, "f");
}

#[test]
fn a_live_holders_lease_is_extended_but_never_shortened_or_moved_to_a_newer_claim() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("t", 1000)).unwrap();
    let c = claim(&s, "Loap-3", "").unwrap(); // expires 65_000
    assert!(!s.work_extend_lease("t", c.attempts, 1000, 1000).unwrap(), "would shorten it");
    assert!(s.work_extend_lease("t", c.attempts, 100_000, 60_000).unwrap());
    assert_eq!(s.work_queue_get("t").unwrap().unwrap().claim_expires, Some(160_000));
    assert!(!s.work_extend_lease("t", c.attempts + 1, 100_000, 120_000).unwrap(), "another attempt's");
}

#[test]
fn card_updates_history_and_reminders_round_trip() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("t", 1000)).unwrap();
    s.work_card_create("t", &NewCard::default(), 1000).unwrap();
    let checklist = vec![ChecklistItem { text: "write it".into(), status: "in_progress".into() }];
    s.work_card_update(
        "t",
        &CardUpdate { note: Some("half way".into()), checklist: Some(checklist.clone()), ..Default::default() },
        2000,
    )
    .unwrap();
    s.work_card_update("t", &CardUpdate { pr_url: Some("https://x/pull/1".into()), ..Default::default() }, 2100).unwrap();
    let card = s.work_board_get("t").unwrap().unwrap().card;
    assert_eq!((card.note.as_str(), card.checklist, card.pr_url.as_str()), ("half way", checklist, "https://x/pull/1"));

    for (at, kind) in [(1000, "created"), (2000, "updated"), (3000, "released")] {
        s.work_event_add(&WorkEvent { work_id: "t".into(), at, actor: "Loap-3".into(), kind: kind.into(), ..Default::default() })
            .unwrap();
    }
    let kinds: Vec<String> = s.work_events("t", 2).unwrap().into_iter().map(|e| e.kind).collect();
    assert_eq!(kinds, ["updated", "released"], "the newest two, oldest first");
    assert!(s.work_recently_released_by("t", "Loap-3", 4000, 3600_000).unwrap());
    assert!(!s.work_recently_released_by("t", "Naki", 4000, 3600_000).unwrap());

    s.work_cards_mark_reminded(&["t".into()], 5000).unwrap();
    assert_eq!(s.work_board_get("t").unwrap().unwrap().card.last_reminded_at, Some(5000));
    assert!(s.work_board_stamp().unwrap() >= 2100);
}

#[test]
fn the_board_shows_unfinished_tasks_and_recently_finished_ones() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("old", 1000)).unwrap();
    s.work_queue_cancel("old", "", 1000).unwrap();
    s.work_queue_enqueue(&item("new", 1000)).unwrap();
    s.work_queue_cancel("new", "", 9000).unwrap();
    s.work_queue_enqueue(&item("open", 500)).unwrap();
    let ids: Vec<String> = s.work_board_list(5000, 100, 10_000).unwrap().into_iter().map(|t| t.item.id).collect();
    assert_eq!(ids, ["new", "open"]);
}

#[test]
fn history_of_long_finished_tasks_is_pruned() {
    let (s, _d) = store();
    s.work_queue_enqueue(&item("t", 1000)).unwrap();
    s.work_queue_cancel("t", "", 1000).unwrap();
    s.work_event_add(&WorkEvent { work_id: "t".into(), at: 1000, kind: "cancelled".into(), ..Default::default() }).unwrap();
    assert_eq!(s.work_events_prune(1000 + EVENTS_RETENTION_MS - 1).unwrap(), 0);
    assert_eq!(s.work_events_prune(1000 + EVENTS_RETENTION_MS + 1).unwrap(), 1);
}
