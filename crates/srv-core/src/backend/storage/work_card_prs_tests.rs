// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_board::CardUpdate;
use crate::backend::storage::work_queue::WorkItem;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

fn add(s: &Store, id: &str) {
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
}

fn report(repo: &str, number: u64, state: CardPrState, updated_at: i64) -> PrReport {
    PrReport {
        repo: repo.into(),
        number,
        state,
        head: Some(format!("sha{updated_at}")),
        opened_at: Some(updated_at - 1),
        merged_at: (state == CardPrState::Merged).then_some(updated_at),
        updated_at,
    }
}

fn states(s: &Store, id: &str) -> Vec<(String, u64, String)> {
    s.work_card_prs(id).unwrap().into_iter().map(|p| (p.repo, p.number, p.state)).collect()
}

fn pr_row(state: &str) -> WorkCardPr {
    WorkCardPr { state: state.into(), ..Default::default() }
}

#[test]
fn a_link_names_a_pr_as_an_address_a_reference_or_a_number() {
    assert_eq!(parse_pr_ref("https://github.com/AgentMuxAI/agentmux/pull/5248", ""), Some(("agentmuxai/agentmux".into(), 5248)));
    assert_eq!(parse_pr_ref("o/r#7", "x/y"), Some(("o/r".into(), 7)));
    assert_eq!(parse_pr_ref("#7", "o/r"), Some(("o/r".into(), 7)));
    assert_eq!(parse_pr_ref("#7", "local:/tmp/x"), None, "a local folder has no pull requests");
    assert_eq!(parse_pr_ref("https://github.com/o/r/issues/7", ""), None);
}

#[test]
fn a_card_lists_every_pull_request_newest_first() {
    let (s, _d) = store();
    add(&s, "a");
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Merged, 10), PrSource::Holder, 100).unwrap();
    s.work_card_pr_apply("a", &report("o/other", 2, CardPrState::Open, 30), PrSource::Holder, 100).unwrap();
    s.work_card_pr_apply("a", &report("o/r", 3, CardPrState::Open, 20), PrSource::Holder, 100).unwrap();

    assert_eq!(
        states(&s, "a"),
        [("o/other".to_string(), 2, "open".to_string()), ("o/r".to_string(), 3, "open".to_string()), ("o/r".to_string(), 1, "merged".to_string())],
        "in every repository, the one opened last first"
    );
    let card = s.work_board_get("a").unwrap().unwrap().card;
    assert_eq!(card.prs.len(), 3);
    assert_eq!(card.prs[0].url, "https://github.com/o/other/pull/2");
    assert_eq!((card.pr_rollup.total, card.pr_rollup.open, card.pr_rollup.merged), (3, 2, 1));
    assert_eq!(card.pr_rollup.state, "open");
    let listed = s.work_board_list(0, 50, 9).unwrap();
    assert_eq!(listed.iter().find(|t| t.item.id == "a").unwrap().card.prs.len(), 3, "the board's list carries them too");
}

#[test]
fn what_the_list_adds_up_to() {
    let r = |rows: &[&str], task: &str| rollup(&rows.iter().map(|s| pr_row(s)).collect::<Vec<_>>(), task);
    assert_eq!(r(&[], "claimed").state, "none");
    assert_eq!(r(&["open", "merged"], "claimed").state, "open", "any open: open work");
    assert_eq!(r(&["merged", "merged"], "claimed").state, "merged");
    assert_eq!(r(&["merged", "closed"], "claimed").state, "merged", "one closed unmerged does not hold it back");
    assert_eq!(r(&["closed"], "claimed").state, "closed", "only closed unmerged: not merged");
    let two = r(&["merged", "merged"], work_state::CLAIMED);
    assert!(two.all_merged_waiting && !two.follow_up_open, "held and all merged: waiting on its holder");
    assert!(!r(&["merged", "open"], work_state::CLAIMED).all_merged_waiting);
    assert!(!r(&["merged"], work_state::DONE).all_merged_waiting, "a finished task waits on no one");
    assert!(r(&["merged", "open"], work_state::DONE).follow_up_open, "a pull request opened after Done is a follow-up");
    assert!(!r(&["open"], work_state::DONE).follow_up_open, "its only pull request is not a follow-up");
}

#[test]
fn the_same_report_twice_and_an_older_one_change_nothing() {
    let (s, _d) = store();
    add(&s, "a");
    let open = report("o/r", 1, CardPrState::Open, 10);
    assert!(s.work_card_pr_apply("a", &open, PrSource::Holder, 100).unwrap().changed);
    assert!(!s.work_card_pr_apply("a", &open, PrSource::Holder, 101).unwrap().changed, "the same report twice");
    let merged = report("o/r", 1, CardPrState::Merged, 20);
    let first = s.work_card_pr_apply("a", &merged, PrSource::Read, 102).unwrap();
    assert!(first.changed && first.newly_merged);
    let again = s.work_card_pr_apply("a", &merged, PrSource::Read, 103).unwrap();
    assert!(!again.changed && !again.newly_merged, "a merge is news once");

    // A stale open, even a newer-looking one, never undoes a merge; nor does an older report of any kind.
    assert!(!s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 30), PrSource::Read, 104).unwrap().changed);
    assert!(!s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Closed, 5), PrSource::Read, 105).unwrap().changed);
    let row = &s.work_card_prs("a").unwrap()[0];
    assert_eq!((row.state.as_str(), row.merged_at, row.updated_at), ("merged", Some(20), 20));
}

#[test]
fn push_then_poll_and_poll_then_push_give_the_same_row() {
    let reports = [report("o/r", 1, CardPrState::Open, 10), report("o/r", 1, CardPrState::Merged, 20)];
    let end = |order: [usize; 2]| {
        let (s, _d) = store();
        add(&s, "a");
        for (i, n) in order.iter().enumerate() {
            let source = if i == 0 { PrSource::Event } else { PrSource::Read };
            s.work_card_pr_apply("a", &reports[*n], source, 100).unwrap();
        }
        let row = s.work_card_prs("a").unwrap().remove(0);
        (row.state, row.merged_at, row.updated_at)
    };
    assert_eq!(end([0, 1]), end([1, 0]));
    assert_eq!(end([0, 1]).0, "merged");
}

#[test]
fn a_closed_pull_request_can_be_reopened_by_a_newer_report() {
    let (s, _d) = store();
    add(&s, "a");
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Closed, 10), PrSource::Holder, 100).unwrap();
    assert_eq!(states(&s, "a")[0].2, "closed");
    assert!(s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 20), PrSource::Read, 101).unwrap().changed);
    assert_eq!(states(&s, "a")[0].2, "open");
}

#[test]
fn a_pull_request_taken_off_the_card_stays_off_until_the_holder_or_a_person_links_it() {
    let (s, _d) = store();
    add(&s, "a");
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 10), PrSource::Holder, 100).unwrap();
    assert!(s.work_card_pr_unlink("a", "o/r", 1, 101).unwrap());
    assert!(!s.work_card_pr_unlink("a", "o/r", 1, 102).unwrap(), "already off");
    assert!(s.work_card_prs("a").unwrap().is_empty());

    // A tag in an event, a read or a hand-in does not put it back, and a newer report is not applied to it.
    for source in [PrSource::Event, PrSource::Read, PrSource::HandIn] {
        assert!(!s.work_card_pr_link("a", "o/r", 1, source, 103).unwrap());
    }
    assert!(!s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Merged, 50), PrSource::Event, 104).unwrap().changed);
    assert!(s.work_card_prs("a").unwrap().is_empty());

    assert!(s.work_card_pr_link("a", "o/r", 1, PrSource::Holder, 105).unwrap(), "the holder puts it back");
    assert_eq!(states(&s, "a").len(), 1);
}

#[test]
fn a_stronger_source_replaces_a_weaker_one_and_never_the_other_way() {
    let (s, _d) = store();
    add(&s, "a");
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 10), PrSource::Event, 100).unwrap();
    assert_eq!(s.work_card_prs("a").unwrap()[0].source, "event");
    assert!(!s.work_card_prs("a").unwrap()[0].may_close(), "a tag alone never closes a task");

    s.work_card_pr_link("a", "o/r", 1, PrSource::Holder, 101).unwrap();
    assert_eq!(s.work_card_prs("a").unwrap()[0].source, "holder");
    assert!(s.work_card_prs("a").unwrap()[0].may_close());
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 20), PrSource::Event, 102).unwrap();
    assert_eq!(s.work_card_prs("a").unwrap()[0].source, "holder", "a later tag does not weaken it");
}

#[test]
fn an_address_set_on_the_card_joins_the_list_with_its_source_and_keeps_the_ones_before() {
    let (s, _d) = store();
    add(&s, "a");
    let set = |url: &str, source: Option<PrSource>| {
        s.work_card_update("a", &CardUpdate { pr_url: Some(url.into()), pr_source: source, ..Default::default() }, 5).unwrap()
    };
    set("https://github.com/o/r/pull/1", None);
    set("https://github.com/o/r/pull/2", Some(PrSource::HandIn));
    set("https://github.com/o/r/pull/2", None);
    let card = s.work_board_get("a").unwrap().unwrap().card;
    assert_eq!(card.pr_url, "https://github.com/o/r/pull/2", "pr_url is the last address set");
    let mut rows: Vec<_> = card.prs.iter().map(|p| (p.number, p.source.as_str())).collect();
    rows.sort();
    assert_eq!(rows, [(1, "holder"), (2, "holder")], "both listed; the holder's own setting is the stronger source");
    set("something that is not a PR", None);
    assert_eq!(s.work_board_get("a").unwrap().unwrap().card.prs.len(), 2);
}

#[test]
fn a_hand_in_link_is_recorded_as_one() {
    let (s, _d) = store();
    add(&s, "a");
    s.work_card_update(
        "a",
        &CardUpdate { pr_url: Some("https://github.com/o/r/pull/9".into()), pr_source: Some(PrSource::HandIn), ..Default::default() },
        5,
    )
    .unwrap();
    assert_eq!(s.work_card_prs("a").unwrap()[0].source, "hand_in");
    assert!(s.work_card_prs("a").unwrap()[0].may_close(), "a hand-in is the task's own link");
}

#[test]
fn cards_from_before_the_list_get_their_one_pull_request_once() {
    let (s, _d) = store();
    add(&s, "open");
    add(&s, "merged");
    add(&s, "none");
    add(&s, "odd");
    // Written as an older build did: the address on the card, and the merge beside it.
    {
        let conn = s.conn().lock().unwrap();
        for (id, url) in [("open", "https://github.com/o/r/pull/1"), ("merged", "https://github.com/o/r/pull/2"), ("odd", "not a pr")] {
            conn.execute("UPDATE db_work_cards SET pr_url = ?2 WHERE work_id = ?1", params![id, url]).unwrap();
        }
    }
    s.work_pr_merged("merged", 40, 50).unwrap();

    assert_eq!(s.work_card_prs_backfill(60).unwrap(), 2, "the one that is not an address, and the one with none, get nothing");
    assert_eq!(states(&s, "open"), [("o/r".to_string(), 1, "open".to_string())]);
    let merged = s.work_card_prs("merged").unwrap();
    assert_eq!((merged[0].state.as_str(), merged[0].merged_at, merged[0].source.as_str()), ("merged", Some(40), "holder"));
    assert!(s.work_card_prs("none").unwrap().is_empty() && s.work_card_prs("odd").unwrap().is_empty());
    assert_eq!(s.work_card_prs_backfill(70).unwrap(), 0, "safe to run on every start");
}

#[test]
fn the_list_is_pruned_with_its_task() {
    let (s, _d) = store();
    add(&s, "a");
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 10), PrSource::Holder, 100).unwrap();
    s.work_card_pr_apply("gone", &report("o/r", 2, CardPrState::Open, 10), PrSource::Holder, 100).unwrap();
    assert_eq!(s.work_card_prs_prune().unwrap(), 1);
    assert_eq!(s.work_card_prs("a").unwrap().len(), 1);
}

#[test]
fn a_change_to_the_list_counts_as_a_change_of_the_task() {
    let (s, _d) = store();
    add(&s, "a");
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 10), PrSource::Holder, 500).unwrap();
    let t = s.work_board_get("a").unwrap().unwrap();
    assert_eq!((t.item.updated_at, t.card.updated_at), (500, 500), "boards elsewhere see it move");
    s.work_card_pr_apply("a", &report("o/r", 1, CardPrState::Open, 10), PrSource::Holder, 900).unwrap();
    assert_eq!(s.work_board_get("a").unwrap().unwrap().item.updated_at, 500, "an ignored report is not a change");
}

#[test]
fn the_review_is_derived_from_the_latest_review_of_each_kind() {
    assert_eq!(review_of("{}"), "none");
    assert_eq!(review_of(r#"{"muxreview":"approved"}"#), "approved");
    assert_eq!(review_of(r#"{"muxreview":"approved","codex":"commented"}"#), "none", "every kind that reviewed must approve");
    assert_eq!(review_of(r#"{"muxreview":"approved","codex":"changes_requested"}"#), "changes_requested");
    assert_eq!(review_of("not json"), "none");
}
