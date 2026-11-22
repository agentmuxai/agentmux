// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Worker notes (SPEC_AGENT_WORKERS_2026_10_15.md §3.3).

use super::*;
use crate::backend::reactive::AuditLogEntry;
use crate::backend::storage::agents::test_agent_def;
use crate::test_support::CoreParts;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

#[test]
fn turn_ends_exits_and_failures_are_read_and_hidden_turns_are_not() {
    let turn = |end: &str, kind: &str| json!({ "block_id": "b1", "turn_id": 7, "started_at_ms": 5, "ended_at_ms": 9, "end": end, "trigger": { "kind": kind } });
    let t = crate::backend::mps::EVENT_AGENT_TURN;
    assert_eq!(
        parse_event(t, &[], Some(&turn("completed", "user"))),
        Some(WorkerEvent::TurnEnded { block_id: "b1".into(), started_at_ms: 5, ended_at_ms: 9, key: "b1/7/9".into() })
    );
    assert_eq!(parse_event(t, &[], Some(&turn("exited", "agent"))), Some(WorkerEvent::Exited { block_id: "b1".into(), key: "b1/7/9".into() }));
    assert_eq!(parse_event(t, &[], Some(&turn("completed", "system"))), None, "AgentMux's own hidden turn");
    assert_eq!(parse_event(t, &[], Some(&json!({ "block_id": "b1", "active": true }))), None, "a turn still running");
    let f = crate::backend::mps::EVENT_AGENT_FAILURE;
    assert_eq!(
        parse_event(f, &["block:b2".into()], Some(&json!({ "code": "usage_limit", "title": "Usage limit reached" }))),
        Some(WorkerEvent::Failed { block_id: "b2".into(), title: "Usage limit reached".into() })
    );
    assert_eq!(parse_event("controllerstatus", &[], Some(&json!({}))), None);
}

fn delivery(source: &str, block: &str, at: u64) -> AuditLogEntry {
    serde_json::from_value(json!({
        "timestamp": at, "source_agent": source, "target_agent": "lead", "block_id": block,
        "message_hash": "h", "message_length": 1, "success": true, "request_id": "r"
    }))
    .unwrap()
}

/// A report is kept past the delivery log's short ring: absorbed once, it is
/// still there when the ring has moved on (#5031).
#[test]
fn a_report_outlives_the_delivery_logs_ring() {
    let panes = vec!["lead-pane-r".to_string()];
    absorb(&[delivery("Worker-R1", "lead-pane-r", 2_000), delivery("worker-r1", "other-pane-r", 9_000)]);
    absorb(&[]); // the ring has moved on
    assert_eq!(last_report("worker-r1", &panes), Some(2_000), "any case; not the delivery to another pane");
    assert_eq!(last_report("worker-r2", &panes), None, "another worker");
    absorb(&[delivery("worker-r1", "lead-pane-r", 1_000)]);
    assert_eq!(last_report("worker-r1", &panes), Some(2_000), "the latest one");
}

/// A turn end published twice is handled once, so it isn't counted as held (#5031).
#[test]
fn a_turn_end_published_twice_is_handled_once() {
    let mut seen = Seen::default();
    let ev = WorkerEvent::TurnEnded { block_id: "b".into(), started_at_ms: 1, ended_at_ms: 9, key: "b/7/9".into() };
    assert!(seen.first(&ev));
    assert!(!seen.first(&ev));
    assert!(
        seen.first(&WorkerEvent::TurnEnded { block_id: "b".into(), started_at_ms: 1, ended_at_ms: 20, key: "b/8/20".into() }),
        "the next turn"
    );
    let failed = WorkerEvent::Failed { block_id: "b".into(), title: "x".into() };
    assert!(seen.first(&failed) && seen.first(&failed), "failures carry no turn identity");
}

#[test]
fn one_note_a_minute_per_worker_and_the_next_says_how_many_were_held() {
    let mut gaps = Gaps::default();
    assert_eq!(gaps.admit("w", "first".into(), 1_000).as_deref(), Some("first"));
    assert_eq!(gaps.admit("w", "second".into(), 2_000), None);
    assert_eq!(gaps.admit("w", "third".into(), 3_000), None);
    assert_eq!(gaps.admit("other", "else".into(), 3_000).as_deref(), Some("else"), "per worker");
    assert_eq!(
        gaps.admit("w", "fourth".into(), 1_000 + NOTE_GAP_MS).as_deref(),
        Some("fourth (2 earlier updates from it were held back.)")
    );
}

#[test]
fn the_note_says_whether_the_worker_is_waiting_and_quotes_it() {
    let asked = turn_end_body("Loap-4", LastWords { text: "Should I use the v2 API?".into(), asked: true });
    assert_eq!(asked, "Your worker Loap-4 is waiting for an answer and hasn't reported to you. Its last words: \"Should I use the v2 API?\" Answer it with Message send.");
    let bare = turn_end_body("Loap-4", LastWords { text: String::new(), asked: true });
    assert_eq!(bare, "Your worker Loap-4 is waiting for an answer and hasn't reported to you. Answer it with Message send.");
    assert_eq!(clip(&"word ".repeat(100), 10).chars().count(), 11);
}

async fn dispatch_apply(state: &CoreParts, cmd: Command) -> Vec<Event> {
    state.dispatch_apply(cmd).await
}

async fn pane(state: &CoreParts, tab: &str, def: &str) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab.into(), meta: json!({ "view": "agent", "agentId": def }) })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

async fn tab(state: &CoreParts) -> String {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateTab { workspace_id: ws, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn the_note_quotes_only_the_turn_that_ended() {
    // The turn's own message is quoted.
    assert_eq!(ended_turn_text("[user] go\n[assistant] Opened the page.\n[tool] Browser"), "Opened the page.");
    // A turn that was only tool calls has no words: the older turn's message is not quoted in its place.
    assert_eq!(ended_turn_text("[user] first\n[assistant] The pane is still blank.\n[user] test run done\n[tool] Bash\n[tool] Bash"), "");
    // After a compaction or restart the digest may open with an assistant message and no user entry: it still counts.
    assert_eq!(ended_turn_text("[assistant] Resumed.\n[tool] Bash"), "Resumed.");
    // The newest of several messages in the turn.
    assert_eq!(ended_turn_text("[user] go\n[assistant] Starting.\n[tool] Bash\n[assistant] Done, merged."), "Done, merged.");
    assert_eq!(ended_turn_text(""), "");
}

#[test]
fn a_tools_only_turn_repeats_no_old_words_in_a_note() {
    assert_eq!(ended_turn_text("[user] one\n[assistant] Old words.\n[user] two\n[tool] Bash"), "");
}

#[test]
fn a_note_names_a_worker_as_one_and_any_other_watched_agent_as_an_agent() {
    assert_eq!(named_for("Your worker Pulp stopped: its process exited.".into(), true), "Your worker Pulp stopped: its process exited.");
    assert_eq!(named_for("Your worker Pulp stopped: its process exited.".into(), false), "Your agent Pulp stopped: its process exited.");
    assert_eq!(named_for("Something else.".into(), false), "Something else.");
}

fn mark_worker(state: &CoreParts, worker_id: &str, lead: &crate::backend::storage::agents::AgentDefinition) {
    state
        .mstore
        .agent_content_set(&crate::backend::storage::store::AgentContent {
            agent_id: worker_id.to_string(),
            content_type: crate::backend::agent_seat::WORKER_OF_CONTENT.into(),
            content: json!({ "agent": lead.name, "id": lead.id }).to_string(),
            updated_at: 1,
        })
        .unwrap();
}

#[test]
fn a_question_is_read_from_the_panes_flag_being_set() {
    let meta = |patch: serde_json::Value| Event::BlockMetaUpdated { block_id: "b1".into(), meta_patch: patch, version: 1 };
    assert_eq!(asked_in(&meta(json!({ "term:awaiting_user": true }))), Some("b1"));
    assert_eq!(asked_in(&meta(json!({ "term:awaiting_user": null }))), None, "answered");
    assert_eq!(asked_in(&meta(json!({ "frame:title": "x" }))), None);
    assert_eq!(asked_in(&Event::TabMetaUpdated { tab_id: "t".into(), meta_patch: json!({ "term:awaiting_user": true }), version: 1 }), None);
}

/// A worker still waiting on a question after the grace notes its leader; one
/// answered by then, or whose turn is over, notes no one.
#[tokio::test]
async fn a_workers_question_mid_turn_notes_its_open_leader() {
    use crate::backend::agent_state::{test_support::register_stub, META_AWAITING_USER};
    use crate::backend::blockcontroller::{delete_controller, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id) = (format!("lead-{id}"), format!("worker-{id}"));
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut lead).unwrap();
    state.mstore.agent_def_insert(&mut worker).unwrap();
    mark_worker(&state, &worker_id, &lead);
    let lead_pane = pane(&state, &tab, &lead_id).await;
    let worker_pane = pane(&state, &tab, &worker_id).await;
    let asked = WorkerEvent::Asked { block_id: worker_pane.clone() };

    register_stub(&worker_pane, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &asked), None, "answered within the grace");

    dispatch_apply(&state, Command::UpdateBlockMeta { block_id: worker_pane.clone(), meta_patch: json!({ (META_AWAITING_USER): true }) }).await;
    let (to, text, wait) = note_for(&state.mstore, None, &state.filestore, &asked).expect("the leader is told");
    assert_eq!(wait, None, "a worker waiting on the user is told at once");
    assert_eq!(to, vec![lead_pane]);
    assert_eq!(
        text,
        format!("[AgentMux] Your worker {} is waiting for the user to answer a question in its pane; it can't go on until they do.", worker.name)
    );

    register_stub(&worker_pane, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &asked), None, "the turn is over: a flag left behind isn't a question");
    delete_controller(&worker_pane);
}

/// A worker's turn that ended with a question for its leader and no report notes its leader's open pane. A turn
/// that ended without a question is no note by itself (`quiet_watch` covers the worker that goes quiet); a pane
/// that isn't a worker's, or a worker whose leader isn't open, notes no one.
#[tokio::test]
async fn a_workers_turn_that_ends_asking_notes_its_open_leader_and_one_that_ends_silent_does_not() {
    use crate::backend::storage::filestore::{FileMeta, FileOpts};
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id) = (format!("lead-{id}"), format!("worker-{id}"));
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut lead).unwrap();
    state.mstore.agent_def_insert(&mut worker).unwrap();
    let worker_pane = pane(&state, &tab, &worker_id).await;
    state.filestore.make_file(&worker_pane, "output", FileMeta::default(), FileOpts::default()).unwrap();
    let say = |text: &str| {
        let line = json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": text }] } });
        state.filestore.append_lines(&worker_pane, "output", format!("{line}\n").as_bytes()).unwrap();
    };
    let ev = WorkerEvent::TurnEnded { block_id: worker_pane.clone(), started_at_ms: 0, ended_at_ms: 0, key: "k".into() };
    say("Should I use the v2 API?");
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &ev), None, "not marked as a worker");

    mark_worker(&state, &worker_id, &lead);
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &ev), None, "its leader isn't open");

    let lead_pane = pane(&state, &tab, &lead_id).await;
    let (to, text, wait) = note_for(&state.mstore, None, &state.filestore, &ev).expect("the leader is told");
    assert_eq!(wait, None, "a question is told at once");
    assert_eq!(to, vec![lead_pane]);
    assert_eq!(
        text,
        format!("[AgentMux] Your worker {} is waiting for an answer and hasn't reported to you. Its last words: \"Should I use the v2 API?\" Answer it with Message send.", worker.name)
    );

    // The next turn ends with a statement: no note at its end, however long the worker then says nothing.
    GAPS.lock().by_worker.remove(&worker_id);
    say("Done with the first half.");
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &ev), None, "a turn that ends without a report is not a note");
    let failed = WorkerEvent::Failed { block_id: worker_pane, title: "Usage limit reached".into() };
    assert!(note_for(&state.mstore, None, &state.filestore, &failed).is_some(), "a failure still is");
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &failed), None, "held: within a minute of the last note");
}

/// A route out of credits stops every agent on it: the route's own note tells the lead once, so no worker's
/// failure adds its own (#47449).
#[tokio::test]
async fn a_workers_credits_failure_adds_no_note_of_its_own() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id) = (format!("lead-{id}"), format!("worker-{id}"));
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut lead).unwrap();
    state.mstore.agent_def_insert(&mut worker).unwrap();
    mark_worker(&state, &worker_id, &lead);
    pane(&state, &tab, &lead_id).await;
    let worker_pane = pane(&state, &tab, &worker_id).await;
    let failed = |title: &str| WorkerEvent::Failed { block_id: worker_pane.clone(), title: title.into() };
    assert!(note_for(&state.mstore, None, &state.filestore, &failed("Not authenticated")).is_some(), "another failure is noted");
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &failed(crate::agents::failure::ROUTE_CREDITS_TITLE)), None);
}

/// A worker's turn that died of a rate limit: its failure's note names the class, that it is transient, and
/// who retries it (#2617); an exit with a failure stored by an earlier turn still says it stopped.
#[tokio::test]
async fn a_rate_limited_turns_note_names_the_cause_and_the_exit_adds_none() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id) = (format!("lead-{id}"), format!("worker-{id}"));
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut lead).unwrap();
    state.mstore.agent_def_insert(&mut worker).unwrap();
    mark_worker(&state, &worker_id, &lead);
    pane(&state, &tab, &lead_id).await;
    let worker_pane = pane(&state, &tab, &worker_id).await;
    let exited = WorkerEvent::Exited { block_id: worker_pane.clone(), key: "k".into() };
    let (_, plain, _) = note_for(&state.mstore, None, &state.filestore, &exited).expect("no failure stored: the exit is the news");
    assert_eq!(plain, format!("[AgentMux] Your worker {} stopped: its process exited.", worker.name));

    GAPS.lock().by_worker.remove(&worker_id); // the plain exit's note used this worker's minute
    let failure = json!({ "code": "rate_limited", "title": "API rate limit", "detail": "", "retryable": true });
    dispatch_apply(&state, Command::UpdateBlockMeta { block_id: worker_pane.clone(), meta_patch: json!({ "agent:last_failure": failure }) }).await;
    GAPS.lock().by_worker.remove(&worker_id);
    assert!(note_for(&state.mstore, None, &state.filestore, &exited).is_some(), "a failure stored by an earlier turn doesn't silence an exit");
    GAPS.lock().by_worker.remove(&worker_id);
    let failed = WorkerEvent::Failed { block_id: worker_pane, title: "API rate limit".into() };
    let (_, text, wait) = note_for(&state.mstore, None, &state.filestore, &failed).expect("the leader is told");
    assert_eq!(wait, None);
    assert!(text.contains("stopped with an error: API rate limit. The provider rate-limited it (HTTP 429)"), "{text}");
    assert!(text.contains("transient and has no reset time") && text.contains("send it another message"), "{text}");
}

#[test]
fn only_a_recent_turn_end_and_a_transcript_written_in_the_turn_count() {
    let now = 10 * TURN_END_FRESH_MS;
    assert!(turn_end_is_fresh(now - 1_000, now), "a turn that just ended");
    assert!(turn_end_is_fresh(now - TURN_END_FRESH_MS, now), "at the limit");
    assert!(!turn_end_is_fresh(now - TURN_END_FRESH_MS - 1, now), "an old turn, read again after a restart");
    assert!(turn_end_is_fresh(0, now), "an end that did not say when: judged by the other checks");

    assert!(written_since(Some(100_000), 99_000), "written after the turn began");
    assert!(written_since(Some(95_000), 99_000), "within the slack of its start");
    assert!(!written_since(Some(1_000), 99_000), "last written before the turn: an older session's file");
    assert!(!written_since(None, 99_000), "no file");
    assert!(written_since(Some(1_000), 0), "a start that did not say when");
}

/// A container agent's session resumed after days: its worker note fired twice mid-turn, quoting words of the old
/// session. An old turn end, and an end heard while the worker is in a turn, tell the leader nothing.
#[tokio::test]
async fn an_old_turn_end_or_one_heard_mid_turn_notes_no_one() {
    use crate::backend::agent_state::test_support::register_stub;
    use crate::backend::blockcontroller::{delete_controller, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id) = (format!("lead-{id}"), format!("worker-{id}"));
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut lead).unwrap();
    state.mstore.agent_def_insert(&mut worker).unwrap();
    mark_worker(&state, &worker_id, &lead);
    pane(&state, &tab, &lead_id).await;
    let worker_pane = pane(&state, &tab, &worker_id).await;
    let now = agentmux_common::time::now_ms_u64();
    let ended = |ended_at_ms: u64| WorkerEvent::TurnEnded { block_id: worker_pane.clone(), started_at_ms: 0, ended_at_ms, key: "k".into() };
    state.filestore.make_file(&worker_pane, "output", crate::backend::storage::filestore::FileMeta::default(), crate::backend::storage::filestore::FileOpts::default()).unwrap();
    let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Which branch should I use?"}]}}"#;
    state.filestore.append_lines(&worker_pane, "output", format!("{line}\n").as_bytes()).unwrap();

    assert_eq!(note_for(&state.mstore, None, &state.filestore, &ended(now - 2 * TURN_END_FRESH_MS)), None, "an end from long ago");

    register_stub(&worker_pane, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
    assert_eq!(note_for(&state.mstore, None, &state.filestore, &ended(now)), None, "the worker is in a turn: nothing has finished");

    register_stub(&worker_pane, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    let (_, text, _) = note_for(&state.mstore, None, &state.filestore, &ended(now)).expect("a turn that just ended asking, with the worker idle");
    assert!(text.starts_with(&format!("[AgentMux] Your worker {} is waiting for an answer", worker.name)), "{text}");
    delete_controller(&worker_pane);
}

/// The words a note quotes are the turn's own: a transcript last written before the turn began is an older
/// session's, and is not quoted.
#[tokio::test]
async fn a_note_does_not_quote_a_transcript_written_before_the_turn() {
    use crate::backend::storage::filestore::{FileMeta, FileOpts};
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let worker_id = format!("worker-{id}");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut worker).unwrap();
    let worker_pane = pane(&state, &tab, &worker_id).await;
    state.filestore.make_file(&worker_pane, "output", FileMeta::default(), FileOpts::default()).unwrap();
    let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Old words."}]}}"#;
    state.filestore.append_lines(&worker_pane, "output", format!("{line}\n").as_bytes()).unwrap();

    let now = agentmux_common::time::now_ms_u64();
    assert_eq!(last_words(&state.mstore, &state.filestore, &worker_pane, 0).text, "Old words.", "a start that is not known: read as before");
    assert_eq!(last_words(&state.mstore, &state.filestore, &worker_pane, now - 1_000).text, "Old words.", "written after the turn began");
    assert_eq!(
        last_words(&state.mstore, &state.filestore, &worker_pane, now + 3_600_000),
        LastWords::default(),
        "the turn began after the file was last written: those words are an older session's"
    );
}
