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
    assert_eq!(store.input_history_delete(entry.id).unwrap().as_deref(), Some("a"));
    assert_eq!(store.input_history_delete(entry.id).unwrap(), None);
    assert_eq!(store.input_history_clear("b").unwrap(), 1);
    assert_eq!(texts(&store.input_history_list("a", None, 10).unwrap()), ["100% done?"]);
}

#[test]
fn the_draft_is_kept_until_sent_and_goes_with_the_agent() {
    let store = store_with(&["a"]);
    assert_eq!(store.input_draft_get("a").unwrap(), None);
    assert!(store.input_draft_set("a", "half a thought", 1).unwrap());
    assert!(store.input_draft_set("a", "half a thought, finished", 2).unwrap());
    assert_eq!(store.input_draft_get("a").unwrap().as_deref(), Some("half a thought, finished"));
    assert!(!store.input_draft_set("nobody", "x", 3).unwrap());
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
