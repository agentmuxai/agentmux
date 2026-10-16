// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Worker notes (SPEC_AGENT_WORKERS_2026_10_15.md §3.3).

use super::*;
use crate::backend::reactive::AuditLogEntry;
use crate::backend::storage::agents::test_agent_def;
use crate::server::AppState;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

#[test]
fn turn_ends_exits_and_failures_are_read_and_hidden_turns_are_not() {
    let turn = |end: &str, kind: &str| json!({ "block_id": "b1", "started_at_ms": 5, "end": end, "trigger": { "kind": kind } });
    let t = crate::backend::mps::EVENT_AGENT_TURN;
    assert_eq!(parse_event(t, &[], Some(&turn("completed", "user"))), Some(WorkerEvent::TurnEnded { block_id: "b1".into(), started_at_ms: 5 }));
    assert_eq!(parse_event(t, &[], Some(&turn("exited", "agent"))), Some(WorkerEvent::Exited { block_id: "b1".into() }));
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

#[test]
fn a_turn_with_a_message_to_the_leader_is_reported() {
    let panes = vec!["lead-pane".to_string()];
    let audit = vec![delivery("worker-1", "lead-pane", 2_000)];
    assert!(reported(&audit, "Worker-1", &panes, 2_500), "during the turn (a second's slack), any case");
    assert!(!reported(&audit, "worker-1", &panes, 9_000), "before the turn");
    assert!(!reported(&audit, "worker-2", &panes, 0), "another worker");
    assert!(!reported(&[delivery("worker-1", "other-pane", 2_000)], "worker-1", &panes, 0), "not to the leader");
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
    let done = turn_end_body("Loap-4", LastWords::default());
    assert_eq!(done, "Your worker Loap-4 finished a turn without reporting to you.");
    assert_eq!(clip(&"word ".repeat(100), 10).chars().count(), 11);
}

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

async fn pane(state: &AppState, tab: &str, def: &str) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab.into(), meta: json!({ "view": "agent", "agentId": def }) })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

/// A worker's turn that ended without a report notes its leader's open pane;
/// a pane that isn't a worker's, or a worker whose leader isn't open, notes no one.
#[tokio::test]
async fn a_workers_unreported_turn_notes_its_open_leader() {
    let state = crate::server::tests::test_state();
    let ws = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab = dispatch_apply(&state, Command::CreateTab { workspace_id: ws, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id) = (format!("lead-{id}"), format!("worker-{id}"));
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut lead).unwrap();
    state.mstore.agent_def_insert(&mut worker).unwrap();
    let worker_pane = pane(&state, &tab, &worker_id).await;
    let ev = WorkerEvent::TurnEnded { block_id: worker_pane.clone(), started_at_ms: 0 };
    assert_eq!(note_for(&state.mstore, &state.filestore, &ev), None, "not marked as a worker");

    state
        .mstore
        .agent_content_set(&crate::backend::storage::store::AgentContent {
            agent_id: worker_id.clone(),
            content_type: crate::server::app_api::agent_spawn::WORKER_OF_CONTENT.into(),
            content: json!({ "agent": lead.name, "id": lead_id }).to_string(),
            updated_at: 1,
        })
        .unwrap();
    assert_eq!(note_for(&state.mstore, &state.filestore, &ev), None, "its leader isn't open");

    let lead_pane = pane(&state, &tab, &lead_id).await;
    let (to, text) = note_for(&state.mstore, &state.filestore, &ev).expect("the leader is told");
    assert_eq!(to, lead_pane);
    assert_eq!(text, format!("[AgentMux] Your worker {} finished a turn without reporting to you.", worker.name));
    let failed = WorkerEvent::Failed { block_id: worker_pane, title: "Usage limit reached".into() };
    assert_eq!(note_for(&state.mstore, &state.filestore, &failed), None, "held: within a minute of the last note");
}
