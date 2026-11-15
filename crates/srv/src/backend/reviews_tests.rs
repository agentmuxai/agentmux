// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::pr_flow::{PrFlowBot, PrRef};

fn card(num: i64, state: &str, agent: &str, repo: &str, number: u64) -> CardIn {
    CardIn {
        task: ReviewsTask { task_ref: format!("#{num}"), num, title: format!("task {num}"), state: state.into() },
        agent: agent.into(),
        repo: repo.into(),
        number,
        branch: format!("b/{number}"),
        merged_at_ms: None,
        released_in: String::new(),
        updated_at_ms: 1_000,
    }
}

fn pr(repo_id: u64, number: u64) -> PrRef {
    PrRef { repo_id, number }
}

fn review(id: &str, agent: &str, p: PrRef, state: &str, by: PrFlowBy, head: Option<&str>, at_ms: u64) -> ReviewFlowEvent {
    ReviewFlowEvent { id: id.into(), at_ms, agent: agent.into(), pr: p, state: state.into(), by, head: head.map(str::to_string) }
}

fn checks(id: &str, agent: &str, p: PrRef, state: &str, head: Option<&str>, at_ms: u64) -> ChecksFlowEvent {
    ChecksFlowEvent { id: id.into(), at_ms, agent: agent.into(), pr: p, state: state.into(), head: head.map(str::to_string) }
}

fn merged(id: &str, agent: &str, p: PrRef, at_ms: u64) -> MergeFlowEvent {
    MergeFlowEvent { id: id.into(), at_ms, agent: agent.into(), pr: p, state: "merged".into() }
}

fn flow(review: Vec<ReviewFlowEvent>, checks: Vec<ChecksFlowEvent>, merge: Vec<MergeFlowEvent>) -> PrFlowSnapshot {
    PrFlowSnapshot { review, checks, merge, bot: PrFlowBot { last_notice_at_ms: Some(9_000) } }
}

fn waits(merge: &str, checks: Option<&str>, reviews: &[(&str, bool)], task: Option<&str>) -> (WaitsOn, &'static str) {
    waits_on(&WaitsInput { merge, checks, reviews, task_state: task })
}

// ---- the rule: every combination of section 4.1 and the stale case ----

#[test]
fn a_merged_or_closed_pr_waits_on_nothing() {
    assert_eq!(waits("merged", Some("failing"), &[("changes_requested", false)], None), (WaitsOn::None, "merged"));
    assert_eq!(waits("closed", None, &[], None), (WaitsOn::None, "closed"));
}

#[test]
fn a_finished_task_with_a_pr_nobody_saw_end_waits_on_no_one() {
    for state in ["done", "cancelled", "failed"] {
        assert_eq!(waits("open", Some("failing"), &[("changes_requested", false)], Some(state)), (WaitsOn::None, "task_finished"), "{state}");
    }
    assert_eq!(waits("open", None, &[], Some("held")), (WaitsOn::Reviewer, "no_review"), "a held task is not over");
}

#[test]
fn a_queued_pr_waits_on_the_queue() {
    assert_eq!(waits("queued", Some("passing"), &[("approved", false)], Some("review")), (WaitsOn::Queue, "queued"));
}

#[test]
fn failing_checks_wait_on_the_author_even_when_approved() {
    assert_eq!(waits("open", Some("failing"), &[("approved", false)], None), (WaitsOn::Author, "checks_failing"));
}

#[test]
fn changes_requested_on_the_current_head_waits_on_the_author() {
    assert_eq!(waits("open", Some("passing"), &[("changes_requested", false)], None), (WaitsOn::Author, "changes_requested"));
}

#[test]
fn changes_requested_on_an_older_head_waits_on_a_reviewer() {
    assert_eq!(waits("open", Some("passing"), &[("changes_requested", true)], None), (WaitsOn::Reviewer, "review_stale"));
}

#[test]
fn running_checks_wait_on_ci_before_a_reviewer() {
    assert_eq!(waits("open", Some("pending"), &[], None), (WaitsOn::Ci, "checks_running"));
    assert_eq!(waits("open", Some("pending"), &[("approved", false)], None), (WaitsOn::Ci, "checks_running"));
}

#[test]
fn no_review_waits_on_a_reviewer() {
    assert_eq!(waits("open", None, &[], None), (WaitsOn::Reviewer, "no_review"));
    assert_eq!(waits("open", Some("passing"), &[("commented", false)], None), (WaitsOn::Reviewer, "no_review"));
}

#[test]
fn an_approval_waits_on_the_author_unless_the_task_awaits_an_accept() {
    assert_eq!(waits("open", Some("passing"), &[("approved", false)], Some("claimed")), (WaitsOn::Author, "ready_to_merge"));
    assert_eq!(waits("open", None, &[("approved", false)], None), (WaitsOn::Author, "ready_to_merge"));
    assert_eq!(waits("open", Some("passing"), &[("approved", false)], Some("review")), (WaitsOn::Person, "awaiting_accept"));
}

#[test]
fn a_stale_approval_is_not_an_approval() {
    assert_eq!(waits("open", Some("passing"), &[("approved", true)], Some("review")), (WaitsOn::Reviewer, "review_stale"));
}

#[test]
fn one_fresh_verdict_among_stale_ones_decides() {
    let reviews = [("changes_requested", true), ("approved", false)];
    assert_eq!(waits("open", Some("passing"), &reviews, None), (WaitsOn::Author, "ready_to_merge"));
}

// ---- the join ----

#[test]
fn a_card_takes_the_events_of_its_agent_and_number() {
    let cards = [card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)];
    let f = flow(
        vec![review("r1", "lazo", pr(55, 100), "approved", PrFlowBy::Bot, Some("aaaaaaaaaaaa"), 2_000)],
        vec![checks("c1", "lazo", pr(55, 100), "passing", Some("aaaaaaaaaaaa"), 2_100)],
        vec![],
    );
    let list = build(&cards, &f, 5_000);
    assert_eq!(list.rows.len(), 1);
    let row = &list.rows[0];
    assert_eq!(row.repo.as_deref(), Some("agentmuxai/agentmux"));
    assert_eq!(row.repo_id, Some(55));
    assert_eq!(row.url.as_deref(), Some("https://github.com/agentmuxai/agentmux/pull/100"));
    assert_eq!(row.task.as_ref().map(|t| t.num), Some(7));
    assert_eq!((row.waits_on, row.why.as_str()), (WaitsOn::Author, "ready_to_merge"));
    assert_eq!(row.updated_at_ms, 2_100);
    assert_eq!((list.as_of_ms, list.last_notice_at_ms), (5_000, Some(9_000)));
}

#[test]
fn another_agents_event_with_the_same_number_is_not_the_cards() {
    let cards = [card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)];
    let f = flow(vec![review("r1", "camper", pr(55, 100), "approved", PrFlowBy::Bot, None, 2_000)], vec![], vec![]);
    let list = build(&cards, &f, 5_000);
    assert_eq!(list.rows.len(), 2, "the card, and the other agent's PR as its own row");
    let card_row = list.rows.iter().find(|r| r.task.is_some()).unwrap();
    assert!(card_row.reviews.is_empty());
    let orphan = list.rows.iter().find(|r| r.task.is_none()).unwrap();
    assert_eq!((orphan.repo_id, orphan.agent.as_deref(), orphan.url.as_deref()), (Some(55), Some("camper"), None));
}

#[test]
fn two_cards_with_one_number_in_two_repositories_are_told_apart_by_a_learned_repository_id() {
    // lazo holds PR 100 of two repositories. Repository 55 is learned from camper's single card.
    let cards = [
        card(1, "claimed", "lazo", "agentmuxai/other", 100),
        card(2, "claimed", "lazo", "agentmuxai/agentmux", 100),
        card(3, "claimed", "camper", "agentmuxai/agentmux", 7),
    ];
    let f = flow(
        vec![
            review("r0", "camper", pr(55, 7), "approved", PrFlowBy::Bot, None, 1_000),
            review("r1", "lazo", pr(55, 100), "changes_requested", PrFlowBy::Bot, None, 2_000),
            review("r2", "lazo", pr(66, 100), "approved", PrFlowBy::Bot, None, 3_000),
        ],
        vec![],
        vec![],
    );
    let list = build(&cards, &f, 5_000);
    let of = |n: i64| list.rows.iter().find(|r| r.task.as_ref().map(|t| t.num) == Some(n)).unwrap();
    assert_eq!(of(2).reviews[0].state, "changes_requested", "repository 55 is agentmux");
    // Repository 66 is not learned from any single match: its event cannot be placed, so it stays a row of its own.
    assert!(of(1).reviews.is_empty());
    assert!(list.rows.iter().any(|r| r.task.is_none() && r.repo_id == Some(66)));
}

#[test]
fn an_event_no_card_holds_is_a_row_with_no_task_and_no_address() {
    let f = flow(vec![], vec![checks("c1", "posa", pr(9, 64), "failing", None, 2_000)], vec![]);
    let list = build(&[], &f, 5_000);
    assert_eq!(list.rows.len(), 1);
    let r = &list.rows[0];
    assert_eq!((r.number, r.repo_id, r.agent.as_deref(), r.task.is_none(), r.url.is_none()), (64, Some(9), Some("posa"), true, true));
    assert_eq!((r.waits_on, r.why.as_str()), (WaitsOn::Author, "checks_failing"));
}

#[test]
fn a_card_with_no_event_is_still_a_row() {
    let list = build(&[card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)], &PrFlowSnapshot::default(), 5_000);
    assert_eq!(list.rows.len(), 1);
    assert_eq!((list.rows[0].waits_on, list.rows[0].why.as_str()), (WaitsOn::Reviewer, "no_review"));
    assert_eq!(list.rows[0].merge, "open");
    assert_eq!(list.last_notice_at_ms, None);
}

#[test]
fn a_card_that_srv_saw_merged_is_merged_without_an_event() {
    let mut c = card(7, "done", "lazo", "agentmuxai/agentmux", 100);
    c.merged_at_ms = Some(4_000);
    c.released_in = "v0.60.1".into();
    let list = build(&[c], &PrFlowSnapshot::default(), 5_000);
    let r = &list.rows[0];
    assert_eq!((r.merge.as_str(), r.waits_on, r.merged_at_ms, r.released_in.as_str()), ("merged", WaitsOn::None, Some(4_000), "v0.60.1"));
}

#[test]
fn a_merge_event_ends_the_pr() {
    let cards = [card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)];
    let f = flow(vec![], vec![checks("c1", "lazo", pr(55, 100), "failing", None, 2_000)], vec![merged("m1", "lazo", pr(55, 100), 3_000)]);
    let r = &build(&cards, &f, 5_000).rows[0];
    assert_eq!((r.merge.as_str(), r.waits_on), ("merged", WaitsOn::None));
}

// ---- stale ----

#[test]
fn a_verdict_is_stale_when_later_checks_name_another_head() {
    let cards = [card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)];
    let f = flow(
        vec![review("r1", "lazo", pr(55, 100), "approved", PrFlowBy::Bot, Some("aaaaaaaaaaaa"), 2_000)],
        vec![checks("c1", "lazo", pr(55, 100), "passing", Some("bbbbbbbbbbbb"), 2_500)],
        vec![],
    );
    let r = &build(&cards, &f, 5_000).rows[0];
    assert!(r.reviews[0].stale);
    assert_eq!((r.waits_on, r.why.as_str()), (WaitsOn::Reviewer, "review_stale"));
}

#[test]
fn a_verdict_on_the_same_head_or_with_no_head_is_not_stale() {
    let cards = [card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)];
    let same = flow(
        vec![review("r1", "lazo", pr(55, 100), "approved", PrFlowBy::Bot, Some("aaaaaaaaaaaa"), 2_000)],
        vec![checks("c1", "lazo", pr(55, 100), "passing", Some("aaaaaaaaaaaa"), 2_500)],
        vec![],
    );
    assert!(!build(&cards, &same, 5_000).rows[0].reviews[0].stale);
    let none = flow(vec![review("r1", "lazo", pr(55, 100), "approved", PrFlowBy::Bot, None, 2_000)], vec![checks("c1", "lazo", pr(55, 100), "passing", Some("bbbbbbbbbbbb"), 2_500)], vec![]);
    assert!(!build(&cards, &none, 5_000).rows[0].reviews[0].stale);
}

#[test]
fn checks_older_than_the_verdict_do_not_make_it_stale() {
    let cards = [card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)];
    let f = flow(
        vec![review("r1", "lazo", pr(55, 100), "approved", PrFlowBy::Bot, Some("bbbbbbbbbbbb"), 3_000)],
        vec![checks("c1", "lazo", pr(55, 100), "passing", Some("aaaaaaaaaaaa"), 2_000)],
        vec![],
    );
    assert!(!build(&cards, &f, 5_000).rows[0].reviews[0].stale);
}

#[test]
fn each_reviewer_kind_keeps_its_newest_verdict() {
    let cards = [card(7, "claimed", "lazo", "agentmuxai/agentmux", 100)];
    let f = flow(
        vec![
            review("r1", "lazo", pr(55, 100), "changes_requested", PrFlowBy::Bot, None, 1_000),
            review("r2", "lazo", pr(55, 100), "approved", PrFlowBy::Bot, None, 2_000),
            review("r3", "lazo", pr(55, 100), "approved", PrFlowBy::Codex, None, 1_500),
        ],
        vec![],
        vec![],
    );
    let r = &build(&cards, &f, 5_000).rows[0];
    let got: Vec<(PrFlowBy, &str)> = r.reviews.iter().map(|v| (v.by, v.state.as_str())).collect();
    assert_eq!(got, vec![(PrFlowBy::Bot, "approved"), (PrFlowBy::Codex, "approved")]);
}

// ---- order and the wire shape ----

#[test]
fn rows_are_newest_change_first() {
    let mut a = card(1, "claimed", "a", "o/r", 1);
    let mut b = card(2, "claimed", "b", "o/r", 2);
    a.updated_at_ms = 10;
    b.updated_at_ms = 20;
    let list = build(&[a, b], &PrFlowSnapshot::default(), 30);
    assert_eq!(list.rows.iter().map(|r| r.number).collect::<Vec<_>>(), vec![2, 1]);
}

#[test]
fn the_wire_names_are_what_the_route_documents() {
    let r = &build(&[card(7, "claimed", "lazo", "o/r", 1)], &PrFlowSnapshot::default(), 1).rows[0];
    let v = serde_json::to_value(r).unwrap();
    assert_eq!(v["waits_on"], "reviewer");
    assert_eq!(v["task"]["ref"], "#7");
    assert_eq!(serde_json::to_value(WaitsOn::None).unwrap(), "none");
    assert!(v.get("repo_id").is_none() && v.get("checks").is_none(), "absent, not null");
}
