// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn store_with(agents: &[&str]) -> Store {
    let store = Store::open_in_memory().unwrap();
    for id in agents {
        store
            .conn()
            .lock()
            .unwrap()
            .execute("INSERT INTO db_agents (id, name, provider) VALUES (?1, 'A', 'claude')", params![id])
            .unwrap();
    }
    store
}

fn send<'a>(agent: &'a str, text: &'a str, at: i64) -> NewInputHistoryEntry<'a> {
    NewInputHistoryEntry { agent_id: agent, sent_at_ms: at, text, kind: "message", source: "composer", ..Default::default() }
}

fn texts(entries: &[InputHistoryEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.text.as_str()).collect()
}

#[test]
fn sends_are_kept_per_agent_newest_first_and_a_repeat_counts_once() {
    let store = store_with(&["a", "b"]);
    store.input_history_append(&send("a", "fix the build", 1), 100).unwrap();
    store.input_history_append(&send("a", "run the tests", 2), 100).unwrap();
    let again = store.input_history_append(&send("a", "run the tests", 3), 100).unwrap().unwrap();
    assert_eq!((again.sends, again.sent_at_ms), (2, 3), "the same text in a row is one entry");
    store.input_history_append(&send("b", "hello", 4), 100).unwrap();
    let a = store.input_history_list("a", None, 10).unwrap();
    assert_eq!(texts(&a), ["run the tests", "fix the build"]);
    assert_eq!(texts(&store.input_history_list("b", None, 10).unwrap()), ["hello"]);
    // The same text after another is a new entry.
    store.input_history_append(&send("a", "fix the build", 5), 100).unwrap();
    assert_eq!(store.input_history_list("a", None, 10).unwrap().len(), 3);
    // A slash command with the same text as a message is its own entry.
    let slash = NewInputHistoryEntry { kind: "slash", ..send("a", "fix the build", 6) };
    store.input_history_append(&slash, 100).unwrap();
    assert_eq!(store.input_history_list("a", None, 10).unwrap()[0].kind, "slash");
}

#[test]
fn only_known_agents_and_real_text_are_kept() {
    let store = store_with(&["a"]);
    assert_eq!(store.input_history_append(&send("claude", "quick-launch pane", 1), 100).unwrap(), None);
    assert_eq!(store.input_history_append(&send("a", "   ", 1), 100).unwrap(), None);
    let huge = "x".repeat(INPUT_HISTORY_MAX_TEXT + 1);
    assert_eq!(store.input_history_append(&send("a", &huge, 1), 100).unwrap(), None);
    assert!(store.input_history_list("a", None, 10).unwrap().is_empty());
}

#[test]
fn the_newest_entries_are_kept_and_older_ones_page_back() {
    let store = store_with(&["a"]);
    for i in 0..8 {
        store.input_history_append(&send("a", &format!("m{i}"), i), 5).unwrap();
    }
    let newest = store.input_history_list("a", None, 3).unwrap();
    assert_eq!(texts(&newest), ["m7", "m6", "m5"]);
    let older = store.input_history_list("a", Some(newest[2].id), 10).unwrap();
    assert_eq!(texts(&older), ["m4", "m3"], "only the newest 5 are kept");
}

#[test]
fn search_delete_and_clear() {
    let store = store_with(&["a", "b"]);
    store.input_history_append(&send("a", "Deploy to staging", 1), 100).unwrap();
    store.input_history_append(&send("a", "100% done?", 2), 100).unwrap();
    store.input_history_append(&send("b", "deploy docs", 3), 100).unwrap();
    assert_eq!(texts(&store.input_history_search(Some("a"), "deploy", 10).unwrap()), ["Deploy to staging"]);
    assert_eq!(store.input_history_search(None, "DEPLOY", 10).unwrap().len(), 2, "every agent, any case");
    assert_eq!(texts(&store.input_history_search(None, "100%", 10).unwrap()), ["100% done?"], "% is literal");
    assert!(store.input_history_search(None, "_", 10).unwrap().is_empty(), "_ is literal");
    let entry = store.input_history_search(Some("a"), "staging", 1).unwrap().remove(0);
    assert_eq!(store.input_history_delete(entry.id, WINDOW, 10).unwrap().map(|(agent, _)| agent).as_deref(), Some("a"));
    assert_eq!(store.input_history_delete(entry.id, WINDOW, 10).unwrap(), None);
    assert_eq!(store.input_history_clear("b", WINDOW, 10).unwrap().1, 1);
    assert_eq!(store.input_history_clear("b", WINDOW, 10).unwrap(), (None, 0), "nothing left, so no empty batch");
    assert_eq!(texts(&store.input_history_list("a", None, 10).unwrap()), ["100% done?"]);
}

const WINDOW: ForgottenBy<'static> = ForgottenBy { name: "armory-ui", uid: "" };
const DAY: i64 = 24 * 60 * 60 * 1000;

#[test]
fn a_forgotten_entry_waits_in_the_trash_with_who_and_comes_back_in_place() {
    let store = store_with(&["a"]);
    for (i, t) in ["one", "two", "three"].iter().enumerate() {
        store.input_history_append(&send("a", t, i as i64), 100).unwrap();
    }
    let two = store.input_history_search(Some("a"), "two", 1).unwrap().remove(0);
    let (_, batch) = store.input_history_delete(two.id, ForgottenBy { name: "korp", uid: "u-1" }, 1_000).unwrap().unwrap();
    assert_eq!(texts(&store.input_history_list("a", None, 10).unwrap()), ["three", "one"]);
    let trash = store.input_history_trash(Some("a"), 2_000).unwrap();
    assert_eq!(
        trash,
        [TrashedBatch { batch_id: batch.clone(), agent_id: "a".into(), deleted_at_ms: 1_000, deleted_by: "korp".into(), count: 1, sample: "two".into() }]
    );
    assert!(store.input_history_trash(Some("b"), 2_000).unwrap().is_empty());
    assert_eq!(store.input_history_restore(&batch, 3_000).unwrap(), Some(("a".to_string(), 1)));
    let back = store.input_history_list("a", None, 10).unwrap();
    assert_eq!(texts(&back), ["three", "two", "one"], "back between its neighbours, under its own id");
    assert_eq!(back[1].id, two.id);
    assert!(store.input_history_trash(None, 3_000).unwrap().is_empty());
    assert_eq!(store.input_history_restore(&batch, 3_000).unwrap(), None, "a batch comes back once");
}

#[test]
fn a_cleared_history_is_one_batch_and_a_taken_id_goes_to_the_newest_end() {
    let store = store_with(&["a"]);
    for (i, t) in ["one", "two", "three"].iter().enumerate() {
        store.input_history_append(&send("a", t, i as i64), 100).unwrap();
    }
    let (batch, cleared) = store.input_history_clear("a", WINDOW, 1_000).unwrap();
    assert_eq!(cleared, 3);
    let batch = batch.unwrap();
    // Something typed meanwhile reuses the newest id.
    let typed = store.input_history_append(&send("a", "typed since", 5), 100).unwrap().unwrap();
    let trash = store.input_history_trash(None, 1_500).unwrap();
    assert_eq!((trash.len(), trash[0].count, trash[0].sample.as_str()), (1, 3, "one"));
    assert_eq!(store.input_history_restore(&batch, 2_000).unwrap(), Some(("a".to_string(), 3)));
    let all = store.input_history_list("a", None, 10).unwrap();
    assert_eq!(all.len(), 4);
    assert!(all.iter().any(|e| e.id == typed.id && e.text == "typed since"), "what was typed since is untouched");
    assert_eq!(all.iter().filter(|e| ["one", "two", "three"].contains(&e.text.as_str())).count(), 3);
}

#[test]
fn the_trash_is_emptied_after_the_retention() {
    let store = store_with(&["a", "b"]);
    store.input_history_append(&send("a", "old news", 1), 100).unwrap();
    store.input_history_append(&send("b", "fresh", 2), 100).unwrap();
    let (old, _) = store.input_history_clear("a", WINDOW, 0).unwrap();
    store.input_history_clear("b", WINDOW, 5 * DAY).unwrap();
    let just_in_time = INPUT_HISTORY_TRASH_RETENTION_MS;
    assert_eq!(store.input_history_trash(None, just_in_time).unwrap().len(), 2, "still restorable at the limit");
    let soon_after = INPUT_HISTORY_TRASH_RETENTION_MS + 1;
    let left = store.input_history_trash(None, soon_after).unwrap();
    assert_eq!(left.iter().map(|b| b.agent_id.as_str()).collect::<Vec<_>>(), ["b"]);
    assert_eq!(store.input_history_restore(&old.unwrap(), soon_after).unwrap(), None, "past the retention it is gone");
    assert_eq!(store.input_history_trash_purge(5 * DAY + INPUT_HISTORY_TRASH_RETENTION_MS + 1).unwrap(), 1);
}

#[test]
fn deleting_the_agent_takes_its_trash_too() {
    let store = store_with(&["a"]);
    store.input_history_append(&send("a", "gone with it", 1), 100).unwrap();
    store.input_history_clear("a", WINDOW, 5).unwrap();
    let conn = store.conn().lock().unwrap();
    super::super::agents::purge_agent_dependents_for_tests(&conn, "a", None).unwrap();
    drop(conn);
    assert!(store.input_history_trash(None, 6).unwrap().is_empty());
}

#[test]
fn the_draft_is_kept_until_sent_and_goes_with_the_agent() {
    let store = store_with(&["a"]);
    assert_eq!(store.input_draft_get("a").unwrap(), None);
    assert!(store.input_draft_set("a", "half a thought", 1).unwrap());
    assert!(store.input_draft_set("a", "half a thought, finished", 2).unwrap());
    assert_eq!(store.input_draft_get("a").unwrap().as_deref(), Some("half a thought, finished"));
    assert!(!store.input_draft_set("nobody", "x", 3).unwrap());
    // A pasted file isn't kept, and the older draft doesn't stay in its place.
    assert!(store.input_draft_set("a", &"x".repeat(INPUT_HISTORY_MAX_TEXT + 1), 3).unwrap());
    assert_eq!(store.input_draft_get("a").unwrap(), None);
    store.input_draft_set("a", "half a thought, finished", 3).unwrap();
    assert!(store.input_draft_set("a", "", 4).unwrap());
    assert_eq!(store.input_draft_get("a").unwrap(), None);
    // Deleting the agent takes its history and draft (purge_agent_dependents).
    store.input_history_append(&send("a", "keep me?", 5), 100).unwrap();
    store.input_draft_set("a", "and me?", 6).unwrap();
    let conn = store.conn().lock().unwrap();
    super::super::agents::purge_agent_dependents_for_tests(&conn, "a", None).unwrap();
    drop(conn);
    assert!(store.input_history_list("a", None, 10).unwrap().is_empty());
    assert_eq!(store.input_draft_get("a").unwrap(), None);
}

#[test]
fn past_conversations_fill_the_history_once_before_what_is_kept() {
    let store = store_with(&["a"]);
    // Kept since input history shipped.
    store.input_history_append(&send("a", "newest, typed today", 50), 100).unwrap();
    // The transcripts also hold what the composer kept (at 50) and anything after.
    let typed = vec![
        (10, "first ever".to_string()),
        (20, "/compact".to_string()),
        (30, "!git status".to_string()),
        (50, "newest, typed today".to_string()),
        (55, "typed after it, in another window".to_string()),
    ];
    assert!(!store.input_history_backfilled("a").unwrap());
    assert_eq!(store.input_history_backfill("a", &typed, 100, 60).unwrap(), 3);
    assert!(store.input_history_backfilled("a").unwrap());
    let all = store.input_history_list("a", None, 10).unwrap();
    assert_eq!(texts(&all), ["newest, typed today", "!git status", "/compact", "first ever"], "older ones go before, by id");
    assert!(all[1..].iter().all(|e| e.source == "backfill" && e.id < 0), "never 0, the composer's mark for a send on its way");
    assert_eq!((all[1].kind.as_str(), all[2].kind.as_str(), all[3].kind.as_str()), ("bang", "slash", "message"));
    // Paging back walks into them.
    let older = store.input_history_list("a", Some(all[1].id), 10).unwrap();
    assert_eq!(texts(&older), ["/compact", "first ever"]);
    // Once only.
    assert_eq!(store.input_history_backfill("a", &typed, 100, 70).unwrap(), 0);
    assert_eq!(store.input_history_list("a", None, 10).unwrap().len(), 4);
}

#[test]
fn a_backfill_keeps_the_newest_up_to_the_cap() {
    let store = store_with(&["a", "b"]);
    store.input_history_append(&send("a", "kept", 100), 3).unwrap();
    let typed: Vec<(i64, String)> = (0..5).map(|i| (i, format!("old {i}"))).collect();
    store.input_history_backfill("a", &typed, 3, 1).unwrap();
    assert_eq!(texts(&store.input_history_list("a", None, 10).unwrap()), ["kept", "old 4", "old 3"]);
    // An agent with nothing typed before is marked filled too.
    assert_eq!(store.input_history_backfill("b", &[], 3, 1).unwrap(), 0);
    assert!(store.input_history_backfilled("b").unwrap());
}

#[test]
fn several_agents_are_filled_in_one_id_space() {
    let store = store_with(&["a", "b", "c"]);
    store.input_history_append(&send("b", "b today", 50), 100).unwrap();
    let typed = |who: &str| vec![(1, format!("{who} first")), (2, format!("{who} second"))];
    assert_eq!(store.input_history_backfill("a", &typed("a"), 100, 9).unwrap(), 2);
    assert_eq!(store.input_history_backfill("b", &typed("b"), 100, 9).unwrap(), 2);
    assert_eq!(store.input_history_backfill("c", &typed("c"), 100, 9).unwrap(), 2);
    assert_eq!(texts(&store.input_history_list("a", None, 10).unwrap()), ["a second", "a first"]);
    assert_eq!(texts(&store.input_history_list("b", None, 10).unwrap()), ["b today", "b second", "b first"]);
    assert_eq!(texts(&store.input_history_list("c", None, 10).unwrap()), ["c second", "c first"]);
}
