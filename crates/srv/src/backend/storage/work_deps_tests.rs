// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_queue::{ClaimFilter, WorkItem};

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

fn add(s: &Store, id: &str, created_at: i64, agent: &str) {
    s.work_queue_enqueue(&WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
        kind: String::new(),
        target_agent: agent.into(),
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
    })
    .unwrap();
    s.work_card_ensure(id, created_at).unwrap();
}

fn claim(s: &Store, agent: &str, now: i64) -> Option<String> {
    let f = ClaimFilter { kind: None, agent_id: agent.into(), agent_uid: String::new(), groups: vec![] };
    s.work_queue_claim(&f, now, 60_000).unwrap().map(|i| i.id)
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_task_waits_for_the_tasks_it_starts_after_to_finish() {
    let (s, _d) = store();
    add(&s, "first", 1000, "");
    add(&s, "second", 900, "");
    s.work_deps_set("second", &ids(&["first"])).unwrap();

    assert_eq!(claim(&s, "Naki", 2000).as_deref(), Some("first"), "second is older but waits");
    assert_eq!(claim(&s, "Naki", 2100), None, "nothing else to take while first is unfinished");
    let attempt = s.work_queue_get("first").unwrap().unwrap().attempts;
    assert!(s.work_deps_freed_by("first").unwrap().is_empty(), "first isn't finished yet");
    s.work_queue_complete("first", "Naki", "", attempt, "done", 3000).unwrap().unwrap();
    assert_eq!(s.work_deps_freed_by("first").unwrap(), ["second"]);
    assert_eq!(claim(&s, "Naki", 3100).as_deref(), Some("second"));
}

#[test]
fn a_cancelled_task_frees_its_followers_and_a_failed_one_does_not() {
    let (s, _d) = store();
    for id in ["a", "b", "after_a", "after_b"] {
        add(&s, id, 1000, "Loap-3");
    }
    s.work_deps_set("after_a", &ids(&["a"])).unwrap();
    s.work_deps_set("after_b", &ids(&["b"])).unwrap();
    s.work_queue_cancel("a", "not needed", 2000).unwrap();
    let waiting: Vec<String> = s.work_board_waiting_for("Loap-3", "", 10, 3000).unwrap().into_iter().map(|t| t.item.id).collect();
    assert_eq!(waiting, ["b", "after_a"], "after_b waits on b; after_a is free");
}

#[test]
fn the_card_lists_what_it_starts_after_with_their_state() {
    let (s, _d) = store();
    add(&s, "a", 1000, "");
    add(&s, "b", 1100, "");
    s.work_deps_set("b", &ids(&["a"])).unwrap();
    let b = s.work_board_get("b").unwrap().unwrap();
    let a_num = s.work_board_get("a").unwrap().unwrap().card.num;
    assert_eq!(b.card.after.len(), 1);
    assert_eq!((b.card.after[0].num, b.card.after[0].state.as_str(), b.card.after[0].finished()), (a_num, "open", false));
    let listed = s.work_board_list(0, 10, 2000).unwrap();
    assert_eq!(listed.iter().find(|t| t.item.id == "b").unwrap().card.after, b.card.after);
    assert!(listed.iter().find(|t| t.item.id == "a").unwrap().card.after.is_empty());

    s.work_deps_set("b", &[]).unwrap();
    assert!(s.work_deps_of("b").unwrap().is_empty(), "setting replaces the whole list");
}

#[test]
fn a_task_cant_wait_on_itself_a_missing_task_or_its_own_followers() {
    let (s, _d) = store();
    for id in ["a", "b", "c"] {
        add(&s, id, 1000, "");
    }
    s.work_deps_set("b", &ids(&["a"])).unwrap();
    s.work_deps_set("c", &ids(&["b"])).unwrap();
    assert!(s.work_deps_refusal("a", &ids(&["a"])).unwrap().unwrap().contains("itself"));
    assert!(s.work_deps_refusal("a", &ids(&["nope"])).unwrap().unwrap().contains("no task"));
    assert!(s.work_deps_refusal("a", &ids(&["c"])).unwrap().unwrap().contains("each other"), "c waits on b, which waits on a");
    assert_eq!(s.work_deps_refusal("c", &ids(&["a", "b"])).unwrap(), None);
    let many: Vec<String> = (0..=MAX_AFTER).map(|i| i.to_string()).collect();
    assert!(s.work_deps_refusal("a", &many).unwrap().is_some());
}

#[test]
fn dependencies_of_tasks_gone_from_the_queue_are_pruned() {
    let (s, _d) = store();
    add(&s, "a", 1000, "");
    add(&s, "b", 1000, "");
    s.work_deps_set("b", &ids(&["a"])).unwrap();
    s.work_deps_set("a", &ids(&["b"])).unwrap();
    assert_eq!(s.work_deps_prune().unwrap(), 0);
    s.conn().lock().unwrap().execute("DELETE FROM db_work_queue WHERE id = 'a'", []).unwrap();
    assert_eq!(s.work_deps_prune().unwrap(), 2, "both rows name a");
}
