// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `process_jsonl_change` as a whole: what one change to a subagent's transcript records, and which events it
//! announces and in what order, for a solo dispatch and for a workflow member.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::types::*;
use super::super::SubagentWatcher;
use crate::backend::eventbus::EventBus;

fn watcher() -> SubagentWatcher {
    let mstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    SubagentWatcher::new(Arc::new(EventBus::new()), mstore.clone(), mstore.clone(), mstore)
}

fn text_line(text: &str) -> String {
    format!("{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n")
}

fn stamped_line(text: &str, timestamp: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{timestamp}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
    )
}

fn append(path: &Path, text: &str) {
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
}

/// A solo subagent's transcript path in a fresh directory.
fn solo_file(dir: &tempfile::TempDir, agent: &str, content: &str) -> PathBuf {
    let path = dir.path().join(format!("agent-{agent}.jsonl"));
    std::fs::write(&path, content).unwrap();
    path
}

/// A workflow member's transcript path: `subagents/workflows/<wf>/agent-<id>.jsonl`.
fn member_file(dir: &tempfile::TempDir, wf: &str, agent: &str, content: &str) -> PathBuf {
    let run_dir = dir.path().join("subagents").join("workflows").join(wf);
    std::fs::create_dir_all(&run_dir).unwrap();
    let path = run_dir.join(format!("agent-{agent}.jsonl"));
    std::fs::write(&path, content).unwrap();
    path
}

fn names(rx: &mut tokio::sync::mpsc::Receiver<serde_json::Value>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let Some(name) = msg.pointer("/data/data/event").and_then(|v| v.as_str()) {
            out.push(name.to_string());
        }
    }
    out
}

fn listen(w: &SubagentWatcher) -> tokio::sync::mpsc::Receiver<serde_json::Value> {
    w.event_bus.register_ws("test-conn", "test-tab").priority
}

#[test]
fn a_file_not_named_agent_dash_something_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.jsonl");
    std::fs::write(&path, text_line("hi")).unwrap();
    let w = watcher();
    let mut rx = listen(&w);

    w.process_jsonl_change("parent-1", "block-1", &path, true);

    assert!(w.sessions.lock().unwrap().is_empty());
    assert!(names(&mut rx).is_empty());
}

#[test]
fn a_transcript_that_cannot_be_read_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let w = watcher();
    let mut rx = listen(&w);

    w.process_jsonl_change("parent-1", "block-1", &dir.path().join("agent-gone.jsonl"), true);

    assert!(w.sessions.lock().unwrap().is_empty());
    assert!(names(&mut rx).is_empty());
}

#[test]
fn a_later_change_reads_only_the_lines_added_since() {
    let dir = tempfile::tempdir().unwrap();
    let path = solo_file(&dir, "inc", &text_line("one"));
    let w = watcher();
    let mut rx = listen(&w);

    w.process_jsonl_change("parent-1", "block-1", &path, true);
    append(&path, &(text_line("two") + &text_line("three")));
    w.process_jsonl_change("parent-1", "block-1", &path, true);

    let info = w.get_info("inc").unwrap();
    assert_eq!(info.event_count, 3);
    let kept: Vec<String> = {
        let sessions = w.sessions.lock().unwrap();
        let state = sessions.values().next().unwrap().subagents.get("inc").unwrap();
        state
            .events
            .iter()
            .map(|e| match &e.event_type {
                SubagentEventType::Text { content } => content.clone(),
                other => format!("{other:?}"),
            })
            .collect()
    };
    assert_eq!(kept, ["one", "two", "three"], "each line once, in order");

    let _ = names(&mut rx);
    w.process_jsonl_change("parent-1", "block-1", &path, true);
    assert_eq!(w.get_info("inc").unwrap().event_count, 3, "nothing added: nothing read again");
    assert!(names(&mut rx).is_empty(), "and nothing announced");
}

#[test]
fn the_kept_events_are_capped_but_the_count_is_the_true_total() {
    let dir = tempfile::tempdir().unwrap();
    let total = MAX_SUBAGENT_EVENTS + 5;
    let content: String = (0..total).map(|i| text_line(&i.to_string())).collect();
    let path = solo_file(&dir, "cap", &content);
    let w = watcher();

    w.process_jsonl_change("parent-1", "block-1", &path, true);

    assert_eq!(w.get_info("cap").unwrap().event_count, total);
    let sessions = w.sessions.lock().unwrap();
    let state = sessions.values().next().unwrap().subagents.get("cap").unwrap();
    assert_eq!(state.events.len(), MAX_SUBAGENT_EVENTS);
    let SubagentEventType::Text { content } = &state.events[0].event_type else { panic!("a text event") };
    assert_eq!(content, "5", "the oldest five were dropped");
}

#[test]
fn a_subagent_seen_again_under_another_block_keeps_the_block_it_was_first_seen_under() {
    let dir = tempfile::tempdir().unwrap();
    let path = solo_file(&dir, "mover", &text_line("one"));
    let w = watcher();

    w.process_jsonl_change("parent-1", "block-1", &path, true);
    append(&path, &text_line("two"));
    w.process_jsonl_change("parent-1", "block-2", &path, true);

    let info = w.get_info("mover").unwrap();
    assert_eq!(info.parent_block_id, "block-1");
    assert_eq!(info.event_count, 2, "the new line was still read");
}

#[test]
fn a_replayed_subagent_is_dated_by_its_events_not_by_the_replay() {
    let dir = tempfile::tempdir().unwrap();
    let content = stamped_line("late", "2020-01-02T00:00:00Z") + &stamped_line("early", "2020-01-01T00:00:00Z");
    let path = solo_file(&dir, "old", &content);
    let w = watcher();

    w.process_jsonl_change("parent-1", "block-old", &path, false);

    let info = w.get_info("old").unwrap();
    assert_eq!(info.spawned_at, 1_577_836_800_000, "the earliest event, not now");
    assert_eq!(info.last_event_at, 1_577_836_800_000, "the last line read");
}

#[test]
fn a_new_solo_subagent_announces_its_spawn_activity_and_dispatch_in_that_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = solo_file(&dir, "solo", &text_line("hi"));
    let w = watcher();
    let mut rx = listen(&w);

    w.process_jsonl_change("parent-1", "block-1", &path, true);

    assert_eq!(names(&mut rx), ["subagent:spawned", "subagent:activity", "dispatch:updated"]);
    let pending = w.pending_activity.lock().unwrap();
    let queued = pending.get("solo:solo").expect("also queued for dispatch:activity");
    assert_eq!(queued.members.len(), 1);
    assert!(queued.spawned.is_empty(), "a solo spawn was announced at once, not buffered");
}

#[test]
fn a_solo_subagent_that_finishes_announces_completion_then_the_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = solo_file(&dir, "fin", &text_line("hi"));
    let w = watcher();
    let mut rx = listen(&w);
    w.process_jsonl_change("parent-1", "block-1", &path, true);
    let _ = names(&mut rx);

    append(&path, "{\"type\":\"result\",\"result\":\"done\"}\n");
    w.process_jsonl_change("parent-1", "block-1", &path, true);

    assert_eq!(names(&mut rx), ["subagent:activity", "subagent:completed", "dispatch:updated"]);
    assert_eq!(w.get_info("fin").unwrap().status, SubAgentStatus::Completed);
}

#[test]
fn a_workflow_member_is_buffered_for_the_coalesced_flush_not_announced() {
    let dir = tempfile::tempdir().unwrap();
    let path = member_file(&dir, "wf_one", "m1", &text_line("hi"));
    let w = watcher();
    let mut rx = listen(&w);

    w.process_jsonl_change("parent-1", "block-1", &path, true);

    let immediate = names(&mut rx);
    assert!(
        !immediate.iter().any(|n| n == "subagent:spawned" || n == "subagent:activity" || n == "subagent:completed"),
        "{immediate:?}"
    );
    {
        let pending = w.pending_activity.lock().unwrap();
        let buffered = pending.get("wf_one").expect("buffered under the workflow's id");
        assert_eq!(buffered.spawned.len(), 1);
        assert_eq!(buffered.members.len(), 1);
        assert!(buffered.completed.is_empty());
    }

    w.flush_pending_dispatch_activity();
    let flushed = names(&mut rx);
    assert!(flushed.contains(&"subagent:spawned".to_string()), "{flushed:?}");
    assert!(flushed.contains(&"dispatch:activity".to_string()), "{flushed:?}");
}

#[test]
fn a_workflow_member_that_finishes_is_buffered_as_done() {
    let dir = tempfile::tempdir().unwrap();
    let path = member_file(&dir, "wf_two", "m1", &(text_line("hi") + "{\"type\":\"result\",\"result\":\"done\"}\n"));
    let w = watcher();

    w.process_jsonl_change("parent-1", "block-1", &path, true);

    let pending = w.pending_activity.lock().unwrap();
    let buffered = pending.get("wf_two").expect("buffered");
    assert_eq!(buffered.completed.len(), 1);
    assert_eq!(buffered.completed[0].agent_id, "m1");
    assert_eq!(buffered.completed[0].total_events, 2);
}

#[test]
fn a_description_that_lands_later_names_the_subagent_once_without_a_second_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let path = solo_file(&dir, "named", &text_line("hi"));
    let w = watcher();
    let mut rx = listen(&w);
    w.process_jsonl_change("parent-1", "block-1", &path, true);
    let _ = names(&mut rx);
    assert_eq!(w.get_info("named").unwrap().display_name, None);

    std::fs::write(
        dir.path().join("agent-named.meta.json"),
        r#"{"agentType":"general-purpose","description":"  Check   the   build ","spawnDepth":1}"#,
    )
    .unwrap();
    w.process_jsonl_change("parent-1", "block-1", &path, true);

    assert_eq!(names(&mut rx), ["subagent:named"]);
    assert_eq!(w.get_info("named").unwrap().display_name.as_deref(), Some("Check the build"));
    w.process_jsonl_change("parent-1", "block-1", &path, true);
    assert!(names(&mut rx).is_empty(), "named once");
}
