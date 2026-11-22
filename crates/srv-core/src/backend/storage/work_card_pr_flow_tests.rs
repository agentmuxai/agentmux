// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Review, checks and merge notices on the card's pull requests: where a notice lands, and what each pull request
//! then waits on.

use super::*;
use crate::backend::storage::work_queue::WorkItem;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

/// A task held by `holder`, linked to `repo#number`.
fn card(s: &Store, id: &str, holder: &str, repo: &str, number: u64) {
    s.work_queue_enqueue(&WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
        kind: "build".into(),
        target_agent: holder.into(),
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
    s.work_card_pr_link(id, repo, number, PrSource::Holder, 1).unwrap();
}

fn note(agent: &str, repo_id: u64, number: u64, at_ms: i64, fact: FlowFact) -> FlowNote {
    FlowNote { agent: agent.into(), repo_id, number, at_ms, fact }
}

fn review(by: &str, state: &str, head: &str) -> FlowFact {
    FlowFact::Review { by: by.into(), state: state.into(), head: (!head.is_empty()).then(|| head.into()) }
}

fn checks(state: &str, head: &str) -> FlowFact {
    FlowFact::Checks { state: state.into(), head: (!head.is_empty()).then(|| head.into()) }
}

fn waits(s: &Store, id: &str) -> (String, String) {
    let t = s.work_board_get(id).unwrap().unwrap();
    (t.card.prs[0].waits_on.clone(), t.card.prs[0].why.clone())
}

fn pair(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

#[test]
fn a_notice_lands_on_the_one_pull_request_the_agent_holds_with_that_number() {
    let (s, _d) = store();
    card(&s, "a", "Pell", "o/r", 7);
    assert_eq!(waits(&s, "a"), pair("reviewer", "no_review"), "nothing said yet");

    assert_eq!(s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h1")), 100).unwrap(), 1);
    assert_eq!(s.work_card_pr_flow_apply(&note("pell", 42, 7, 101, checks("passing", "h1")), 101).unwrap(), 1);
    let pr = s.work_board_get("a").unwrap().unwrap().card.prs.remove(0);
    assert_eq!((pr.review.as_str(), pr.checks.as_str(), pr.repo_id), ("approved", "passing", Some(42)));
    assert_eq!(waits(&s, "a"), pair("author", "ready_to_merge"));

    s.work_card_pr_flow_apply(&note("pell", 42, 7, 102, FlowFact::Merge { state: "queued".into() }), 102).unwrap();
    assert_eq!(waits(&s, "a").0, "queue");
}

#[test]
fn a_notice_for_another_agent_or_number_changes_nothing() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    assert_eq!(s.work_card_pr_flow_apply(&note("lazo", 42, 7, 100, review("muxreview", "approved", "h")), 100).unwrap(), 0);
    assert_eq!(s.work_card_pr_flow_apply(&note("pell", 42, 8, 100, review("muxreview", "approved", "h")), 100).unwrap(), 0);
    assert_eq!(waits(&s, "a").1, "no_review");
}

#[test]
fn the_same_number_in_two_repositories_is_not_guessed() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    card(&s, "b", "pell", "o/other", 7);
    assert_eq!(s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h")), 100).unwrap(), 0);
    assert_eq!((waits(&s, "a").1.as_str(), waits(&s, "b").1.as_str()), ("no_review", "no_review"), "neither takes it");
    assert_eq!(s.work_card_prs("a").unwrap()[0].repo_id, None, "no id is learned from an ambiguous notice");
}

#[test]
fn an_id_that_is_another_repositorys_does_not_move() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    card(&s, "b", "lazo", "o/other", 9);
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h")), 100).unwrap();
    // Id 42 is o/r's: a notice that names it for the other repository's pull request is refused.
    assert_eq!(s.work_card_pr_flow_apply(&note("lazo", 42, 9, 101, review("muxreview", "approved", "h")), 101).unwrap(), 0);
    assert_eq!(waits(&s, "b").1, "no_review");
    // And a repository that has an id keeps it.
    assert_eq!(s.work_card_pr_flow_apply(&note("pell", 43, 7, 102, review("muxreview", "changes_requested", "h")), 102).unwrap(), 0);
    assert_eq!(s.work_card_prs("a").unwrap()[0].repo_id, Some(42));
}

#[test]
fn once_known_the_repository_takes_the_notice_on_every_card_that_holds_it() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    card(&s, "b", "lazo", "o/r", 7);
    // Only pell's card is found by the agent, but the repository's id is then known for both.
    assert_eq!(s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h")), 100).unwrap(), 2);
    assert_eq!(s.work_card_prs("b").unwrap()[0].review, "approved");
}

#[test]
fn a_new_commit_makes_the_old_verdict_stale_and_the_wait_moves_to_a_reviewer() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h1")), 100).unwrap();
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 101, checks("passing", "h1")), 101).unwrap();
    assert_eq!(waits(&s, "a").0, "author");

    s.work_card_pr_flow_apply(&note("pell", 42, 7, 200, checks("pending", "h2")), 200).unwrap();
    let pr = s.work_board_get("a").unwrap().unwrap().card.prs.remove(0);
    assert!(pr.verdicts[0].stale, "the approval is of the commit before");
    assert_eq!(pr.review, "none");
    assert_eq!(waits(&s, "a"), pair("ci", "checks_running"), "the new commit's checks come first");
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 201, checks("passing", "h2")), 201).unwrap();
    assert_eq!(waits(&s, "a"), pair("reviewer", "review_stale"));
}

#[test]
fn changes_requested_on_the_current_commit_waits_on_the_author() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "changes_requested", "h1")), 100).unwrap();
    assert_eq!(waits(&s, "a"), pair("author", "changes_requested"));
}

#[test]
fn a_late_older_notice_is_ignored() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 200, review("muxreview", "changes_requested", "h")), 200).unwrap();
    assert_eq!(s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h")), 300).unwrap(), 0);
    assert_eq!(s.work_card_prs("a").unwrap()[0].review, "changes_requested");
}

#[test]
fn a_merge_on_github_wins_over_a_notice() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    s.work_card_pr_apply(
        "a",
        &PrReport { repo: "o/r".into(), number: 7, state: CardPrState::Merged, head: None, opened_at: Some(1), merged_at: Some(5), updated_at: 5 },
        PrSource::Holder,
        10,
    )
    .unwrap();
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, FlowFact::Merge { state: "queued".into() }), 100).unwrap();
    assert_eq!(waits(&s, "a").0, "none", "a merged pull request waits on no one");
}

#[test]
fn what_arrived_is_still_there_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity-store.db");
    {
        let s = Store::open_identity_store(&path).unwrap();
        card(&s, "a", "pell", "o/r", 7);
        s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h1")), 100).unwrap();
        s.work_card_pr_flow_apply(&note("pell", 42, 7, 101, checks("passing", "h1")), 101).unwrap();
    }
    let s = Store::open_identity_store(&path).unwrap();
    assert_eq!(waits(&s, "a"), pair("author", "ready_to_merge"));
}

#[test]
fn the_card_says_which_pull_request_blocks_it() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    s.work_card_pr_link("a", "o/r", 8, PrSource::Holder, 2).unwrap();
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 100, review("muxreview", "approved", "h")), 100).unwrap();
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 101, checks("passing", "h")), 101).unwrap();

    let roll = s.work_board_get("a").unwrap().unwrap().card.pr_rollup;
    assert_eq!((roll.waits_on.as_str(), roll.blocking.as_str()), ("author", "o/r#7"), "ready to merge presses before a pull request nobody reviewed");
    assert_eq!(roll.waits_more, 1);
}

#[test]
fn what_a_card_adds_to_the_board_stays_small() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    for (i, by) in ["bot", "codex", "person"].into_iter().enumerate() {
        s.work_card_pr_flow_apply(&note("pell", 42, 7, 100 + i as i64, review(by, "approved", "0123456789abcdef0123456789abcdef01234567")), 100).unwrap();
    }
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 110, checks("passing", "0123456789abcdef0123456789abcdef01234567")), 110).unwrap();
    let pr = s.work_board_get("a").unwrap().unwrap().card.prs.remove(0);
    let plain = WorkCardPr { verdicts: vec![], waits_on: String::new(), why: String::new(), ..pr.clone() };
    let (with, without) = (serde_json::to_string(&pr).unwrap().len(), serde_json::to_string(&plain).unwrap().len());
    // Three reviewers on a full commit id, checks, and the wait: well under half a kilobyte more per pull request.
    assert!(with - without < 500, "a pull request grew by {} bytes", with - without);
}

#[test]
fn a_head_the_pull_request_reported_after_a_verdict_makes_that_verdict_stale() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 200, checks("failing", "h2")), 200).unwrap();
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 250, review("muxreview", "approved", "h1")), 250).unwrap();
    // The pull request's own report, of a time after both, says its head is h2.
    s.work_card_pr_apply(
        "a",
        &PrReport { repo: "o/r".into(), number: 7, state: CardPrState::Open, head: Some("h2".into()), opened_at: Some(1), merged_at: None, updated_at: 300 },
        PrSource::Holder,
        300,
    )
    .unwrap();

    let pr = s.work_board_get("a").unwrap().unwrap().card.prs.remove(0);
    assert!(pr.verdicts[0].stale, "the approval is of h1, not the pull request's head");
    assert_eq!(pr.checks, "failing", "the current commit's checks stay");
    assert_eq!(waits(&s, "a"), pair("author", "checks_failing"));
}

#[test]
fn a_push_the_pull_request_never_reported_does_not_make_the_new_notices_stale() {
    let (s, _d) = store();
    card(&s, "a", "pell", "o/r", 7);
    // The report knows h1 only; a push to h2 came with no report (nothing tells of a push), then its notices.
    s.work_card_pr_apply(
        "a",
        &PrReport { repo: "o/r".into(), number: 7, state: CardPrState::Open, head: Some("h1".into()), opened_at: Some(1), merged_at: None, updated_at: 100 },
        PrSource::Holder,
        100,
    )
    .unwrap();
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 200, checks("passing", "h2")), 200).unwrap();
    s.work_card_pr_flow_apply(&note("pell", 42, 7, 250, review("muxreview", "approved", "h2")), 250).unwrap();

    let pr = s.work_board_get("a").unwrap().unwrap().card.prs.remove(0);
    assert!(!pr.verdicts[0].stale);
    assert_eq!((pr.checks.as_str(), pr.review.as_str()), ("passing", "approved"));
    assert_eq!(waits(&s, "a"), pair("author", "ready_to_merge"));
}

#[test]
fn an_abbreviated_commit_id_names_the_same_commit() {
    assert!(same_head("0123456789abcdef", "0123456"));
    assert!(same_head("0123456", "0123456789abcdef"));
    assert!(!same_head("0123456789abcdef", "fedcba9"));
}
