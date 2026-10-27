// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_board::NewCard;
use crate::backend::storage::work_queue::WorkItem;

fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (s, dir)
}

fn task(s: &Store, id: &str) {
    let item = WorkItem {
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
        created_at: 1000,
        updated_at: 1000,
        not_before: None,
        result: String::new(),
    };
    s.work_queue_enqueue(&item).unwrap();
    s.work_card_create(id, &NewCard::default(), 1000).unwrap();
}

fn file(sha: &str, size: i64, attempt: i64) -> NewMedia {
    NewMedia {
        sha256: sha.into(),
        name: format!("{sha}.png"),
        kind: media_kind::IMAGE.into(),
        mime: "image/png".into(),
        size,
        caption: "after".into(),
        attached_by: "Naki-2".into(),
        attempt,
        ..Default::default()
    }
}

#[test]
fn a_file_is_attached_once_and_shows_on_the_card() {
    let (s, _d) = store();
    task(&s, "a");
    let first = s.work_media_add("a", &file("aa", 10, 1), 2000).unwrap().unwrap();
    let again = s.work_media_add("a", &file("aa", 10, 1), 2100).unwrap().unwrap();
    assert_eq!(first.id, again.id, "the same content is not attached twice");
    s.work_media_add("a", &file("bb", 10, 1), 2200).unwrap().unwrap();
    let card = s.work_board_get("a").unwrap().unwrap().card;
    assert_eq!(card.media.count, 2);
    assert_eq!(card.media.first.iter().map(|p| p.sha256.as_str()).collect::<Vec<_>>(), ["aa", "bb"]);
    assert_eq!(card.media.first[0].mime, "image/png");
}

#[test]
fn the_card_previews_only_the_first_three() {
    let (s, _d) = store();
    task(&s, "a");
    for sha in ["a1", "a2", "a3", "a4", "a5"] {
        s.work_media_add("a", &file(sha, 1, 1), 2000).unwrap().unwrap();
    }
    let card = s.work_board_get("a").unwrap().unwrap().card;
    assert_eq!(card.media.count, 5);
    assert_eq!(card.media.first.len(), PREVIEWS);
}

#[test]
fn a_task_carries_at_most_its_file_and_byte_limits() {
    let (s, _d) = store();
    task(&s, "a");
    for i in 0..MAX_FILES_PER_TASK {
        s.work_media_add("a", &file(&format!("f{i:02}"), 1, 1), 2000).unwrap().unwrap();
    }
    assert_eq!(s.work_media_add("a", &file("over", 1, 1), 2000).unwrap().unwrap_err(), AttachError::TooMany);

    task(&s, "b");
    s.work_media_add("b", &file("big1", MAX_BYTES_PER_TASK - 5, 1), 2000).unwrap().unwrap();
    assert_eq!(s.work_media_add("b", &file("big2", 6, 1), 2000).unwrap().unwrap_err(), AttachError::TooBig);
    s.work_media_add("b", &file("big3", 5, 1), 2000).unwrap().unwrap();
}

#[test]
fn the_same_picture_again_after_a_send_back_counts_for_the_new_attempt() {
    let (s, _d) = store();
    task(&s, "a");
    let first = s.work_media_add("a", &file("aa", 10, 1), 2000).unwrap().unwrap();
    let again = s.work_media_add("a", &file("aa", 10, 2), 3000).unwrap().unwrap();
    assert_ne!(first.id, again.id, "a new row for the new attempt");
    assert!(s.work_media_in_attempt("a", 2).unwrap());
    assert_eq!(s.work_media_add("a", &file("aa", 10, 2), 3100).unwrap().unwrap().id, again.id, "once within an attempt");
}

#[test]
fn only_media_from_this_attempt_counts_and_a_removed_file_does_not() {
    let (s, _d) = store();
    task(&s, "a");
    let m = s.work_media_add("a", &file("aa", 1, 1), 2000).unwrap().unwrap();
    assert!(s.work_media_in_attempt("a", 1).unwrap());
    assert!(!s.work_media_in_attempt("a", 2).unwrap(), "an earlier attempt's picture does not satisfy a new submit");
    let gone = s.work_media_remove("a", m.id, "operator", 3000).unwrap().unwrap();
    assert_eq!(gone.removed_at, Some(3000));
    assert!(!s.work_media_in_attempt("a", 1).unwrap());
    assert!(s.work_media_remove("a", m.id, "operator", 3100).unwrap().is_none(), "removing twice finds nothing");
    let all = s.work_media_list("a").unwrap();
    assert_eq!(all.len(), 1, "a removed file leaves its row");
    assert_eq!(all[0].removed_at, Some(3000));
    assert_eq!(s.work_board_get("a").unwrap().unwrap().card.media.count, 0);
}

#[test]
fn the_ui_flag_and_the_reason_are_kept_apart() {
    let (s, _d) = store();
    task(&s, "a");
    assert_eq!(s.work_ui_get("a").unwrap(), (false, String::new(), 0));
    s.work_ui_set("a", Some(true), None, 0, 2000).unwrap();
    s.work_ui_set("a", None, Some("headless host"), 2, 2100).unwrap();
    assert_eq!(s.work_ui_get("a").unwrap(), (true, "headless host".to_string(), 2), "the reason knows its attempt");
    let card = s.work_board_get("a").unwrap().unwrap().card;
    assert!(card.media.ui_change);
    assert_eq!(card.media.no_media_reason, "headless host");
    s.work_ui_set("a", Some(false), None, 0, 2200).unwrap();
    assert_eq!(s.work_ui_get("a").unwrap().1, "headless host", "unsetting the flag keeps the reason");
}

#[test]
fn the_files_of_a_task_go_thirty_days_after_it_finishes_and_not_before() {
    let (s, _d) = store();
    task(&s, "done");
    task(&s, "open");
    s.work_media_add("done", &file("shared", 1, 1), 2000).unwrap().unwrap();
    s.work_media_add("done", &file("only-done", 1, 1), 2000).unwrap().unwrap();
    s.work_media_add("open", &file("shared", 1, 1), 2000).unwrap().unwrap();
    let conn = s.conn().lock().unwrap();
    conn.execute("UPDATE db_work_queue SET state = ?1, updated_at = 5000 WHERE id = 'done'", [work_state::DONE]).unwrap();
    drop(conn);

    let early = s.work_media_prune(5000 + EVENTS_RETENTION_MS - 1).unwrap();
    assert!(early.is_empty(), "not before the thirty days");
    assert_eq!(s.work_media_list("done").unwrap().len(), 2);

    let free = s.work_media_prune(5000 + EVENTS_RETENTION_MS + 1).unwrap();
    assert_eq!(free, vec!["only-done".to_string()], "a file another task still carries stays");
    assert!(s.work_media_list("done").unwrap().is_empty());
    assert_eq!(s.work_media_list("open").unwrap().len(), 1);
    assert!(s.work_media_named("shared").unwrap());
    assert!(!s.work_media_named("only-done").unwrap());
}
