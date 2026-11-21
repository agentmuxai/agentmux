// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_card_prs::PrSource;
use crate::backend::storage::work_queue::WorkItem;

const DAY: i64 = 24 * 60 * 60 * 1000;
const NOW: i64 = 100 * DAY;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

fn agent(id: &str) -> AgentRef {
    AgentRef { id: id.into(), uid: format!("uid-{id}") }
}

/// A task `num` on `board` held (or last held) by `holder`, in `state`, last touched at `touched`.
fn task(s: &Store, id: &str, board: &str, num: i64, holder: &str, state: &str, touched: i64) {
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
    let conn = s.conn().lock().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO db_boards (id, slug, name) VALUES (?1, ?1, ?1)",
        params![board],
    )
    .unwrap();
    conn.execute("UPDATE db_work_cards SET board_id = ?2, board_num = ?3 WHERE work_id = ?1", params![id, board, num]).unwrap();
    conn.execute(
        "UPDATE db_work_queue SET state = ?2, claimed_by = ?3, claimed_by_uid = ?4, updated_at = ?5 WHERE id = ?1",
        params![id, state, holder, format!("uid-{holder}"), touched],
    )
    .unwrap();
}

fn set_repo(s: &Store, id: &str, repo: &str) {
    s.conn().lock().unwrap().execute("UPDATE db_work_cards SET repo = ?2 WHERE work_id = ?1", params![id, repo]).unwrap();
}

fn m(s: &Store, number: u64, r: Option<TaskRef>, who: Option<&AgentRef>) -> Matched {
    s.work_pr_match("o/r", number, r.as_ref(), who, NOW).unwrap()
}

#[test]
fn a_task_reference_is_a_number_or_a_board_and_a_number() {
    assert_eq!(TaskRef::parse("#12"), Some(TaskRef::Bare(12)));
    assert_eq!(TaskRef::parse(" 12 "), Some(TaskRef::Bare(12)));
    assert_eq!(TaskRef::parse("Work#7"), Some(TaskRef::Board { board: "work".into(), num: 7 }));
    for bad in ["", "#", "#0", "#-3", "#x", "a b#1", "12 13"] {
        assert_eq!(TaskRef::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn a_pull_request_the_card_already_has_is_that_card_whatever_it_carries() {
    let (s, _d) = store();
    task(&s, "a", "main", 1, "pell", "claimed", NOW);
    s.work_card_pr_link("a", "o/r", 5, PrSource::Holder, 1).unwrap();
    assert_eq!(m(&s, 5, None, None), Matched::Card("a".into()));
    // Taken off the card: nothing links it again by this rule.
    s.work_card_pr_unlink("a", "o/r", 5, 2).unwrap();
    assert_eq!(m(&s, 5, None, None), Matched::Unmatched(Why::NoReference));
}

#[test]
fn a_bare_number_names_the_one_task_of_that_agent_with_that_number() {
    let (s, _d) = store();
    task(&s, "a", "main", 12, "pell", "claimed", NOW);
    task(&s, "other", "main", 13, "pell", "claimed", NOW);
    task(&s, "not-hers", "work", 12, "naki-2", "claimed", NOW);
    let pell = agent("pell");
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), Some(&pell)), Matched::Card("a".into()));
    assert_eq!(m(&s, 1, TaskRef::parse("12"), Some(&pell)), Matched::Card("a".into()));
    assert_eq!(m(&s, 1, TaskRef::parse("#99"), Some(&pell)), Matched::Unmatched(Why::NoTask));
    let naki = agent("naki-2");
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), Some(&naki)), Matched::Card("not-hers".into()), "the same number on another board is another agent's");
}

#[test]
fn a_number_held_on_two_boards_is_ambiguous_until_the_reference_names_the_board() {
    let (s, _d) = store();
    task(&s, "main12", "main", 12, "pell", "claimed", NOW);
    task(&s, "work12", "work", 12, "pell", "review", NOW);
    let pell = agent("pell");
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), Some(&pell)), Matched::Unmatched(Why::Ambiguous), "never a guess");
    assert_eq!(m(&s, 1, TaskRef::parse("work#12"), Some(&pell)), Matched::Card("work12".into()));
    assert_eq!(m(&s, 1, TaskRef::parse("main#12"), Some(&pell)), Matched::Card("main12".into()));
    assert_eq!(m(&s, 1, TaskRef::parse("nowhere#12"), Some(&pell)), Matched::Unmatched(Why::NoTask));
}

#[test]
fn a_task_counts_as_the_agents_while_held_and_for_fourteen_days_after() {
    let (s, _d) = store();
    task(&s, "held-long", "main", 1, "pell", "claimed", NOW - 90 * DAY);
    task(&s, "done-13d", "main", 2, "pell", "done", NOW - 13 * DAY);
    task(&s, "done-15d", "main", 3, "pell", "done", NOW - 15 * DAY);
    let pell = agent("pell");
    assert_eq!(m(&s, 1, TaskRef::parse("#1"), Some(&pell)), Matched::Card("held-long".into()), "still held, however long ago");
    assert_eq!(m(&s, 1, TaskRef::parse("#2"), Some(&pell)), Matched::Card("done-13d".into()));
    assert_eq!(m(&s, 1, TaskRef::parse("#3"), Some(&pell)), Matched::Unmatched(Why::NoTask));
}

#[test]
fn without_a_reference_or_an_agent_of_this_computer_it_is_unmatched() {
    let (s, _d) = store();
    task(&s, "a", "main", 12, "pell", "claimed", NOW);
    let pell = agent("pell");
    assert_eq!(m(&s, 1, None, Some(&pell)), Matched::Unmatched(Why::NoReference));
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), None), Matched::Unmatched(Why::NoAgent));
}

#[test]
fn a_card_that_names_another_repository_is_not_matched_to_this_ones_pull_request() {
    let (s, _d) = store();
    task(&s, "a", "main", 12, "pell", "claimed", NOW);
    let pell = agent("pell");
    set_repo(&s, "a", "other/repo");
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), Some(&pell)), Matched::Unmatched(Why::NoTask));
    set_repo(&s, "a", "O/R");
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), Some(&pell)), Matched::Card("a".into()), "the same repository, in any case");
    set_repo(&s, "a", "local:/tmp/x");
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), Some(&pell)), Matched::Card("a".into()), "a local folder names no repository");
    set_repo(&s, "a", "");
    assert_eq!(m(&s, 1, TaskRef::parse("#12"), Some(&pell)), Matched::Card("a".into()));
}

#[test]
fn a_card_that_took_the_pull_request_off_is_not_a_candidate_again() {
    let (s, _d) = store();
    task(&s, "a", "main", 12, "pell", "claimed", NOW);
    let pell = agent("pell");
    s.work_card_pr_link("a", "o/r", 7, PrSource::Holder, 1).unwrap();
    s.work_card_pr_unlink("a", "o/r", 7, 2).unwrap();
    assert_eq!(m(&s, 7, TaskRef::parse("#12"), Some(&pell)), Matched::Unmatched(Why::NoTask));
    assert_eq!(m(&s, 8, TaskRef::parse("#12"), Some(&pell)), Matched::Card("a".into()), "another pull request is not affected");
}

#[test]
fn the_unmatched_list_keeps_the_newest_report_the_last_fifty_and_forgets_closed_ones_after_a_week() {
    let (s, _d) = store();
    let note = |n: u64, state: &str, updated: i64, at: i64| {
        s.work_pr_unmatched_note("o/r", n, state, "pell", "#12", Why::Ambiguous, "sha", updated, at).unwrap()
    };
    note(1, "open", 10, NOW);
    note(1, "merged", 5, NOW + 1); // older than what is held: ignored
    assert_eq!(s.work_pr_unmatched_list().unwrap()[0].state, "open");
    note(1, "merged", 20, NOW + 2);
    let row = &s.work_pr_unmatched_list().unwrap()[0];
    assert_eq!((row.state.as_str(), row.url.as_str(), row.why.as_str()), ("merged", "https://github.com/o/r/pull/1", "more than one task of that agent has that number"));

    // A closed one goes after a week; an open one stays.
    note(2, "open", 1, NOW + 3);
    note(3, "open", 1, NOW + 8 * DAY);
    let numbers: Vec<u64> = s.work_pr_unmatched_list().unwrap().into_iter().map(|u| u.number).collect();
    assert_eq!(numbers, [3, 2], "the merged one was forgotten after a week");

    for n in 100..160 {
        note(n, "open", 1, NOW + 9 * DAY + n as i64);
    }
    assert_eq!(s.work_pr_unmatched_list().unwrap().len(), UNMATCHED_KEPT as usize);
    assert_eq!(s.work_pr_unmatched_list().unwrap()[0].number, 159, "newest first");

    assert!(s.work_pr_unmatched_remove("o/r", 159).unwrap());
    assert!(!s.work_pr_unmatched_remove("o/r", 159).unwrap());
}
