// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_board::NewCard;
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

/// File a task on `board` (an id; empty is the default) and return its number there.
fn file(s: &Store, id: &str, board: &str) -> i64 {
    s.work_queue_enqueue(&item(id, 1000)).unwrap();
    s.work_card_create(id, &NewCard { board: board.into(), filed_by: "tester".into(), ..Default::default() }, 1000).unwrap()
}

fn edit(name: &str) -> BoardEdit {
    BoardEdit { name: Some(name.into()), ..Default::default() }
}

fn claim(s: &Store, agent: &str) -> Option<WorkItem> {
    let f = ClaimFilter { kind: None, agent_id: agent.into(), agent_uid: String::new(), groups: vec![] };
    s.work_queue_claim(&f, 5000, 60_000).unwrap()
}

#[test]
fn a_fresh_store_has_main_as_the_default_board() {
    let (s, _d) = store();
    let boards = s.work_boards_list(true, 1).unwrap();
    assert_eq!(boards.len(), 1);
    assert_eq!((boards[0].slug.as_str(), boards[0].name.as_str()), ("main", "Main"));
    assert!(boards[0].is_default && !boards[0].archived);
}

#[test]
fn today_s_cards_keep_their_numbers_when_the_schema_first_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity-store.db");
    // A store from before boards: cards numbered 1..=3 (with a gap), and no db_boards table.
    {
        let s = Store::open_identity_store(&path).unwrap();
        for id in ["a", "b", "c"] {
            s.work_queue_enqueue(&item(id, 1000)).unwrap();
        }
        let conn = s.conn().lock().unwrap();
        conn.execute_batch(
            "DROP TABLE db_boards;
             DELETE FROM db_work_cards;
             INSERT INTO db_work_cards (num, work_id, updated_at) VALUES (167, 'a', 1), (168, 'b', 1), (200, 'c', 1);",
        )
        .unwrap();
    }
    let s = Store::open_identity_store(&path).unwrap();
    assert_eq!(s.work_board_get("b").unwrap().map(|t| t.card.num), Some(168), "a number is not changed");
    assert_eq!(s.work_board_get("#168").unwrap().unwrap().item.id, "b");
    assert_eq!(s.work_board_get("main#200").unwrap().unwrap().item.id, "c");
    assert_eq!(file(&s, "d", ""), 201, "the next number follows the highest");
}

#[test]
fn numbers_are_counted_per_board() {
    let (s, _d) = store();
    let work = s.work_board_create(&edit("Work"), "tester", 10).unwrap();
    assert_eq!(work.slug, "work");
    assert_eq!(file(&s, "m1", ""), 1);
    assert_eq!(file(&s, "w1", &work.id), 1);
    assert_eq!(file(&s, "w2", &work.id), 2);
    assert_eq!(file(&s, "m2", ""), 2);
    // A bare number is the default board's; another board's task needs its slug.
    assert_eq!(s.work_board_get("#2").unwrap().unwrap().item.id, "m2");
    assert_eq!(s.work_board_get("work#2").unwrap().unwrap().item.id, "w2");
    assert_eq!(s.work_board_get("WORK#1").unwrap().unwrap().item.id, "w1");
    assert!(s.work_board_get("nope#1").unwrap().is_none());
    let w2 = s.work_board_get("w2").unwrap().unwrap();
    assert_eq!((w2.card.board.as_str(), w2.card.task_ref.as_str()), ("work", "work#2"));
    assert_eq!(s.work_board_get("m2").unwrap().unwrap().card.task_ref, "#2");
}

#[test]
fn a_new_default_board_takes_the_bare_numbers() {
    let (s, _d) = store();
    let work = s.work_board_create(&edit("Work"), "tester", 10).unwrap();
    file(&s, "m1", "");
    file(&s, "w1", &work.id);
    s.work_board_set_default("work", 20).unwrap();
    assert_eq!(s.work_board_get("#1").unwrap().unwrap().item.id, "w1");
    assert_eq!(s.work_board_get("main#1").unwrap().unwrap().item.id, "m1");
    assert_eq!(s.work_board_get("m1").unwrap().unwrap().card.task_ref, "main#1", "the old default prints its slug");
    assert_eq!(file(&s, "w2", ""), 2, "a filing that names no board goes to the new default");
}

#[test]
fn slugs_are_made_from_names_and_kept_unique() {
    let (s, _d) = store();
    assert_eq!(s.work_board_create(&edit("Client X!"), "t", 1).unwrap().slug, "client-x");
    assert_eq!(s.work_board_create(&edit("client x"), "t", 1).unwrap().slug, "client-x-2");
    assert_eq!(s.work_board_create(&edit("Main"), "t", 1).unwrap().slug, "main-2");
    assert!(matches!(
        s.work_board_create(&BoardEdit { name: Some("A".into()), slug: Some("client-x".into()), ..Default::default() }, "t", 1),
        Err(BoardError::Refused(_))
    ));
    assert!(matches!(s.work_board_create(&BoardEdit { name: Some("A".into()), slug: Some("all".into()), ..Default::default() }, "t", 1), Err(BoardError::Refused(_))));
    assert!(matches!(s.work_board_create(&edit("  "), "t", 1), Err(BoardError::Refused(_))));
    assert!(matches!(s.work_board_create(&BoardEdit { tier: Some("lan".into()), ..edit("Shared") }, "t", 1), Err(BoardError::Refused(_))));
}

#[test]
fn a_changed_slug_keeps_working_as_an_alias_for_ninety_days() {
    let (s, _d) = store();
    // The real clock: a lookup by ref checks an alias against it.
    let t0 = now_ms_clock();
    let b = s.work_board_create(&edit("Work"), "t", t0).unwrap();
    file(&s, "w1", &b.id);
    let renamed = s.work_board_edit("work", &BoardEdit { slug: Some("job".into()), name: Some("The job".into()), ..Default::default() }, t0 + 1000).unwrap();
    assert_eq!((renamed.slug.as_str(), renamed.name.as_str()), ("job", "The job"));
    assert_eq!(s.work_board_get("job#1").unwrap().unwrap().item.id, "w1");
    assert_eq!(s.work_board_get("work#1").unwrap().unwrap().item.id, "w1", "the old slug still works");
    assert!(s.work_board_resolve(Some("The job"), t0 + 1000).is_ok(), "and so does the display name");
    // The old slug is not free for another board while its alias lives, and is after.
    assert!(s.work_board_create(&BoardEdit { slug: Some("work".into()), ..edit("Other") }, "t", t0 + 3000).is_err());
    assert!(s.work_board_create(&BoardEdit { slug: Some("work".into()), ..edit("Other") }, "t", t0 + 1000 + ALIAS_MS + 1).is_ok());
    // Main's slug is what bare refs rest on.
    assert!(s.work_board_edit("main", &BoardEdit { slug: Some("home".into()), ..Default::default() }, 5).is_err());
    assert_eq!(s.work_board_edit("main", &edit("Everything"), 5).unwrap().slug, "main");
}

#[test]
fn a_claim_takes_the_default_board_first_and_skips_archived_and_unlisted_boards() {
    let (s, _d) = store();
    let work = s.work_board_create(&edit("Work"), "t", 1).unwrap();
    let team = s.work_board_create(&BoardEdit { agents: Some(vec!["agentx".into()]), ..edit("Team") }, "t", 1).unwrap();
    file(&s, "w", &work.id);
    file(&s, "t", &team.id);
    file(&s, "m", "");
    assert_eq!(claim(&s, "naki-2").unwrap().id, "m", "the default board goes first on a tie");
    assert_eq!(claim(&s, "naki-2").unwrap().id, "w");
    assert!(claim(&s, "naki-2").is_none(), "the Team board's list leaves naki-2 out");
    assert_eq!(claim(&s, "agentx").unwrap().id, "t");
    // Archived: read-only.
    file(&s, "w2", &work.id);
    s.work_board_archive("work", true, 10).unwrap();
    assert!(claim(&s, "naki-2").is_none());
    s.work_board_archive("work", false, 11).unwrap();
    assert_eq!(claim(&s, "naki-2").unwrap().id, "w2");
}

#[test]
fn a_claim_can_be_limited_to_one_board() {
    let (s, _d) = store();
    let work = s.work_board_create(&edit("Work"), "t", 1).unwrap();
    file(&s, "m", "");
    file(&s, "w", &work.id);
    let f = ClaimFilter { kind: None, agent_id: "naki-2".into(), agent_uid: String::new(), groups: vec!["g".into()] };
    let got = s.work_queue_claim_in(&f, Some(&work.id), 5000, 60_000, None).unwrap().unwrap();
    assert_eq!(got.id, "w");
}

#[test]
fn the_default_board_cannot_be_archived_or_deleted_and_only_archived_boards_go() {
    let (s, _d) = store();
    assert!(s.work_board_archive("main", true, 1).is_err());
    assert!(s.work_board_delete("main", 1).is_err());
    let b = s.work_board_create(&edit("Scratch"), "t", 1).unwrap();
    assert!(s.work_board_delete("scratch", 2).is_err(), "not archived");
    s.work_board_archive("scratch", true, 3).unwrap();
    assert!(s.work_board_set_default("scratch", 4).is_err(), "an archived board can't be the default");
    // Empty: gone now.
    assert_eq!(s.work_board_delete("scratch", 5).unwrap(), Deletion::Deleted);
    assert_eq!(s.work_boards_list(true, 6).unwrap().len(), 1);
    // With tasks: a countdown with a veto.
    let b2 = s.work_board_create(&edit("Old"), "t", 10).unwrap();
    file(&s, "o1", &b2.id);
    s.work_board_archive("old", true, 11).unwrap();
    let Deletion::Pending(at) = s.work_board_delete("old", 20).unwrap() else { panic!("a board with a task waits") };
    assert_eq!(at, 20 + DELETE_COUNTDOWN_MS);
    assert!(s.work_boards_finish_deletes(at - 1).unwrap().is_empty());
    s.work_board_cancel_delete("old", at - 1).unwrap();
    assert!(s.work_boards_finish_deletes(at + 1).unwrap().is_empty(), "the veto stands");
    s.work_board_delete("old", 100).unwrap();
    assert_eq!(s.work_boards_finish_deletes(100 + DELETE_COUNTDOWN_MS).unwrap(), vec!["old".to_string()]);
    assert!(s.work_board_get("o1").unwrap().is_none(), "its tasks go with it");
    assert!(s.work_queue_get("o1").unwrap().is_none());
    let _ = b;
}

#[test]
fn a_task_moves_with_a_new_number_and_a_trail_on_both_sides() {
    let (s, _d) = store();
    let work = s.work_board_create(&edit("Work"), "t", 1).unwrap();
    file(&s, "m1", "");
    file(&s, "m2", "");
    file(&s, "w1", &work.id);
    let moved = s.work_task_move("#2", "work", "Naki-2", false, 50).unwrap();
    assert_eq!((moved.from_ref.as_str(), moved.to_ref.as_str(), moved.num), ("#2", "work#2", 2));
    let new = s.work_board_get("work#2").unwrap().unwrap();
    assert_eq!(new.item.title, "task m2");
    assert_eq!(new.item.state, "open");
    let old = s.work_board_get("m2").unwrap().unwrap();
    assert_eq!(old.item.state, "done");
    assert_eq!(old.item.result, "Moved to work#2");
    let trail = |id: &str| s.work_events(id, 10).unwrap().into_iter().filter(|e| e.kind == "moved").map(|e| e.body).collect::<Vec<_>>();
    assert_eq!(trail(&new.item.id), vec!["moved from #2 by Naki-2".to_string()]);
    assert_eq!(trail("m2"), vec!["moved to work#2 by Naki-2".to_string()]);
    assert!(s.work_task_move("work#2", "work", "x", false, 51).is_err(), "already there");
    assert!(s.work_task_move("m2", "main", "x", false, 51).is_err(), "a finished task does not move");
}

#[test]
fn a_claimed_task_or_one_with_dependencies_does_not_move_until_asked() {
    let (s, _d) = store();
    let work = s.work_board_create(&edit("Work"), "t", 1).unwrap();
    file(&s, "a", "");
    file(&s, "b", "");
    assert_eq!(claim(&s, "naki-2").unwrap().id, "a");
    let why = s.work_task_move("#1", "work", "x", false, 9).unwrap_err().to_string();
    assert!(why.contains("release it or finish it"), "{why}");
    // b starts after a: either one has an edge.
    s.work_queue_enqueue_after(&item("c", 1000), &["b".to_string()]).unwrap();
    s.work_card_create("c", &NewCard::default(), 1000).unwrap();
    let why = s.work_task_move("#2", "work", "x", false, 9).unwrap_err().to_string();
    assert!(why.contains("dependencies") && why.contains("#3"), "{why}");
    let moved = s.work_task_move("#2", "work", "x", true, 9).unwrap();
    assert_eq!(moved.dropped_deps, vec!["#3".to_string()]);
    let _ = work;
}

#[test]
fn a_moved_task_loses_a_target_the_board_does_not_list() {
    let (s, _d) = store();
    let team = s.work_board_create(&BoardEdit { agents: Some(vec!["agentx".into()]), ..edit("Team") }, "t", 1).unwrap();
    s.work_queue_enqueue(&WorkItem { target_agent: "naki-2".into(), ..item("a", 1000) }).unwrap();
    s.work_card_create("a", &NewCard::default(), 1000).unwrap();
    let moved = s.work_task_move("#1", "team", "x", false, 9).unwrap();
    assert!(moved.target_cleared);
    assert_eq!(s.work_queue_get(&moved.id).unwrap().unwrap().target_agent, "");
    let _ = team;
}

#[test]
fn an_archived_board_takes_no_moves() {
    let (s, _d) = store();
    s.work_board_create(&edit("Old"), "t", 1).unwrap();
    s.work_board_archive("old", true, 2).unwrap();
    file(&s, "a", "");
    assert!(s.work_task_move("#1", "old", "x", false, 9).is_err());
}

#[test]
fn refs_parse() {
    assert_eq!(parse_ref("#168"), Some((None, 168)));
    assert_eq!(parse_ref("168"), Some((None, 168)));
    assert_eq!(parse_ref("work#12"), Some((Some("work".into()), 12)));
    assert_eq!(parse_ref(" Client-X#3 "), Some((Some("client-x".into()), 3)));
    assert_eq!(parse_ref("work#"), None);
    assert_eq!(parse_ref("a b#1"), None);
    assert_eq!(parse_ref("3f2a9c10-aaaa-bbbb-cccc-000000000000"), None);
}

#[test]
fn a_dependency_is_shown_by_its_number_on_its_board() {
    let (s, _d) = store();
    let work = s.work_board_create(&edit("Work"), "t", 1).unwrap();
    // Cards on Main first, so the store's own keys and the boards' numbers no longer agree.
    file(&s, "m1", "");
    file(&s, "m2", "");
    file(&s, "w1", &work.id);
    s.work_queue_enqueue_after(&item("w2", 1000), &["w1".to_string()]).unwrap();
    s.work_card_create("w2", &NewCard { board: work.id.clone(), ..Default::default() }, 1000).unwrap();
    let w2 = s.work_board_get("work#2").unwrap().unwrap();
    let after = &w2.card.after[0];
    assert_eq!((after.num, after.board.as_str(), after.task_ref.as_str(), after.label().as_str()), (1, "work", "work#1", "work#1"));
    let w1 = s.work_board_get("work#1").unwrap().unwrap();
    assert_eq!(w1.card.followers[0].task_ref, "work#2");
    // On the default board a dependency is a bare number.
    s.work_queue_enqueue_after(&item("m3", 1000), &["m1".to_string()]).unwrap();
    s.work_card_create("m3", &NewCard::default(), 1000).unwrap();
    assert_eq!(s.work_board_get("#3").unwrap().unwrap().card.after[0].label(), "#1");
}
