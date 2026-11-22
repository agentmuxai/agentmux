// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::pr_flow::PrFlowSnapshot;
use crate::backend::reviews::CardIn;

fn card(num: i64, agent: &str, repo: &str, number: u64) -> CardIn {
    CardIn {
        task: ReviewsTask { task_ref: format!("#{num}"), num, title: String::new(), state: "claimed".into() },
        agent: agent.into(),
        repo: repo.into(),
        number,
        branch: String::new(),
        merged_at_ms: None,
        released_in: String::new(),
        updated_at_ms: num,
    }
}

fn on_board(mut c: CardIn, task_ref: &str) -> CardIn {
    c.task.task_ref = task_ref.into();
    c
}

fn list() -> ReviewsList {
    reviews::build(
        &[
            card(1, "lazo", "agentmuxai/agentmux", 10),
            card(2, "camper", "agentmuxai/agentmux", 11),
            card(3, "lazo", "agentmuxai/muxcode", 12),
            on_board(card(2, "lazo", "agentmuxai/other", 13), "work#2"),
        ],
        &PrFlowSnapshot::default(),
        100,
    )
}

fn numbers(l: &ReviewsList) -> Vec<u64> {
    let mut n: Vec<u64> = l.rows.iter().map(|r| r.number).collect();
    n.sort();
    n
}

fn q(agent: Option<&str>, waits_on: Option<&str>, task: Option<&str>, repo: Option<&str>) -> ReviewsQuery {
    let own = |s: Option<&str>| s.map(str::to_string);
    ReviewsQuery { agent: own(agent), waits_on: own(waits_on), task: own(task), repo: own(repo) }
}

#[test]
fn no_query_keeps_every_row() {
    assert_eq!(numbers(&narrow(list(), &q(None, None, None, None))), vec![10, 11, 12, 13]);
}

#[test]
fn the_agent_filter_is_the_holder_and_ignores_case() {
    assert_eq!(numbers(&narrow(list(), &q(Some("Lazo"), None, None, None))), vec![10, 12, 13]);
}

#[test]
fn the_repo_and_task_filters_narrow_together() {
    assert_eq!(numbers(&narrow(list(), &q(None, None, None, Some("AgentMuxAI/agentmux")))), vec![10, 11]);
    for t in ["2", "#2"] {
        assert_eq!(numbers(&narrow(list(), &q(None, None, Some(t), None))), vec![11], "task {t}");
    }
    assert_eq!(numbers(&narrow(list(), &q(Some("lazo"), None, None, Some("agentmuxai/agentmux")))), vec![10]);
}

#[test]
fn a_task_number_names_the_default_boards_task_and_another_board_has_its_own_ref() {
    assert_eq!(numbers(&narrow(list(), &q(None, None, Some("2"), None))), vec![11], "a bare number is the default board's");
    assert_eq!(numbers(&narrow(list(), &q(None, None, Some("#2"), None))), vec![11]);
    assert_eq!(numbers(&narrow(list(), &q(None, None, Some("work#2"), None))), vec![13]);
    assert_eq!(numbers(&narrow(list(), &q(None, None, Some("WORK#2"), None))), vec![13]);
}

#[test]
fn the_waits_on_filter_is_the_wire_word() {
    assert_eq!(numbers(&narrow(list(), &q(None, Some("reviewer"), None, None))), vec![10, 11, 12, 13]);
    assert!(narrow(list(), &q(None, Some("person"), None, None)).rows.is_empty());
    assert!(narrow(list(), &q(None, Some("nonsense"), None, None)).rows.is_empty());
}

#[test]
fn a_blank_filter_is_no_filter() {
    assert_eq!(numbers(&narrow(list(), &q(Some(" "), Some(""), Some(" "), Some("")))), vec![10, 11, 12, 13]);
}

fn row(number: u64, repo_id: Option<u64>, agent: &str, task: bool, at: i64) -> ReviewRow {
    ReviewRow {
        repo: task.then(|| "o/r".to_string()),
        repo_id,
        number,
        url: None,
        task: task.then(|| ReviewsTask { task_ref: format!("#{number}"), num: number as i64, title: String::new(), state: "claimed".into() }),
        agent: Some(agent.into()),
        branch: String::new(),
        reviews: vec![],
        checks: None,
        merge: "open".into(),
        merged_at_ms: None,
        released_in: String::new(),
        waits_on: WaitsOn::Reviewer,
        why: "no_review".into(),
        updated_at_ms: at,
    }
}

fn orphans(rows: Vec<ReviewRow>) -> ReviewsList {
    ReviewsList { rows, as_of_ms: 9, last_notice_at_ms: Some(8) }
}

#[test]
fn a_notice_a_card_settled_is_not_a_row_of_its_own() {
    let cards = vec![row(7, Some(42), "pell", true, 5)];
    let joined = join(cards, orphans(vec![row(7, Some(42), "pell", false, 6), row(8, Some(43), "pell", false, 4)]), None);
    assert_eq!(numbers(&joined), [7, 8], "the card's row, and the notice no card holds");
    assert!(joined.rows.iter().find(|r| r.number == 7).unwrap().task.is_some());
    assert_eq!((joined.as_of_ms, joined.last_notice_at_ms), (9, Some(8)));
}

#[test]
fn a_caller_gets_a_notice_only_row_only_when_it_went_to_it() {
    let notices = || orphans(vec![row(8, Some(43), "pell", false, 4), row(9, Some(44), "lazo", false, 3)]);
    assert_eq!(numbers(&join(vec![], notices(), Some("PELL"))), [8]);
    assert_eq!(numbers(&join(vec![], notices(), None)), [8, 9], "the app's own windows see all");
}

#[test]
fn newest_change_first() {
    let joined = join(vec![row(1, None, "a", true, 10), row(2, None, "a", true, 30)], orphans(vec![row(3, Some(1), "a", false, 20)]), None);
    assert_eq!(joined.rows.iter().map(|r| r.number).collect::<Vec<_>>(), [2, 3, 1]);
}
