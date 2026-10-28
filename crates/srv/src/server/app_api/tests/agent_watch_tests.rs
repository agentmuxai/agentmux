// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents watch`: the line slicing, and replay then tail through the handler.

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};
use crate::sagas::park_agents_tests::{seed_agent_block, seed_workspace};
use crate::server::tests::test_state;

fn texts(l: &Lines) -> Vec<&str> {
    l.lines.iter().map(String::as_str).collect()
}

#[test]
fn a_tail_returns_whole_lines_and_leaves_a_line_being_written() {
    let data = b"one\ntwo\n\nthr";
    let l = lines_of(data, 100, false, false, 10);
    assert_eq!(texts(&l), ["one", "two"]);
    assert_eq!(l.next, 100 + "one\ntwo\n\n".len() as u64, "next is before the unfinished line");
    assert!(!l.more);
    // Nothing complete yet: the cursor stays.
    let l = lines_of(b"par", 7, false, false, 10);
    assert_eq!((l.lines.len(), l.next), (0, 7));
}

#[test]
fn a_replay_keeps_the_last_lines_and_drops_the_piece_the_window_starts_in() {
    let data = b"ial\nb\nc\nd\n";
    let l = lines_of(data, 50, true, true, 2);
    assert_eq!(texts(&l), ["c", "d"]);
    assert!(l.more, "older lines were left out");
    assert_eq!(l.next, 50 + data.len() as u64);
    // A window with no newline at all is inside one line: nothing to show, nothing consumed.
    assert_eq!(lines_of(b"abcdef", 9, true, true, 5), Lines { lines: vec![], next: 9, more: false });
}

#[test]
fn a_tail_with_more_than_asked_returns_the_first_and_says_where_to_go_on() {
    let data = b"a\nb\nc\nd\n";
    let l = lines_of(data, 0, false, false, 2);
    assert_eq!(texts(&l), ["a", "b"]);
    assert_eq!((l.next, l.more), (4, true));
    let rest = lines_of(&data[l.next as usize..], l.next, false, false, 2);
    assert_eq!((texts(&rest), rest.next, rest.more), (vec!["c", "d"], 8, false));
}

#[test]
fn a_long_line_is_shortened_and_says_by_how_much() {
    let long = "x".repeat(MAX_LINE_CHARS + 5);
    let l = lines_of(format!("{long}\n").as_bytes(), 0, false, false, 5);
    assert!(l.lines[0].ends_with("…[5 more characters]"), "{}", &l.lines[0][MAX_LINE_CHARS..]);
    assert_eq!(l.lines[0].chars().filter(|c| *c == 'x').count(), MAX_LINE_CHARS);
}

async fn call(state: &AppState, caller: Option<&str>, body: Value) -> (StatusCode, Value) {
    let caller = caller.map(|uid| Extension(Caller::Agent { uid: uid.to_string() }));
    let req: WatchRequest = serde_json::from_value(body).unwrap();
    let resp = handle_agent_watch(State(state.clone()), caller, Json(req)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn append(state: &AppState, block: &str, text: &str) {
    if state.filestore.stat(block, OUTPUT_FILE).unwrap().is_none() {
        state.filestore.make_file(block, OUTPUT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
    }
    state.filestore.append_data(block, OUTPUT_FILE, text.as_bytes()).unwrap();
}

fn lines(body: &Value) -> Vec<&str> {
    body["lines"].as_array().unwrap().iter().filter_map(|l| l.as_str()).collect()
}

#[tokio::test]
async fn a_watch_with_no_agent_token_is_refused() {
    let state = test_state();
    let (status, body) = call(&state, None, json!({ "agent": "anyone" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().unwrap().contains("Agents watch acts as the calling agent"), "{body}");
}

#[tokio::test]
async fn replay_then_tail_returns_each_line_once_and_a_cursor_past_the_end_replays() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "wt-caller").await;
    let block = seed_agent_block(&state, &tabs[0], "wt-target").await;

    // Written lines the watcher has not seen: the replay shows the last two.
    append(&state, &block, "l1\nl2\nl3\n");
    let (status, first) = call(&state, Some("wt-caller"), json!({ "agent": "wt-target", "max_lines": 2 })).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(lines(&first), ["l2", "l3"]);
    assert_eq!(first["more"], true);
    let next = first["next"].as_u64().unwrap();
    assert_eq!(next, "l1\nl2\nl3\n".len() as u64);

    // Nothing new: an empty answer with the same cursor.
    let (_, none) = call(&state, Some("wt-caller"), json!({ "agent": "wt-target", "after": next })).await;
    assert_eq!((lines(&none).len(), none["next"].clone()), (0, json!(next)));

    // A line half written is not shown; once finished it is, once.
    append(&state, &block, "l4\nl5-part");
    let (_, tail) = call(&state, Some("wt-caller"), json!({ "agent": "wt-target", "after": next })).await;
    assert_eq!(lines(&tail), ["l4"]);
    let next = tail["next"].as_u64().unwrap();
    append(&state, &block, "ial\n");
    let (_, tail) = call(&state, Some("wt-caller"), json!({ "agent": "wt-target", "after": next })).await;
    assert_eq!(lines(&tail), ["l5-partial"]);

    // The transcript restarted shorter than the cursor: a replay, and it says so.
    let (_, reset) = call(&state, Some("wt-caller"), json!({ "agent": "wt-target", "after": 9_999_999, "max_lines": 1 })).await;
    assert_eq!((reset["reset"].clone(), lines(&reset)), (json!(true), vec!["l5-partial"]));
}

#[tokio::test]
async fn a_wait_returns_when_a_line_arrives() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "ww-caller").await;
    let block = seed_agent_block(&state, &tabs[0], "ww-target").await;
    append(&state, &block, "first\n");
    let (_, first) = call(&state, Some("ww-caller"), json!({ "agent": "ww-target" })).await;
    let next = first["next"].as_u64().unwrap();

    let writer = {
        let (state, block) = (state.clone(), block.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            append(&state, &block, "second\n");
        })
    };
    let started = std::time::Instant::now();
    let (_, waited) = call(&state, Some("ww-caller"), json!({ "agent": "ww-target", "after": next, "wait_seconds": 10 })).await;
    writer.await.unwrap();
    assert_eq!(lines(&waited), ["second"]);
    assert!(started.elapsed() < std::time::Duration::from_secs(8), "it did not wait out the whole time");
}

#[tokio::test]
async fn an_agent_that_never_ran_has_nothing_to_watch_and_says_how_to_run_it() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "wn-caller").await;
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(json!({
        "id": "wn-fresh", "slug": "wn-fresh", "name": "Fresh", "icon": "robot", "provider": "claude",
        "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let (status, body) = call(&state, Some("wn-caller"), json!({ "agent": "wn-fresh" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["note"].as_str().unwrap().contains("Agents start"), "{body}");
    assert_eq!(lines(&body).len(), 0);
}

/// A line longer than one read: the tail widens its read to reach the line's end, a replay widens its
/// window to start on a line, and a line past the scan limit is skipped with a marker; the cursor never
/// stops in front of a line or lands inside one.
#[tokio::test]
async fn a_line_longer_than_a_read_neither_stalls_a_tail_nor_splits_a_replay() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "wl-caller").await;

    // A tail: a 600 KiB line, then a short one.
    let tail = seed_agent_block(&state, &tabs[0], "wl-tail").await;
    append(&state, &tail, &format!("{}
after
", "x".repeat(600 * 1024)));
    let (_, body) = call(&state, Some("wl-caller"), json!({ "agent": "wl-tail", "after": 0 })).await;
    let got = lines(&body);
    assert_eq!(got.len(), 2, "{} lines", got.len());
    assert!(got[0].starts_with("xxx") && got[0].ends_with("more characters]"));
    assert_eq!(got[1], "after");
    assert_eq!(body["next"].as_u64().unwrap(), (600 * 1024 + 1 + "after
".len()) as u64);

    // A replay whose last line is longer than the window: it starts on a line and next is the end.
    let replay = seed_agent_block(&state, &tabs[0], "wl-replay").await;
    append(&state, &replay, &format!("small
{}
", "y".repeat(300 * 1024)));
    let (_, body) = call(&state, Some("wl-caller"), json!({ "agent": "wl-replay" })).await;
    let got = lines(&body);
    assert_eq!(got[0], "small");
    assert!(got[1].starts_with("yyy"));
    let next = body["next"].as_u64().unwrap();
    append(&state, &replay, "more
");
    let (_, body) = call(&state, Some("wl-caller"), json!({ "agent": "wl-replay", "after": next })).await;
    assert_eq!(lines(&body), ["more"], "the follow-up does not return a fragment of the long line");

    // A line past the scan limit is skipped with a marker, and the next call is on the line after it.
    let huge = seed_agent_block(&state, &tabs[0], "wl-huge").await;
    append(&state, &huge, &format!("{}
after
", "z".repeat(2 * 1024 * 1024)));
    let (_, body) = call(&state, Some("wl-caller"), json!({ "agent": "wl-huge", "after": 0 })).await;
    assert!(lines(&body)[0].contains("too long to show and was skipped"), "{body}");
    let next = body["next"].as_u64().unwrap();
    let (_, body) = call(&state, Some("wl-caller"), json!({ "agent": "wl-huge", "after": next })).await;
    assert_eq!(lines(&body), ["after"]);
}

/// `more` counts the bytes past the window too: a watcher that drains while it is true reaches the end.
#[tokio::test]
async fn more_is_true_while_unread_bytes_remain_past_the_window() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    seed_agent_block(&state, &tabs[0], "wm-caller").await;
    let block = seed_agent_block(&state, &tabs[0], "wm-target").await;
    append(&state, &block, &format!("short
{}
last
", "x".repeat(600 * 1024)));
    let (_, first) = call(&state, Some("wm-caller"), json!({ "agent": "wm-target", "after": 0 })).await;
    assert_eq!(lines(&first), ["short"]);
    assert_eq!(first["more"], true, "two lines remain");
    let (_, second) = call(&state, Some("wm-caller"), json!({ "agent": "wm-target", "after": first["next"] })).await;
    assert_eq!(lines(&second).len(), 2);
    assert_eq!(second["more"], false);
}
