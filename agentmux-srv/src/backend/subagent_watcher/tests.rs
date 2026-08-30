// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for the whole `subagent_watcher` module tree. Kept as one file
//! (rather than split per-submodule) because most tests share fixture
//! helpers (`fixture_watcher`, `fixture_state`, `p`, `StubController`) that
//! cut across the lifecycle/query/scan/jsonl seams, and several tests
//! (e.g. the `reconcile_stale_subagents`/`scan_session_subagents`
//! end-to-end cases) deliberately exercise more than one of those layers at
//! once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::parse::*;
use super::types::*;
use super::*;

fn fixture_watcher() -> SubagentWatcher {
    let wstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    SubagentWatcher::new(Arc::new(EventBus::new()), wstore)
}

/// Write a minimal terminated subagent JSONL file with an explicit mtime
/// (`UNIX_EPOCH + offset_secs`), so backfill-ordering tests don't depend
/// on real wall-clock write speed / filesystem timestamp resolution.
fn write_agent_file_with_mtime(path: &Path, offset_secs: u64) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"{\"type\":\"result\",\"result\":\"done\"}\n").unwrap();
    f.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(offset_secs))
        .unwrap();
}

fn fixture_state(parent_agent: &str, agent_id: &str, session_id: &str) -> SubagentState {
    SubagentState {
        info: SubAgent {
            agent_id: agent_id.to_string(),
            slug: String::new(),
            jsonl_path: String::new(),
            parent_agent: parent_agent.to_string(),
            parent_block_id: String::new(),
            session_id: session_id.to_string(),
            spawned_at: 0,
            last_event_at: 0,
            status: SubAgentStatus::Active,
            event_count: 0,
            model: None,
            dispatch_id: solo_dispatch_id(agent_id),
            display_name: None,
            spawned_from_agent_id: None,
        },
        file_offset: 0,
        events: Vec::new(),
    }
}

#[test]
fn unwatch_agent_prunes_only_matching_parent_subagents() {
    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        // Two sessions; session "s1" has subagents from two different
        // parents, session "s2" has a subagent from a third parent.
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
        s1.subagents.insert("sub-b".to_string(), fixture_state("parent-2", "sub-b", "s1"));
        sessions.insert("s1".to_string(), s1);

        let mut s2 = SessionWatch { subagents: HashMap::new() };
        s2.subagents.insert("sub-c".to_string(), fixture_state("parent-1", "sub-c", "s2"));
        sessions.insert("s2".to_string(), s2);
    }

    watcher.unwatch_agent("parent-1", None);

    let sessions = watcher.sessions.lock().unwrap();
    // s1: parent-1's subagent gone, parent-2's remains.
    let s1 = sessions.get("s1").expect("s1 still has parent-2's subagent, should not be dropped");
    assert!(!s1.subagents.contains_key("sub-a"));
    assert!(s1.subagents.contains_key("sub-b"));
    // s2: its only subagent belonged to parent-1, so the whole session
    // entry is pruned (not left behind as an empty HashMap).
    assert!(!sessions.contains_key("s2"), "session left with zero subagents must be removed, not left empty");
}

#[test]
fn get_info_finds_a_subagent_by_id_without_scanning_the_full_list() {
    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
        s1.subagents.insert("sub-b".to_string(), fixture_state("parent-1", "sub-b", "s1"));
        sessions.insert("s1".to_string(), s1);
    }

    let found = watcher.get_info("sub-b").expect("sub-b should be found");
    assert_eq!(found.agent_id, "sub-b");
    assert_eq!(found.parent_agent, "parent-1");

    assert!(watcher.get_info("never-spawned").is_none());
}

#[test]
fn set_display_name_updates_info_and_reports_found() {
    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
        sessions.insert("s1".to_string(), s1);
    }

    assert!(watcher.set_display_name("sub-a", "Refactor shell module"));
    let info = watcher.get_info("sub-a").expect("sub-a should be found");
    assert_eq!(info.display_name.as_deref(), Some("Refactor shell module"));
}

#[test]
fn set_display_name_on_unknown_agent_is_noop_and_reports_not_found() {
    let watcher = fixture_watcher();
    assert!(!watcher.set_display_name("never-spawned", "Some name"));
}

#[test]
fn read_task_prompt_extracts_plain_string_content_from_first_line() {
    // Pre-existing bug fixed in passing: this and its two sibling tests
    // below all shared one directory keyed on std::process::id() (constant
    // for the whole test binary, not per-test) — under parallel test
    // execution, one test's std::fs::remove_dir_all teardown could race
    // another's still-in-progress create_dir_all/write/read, producing
    // flaky failures unrelated to what each test actually exercises.
    // now_millis() (already used elsewhere in this file for the same
    // per-test-uniqueness purpose) gives each test its own directory.
    let dir = std::env::temp_dir().join(format!("amx-test-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-prompt-string.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Analyze the shell module\"}}\n\
         {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n",
    )
    .unwrap();

    let prompt = read_task_prompt(jsonl_path.to_str().unwrap());
    assert_eq!(prompt.as_deref(), Some("Analyze the shell module"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn read_task_prompt_extracts_joined_text_blocks_from_content_array() {
    let dir = std::env::temp_dir().join(format!("amx-test-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-prompt-array.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Part one\"},{\"type\":\"text\",\"text\":\"Part two\"}]}}\n",
    )
    .unwrap();

    let prompt = read_task_prompt(jsonl_path.to_str().unwrap());
    assert_eq!(prompt.as_deref(), Some("Part one\nPart two"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn read_task_prompt_returns_none_when_first_line_is_not_a_user_record() {
    let dir = std::env::temp_dir().join(format!("amx-test-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-prompt-none.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
    )
    .unwrap();

    assert!(read_task_prompt(jsonl_path.to_str().unwrap()).is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unwatch_agent_on_unknown_agent_is_noop() {
    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
        sessions.insert("s1".to_string(), s1);
    }

    watcher.unwatch_agent("never-watched", None);

    let sessions = watcher.sessions.lock().unwrap();
    assert!(sessions.get("s1").unwrap().subagents.contains_key("sub-a"));
}

fn fixture_state_for_block(parent_block_id: &str, agent_id: &str, session_id: &str) -> SubagentState {
    let mut state = fixture_state("some-agent", agent_id, session_id);
    state.info.parent_block_id = parent_block_id.to_string();
    state
}

fn fixture_dispatch_state(dispatch_id: &str, parent_block_id: &str) -> DispatchState {
    DispatchState {
        info: AgentDispatch {
            dispatch_id: dispatch_id.to_string(),
            kind: DispatchKind::Workflow,
            parent_agent: "some-agent".to_string(),
            parent_block_id: parent_block_id.to_string(),
            session_id: "s1".to_string(),
            member_count: 1,
            members_done: 0,
            status: DispatchStatus::Running,
            last_event_at: 0,
            dispatch_name: None,
        },
        journal_offset: 0,
        journal_started: 0,
        journal_results: 0,
        member_files: 0,
        members_completed: 0,
    }
}

#[test]
fn prune_block_prunes_only_matching_parent_block_subagents() {
    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        s1.subagents.insert("sub-a".to_string(), fixture_state_for_block("block-1", "sub-a", "s1"));
        s1.subagents.insert("sub-b".to_string(), fixture_state_for_block("block-2", "sub-b", "s1"));
        sessions.insert("s1".to_string(), s1);

        let mut s2 = SessionWatch { subagents: HashMap::new() };
        s2.subagents.insert("sub-c".to_string(), fixture_state_for_block("block-1", "sub-c", "s2"));
        sessions.insert("s2".to_string(), s2);
    }

    let pruned = watcher.prune_block("block-1");
    assert!(pruned);

    let sessions = watcher.sessions.lock().unwrap();
    let s1 = sessions.get("s1").expect("s1 still has block-2's subagent, should not be dropped");
    assert!(!s1.subagents.contains_key("sub-a"));
    assert!(s1.subagents.contains_key("sub-b"));
    assert!(!sessions.contains_key("s2"), "session left with zero subagents must be removed, not left empty");
}

#[test]
fn prune_block_prunes_matching_dispatches_and_leaves_others() {
    let watcher = fixture_watcher();
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        dispatches.insert("wf_1".to_string(), fixture_dispatch_state("wf_1", "block-1"));
        dispatches.insert("wf_2".to_string(), fixture_dispatch_state("wf_2", "block-2"));
    }

    let pruned = watcher.prune_block("block-1");
    assert!(pruned);

    let dispatches = watcher.dispatches.lock().unwrap();
    assert!(!dispatches.contains_key("wf_1"));
    assert!(dispatches.contains_key("wf_2"));
}

#[test]
fn prune_block_prunes_matching_pending_activity_and_leaves_others() {
    let watcher = fixture_watcher();
    {
        let mut pending = watcher.pending_activity.lock().unwrap();
        pending.insert("wf_1".to_string(), PendingDispatchActivity::new("some-agent", "block-1", "s1"));
        pending.insert("wf_2".to_string(), PendingDispatchActivity::new("some-agent", "block-2", "s1"));
    }

    let pruned = watcher.prune_block("block-1");
    assert!(pruned);

    let pending = watcher.pending_activity.lock().unwrap();
    assert!(!pending.contains_key("wf_1"));
    assert!(pending.contains_key("wf_2"));
}

/// reagentx P2 (PR #2781, round 3): `backfill_generation` is keyed by
/// `parent_block_id` like every other per-block map `prune_block` already
/// cleans up above -- without pruning it too, every distinct block that
/// ever called `scan_session_subagents` would leak an entry for the life
/// of the srv process.
#[test]
fn prune_block_prunes_matching_backfill_generation_and_leaves_others() {
    let watcher = fixture_watcher();
    {
        let mut gens = watcher.backfill_generation.lock().unwrap();
        gens.insert("block-1".to_string(), 3);
        gens.insert("block-2".to_string(), 1);
    }

    watcher.prune_block("block-1");

    let gens = watcher.backfill_generation.lock().unwrap();
    assert!(!gens.contains_key("block-1"));
    assert!(gens.contains_key("block-2"));
}

#[test]
fn prune_block_on_unknown_block_is_noop_and_returns_false() {
    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        s1.subagents.insert("sub-a".to_string(), fixture_state_for_block("block-1", "sub-a", "s1"));
        sessions.insert("s1".to_string(), s1);
    }

    let pruned = watcher.prune_block("never-tracked");
    assert!(!pruned);

    let sessions = watcher.sessions.lock().unwrap();
    assert!(sessions.get("s1").unwrap().subagents.contains_key("sub-a"));
}

#[test]
fn unwatch_agent_also_prunes_matching_dispatches_and_pending_activity() {
    let watcher = fixture_watcher();
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        let mut d1 = fixture_dispatch_state("wf_1", "block-1");
        d1.info.parent_agent = "parent-1".to_string();
        dispatches.insert("wf_1".to_string(), d1);
        let mut d2 = fixture_dispatch_state("wf_2", "block-2");
        d2.info.parent_agent = "parent-2".to_string();
        dispatches.insert("wf_2".to_string(), d2);
    }
    {
        let mut pending = watcher.pending_activity.lock().unwrap();
        pending.insert("wf_1".to_string(), PendingDispatchActivity::new("parent-1", "block-1", "s1"));
        pending.insert("wf_2".to_string(), PendingDispatchActivity::new("parent-2", "block-2", "s1"));
    }

    watcher.unwatch_agent("parent-1", None);

    let dispatches = watcher.dispatches.lock().unwrap();
    assert!(!dispatches.contains_key("wf_1"));
    assert!(dispatches.contains_key("wf_2"));

    let pending = watcher.pending_activity.lock().unwrap();
    assert!(!pending.contains_key("wf_1"));
    assert!(pending.contains_key("wf_2"));
}

// ── session_belongs_to_block (docs/retro/retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md) ──

#[test]
fn session_belongs_to_block_matches_only_the_blocks_own_persisted_session() {
    let watcher = fixture_watcher();
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(
        crate::backend::blockcontroller::core::META_SESSION_ID.to_string(),
        serde_json::Value::String("s1".to_string()),
    );
    let mut block = crate::backend::obj::Block {
        oid: "block-1".to_string(),
        meta,
        ..Default::default()
    };
    watcher.wstore.insert(&mut block).unwrap();

    assert!(watcher.session_belongs_to_block("block-1", "s1"));
    assert!(
        !watcher.session_belongs_to_block("block-1", "s2"),
        "a different session id must not match, even for a real block"
    );
}

#[test]
fn session_belongs_to_block_is_false_for_a_block_that_no_longer_exists() {
    let watcher = fixture_watcher();
    assert!(
        !watcher.session_belongs_to_block("closed-block", "s1"),
        "a closed/deleted block owns nothing — must reject, not fall through"
    );
}

#[test]
fn subagent_events_are_capped_at_max() {
    let mut state = fixture_state("parent-1", "sub-a", "s1");
    // Simulate what process_jsonl_change's push+trim loop does, without
    // going through real JSONL files.
    for i in 0..(MAX_SUBAGENT_EVENTS + 100) {
        state.info.event_count += 1;
        state.events.push(SubagentEvent {
            agent_id: "sub-a".to_string(),
            event_type: SubagentEventType::Text { content: i.to_string() },
            timestamp: i as u64,
        });
    }
    if state.events.len() > MAX_SUBAGENT_EVENTS {
        let excess = state.events.len() - MAX_SUBAGENT_EVENTS;
        state.events.drain(..excess);
    }

    assert_eq!(state.events.len(), MAX_SUBAGENT_EVENTS);
    // event_count kept the true cumulative total despite truncation.
    assert_eq!(state.info.event_count, MAX_SUBAGENT_EVENTS + 100);
    // Oldest events were dropped — the retained window is the newest ones.
    let SubagentEventType::Text { content } = &state.events[0].event_type else {
        panic!("expected Text event");
    };
    assert_eq!(content, "100"); // first 100 (0..100) were trimmed away
}

#[test]
fn parse_event_type_result_line_with_content() {
    let value: serde_json::Value =
        serde_json::from_str(r#"{"type":"result","result":"final answer"}"#).unwrap();
    let parsed = parse_event_type(&value);
    assert!(matches!(
        parsed,
        Some(SubagentEventType::Result { content }) if content == "final answer"
    ));
}

#[test]
fn parse_event_type_result_line_without_content_falls_back() {
    // Real Claude Code result events always populate `result`/`content`;
    // this fallback only exists for malformed/unexpected lines.
    let value: serde_json::Value = serde_json::from_str(r#"{"type":"result"}"#).unwrap();
    let parsed = parse_event_type(&value);
    assert!(matches!(
        parsed,
        Some(SubagentEventType::Result { content }) if content == "Subagent completed"
    ));
}

#[test]
fn process_jsonl_change_marks_completed_on_result_event() {
    let dir = std::env::temp_dir().join(format!("amx-subagent-test-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-sub-a.jsonl");
    std::fs::write(
        &jsonl_path,
        concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
            "{\"type\":\"result\",\"result\":\"final answer\"}\n",
        ),
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    {
        let sessions = watcher.sessions.lock().unwrap();
        let session = sessions.values().next().expect("session recorded");
        let state = session.subagents.get("sub-a").expect("subagent recorded");
        assert_eq!(state.info.status, SubAgentStatus::Completed);
        assert!(matches!(
            state.events.last().unwrap().event_type,
            SubagentEventType::Result { .. }
        ));
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn process_jsonl_change_stays_active_without_result_event() {
    let dir = std::env::temp_dir().join(format!("amx-subagent-test-active-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-sub-b.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"still working\"}]}}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    {
        let sessions = watcher.sessions.lock().unwrap();
        let session = sessions.values().next().expect("session recorded");
        let state = session.subagents.get("sub-b").expect("subagent recorded");
        assert_eq!(state.info.status, SubAgentStatus::Active);
    }

    std::fs::remove_dir_all(&dir).ok();
}

// ── AgentDispatch (SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17) ──

#[test]
fn process_jsonl_change_parses_spawned_from_agent_id_from_parent_uuid() {
    // Empirically null in every real transcript checked (SPEC §9.2), but
    // the field is captured defensively — verify it round-trips when
    // present so a future real occurrence isn't silently dropped.
    let dir = std::env::temp_dir().join(format!("amx-subagent-parentuuid-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-child-a.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"parentUuid\":\"parent-turn-uuid-123\",\"agentId\":\"child-a\",\"type\":\"user\"}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    let sessions = watcher.sessions.lock().unwrap();
    let session = sessions.values().next().expect("session recorded");
    let state = session.subagents.get("child-a").expect("subagent recorded");
    assert_eq!(
        state.info.spawned_from_agent_id.as_deref(),
        Some("parent-turn-uuid-123")
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn process_jsonl_change_leaves_spawned_from_agent_id_none_when_parent_uuid_is_null() {
    let dir = std::env::temp_dir().join(format!("amx-subagent-parentuuid-null-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-child-b.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"parentUuid\":null,\"agentId\":\"child-b\",\"type\":\"user\"}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    let sessions = watcher.sessions.lock().unwrap();
    let session = sessions.values().next().expect("session recorded");
    let state = session.subagents.get("child-b").expect("subagent recorded");
    assert_eq!(state.info.spawned_from_agent_id, None);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_dispatches_synthesizes_a_solo_dispatch_for_a_loose_subagent() {
    let dir = std::env::temp_dir().join(format!("amx-solo-dispatch-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-solo-a.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    let dispatches = watcher.list_dispatches();
    assert_eq!(dispatches.len(), 1, "one solo dispatch, synthesized on demand");
    let d = &dispatches[0];
    assert_eq!(d.dispatch_id, "solo:solo-a");
    assert_eq!(d.kind, DispatchKind::Solo);
    assert_eq!(d.member_count, 1);
    assert_eq!(d.members_done, 0, "still active, not yet completed");
    assert_eq!(d.status, DispatchStatus::Running);

    // The same dispatch_id is stamped on the member itself.
    let active = watcher.list_active();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].dispatch_id, "solo:solo-a");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_dispatches_marks_solo_dispatch_done_once_its_member_completes() {
    let dir = std::env::temp_dir().join(format!("amx-solo-dispatch-done-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-solo-b.jsonl");
    std::fs::write(&jsonl_path, "{\"type\":\"result\",\"result\":\"done\"}\n").unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    let dispatches = watcher.list_dispatches();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].members_done, 1);
    assert_eq!(dispatches[0].status, DispatchStatus::Completed);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_dispatches_includes_a_tracked_workflow_dispatch_from_its_journal() {
    let dir = std::env::temp_dir().join(format!("amx-workflow-dispatch-{}", now_millis()));
    let run_dir = dir.join("subagents").join("workflows").join("wf_xyz789");
    std::fs::create_dir_all(&run_dir).unwrap();
    let journal_path = run_dir.join("journal.jsonl");
    std::fs::write(
        &journal_path,
        concat!(
            "{\"type\":\"started\",\"agent_id\":\"m1\"}\n",
            "{\"type\":\"started\",\"agent_id\":\"m2\"}\n",
        ),
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_journal_change("parent-1", "block-1", &journal_path);

    let dispatches = watcher.list_dispatches();
    assert_eq!(dispatches.len(), 1);
    let d = &dispatches[0];
    assert_eq!(d.dispatch_id, "wf_xyz789");
    assert_eq!(d.kind, DispatchKind::Workflow);
    assert_eq!(d.member_count, 2);
    assert_eq!(d.members_done, 0);
    assert_eq!(d.status, DispatchStatus::Running);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn workflow_member_activity_is_buffered_not_broadcast_immediately() {
    // SPEC §7: a Workflow-kind member's new events are coalesced into
    // pending_activity, not broadcast per-member — the direct fix for
    // the crash-storm mechanism in
    // docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md.
    let dir = std::env::temp_dir().join(format!("amx-coalesce-{}", now_millis()));
    let run_dir = dir.join("subagents").join("workflows").join("wf_coalesce1");
    std::fs::create_dir_all(&run_dir).unwrap();
    let jsonl_path = run_dir.join("agent-member-a.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    {
        let pending = watcher.pending_activity.lock().unwrap();
        let buffered = pending.get("wf_coalesce1").expect("activity buffered for this dispatch");
        assert_eq!(buffered.members.len(), 1);
        assert_eq!(buffered.members[0].0, "member-a");
    }

    // Flushing drains the buffer.
    watcher.flush_pending_dispatch_activity();
    {
        let pending = watcher.pending_activity.lock().unwrap();
        assert!(pending.is_empty(), "flush must drain every dispatch's buffer");
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// Issue: SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A —
/// a Workflow dispatch's `dispatch_id` is shared by every member, so
/// `naming_triggered` must claim it exactly once, on the first member's
/// live `is_new`, not once per member. `fixture_watcher()` has no
/// `self_ref` (built via bare `new()`), so `trigger_eager_naming` itself
/// no-ops after the gate — this test only exercises the synchronous
/// gating logic in `process_jsonl_change`, not the async Haiku call.
#[test]
fn process_jsonl_change_claims_naming_triggered_once_per_workflow_not_once_per_member() {
    let dir = std::env::temp_dir().join(format!("amx-naming-wf-{}", now_millis()));
    let run_dir = dir.join("subagents").join("workflows").join("wf_naming1");
    std::fs::create_dir_all(&run_dir).unwrap();

    let watcher = fixture_watcher();
    assert!(!watcher.naming_triggered_contains("wf_naming1"));

    for member in ["member-a", "member-b"] {
        let jsonl_path = run_dir.join(format!("agent-{member}.jsonl"));
        std::fs::write(
            &jsonl_path,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        )
        .unwrap();
        watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);
    }

    // Claimed after the FIRST member's live is_new — this is the
    // dedup key that keeps the eventual Haiku call to exactly one per
    // dispatch regardless of member count.
    assert!(watcher.naming_triggered_contains("wf_naming1"));

    std::fs::remove_dir_all(&dir).ok();
}

/// Issue: SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A —
/// the non-negotiable backfill guard: `live=false` (the exact value
/// `scan_subagents_dir`'s cold-backfill replay passes) must never claim
/// `naming_triggered`, even though `is_new` is still true for a file the
/// in-memory `sessions` map has never seen before (which is the whole
/// backfill mechanism). Without this, every srv restart / pane reopen
/// against a long-lived session would re-fire a Haiku call for every
/// dispatch replayed from history — the exact incident class in
/// docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md, just for
/// Haiku spend instead of WS broadcast volume.
#[test]
fn process_jsonl_change_never_claims_naming_triggered_during_backfill_replay() {
    let dir = std::env::temp_dir().join(format!("amx-naming-backfill-{}", now_millis()));
    let subagents_dir = dir.join("subagents");
    std::fs::create_dir_all(&subagents_dir).unwrap();
    let jsonl_path = subagents_dir.join("agent-replayed.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, false);

    assert!(
        !watcher.naming_triggered_contains("solo:replayed"),
        "backfill replay (live=false) must never claim naming_triggered"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Issue: agentmuxai/agentmux#2829 — `select_unnamed_backlog`'s core
/// contract: unnamed solo subagents come back most-recently-active first,
/// an already-named one is excluded, an unnamed Workflow dispatch comes
/// back too (using a current member as its representative), and every
/// returned item's `dispatch_id` ends up claimed in `naming_triggered`
/// (same dedup structure the live eager path uses) so a later live spawn
/// of the same dispatch can't double-fire naming.
#[test]
fn select_unnamed_backlog_returns_unnamed_items_most_recent_first_and_claims_them() {
    let watcher = fixture_watcher();
    let block_id = format!("backlog-basic-{}", now_millis());
    let dispatch_id = "wf-backlog-1";

    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };

        let mut older = fixture_state_for_block(&block_id, "sub-older", "s1");
        older.info.last_event_at = 100;
        s1.subagents.insert("sub-older".to_string(), older);

        let mut newer = fixture_state_for_block(&block_id, "sub-newer", "s1");
        newer.info.last_event_at = 200;
        s1.subagents.insert("sub-newer".to_string(), newer);

        let mut already_named = fixture_state_for_block(&block_id, "sub-named", "s1");
        already_named.info.last_event_at = 300;
        already_named.info.display_name = Some("Already named".to_string());
        s1.subagents.insert("sub-named".to_string(), already_named);

        // A representative member for the Workflow dispatch below — its
        // own dispatch_id is the workflow's, not "solo:...", so it must
        // never be selected as a solo candidate itself.
        let mut wf_member = fixture_state_for_block(&block_id, "sub-wf-member", "s1");
        wf_member.info.dispatch_id = dispatch_id.to_string();
        wf_member.info.last_event_at = 250;
        s1.subagents.insert("sub-wf-member".to_string(), wf_member);

        sessions.insert("s1".to_string(), s1);
    }
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        let mut wf = fixture_dispatch_state(dispatch_id, &block_id);
        wf.info.last_event_at = 150;
        dispatches.insert(dispatch_id.to_string(), wf);
    }

    let selected = watcher.select_unnamed_backlog(10);

    assert_eq!(selected.len(), 3, "the already-named solo subagent must be excluded");

    let dispatch_ids: Vec<String> = selected.iter().map(|item| item.dispatch_id()).collect();
    assert_eq!(
        dispatch_ids,
        vec!["solo:sub-newer".to_string(), dispatch_id.to_string(), "solo:sub-older".to_string()],
        "must be sorted most-recently-active first (200, 150, 100), by dispatch"
    );

    match &selected[1] {
        BacklogNamingItem::Workflow { dispatch_id: got_id, representative_agent_id } => {
            assert_eq!(got_id, dispatch_id);
            assert_eq!(representative_agent_id, "sub-wf-member");
        }
        BacklogNamingItem::Solo { .. } => panic!("expected the Workflow item at this position"),
    }

    for id in ["solo:sub-newer", "solo:sub-older", dispatch_id] {
        assert!(watcher.naming_triggered_contains(id), "{id} must be claimed after selection");
    }
    assert!(
        !watcher.naming_triggered_contains("solo:sub-named"),
        "an already-named subagent was never a candidate, so it must never be claimed either"
    );
}

/// A dispatch_id already claimed in `naming_triggered` (e.g. a live spawn
/// of the same dispatch won the race first) must never be re-selected by
/// the backlog pass — this is what keeps a live claim and a backlog claim
/// from ever double-firing naming for the same dispatch.
#[test]
fn select_unnamed_backlog_excludes_items_already_in_naming_triggered() {
    let watcher = fixture_watcher();
    let block_id = format!("backlog-already-claimed-{}", now_millis());
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        s1.subagents.insert("sub-a".to_string(), fixture_state_for_block(&block_id, "sub-a", "s1"));
        sessions.insert("s1".to_string(), s1);
    }
    {
        let mut naming_triggered = watcher.naming_triggered.lock().unwrap();
        naming_triggered.insert("solo:sub-a".to_string());
    }

    let selected = watcher.select_unnamed_backlog(10);
    assert!(selected.is_empty(), "an already-claimed dispatch_id must never be re-selected");
}

/// Two calls with a limit smaller than the backlog must return disjoint
/// sets — proves the claim inside `select_unnamed_backlog` actually
/// prevents re-selection, which is what makes it safe to fire on every
/// Swarm-pane-open with no extra debounce (`resolve_unnamed_backlog`'s doc
/// comment).
#[test]
fn select_unnamed_backlog_two_calls_return_disjoint_sets_and_respect_limit() {
    let watcher = fixture_watcher();
    let block_id = format!("backlog-disjoint-{}", now_millis());
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        for (i, agent_id) in ["sub-1", "sub-2", "sub-3", "sub-4", "sub-5"].iter().enumerate() {
            let mut state = fixture_state_for_block(&block_id, agent_id, "s1");
            state.info.last_event_at = i as u64;
            s1.subagents.insert(agent_id.to_string(), state);
        }
        sessions.insert("s1".to_string(), s1);
    }

    let first = watcher.select_unnamed_backlog(3);
    assert_eq!(first.len(), 3, "capped at the given limit");

    let second = watcher.select_unnamed_backlog(3);
    assert_eq!(second.len(), 2, "only the 2 remaining unclaimed items are left");

    let first_ids: std::collections::HashSet<String> = first.iter().map(|i| i.dispatch_id()).collect();
    let second_ids: std::collections::HashSet<String> = second.iter().map(|i| i.dispatch_id()).collect();
    assert!(first_ids.is_disjoint(&second_ids), "the two calls must never select the same dispatch_id twice");
    assert_eq!(first_ids.len() + second_ids.len(), 5, "together, every candidate is eventually drained");
}

/// A Workflow dispatch with no currently-visible member (e.g. its member
/// file hasn't been picked up by the filesystem watcher yet — the same
/// lag `reconcile_stale_subagents_does_not_abandon_a_workflow_dispatch_when_a_member_is_not_yet_visible`
/// guards against) has no task prompt to name from — must be silently
/// skipped, not panic or produce an item with an empty representative.
#[test]
fn select_unnamed_backlog_skips_a_workflow_dispatch_with_no_visible_member() {
    let watcher = fixture_watcher();
    let block_id = format!("backlog-no-member-{}", now_millis());
    let dispatch_id = "wf-backlog-orphan";
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        dispatches.insert(dispatch_id.to_string(), fixture_dispatch_state(dispatch_id, &block_id));
    }
    // No matching member ever inserted into `sessions`.

    let selected = watcher.select_unnamed_backlog(10);
    assert!(selected.is_empty());
    assert!(
        !watcher.naming_triggered_contains(dispatch_id),
        "a skipped (never-selected) dispatch must not be claimed either — it should be retried on a later pass"
    );
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s.replace('/', std::path::MAIN_SEPARATOR_STR))
}

#[test]
fn workflow_id_from_nested_member_path() {
    let path = p("projects/ws/sess-1/subagents/workflows/wf_abc123/agent-a1.jsonl");
    assert_eq!(parse_workflow_id(&path), Some("wf_abc123".to_string()));
}

#[test]
fn workflow_id_from_journal_path() {
    let path = p("projects/ws/sess-1/subagents/workflows/wf_abc123/journal.jsonl");
    assert_eq!(parse_workflow_id(&path), Some("wf_abc123".to_string()));
}

#[test]
fn workflow_id_none_for_direct_subagent() {
    let path = p("projects/ws/subagents/agent-a1.jsonl");
    assert_eq!(parse_workflow_id(&path), None);
}

#[test]
fn workflow_id_none_for_stray_file_in_workflows_dir() {
    let path = p("projects/ws/subagents/workflows/agent-a1.jsonl");
    assert_eq!(parse_workflow_id(&path), None);
}

#[test]
fn session_id_flat_layout() {
    let path = p("projects/proj-enc/subagents/agent-a1.jsonl");
    assert_eq!(derive_session_id(&path), "proj-enc");
}

#[test]
fn nearest_existing_ancestor_finds_first_existing_parent() {
    // Must live under the home dir — nearest_existing_ancestor's floor
    // (see its doc comment) would otherwise reject the whole path before
    // ever reaching `dir`. std::env::temp_dir() is NOT reliably under
    // home (e.g. plain /tmp on Linux CI runners), so build the temp path
    // from home_dir() directly.
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    let dir = home.join(format!("amx-ancestor-test-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();

    // dir exists; dir/a/b/c does not.
    let missing = dir.join("a").join("b").join("c");
    assert_eq!(nearest_existing_ancestor(&missing), Some(dir.clone()));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn nearest_existing_ancestor_returns_none_for_a_root_that_does_not_exist() {
    // A path with no ancestors at all (bare filename) has nothing to
    // walk up to — real callers always pass an absolute config dir, but
    // the function must not panic on this input.
    assert_eq!(nearest_existing_ancestor(Path::new("bare-name")), None);
}

/// Regression test for reagent's finding on PR #2008: without a floor,
/// a path whose entire ancestor chain up to and including the home
/// directory is missing would walk PAST home — risking a
/// `notify::Watcher::watch` recursive walk of an enormous, unrelated
/// tree. Must return None instead once the walk would have to cross the
/// home directory boundary.
#[test]
fn nearest_existing_ancestor_never_walks_above_the_home_directory() {
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    // Every ancestor from `missing` up through (and including) `home`
    // is guaranteed nonexistent (home itself always exists — it's the
    // real user's home dir — so nest deep enough that none of the
    // intermediate synthetic segments exist either).
    let missing = home
        .join("amx-never-created-1")
        .join("amx-never-created-2")
        .join("amx-never-created-3");
    // `home` itself exists, so the walk finds it — proving the floor is
    // inclusive of home, not exclusive.
    assert_eq!(nearest_existing_ancestor(&missing), Some(home));
}

/// Regression test for the observed bug: an agent without an explicit
/// per-identity bundle override launches under the shared default auth
/// dir (`~/.agentmux/shared/providers/claude/`), not
/// `derive_claude_config_dir`'s `~/.config/claude-<agent_id>` guess.
/// `resolve_claude_config_dir` must prefer the block's real `cmd:env`
/// over that guess whenever it's actually set.
#[test]
fn resolve_claude_config_dir_prefers_cmd_env_over_the_legacy_guess() {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(
        "cmd:env".to_string(),
        serde_json::json!({ "CLAUDE_CONFIG_DIR": "/agentmux/shared/providers/claude" }),
    );

    let resolved = resolve_claude_config_dir(&meta, "some-agent", None).unwrap();
    assert_eq!(resolved, PathBuf::from("/agentmux/shared/providers/claude"));
}

#[test]
fn resolve_claude_config_dir_falls_back_to_the_legacy_guess_when_cmd_env_is_absent() {
    let meta = crate::backend::obj::MetaMapType::new();
    let resolved = resolve_claude_config_dir(&meta, "SomeAgent", None).unwrap();
    assert_eq!(resolved, derive_claude_config_dir("SomeAgent").unwrap());
}

#[test]
fn resolve_claude_config_dir_falls_back_when_cmd_env_lacks_the_key() {
    let mut meta = crate::backend::obj::MetaMapType::new();
    // cmd:env is present but doesn't carry CLAUDE_CONFIG_DIR (e.g. a
    // non-Claude provider, or a race before the key is written).
    meta.insert("cmd:env".to_string(), serde_json::json!({ "OTHER_VAR": "x" }));

    let resolved = resolve_claude_config_dir(&meta, "SomeAgent", None).unwrap();
    assert_eq!(resolved, derive_claude_config_dir("SomeAgent").unwrap());
}

// SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md: for an
// identity-bound agent, `cmd:env.CLAUDE_CONFIG_DIR` is a stale launch-time
// snapshot of the GENERIC shared-provider dir — the real, identity-bound
// dir (what `resolve_bound_oauth_config_dir` returns) must win whenever
// it's available, regardless of what `cmd:env` says.
#[test]
fn resolve_claude_config_dir_prefers_the_identity_bound_dir_over_stale_cmd_env() {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(
        "cmd:env".to_string(),
        serde_json::json!({ "CLAUDE_CONFIG_DIR": "/agentmux/shared/providers/claude" }),
    );

    let bound = PathBuf::from("/agentmux/shared/identities/acct-1/claude");
    let resolved = resolve_claude_config_dir(&meta, "some-agent", Some(bound.clone())).unwrap();
    assert_eq!(resolved, bound);
}

#[tokio::test]
async fn watch_agent_falls_back_to_nearest_existing_ancestor_when_config_dir_is_missing() {
    // Regression test for the observed bug: watch_agent() is called from
    // the reactive-register handshake, which fires well before the CLI
    // process has created CLAUDE_CONFIG_DIR on disk. Watching a
    // nonexistent path used to fail outright with no retry, permanently
    // disabling subagent tracking for that agent's whole session.
    // Must live under the home dir — nearest_existing_ancestor's floor
    // would otherwise reject this path outright on platforms where
    // std::env::temp_dir() isn't under home (e.g. plain /tmp on Linux
    // CI runners).
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    let root = home.join(format!("amx-watch-fallback-test-{}", now_millis()));
    std::fs::create_dir_all(&root).unwrap(); // ancestor exists...
    let config_dir = root.join("claude-testagent"); // ...but this does not.
    assert!(!config_dir.exists());

    let watcher = Arc::new(fixture_watcher());
    watcher.watch_agent("test-agent", "block-1", config_dir.clone());

    // watch_agent must have succeeded (registered itself) instead of
    // bailing out — the old behavior returned early on the failed
    // notify::watch() call, before ever reaching this point.
    assert_eq!(watcher.watched_agents.lock().unwrap().len(), 1);

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn prune_block_also_tears_down_that_blocks_filesystem_watcher() {
    // Regression test for docs/retro/retro-subagent-watcher-shared-dir-
    // fanout-and-leak-2026-07-23.md Bug B: prune_block used to only clear
    // derived session/dispatch/pending-activity state, leaving the
    // underlying notify watcher (and its dedicated tokio task) running
    // forever for a closed block — which kept re-creating fresh
    // (mis-)attributed entries the next time another agent sharing its
    // watched directory wrote to its own subagent transcript.
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    let root = home.join(format!("amx-prune-watch-test-{}", now_millis()));
    std::fs::create_dir_all(&root).unwrap();

    let watcher = Arc::new(fixture_watcher());
    watcher.watch_agent("agent-1", "block-1", root.join("claude-agent-1"));
    watcher.watch_agent("agent-2", "block-2", root.join("claude-agent-2"));
    assert_eq!(watcher.watched_agents.lock().unwrap().len(), 2);

    watcher.prune_block("block-1");

    let watched = watcher.watched_agents.lock().unwrap();
    assert_eq!(watched.len(), 1, "prune_block must tear down the closed block's own watcher");
    assert!(
        watched[0].parent_block_ids.contains("block-2"),
        "a different, still-open block's watcher must be untouched"
    );
    drop(watched);

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn prune_block_does_not_kill_a_watcher_still_depended_on_by_another_block() {
    // Regression test for reagent's P1 on the first version of this fix:
    // watch_agent dedupes by agent_id, so two blocks registering the SAME
    // agent_id share one WatchedAgent entry. Pruning the first-registered
    // block must not silently kill live tracking for the second, still-open
    // block sharing that agent identity — only pruning BOTH should tear
    // down the underlying watcher.
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    let root = home.join(format!("amx-prune-shared-agent-test-{}", now_millis()));
    std::fs::create_dir_all(&root).unwrap();
    let config_dir = root.join("claude-shared-agent");

    let watcher = Arc::new(fixture_watcher());
    watcher.watch_agent("shared-agent", "block-1", config_dir.clone());
    watcher.watch_agent("shared-agent", "block-2", config_dir.clone());
    // Same agent_id dedupes to one entry, now depended on by both blocks.
    assert_eq!(watcher.watched_agents.lock().unwrap().len(), 1);

    watcher.prune_block("block-1");
    {
        let watched = watcher.watched_agents.lock().unwrap();
        assert_eq!(watched.len(), 1, "watcher must survive: block-2 still depends on it");
        assert!(!watched[0].parent_block_ids.contains("block-1"));
        assert!(watched[0].parent_block_ids.contains("block-2"));
    }

    watcher.prune_block("block-2");
    assert_eq!(
        watcher.watched_agents.lock().unwrap().len(),
        0,
        "once the last dependent block is pruned, the watcher must be torn down"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn unwatch_agent_does_not_kill_a_watcher_still_depended_on_by_another_block() {
    // Regression test for reagent's P1 on the second version of this fix:
    // unwatch_agent is the PRIMARY teardown path (called on every graceful
    // pane close via /agentmux/reactive/unregister — far more common than
    // prune_block's crash/API-delete backstop). It must respect the same
    // parent_block_ids dependency set prune_block/unwatch_block do, or two
    // blocks sharing one agent_id still kill each other's live tracking the
    // moment either one gracefully closes.
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    let root = home.join(format!("amx-unwatch-agent-shared-test-{}", now_millis()));
    std::fs::create_dir_all(&root).unwrap();
    let config_dir = root.join("claude-shared-agent");

    let watcher = Arc::new(fixture_watcher());
    watcher.watch_agent("shared-agent", "block-1", config_dir.clone());
    watcher.watch_agent("shared-agent", "block-2", config_dir.clone());
    assert_eq!(watcher.watched_agents.lock().unwrap().len(), 1);

    // block-1's process disconnects gracefully.
    watcher.unwatch_agent("shared-agent", Some("block-1"));
    {
        let watched = watcher.watched_agents.lock().unwrap();
        assert_eq!(watched.len(), 1, "watcher must survive: block-2 still depends on it");
        assert!(!watched[0].parent_block_ids.contains("block-1"));
        assert!(watched[0].parent_block_ids.contains("block-2"));
    }

    // block-2's process disconnects too — now the watcher must go.
    watcher.unwatch_agent("shared-agent", Some("block-2"));
    assert_eq!(
        watcher.watched_agents.lock().unwrap().len(),
        0,
        "once the last dependent block is unwatched, the watcher must be torn down"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn live_fs_event_is_not_misattributed_to_a_block_that_does_not_own_the_session() {
    // End-to-end regression test for docs/retro/retro-subagent-watcher-
    // shared-dir-fanout-and-leak-2026-07-23.md Bug A: two blocks share
    // one config_dir (the common case — every agent without a per-
    // identity bundle override resolves to the same default provider
    // path), so both watchers see the same raw filesystem event. Only
    // the block that actually owns the session (via its own persisted
    // agent:sessionid meta) may record the subagent; the other must
    // drop the event, not misattribute it to itself.
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    let config_dir = home.join(format!("amx-shared-dir-fanout-test-{}", now_millis()));
    let session_id = "owned-session";
    let subagents_dir = config_dir.join("projects").join("ws-enc").join(session_id).join("subagents");
    std::fs::create_dir_all(&subagents_dir).unwrap();

    let watcher = Arc::new(fixture_watcher());

    let mut owner_block = crate::backend::obj::Block {
        oid: "block-owner".to_string(),
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert(
                crate::backend::blockcontroller::core::META_SESSION_ID.to_string(),
                serde_json::Value::String(session_id.to_string()),
            );
            m
        },
        ..Default::default()
    };
    watcher.wstore.insert(&mut owner_block).unwrap();

    let mut other_block = crate::backend::obj::Block {
        oid: "block-other".to_string(),
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert(
                crate::backend::blockcontroller::core::META_SESSION_ID.to_string(),
                serde_json::Value::String("some-other-session".to_string()),
            );
            m
        },
        ..Default::default()
    };
    watcher.wstore.insert(&mut other_block).unwrap();

    // Both watch the SAME shared config_dir — simulating two agents
    // without a per-identity bundle override.
    watcher.watch_agent("agent-owner", "block-owner", config_dir.clone());
    watcher.watch_agent("agent-other", "block-other", config_dir.clone());

    std::fs::write(
        subagents_dir.join("agent-x.jsonl"),
        "{\"type\":\"result\",\"result\":\"done\"}\n",
    )
    .unwrap();

    // Debounce is 200ms; give the watcher's async task ample margin to
    // observe the write and process it.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let active = watcher.list_active();
    assert_eq!(active.len(), 1, "the subagent must be recorded exactly once");
    assert_eq!(
        active[0].parent_block_id, "block-owner",
        "must be attributed to the block that actually owns the session, never the other block sharing its watched directory"
    );

    std::fs::remove_dir_all(&config_dir).ok();
}

#[tokio::test]
async fn live_fs_event_with_empty_block_id_bypasses_the_ownership_check() {
    // Regression test for reagent's P1 on the first version of this fix:
    // the legacy/manual `subagent.WatchAgent` RPC entry point
    // (server/service/misc.rs) deliberately passes block_id="" for callers
    // with no pane to scope events to. No block has oid "", so the new
    // session-ownership gate must not apply to it — otherwise every live
    // event from that path is silently dropped.
    let home = dirs::home_dir().expect("test requires a resolvable home dir");
    let config_dir = home.join(format!("amx-empty-block-id-test-{}", now_millis()));
    let subagents_dir = config_dir.join("projects").join("ws-enc").join("some-session").join("subagents");
    std::fs::create_dir_all(&subagents_dir).unwrap();

    let watcher = Arc::new(fixture_watcher());
    watcher.watch_agent("manual-agent", "", config_dir.clone());

    std::fs::write(
        subagents_dir.join("agent-x.jsonl"),
        "{\"type\":\"result\",\"result\":\"done\"}\n",
    )
    .unwrap();

    tokio::time::sleep(Duration::from_millis(800)).await;

    let active = watcher.list_active();
    assert_eq!(active.len(), 1, "an empty block_id must not cause every live event to be dropped");
    assert_eq!(active[0].parent_block_id, "");

    std::fs::remove_dir_all(&config_dir).ok();
}

/// Regression test for the observed flood: reopening a pane for an
/// agent identity that has spawned subagents across many past sessions
/// (in this project) must only backfill the ONE session being resumed,
/// not every session the identity has ever run.
#[test]
fn scan_session_subagents_only_backfills_the_named_session() {
    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-session-test-{}", now_millis()));
    let target_session = "target-session-uuid";
    let other_session = "other-session-uuid";

    let target_dir = config_dir
        .join("projects")
        .join("ws-enc")
        .join(target_session)
        .join("subagents");
    let other_dir = config_dir
        .join("projects")
        .join("ws-enc")
        .join(other_session)
        .join("subagents");
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();

    std::fs::write(
        target_dir.join("agent-wanted.jsonl"),
        "{\"type\":\"result\",\"result\":\"done\"}\n",
    )
    .unwrap();
    std::fs::write(
        other_dir.join("agent-unwanted.jsonl"),
        "{\"type\":\"result\",\"result\":\"done\"}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.scan_session_subagents("parent-1", "block-1", &config_dir, target_session);

    let active = watcher.list_active();
    assert_eq!(active.len(), 1, "only the target session's subagent should be backfilled");
    assert_eq!(active[0].agent_id, "wanted");
    assert_eq!(active[0].session_id, target_session);

    std::fs::remove_dir_all(&config_dir).ok();
}

#[test]
fn scan_session_subagents_is_a_noop_for_an_unknown_session_id() {
    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-session-unknown-test-{}", now_millis()));
    let existing_dir = config_dir
        .join("projects")
        .join("ws-enc")
        .join("some-other-session")
        .join("subagents");
    std::fs::create_dir_all(&existing_dir).unwrap();
    std::fs::write(
        existing_dir.join("agent-a.jsonl"),
        "{\"type\":\"result\",\"result\":\"done\"}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.scan_session_subagents("parent-1", "block-1", &config_dir, "never-existed");

    assert!(watcher.list_active().is_empty(), "unknown session id must not fall back to scanning everything");

    std::fs::remove_dir_all(&config_dir).ok();
}

// ── subagent:backfill_status (docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md) ──

/// `scan_session_subagents` must publish "started" then "done", in that
/// order, regardless of whether the target session directory is actually
/// found -- the whole point is to bracket the pane's own backfill attempt,
/// not to report whether anything was backfilled.
#[test]
fn scan_session_subagents_publishes_started_then_done_when_session_is_found() {
    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-backfill-status-found-{}", now_millis()));
    let target_session = "target-session-uuid";
    let target_dir = config_dir.join("projects").join("ws-enc").join(target_session).join("subagents");
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(
        target_dir.join("agent-wanted.jsonl"),
        "{\"type\":\"result\",\"result\":\"done\"}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    let broker = Arc::new(crate::backend::wps::Broker::new());
    watcher.set_broker(broker.clone());
    watcher.scan_session_subagents("parent-1", "block-status-found", &config_dir, target_session);

    let history = broker.read_event_history(
        crate::backend::wps::EVENT_SUBAGENT_BACKFILL_STATUS,
        "block:block-status-found",
        10,
    );
    let statuses: Vec<Option<&str>> = history
        .iter()
        .map(|e| e.data.as_ref().and_then(|d| d.get("status")).and_then(|v| v.as_str()))
        .collect();
    assert_eq!(statuses, vec![Some("started"), Some("done")], "got: {statuses:?}");

    std::fs::remove_dir_all(&config_dir).ok();
}

/// The other half: even when the session directory is never found at all
/// (the existing `..._is_a_noop_for_an_unknown_session_id` case above),
/// "done" must still fire -- a pane whose backfill attempt found nothing
/// must not be left permanently gated as "still backfilling."
#[test]
fn scan_session_subagents_publishes_started_then_done_when_session_is_not_found() {
    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-backfill-status-notfound-{}", now_millis()));
    std::fs::create_dir_all(config_dir.join("projects")).unwrap();

    let watcher = fixture_watcher();
    let broker = Arc::new(crate::backend::wps::Broker::new());
    watcher.set_broker(broker.clone());
    watcher.scan_session_subagents("parent-1", "block-status-notfound", &config_dir, "never-existed");

    let history = broker.read_event_history(
        crate::backend::wps::EVENT_SUBAGENT_BACKFILL_STATUS,
        "block:block-status-notfound",
        10,
    );
    let statuses: Vec<Option<&str>> = history
        .iter()
        .map(|e| e.data.as_ref().and_then(|d| d.get("status")).and_then(|v| v.as_str()))
        .collect();
    assert_eq!(statuses, vec![Some("started"), Some("done")], "got: {statuses:?}");

    std::fs::remove_dir_all(&config_dir).ok();
}

/// A `SubagentWatcher` built via bare `fixture_watcher()` (no `set_broker`
/// call) must not panic -- every existing test in this file already
/// exercises this implicitly, but this pins the "no broker wired" no-op
/// posture explicitly, matching `self_ref`'s established convention.
#[test]
fn scan_session_subagents_does_not_panic_without_a_broker_wired() {
    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-backfill-status-nobroker-{}", now_millis()));
    std::fs::create_dir_all(config_dir.join("projects")).unwrap();

    let watcher = fixture_watcher();
    watcher.scan_session_subagents("parent-1", "block-1", &config_dir, "never-existed");

    std::fs::remove_dir_all(&config_dir).ok();
}

/// reagentx P2 (PR #2781, round 2): two overlapping `scan_session_subagents`
/// calls for the same `parent_block_id` (a block re-registered under a new
/// `agent_id` while an earlier scan for it is still in flight, see
/// `server/reactive.rs`'s caller comment) must not let the OLDER call's
/// "done" fire after a NEWER call has already started -- that would
/// prematurely clear the gate while the newer scan is still running. Tests
/// `is_backfill_generation_current` directly rather than fabricating real
/// thread-level concurrency in a synchronous unit test: the two
/// end-to-end tests above already prove the ordinary (non-overlapping)
/// single-caller path publishes "started" then "done" correctly.
#[test]
fn is_backfill_generation_current_returns_false_once_superseded() {
    let watcher = fixture_watcher();
    watcher.backfill_generation.lock().unwrap().insert("block-1".to_string(), 1);
    assert!(watcher.is_backfill_generation_current("block-1", 1));

    // A newer, overlapping call bumps the generation before generation 1's
    // own scan has finished.
    watcher.backfill_generation.lock().unwrap().insert("block-1".to_string(), 2);
    assert!(
        !watcher.is_backfill_generation_current("block-1", 1),
        "generation 1 must now be considered superseded"
    );
    assert!(watcher.is_backfill_generation_current("block-1", 2));
}

// ── scan_subagents_dir backfill cap (docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md) ──
//
// A long-lived pane's subagents/ directory accumulates forever; without a
// cap, every cold backfill (pane reopen, srv restart) replays the WHOLE
// history — a live incident hit 1,000+ replayed files across three
// back-to-back srv crash-restarts in under 10 seconds. These tests lock
// in the fix: the cap applies regardless of corpus size, and it always
// keeps the most RECENT files, not an arbitrary subset.

#[test]
fn scan_subagents_dir_caps_cold_backfill_to_the_most_recent_files() {
    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-backfill-cap-test-{}", now_millis()));
    let session_id = "backfill-cap-session";
    let subagents_dir = config_dir
        .join("projects")
        .join("ws-enc")
        .join(session_id)
        .join("subagents");
    std::fs::create_dir_all(&subagents_dir).unwrap();

    // One more file than the cap, mtimes strictly increasing by index —
    // "newest BACKFILL_MAX_FILES" is unambiguous regardless of directory
    // enumeration order.
    let total = BACKFILL_MAX_FILES + 1;
    for i in 0..total {
        let path = subagents_dir.join(format!("agent-id{i:04}.jsonl"));
        write_agent_file_with_mtime(&path, i as u64);
    }

    let watcher = fixture_watcher();
    watcher.scan_session_subagents("parent-1", "block-1", &config_dir, session_id);

    let active = watcher.list_active();
    assert_eq!(
        active.len(),
        BACKFILL_MAX_FILES,
        "cold backfill must not replay more than the cap regardless of corpus size"
    );
    assert!(
        !active.iter().any(|a| a.agent_id == "id0000"),
        "the single oldest file must be the one dropped"
    );
    assert!(
        active
            .iter()
            .any(|a| a.agent_id == format!("id{:04}", total - 1)),
        "the newest file must always survive the cap"
    );

    std::fs::remove_dir_all(&config_dir).ok();
}

#[test]
fn scan_subagents_dir_processes_workflow_journal_even_beyond_the_member_cap() {
    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-backfill-journal-test-{}", now_millis()));
    let session_id = "backfill-journal-session";
    let run_dir = config_dir
        .join("projects")
        .join("ws-enc")
        .join(session_id)
        .join("subagents")
        .join("workflows")
        .join("wf_test-run");
    std::fs::create_dir_all(&run_dir).unwrap();

    // More member files than the cap — the cap must still apply here...
    let total = BACKFILL_MAX_FILES + 5;
    for i in 0..total {
        let path = run_dir.join(format!("agent-id{i:04}.jsonl"));
        write_agent_file_with_mtime(&path, i as u64);
    }
    // ...but the run's journal (one small file, not one per member) is
    // always processed regardless — it drives `workflow:updated`/run
    // status, which must stay accurate even when membership is capped.
    std::fs::write(
        run_dir.join("journal.jsonl"),
        "{\"type\":\"started\",\"agent_id\":\"id0000\"}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.scan_session_subagents("parent-1", "block-1", &config_dir, session_id);

    assert_eq!(
        watcher.list_active().len(),
        BACKFILL_MAX_FILES,
        "member files are still capped inside a workflow run"
    );
    let dispatches = watcher.list_dispatches();
    assert_eq!(dispatches.len(), 1, "the run's journal must still be processed");
    assert_eq!(dispatches[0].dispatch_id, "wf_test-run");

    std::fs::remove_dir_all(&config_dir).ok();
}

// ── reconcile_stale_subagents ─────────────────────────────────────────
//
// A stub `Controller` so these tests can control what
// `get_block_controller_status` reports without spinning up a real
// subprocess. `CONTROLLER_REGISTRY` is process-global (shared across
// every test in this binary) — each test below registers its stub
// under a unique, per-test block id (never a literal shared with any
// other test) so parallel test execution can't cross-contaminate.

struct StubController {
    block_id: String,
    turn_active: bool,
}

impl crate::backend::blockcontroller::Controller for StubController {
    fn start(&self, _: crate::backend::obj::MetaMapType, _: Option<serde_json::Value>, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn stop(&self, _: bool, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn get_runtime_status(&self) -> crate::backend::blockcontroller::BlockControllerRuntimeStatus {
        crate::backend::blockcontroller::BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            turn_active: self.turn_active,
            ..Default::default()
        }
    }
    fn send_input(&self, _: crate::backend::blockcontroller::BlockInputUnion, _: Option<u64>) -> Result<(), String> {
        Ok(())
    }
    fn controller_type(&self) -> &str {
        "stub"
    }
    fn block_id(&self) -> &str {
        &self.block_id
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn register_stub_controller(block_id: &str, turn_active: bool) {
    crate::backend::blockcontroller::register_controller(
        block_id,
        Arc::new(StubController { block_id: block_id.to_string(), turn_active }),
    );
}

#[test]
fn reconcile_stale_subagents_downgrades_active_to_abandoned_when_parent_turn_is_confirmed_idle() {
    let block_id = format!("recon-idle-{}", now_millis());
    register_stub_controller(&block_id, false);

    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut state = fixture_state("parent-1", "sub-a", "s1");
        state.info.parent_block_id = block_id.clone();
        s1.subagents.insert("sub-a".to_string(), state);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&block_id, "s1");

    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Abandoned);
}

#[test]
fn reconcile_stale_subagents_leaves_active_alone_when_parent_turn_is_active() {
    let block_id = format!("recon-active-{}", now_millis());
    register_stub_controller(&block_id, true);

    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut state = fixture_state("parent-1", "sub-a", "s1");
        state.info.parent_block_id = block_id.clone();
        s1.subagents.insert("sub-a".to_string(), state);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&block_id, "s1");

    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Active, "a genuinely active parent turn must never be reconciled away");
}

#[test]
fn reconcile_stale_subagents_leaves_active_alone_when_no_controller_is_registered() {
    // No register_stub_controller call — block id is guaranteed unique
    // (per-test suffix) so get_block_controller_status returns None. The
    // public entry point's synchronous behavior is unchanged by the
    // None-retry fix: it queues a bounded one-shot retry
    // (`retry_reconcile_once`) rather than leaving the entry untouched
    // forever, but that retry itself silently no-ops here because
    // `fixture_watcher()` is built via bare `new()` (no `self_ref` to
    // upgrade to a real `Arc` — see `retry_reconcile_once`'s doc comment,
    // same "untracked -> safe no-op" convention `trigger_eager_naming`
    // uses). So immediately after this call, nothing has changed yet —
    // covered directly (not via a real spawned retry) by the
    // `_impl(..., allow_retry: false)` tests below.
    let block_id = format!("recon-unregistered-{}", now_millis());

    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut state = fixture_state("parent-1", "sub-a", "s1");
        state.info.parent_block_id = block_id.clone();
        s1.subagents.insert("sub-a".to_string(), state);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&block_id, "s1");

    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Active);
}

#[test]
fn reconcile_stale_subagents_impl_with_retry_exhausted_leaves_active_alone_when_no_controller_is_registered() {
    // The bounded-retry fix's terminal case: a second attempt (allow_retry:
    // false, as the real tokio::spawn'd retry calls it) with the controller
    // STILL unregistered must give up, not chain into another retry or
    // panic. See SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2.
    let block_id = format!("recon-unregistered-exhausted-{}", now_millis());

    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut state = fixture_state("parent-1", "sub-a", "s1");
        state.info.parent_block_id = block_id.clone();
        s1.subagents.insert("sub-a".to_string(), state);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents_impl(&block_id, "s1", false);

    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Active, "unregistered + retry exhausted must still not guess");
}

#[test]
fn reconcile_stale_subagents_impl_reconciles_normally_once_the_controller_registers_before_the_retry() {
    // The success path the retry exists for: controller was unregistered
    // on the first attempt, but has since registered (confirmed idle) by
    // the time the retry runs — reconciliation should proceed exactly as
    // if it had been confirmed idle from the start.
    let block_id = format!("recon-registers-before-retry-{}", now_millis());
    register_stub_controller(&block_id, false);

    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut state = fixture_state("parent-1", "sub-a", "s1");
        state.info.parent_block_id = block_id.clone();
        s1.subagents.insert("sub-a".to_string(), state);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents_impl(&block_id, "s1", false);

    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Abandoned);
}

#[test]
fn reconcile_stale_subagents_never_downgrades_an_already_completed_subagent() {
    let block_id = format!("recon-completed-{}", now_millis());
    register_stub_controller(&block_id, false);

    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut state = fixture_state("parent-1", "sub-a", "s1");
        state.info.parent_block_id = block_id.clone();
        state.info.status = SubAgentStatus::Completed;
        s1.subagents.insert("sub-a".to_string(), state);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&block_id, "s1");

    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Completed, "a subagent that genuinely finished must stay Completed, not be downgraded");
}

#[test]
fn reconcile_stale_subagents_never_touches_a_sibling_blocks_subagent_in_the_same_session() {
    // Two blocks can both have subagents recorded under the same
    // session_id (the watcher dedupes purely by agent_id — see
    // watch_agent's doc comment). reconcile_stale_subagents only has a
    // confirmed-idle read for the ONE block it was called with; a
    // sibling block sharing that session_id could still be genuinely
    // active, so its subagent must be left alone. Reagent P1 on #2131.
    let idle_block = format!("recon-sibling-idle-{}", now_millis());
    let active_block = format!("recon-sibling-active-{}", now_millis());
    register_stub_controller(&idle_block, false);
    register_stub_controller(&active_block, true);

    let watcher = fixture_watcher();
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut owned = fixture_state("parent-1", "sub-owned", "s1");
        owned.info.parent_block_id = idle_block.clone();
        let mut sibling = fixture_state("parent-2", "sub-sibling", "s1");
        sibling.info.parent_block_id = active_block.clone();
        s1.subagents.insert("sub-owned".to_string(), owned);
        s1.subagents.insert("sub-sibling".to_string(), sibling);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&idle_block, "s1");

    let owned_info = watcher.get_info("sub-owned").expect("sub-owned should still exist");
    assert_eq!(owned_info.status, SubAgentStatus::Abandoned, "this block's own subagent should still be reconciled");
    let sibling_info = watcher.get_info("sub-sibling").expect("sub-sibling should still exist");
    assert_eq!(sibling_info.status, SubAgentStatus::Active, "a sibling block's subagent must never be reconciled by an unrelated block's idle read");
}

#[test]
fn reconcile_stale_subagents_marks_workflow_dispatch_abandoned_when_all_members_done_and_one_abandoned() {
    // SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2's
    // aggregation rule: a Workflow-kind AgentDispatch's own status must
    // become Abandoned (not stay ambiguously Running/Completed) once every
    // member is Completed|Abandoned and at least one is genuinely Abandoned.
    let block_id = format!("recon-wf-abandon-{}", now_millis());
    register_stub_controller(&block_id, false);
    let dispatch_id = "wf-recon-1";

    let watcher = fixture_watcher();
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        dispatches.insert(dispatch_id.to_string(), fixture_dispatch_state(dispatch_id, &block_id));
    }
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut done_member = fixture_state_for_block(&block_id, "sub-done", "s1");
        done_member.info.dispatch_id = dispatch_id.to_string();
        done_member.info.status = SubAgentStatus::Completed;
        let mut still_active_member = fixture_state_for_block(&block_id, "sub-active", "s1");
        still_active_member.info.dispatch_id = dispatch_id.to_string();
        // Left Active — reconcile_stale_subagents will flip this one to
        // Abandoned, which is what should trigger the dispatch-level
        // aggregation (one member done+one already-completed = "all done,
        // at least one abandoned").
        s1.subagents.insert("sub-done".to_string(), done_member);
        s1.subagents.insert("sub-active".to_string(), still_active_member);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&block_id, "s1");

    let dispatches = watcher.list_dispatches();
    let d = dispatches.iter().find(|d| d.dispatch_id == dispatch_id).expect("dispatch should still exist");
    assert_eq!(d.status, DispatchStatus::Abandoned);
}

/// reagent P2 on PR #2677: a member whose JSONL file hasn't been picked up
/// by the filesystem watcher yet (async notify/debounce lag) is invisible
/// to `member_statuses_by_dispatch`, so the abandonment aggregation must
/// not trust `all_done` when it has fewer statuses than the dispatch's own
/// authoritative `member_count` — otherwise a dispatch could be marked
/// Abandoned based on an incomplete member set.
#[test]
fn reconcile_stale_subagents_does_not_abandon_a_workflow_dispatch_when_a_member_is_not_yet_visible() {
    let block_id = format!("recon-wf-missing-member-{}", now_millis());
    register_stub_controller(&block_id, false);
    let dispatch_id = "wf-recon-missing";

    let watcher = fixture_watcher();
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        let mut state = fixture_dispatch_state(dispatch_id, &block_id);
        // The dispatch itself believes it has 2 members (e.g. from journal
        // `started` records already read) — only 1 is visible below.
        state.info.member_count = 2;
        dispatches.insert(dispatch_id.to_string(), state);
    }
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut only_visible_member = fixture_state_for_block(&block_id, "sub-a", "s1");
        only_visible_member.info.dispatch_id = dispatch_id.to_string();
        // Left Active — reconcile flips it to Abandoned, which alone would
        // satisfy "all done, at least one abandoned" if the second,
        // not-yet-visible member weren't cross-checked against member_count.
        s1.subagents.insert("sub-a".to_string(), only_visible_member);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&block_id, "s1");

    let dispatches = watcher.list_dispatches();
    let d = dispatches.iter().find(|d| d.dispatch_id == dispatch_id).expect("dispatch should still exist");
    assert_eq!(
        d.status,
        DispatchStatus::Running,
        "must not abandon a dispatch whose member set is incomplete relative to its own member_count"
    );
}

#[test]
fn reconcile_stale_subagents_does_not_touch_workflow_dispatch_status_when_parent_turn_is_active() {
    let block_id = format!("recon-wf-active-{}", now_millis());
    register_stub_controller(&block_id, true);
    let dispatch_id = "wf-recon-2";

    let watcher = fixture_watcher();
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        dispatches.insert(dispatch_id.to_string(), fixture_dispatch_state(dispatch_id, &block_id));
    }
    {
        let mut sessions = watcher.sessions.lock().unwrap();
        let mut s1 = SessionWatch { subagents: HashMap::new() };
        let mut member = fixture_state_for_block(&block_id, "sub-a", "s1");
        member.info.dispatch_id = dispatch_id.to_string();
        s1.subagents.insert("sub-a".to_string(), member);
        sessions.insert("s1".to_string(), s1);
    }

    watcher.reconcile_stale_subagents(&block_id, "s1");

    let dispatches = watcher.list_dispatches();
    let d = dispatches.iter().find(|d| d.dispatch_id == dispatch_id).expect("dispatch should still exist");
    assert_eq!(d.status, DispatchStatus::Running, "a genuinely active parent turn must never abandon the dispatch");
}

/// SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §7 Open
/// Question 1: does live reconciliation racing a subagent's own
/// (not-yet-read) `Result` line permanently strand it at `Abandoned`?
/// No — `process_jsonl_change`'s completion check overwrites `status`
/// unconditionally on seeing `Result`, regardless of what it was before,
/// so a late-arriving completion always wins.
#[test]
fn reconcile_stale_subagents_then_late_result_line_ends_completed_not_stuck_abandoned() {
    let block_id = format!("recon-late-result-{}", now_millis());
    register_stub_controller(&block_id, false);

    let dir = std::env::temp_dir().join(format!("amx-recon-late-result-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-sub-a.jsonl");
    // Turn ends before the subagent's own Result line has been written —
    // only an assistant message is on disk so far.
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", &block_id, &jsonl_path, true);
    let info = watcher.get_info("sub-a").expect("sub-a should be tracked");
    assert_eq!(info.status, SubAgentStatus::Active);

    // `derive_session_id` returns "unknown" for a flat (non-`subagents/`-
    // nested) test path — matches this file's other flat-layout tests.
    watcher.reconcile_stale_subagents(&block_id, "unknown");
    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Abandoned);

    // The Result line lands moments later (fs-watcher debounce, or the
    // subagent process finishing its write just after the parent's own
    // turn-end fired) — appended, not rewritten, so file_offset tracking
    // from the first process_jsonl_change call stays valid.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&jsonl_path).unwrap();
        f.write_all(b"{\"type\":\"result\",\"result\":\"done\"}\n").unwrap();
    }
    watcher.process_jsonl_change("parent-1", &block_id, &jsonl_path, true);

    let info = watcher.get_info("sub-a").expect("sub-a should still exist");
    assert_eq!(info.status, SubAgentStatus::Completed, "a late-arriving Result line must win over an earlier Abandoned reconciliation, not be stuck behind it");

    std::fs::remove_dir_all(&dir).ok();
}

/// Dispatch-level counterpart (codex P2 on PR #2677): a Workflow dispatch
/// manually forced into `Abandoned` must NOT be stuck there once new member
/// evidence (a completing member) genuinely lands — `refresh_dispatch_info`
/// (triggered by that new evidence, via `update_dispatch_membership`) must
/// recompute, unlike the read-only `list_dispatches()` path which must NOT.
/// Recomputing immediately after fresh evidence yields `Running`, not
/// `Completed` (the 60s quiet window hasn't elapsed) — same lazy-completion
/// behavior a normal, never-abandoned dispatch already has; the point of
/// this test is only that it's no longer PERMANENTLY stuck at `Abandoned`.
#[test]
fn dispatch_marked_abandoned_is_not_stuck_once_new_member_evidence_lands() {
    let dir = std::env::temp_dir().join(format!("amx-dispatch-late-evidence-{}", now_millis()));
    let run_dir = dir.join("subagents").join("workflows").join("wf_late1");
    std::fs::create_dir_all(&run_dir).unwrap();
    let jsonl_path = run_dir.join("agent-member-a.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    // Force the dispatch into Abandoned, simulating a completed reconcile pass.
    {
        let mut dispatches = watcher.dispatches.lock().unwrap();
        let state = dispatches.get_mut("wf_late1").expect("dispatch should be tracked");
        state.info.status = DispatchStatus::Abandoned;
    }
    let abandoned = watcher.list_dispatches();
    let d = abandoned.iter().find(|d| d.dispatch_id == "wf_late1").unwrap();
    assert_eq!(d.status, DispatchStatus::Abandoned, "read-only list_dispatches() must never clobber Abandoned");

    // New member evidence lands: the member's own Result line.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&jsonl_path).unwrap();
        f.write_all(b"{\"type\":\"result\",\"result\":\"done\"}\n").unwrap();
    }
    watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path, true);

    let dispatches = watcher.list_dispatches();
    let d = dispatches.iter().find(|d| d.dispatch_id == "wf_late1").unwrap();
    assert_ne!(d.status, DispatchStatus::Abandoned, "new member evidence must be allowed to move the dispatch off Abandoned, not leave it permanently stuck");

    std::fs::remove_dir_all(&dir).ok();
}

/// End-to-end: a subagent JSONL with no terminal `result` line, backfilled
/// via a real `scan_session_subagents` call while the parent's turn is
/// confirmed idle, comes out `Abandoned` — not `Active` forever. This is
/// the exact user-reported symptom (SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md).
#[test]
fn scan_session_subagents_reconciles_an_unterminated_file_to_abandoned_when_parent_turn_is_idle() {
    let block_id = format!("recon-scan-{}", now_millis());
    register_stub_controller(&block_id, false);

    let config_dir = std::env::temp_dir()
        .join(format!("amx-scan-reconcile-test-{}", now_millis()));
    let session_id = "target-session-uuid";
    let target_dir = config_dir
        .join("projects")
        .join("ws-enc")
        .join(session_id)
        .join("subagents");
    std::fs::create_dir_all(&target_dir).unwrap();
    // No "type":"result" line — this subagent never got a terminal event,
    // simulating a crash/kill/interrupted-by-restart.
    std::fs::write(
        target_dir.join("agent-crashed.jsonl"),
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working...\"}]}}\n",
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.scan_session_subagents("parent-1", &block_id, &config_dir, session_id);

    let active = watcher.list_active();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].agent_id, "crashed");
    assert_eq!(active[0].status, SubAgentStatus::Abandoned);

    std::fs::remove_dir_all(&config_dir).ok();
}

#[test]
fn session_id_nested_workflow_layout() {
    let path = p("projects/ws/sess-uuid/subagents/workflows/wf_x/agent-a1.jsonl");
    assert_eq!(derive_session_id(&path), "sess-uuid");
}

#[test]
fn journal_counts_incremental() {
    let dir = std::env::temp_dir().join(format!("amx-journal-test-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let journal = dir.join("journal.jsonl");

    std::fs::write(
        &journal,
        "{\"type\":\"started\",\"agentId\":\"a1\"}\n{\"type\":\"result\",\"agentId\":\"a1\",\"result\":{}}\n",
    )
    .unwrap();
    let (started, results, offset) = read_journal_counts(&journal, 0).unwrap();
    assert_eq!((started, results), (1, 1));

    // Append two more records; re-read from the saved offset.
    let mut existing = std::fs::read(&journal).unwrap();
    existing.extend_from_slice(
        b"{\"type\":\"started\",\"agentId\":\"a2\"}\n{\"type\":\"started\",\"agentId\":\"a3\"}\n",
    );
    std::fs::write(&journal, existing).unwrap();
    let (started2, results2, _) = read_journal_counts(&journal, offset).unwrap();
    assert_eq!((started2, results2), (2, 0));

    std::fs::remove_dir_all(&dir).ok();
}

/// Regression test for a race where the journal writer has flushed a
/// record's bytes but not yet its trailing `\n` (mid-`write!` on a
/// concurrently-appended file). The unterminated line must be neither
/// counted nor consumed — `new_offset` should sit exactly at its start —
/// so the next read picks it up whole once the newline lands, instead of
/// silently losing the record.
#[test]
fn journal_counts_skips_unterminated_trailing_line() {
    let dir = std::env::temp_dir().join(format!("amx-journal-test-partial-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let journal = dir.join("journal.jsonl");

    // One complete record, then a partial record with no trailing newline.
    let first_line = "{\"type\":\"started\",\"agentId\":\"a1\"}\n";
    let partial_line = "{\"type\":\"started\",\"agentId\":\"a2";
    std::fs::write(&journal, format!("{first_line}{partial_line}")).unwrap();

    let (started, results, offset) = read_journal_counts(&journal, 0).unwrap();
    assert_eq!((started, results), (1, 0), "partial trailing line must not be counted");
    assert_eq!(
        offset, first_line.len() as u64,
        "offset must stop at the start of the partial line, not past it"
    );

    // The writer finishes the line; a re-read from the same offset must
    // now see the complete record rather than a truncated/corrupted one.
    let mut existing = std::fs::read(&journal).unwrap();
    existing.extend_from_slice(b"\"}\n");
    std::fs::write(&journal, existing).unwrap();
    let (started2, results2, _) = read_journal_counts(&journal, offset).unwrap();
    assert_eq!((started2, results2), (1, 0), "completed line must be picked up whole, not dropped");

    std::fs::remove_dir_all(&dir).ok();
}

// ── backfill replay must not assert false-Active state ───────────────
//
// docs/reports/REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md §2:
// a cold backfill replayed ~200 historical spawns straight into the live
// `Active` set, and `reconcile_stale_subagents` then retracted all 200 a few
// hundred ms later — while the pane was still loading. For that window
// `subagent.ListActive` (the RPC the Activity Dock's rows are built from)
// genuinely returned rows the backend was about to disown, which is what the
// dock rendered as rows appearing and vanishing on every reopen.
//
// These cover the insert-time decision only. The reconcile pass keeps its own
// tests above; it remains the authority whenever the insert can't decide.

/// Writes a single-line transcript with no `result` event — i.e. the file
/// alone gives no reason to think the subagent finished.
fn write_unfinished_transcript(dir: &std::path::Path, agent_id: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(format!("agent-{agent_id}.jsonl"));
    std::fs::write(
        &p,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}\n",
    )
    .unwrap();
    p
}

#[test]
fn backfill_replay_is_born_abandoned_when_the_parent_turn_is_confirmed_idle() {
    let block_id = format!("backfill-idle-{}", now_millis());
    register_stub_controller(&block_id, false);
    let dir = std::env::temp_dir().join(format!("amx-backfill-idle-{}", now_millis()));
    let jsonl_path = write_unfinished_transcript(&dir, "sub-replay");

    let watcher = fixture_watcher();
    // live: false — this is a cold-backfill replay, not a real spawn.
    watcher.process_jsonl_change("parent-1", &block_id, &jsonl_path, false);

    let info = watcher.get_info("sub-replay").expect("subagent recorded");
    assert_eq!(
        info.status,
        SubAgentStatus::Abandoned,
        "a replayed spawn under a confirmed-idle parent must never enter the Active set"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn live_spawn_stays_active_even_when_the_parent_turn_reads_idle() {
    // The turn_active flag can lag a genuine spawn, so `live` must win — this
    // is the regression that would break real-time subagent rows.
    let block_id = format!("backfill-live-{}", now_millis());
    register_stub_controller(&block_id, false);
    let dir = std::env::temp_dir().join(format!("amx-backfill-live-{}", now_millis()));
    let jsonl_path = write_unfinished_transcript(&dir, "sub-live");

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", &block_id, &jsonl_path, true);

    let info = watcher.get_info("sub-live").expect("subagent recorded");
    assert_eq!(info.status, SubAgentStatus::Active);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn backfill_replay_stays_active_when_the_parent_turn_is_running() {
    // A live turn can legitimately own still-running subagents whose files are
    // already on disk, so the replay must not disown them. Same predicate
    // `reconcile_stale_subagents` uses, applied earlier.
    let block_id = format!("backfill-busy-{}", now_millis());
    register_stub_controller(&block_id, true);
    let dir = std::env::temp_dir().join(format!("amx-backfill-busy-{}", now_millis()));
    let jsonl_path = write_unfinished_transcript(&dir, "sub-busy");

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", &block_id, &jsonl_path, false);

    let info = watcher.get_info("sub-busy").expect("subagent recorded");
    assert_eq!(info.status, SubAgentStatus::Active);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn backfill_replay_stays_active_when_no_controller_is_registered() {
    // Unknown, not idle: `scan_session_subagents` can run before the
    // controller registers. Falling back to today's behaviour keeps
    // `reconcile_stale_subagents`'s retry path as the authority rather than
    // guessing Abandoned from absence of information.
    let block_id = format!("backfill-unknown-{}", now_millis());
    let dir = std::env::temp_dir().join(format!("amx-backfill-unknown-{}", now_millis()));
    let jsonl_path = write_unfinished_transcript(&dir, "sub-unknown");

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", &block_id, &jsonl_path, false);

    let info = watcher.get_info("sub-unknown").expect("subagent recorded");
    assert_eq!(info.status, SubAgentStatus::Active);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_finished_replayed_transcript_is_completed_not_abandoned() {
    // The file's own `result` event outranks the turn-idle inference — a
    // subagent that demonstrably finished its work is Completed, and must not
    // be relabelled as abandoned just because it arrived via backfill.
    let block_id = format!("backfill-done-{}", now_millis());
    register_stub_controller(&block_id, false);
    let dir = std::env::temp_dir().join(format!("amx-backfill-done-{}", now_millis()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl_path = dir.join("agent-sub-done.jsonl");
    std::fs::write(
        &jsonl_path,
        concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
            "{\"type\":\"result\",\"result\":\"final answer\"}\n",
        ),
    )
    .unwrap();

    let watcher = fixture_watcher();
    watcher.process_jsonl_change("parent-1", &block_id, &jsonl_path, false);

    let info = watcher.get_info("sub-done").expect("subagent recorded");
    assert_eq!(info.status, SubAgentStatus::Completed);

    std::fs::remove_dir_all(&dir).ok();
}
