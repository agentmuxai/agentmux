// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_queue::WorkItem;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

fn add(s: &Store, id: &str, kind: &str) {
    s.work_queue_enqueue(&WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
        kind: kind.into(),
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

#[test]
fn a_merge_then_a_release_show_on_the_card_once_each() {
    let (s, _d) = store();
    add(&s, "a", "build");
    assert_eq!(s.work_pr_get("a").unwrap(), None);
    assert!(!s.work_pr_released("a", "v1", 50, 2).unwrap(), "no release before the merge is known");

    assert!(s.work_pr_merged("a", 10, 2).unwrap());
    assert!(!s.work_pr_merged("a", 11, 3).unwrap(), "the first merge time stays");
    assert!(s.work_pr_released("a", "v0.59.41", 20, 4).unwrap());
    let t = s.work_board_get("a").unwrap().unwrap();
    assert_eq!((t.item.updated_at, t.card.updated_at), (4, 4), "a release counts as a change");
    assert!(!s.work_pr_released("a", "v0.59.42", 30, 5).unwrap(), "the first release that holds it stays");

    let card = s.work_board_get("a").unwrap().unwrap().card;
    assert_eq!(
        card.pr,
        Some(WorkPr { merged_at: Some(10), released_in: "v0.59.41".into(), released_at: Some(20) })
    );
    let listed = s.work_board_list(0, 50, 9).unwrap();
    assert_eq!(listed.iter().find(|t| t.item.id == "a").unwrap().card.pr.as_ref().unwrap().released_in, "v0.59.41");
}

#[test]
fn a_cards_followers_and_the_kinds_of_what_it_waits_on() {
    let (s, _d) = store();
    add(&s, "spec", "spec");
    add(&s, "build", "build");
    assert_eq!(s.work_deps_set("build", &["spec".to_string()]).unwrap(), None);
    let build = s.work_board_get("build").unwrap().unwrap().card;
    assert_eq!(build.after.iter().map(|a| (a.id.as_str(), a.kind.as_str())).collect::<Vec<_>>(), [("spec", "spec")]);
    let spec = s.work_board_get("spec").unwrap().unwrap().card;
    assert_eq!(spec.followers.iter().map(|f| (f.id.as_str(), f.kind.as_str())).collect::<Vec<_>>(), [("build", "build")]);
    assert_eq!(s.work_followers_of("spec").unwrap().len(), 1);

    // The spec's document travels with the dependency, so the build shows it
    // even when the spec itself is long off the board.
    s.work_doc_set("spec", "docs/specs/SPEC_X.md", 2).unwrap();
    let build = s.work_board_get("build").unwrap().unwrap().card;
    assert_eq!(build.after[0].doc, "docs/specs/SPEC_X.md");
}

#[test]
fn pruned_with_its_task() {
    let (s, _d) = store();
    add(&s, "a", "");
    s.work_pr_merged("a", 10, 2).unwrap();
    s.work_pr_merged("gone", 10, 2).unwrap();
    assert_eq!(s.work_prs_prune().unwrap(), 1);
    assert!(s.work_pr_get("a").unwrap().is_some());
}

#[test]
fn a_new_pr_link_forgets_the_old_prs_merge() {
    use crate::backend::storage::work_board::CardUpdate;
    let (s, _d) = store();
    add(&s, "a", "build");
    let link = |url: &str| CardUpdate { pr_url: Some(url.into()), ..Default::default() };
    s.work_card_update("a", &link("https://github.com/o/r/pull/1"), 2).unwrap();
    s.work_pr_merged("a", 10, 3).unwrap();
    s.work_card_update("a", &link("https://github.com/o/r/pull/1"), 4).unwrap();
    assert!(s.work_pr_get("a").unwrap().is_some(), "the same link keeps it");
    s.work_card_update("a", &link("https://github.com/o/r/pull/2"), 5).unwrap();
    assert_eq!(s.work_pr_get("a").unwrap(), None, "a different PR starts unknown");
}
