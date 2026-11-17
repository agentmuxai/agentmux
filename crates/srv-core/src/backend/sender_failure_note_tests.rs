// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::agents::test_agent_def;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

fn trigger_of(kind: &str, from: Option<&str>) -> serde_json::Value {
    let mut t = json!({ "kind": kind });
    if let Some(from) = from {
        t["from"] = json!(from);
    }
    t
}

fn turn(end: &str, kind: &str, from: Option<&str>) -> serde_json::Value {
    json!({ "block_id": "b1", "turn_id": 7, "ended_at_ms": 9, "end": end, "trigger": trigger_of(kind, from) })
}

/// A turn `trigger.from` started, joined by `absorbed` senders (each a `(kind, from)` pair).
fn turn_with_absorbed(trigger_from: &str, absorbed: &[(&str, &str)]) -> serde_json::Value {
    let mut t = turn("exited", "agent", Some(trigger_from));
    t["absorbed"] = json!(absorbed.iter().map(|(kind, from)| trigger_of(kind, Some(from))).collect::<Vec<_>>());
    t
}

#[test]
fn only_an_agents_jekt_that_exited_is_parsed() {
    assert_eq!(
        parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn("exited", "agent", Some("pulp")))),
        Some(SenderExit { block_id: "b1".into(), senders: vec!["pulp".into()], key: "b1/7/9".into() })
    );
    assert_eq!(parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn("completed", "agent", Some("pulp")))), None, "only an exit is a death, not an answer");
    assert_eq!(parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn("exited", "user", None))), None, "the user's own message has no sender to tell");
    assert_eq!(parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn("exited", "task", Some("a background task")))), None, "a woken task, not a message");
    assert_eq!(parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn("exited", "agent", None))), None, "no sender named");
    assert_eq!(parse_event("controllerstatus", Some(&json!({}))), None);
}

/// A sender who only joined an already-running turn (recorded in `absorbed`, not `trigger`) is still told —
/// the one `trigger` only read before dropped them (#2629). Every distinct agent sender is collected, in
/// order, deduplicated, and a non-agent absorbed sender (the user typing mid-turn) contributes nothing.
#[test]
fn a_sender_who_only_joined_the_turn_is_collected_from_absorbed() {
    assert_eq!(
        parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn_with_absorbed("pulp", &[("agent", "loap")]))),
        Some(SenderExit { block_id: "b1".into(), senders: vec!["pulp".into(), "loap".into()], key: "b1/7/9".into() })
    );
    // The same sender in both trigger and absorbed (or twice in absorbed) is told once.
    assert_eq!(
        parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn_with_absorbed("pulp", &[("agent", "pulp"), ("agent", "loap"), ("agent", "loap")]))),
        Some(SenderExit { block_id: "b1".into(), senders: vec!["pulp".into(), "loap".into()], key: "b1/7/9".into() })
    );
    // A turn the user started, joined mid-turn by an agent: the joiner is still told, with no trigger sender.
    let mut user_turn = turn("exited", "user", None);
    user_turn["absorbed"] = json!([trigger_of("agent", Some("loap"))]);
    assert_eq!(
        parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&user_turn)),
        Some(SenderExit { block_id: "b1".into(), senders: vec!["loap".into()], key: "b1/7/9".into() })
    );
    // An absorbed user input (someone typed into the pane mid-turn) names no agent sender.
    assert_eq!(
        parse_event(crate::backend::mps::EVENT_AGENT_TURN, Some(&turn_with_absorbed("pulp", &[("user", "")]))),
        Some(SenderExit { block_id: "b1".into(), senders: vec!["pulp".into()], key: "b1/7/9".into() })
    );
}

async fn dispatch_apply(state: &crate::test_support::CoreParts, cmd: Command) -> Vec<Event> {
    state.dispatch_apply(cmd).await
}

async fn tab(state: &crate::test_support::CoreParts) -> String {
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

async fn pane(state: &crate::test_support::CoreParts, tab: &str, def: &str) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab.into(), meta: json!({ "view": "agent", "agentId": def }) })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

fn mark_worker(state: &crate::test_support::CoreParts, worker_id: &str, lead: &crate::backend::storage::agents::AgentDefinition) {
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

/// A message's turn that died of a rate limit: its sender is told, naming the target and that only a mounted
/// pane retries it (#2627). No failure heard around the exit, or one that isn't retryable, says nothing.
#[tokio::test]
async fn a_senders_message_that_died_of_a_rate_limit_is_told_once() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (sender_id, target_id) = (format!("sender-{id}"), format!("target-{id}"));
    let mut sender = test_agent_def(&sender_id, &format!("Asker {}", &id[..8]), "claude", "host", 1, "");
    let mut target = test_agent_def(&target_id, &format!("Lowly {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut sender).unwrap();
    state.mstore.agent_def_insert(&mut target).unwrap();
    let target_block = pane(&state, &tab, &target_id).await;
    let heard_at = std::time::Instant::now();

    assert_eq!(note_for(&state.mstore, &target_block, &sender.slug, heard_at), None, "no failure heard yet");

    super::super::worker_failure_note::FAILED_AT.lock().record(&target_block, std::time::Instant::now());
    let failure = json!({ "code": "rate_limited", "title": "API rate limit", "detail": "", "retryable": true });
    dispatch_apply(&state, Command::UpdateBlockMeta { block_id: target_block.clone(), meta_patch: json!({ "agent:last_failure": failure }) }).await;
    let body = note_for(&state.mstore, &target_block, &sender.slug, heard_at).expect("the sender is told");
    assert!(body.starts_with(&format!("Your message to {} wasn't answered", target.name)), "{body}");
    assert!(body.contains("rate-limited it (HTTP 429)") && body.contains("send it another message"), "{body}");

    // A usage limit (not this feature's scope: it has its own path) says nothing.
    let limit = json!({ "code": "usage_limit", "title": "Usage limit reached", "retryable": false });
    dispatch_apply(&state, Command::UpdateBlockMeta { block_id: target_block.clone(), meta_patch: json!({ "agent:last_failure": limit }) }).await;
    assert_eq!(note_for(&state.mstore, &target_block, &sender.slug, heard_at), None);
}

/// The sender of a message to its OWN worker is not told twice: `worker_notes` already tells it, richer, so
/// `note_for` says nothing when the sender is that worker's open leader AND that note was actually admitted
/// (#2627, #2629). Not this worker's leader, or an unresolved name, still hears of its own lost message.
#[tokio::test]
async fn a_leader_messaging_its_own_worker_is_not_told_twice() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id) = (format!("lead-{id}"), format!("worker-{id}"));
    let other_id = format!("other-{id}");
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    let mut other = test_agent_def(&other_id, &format!("Other {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut lead).unwrap();
    state.mstore.agent_def_insert(&mut worker).unwrap();
    state.mstore.agent_def_insert(&mut other).unwrap();
    mark_worker(&state, &worker_id, &lead);
    pane(&state, &tab, &lead_id).await; // the leader's own pane is open
    let worker_block = pane(&state, &tab, &worker_id).await;
    let heard_at = std::time::Instant::now();
    super::super::worker_failure_note::FAILED_AT.lock().record(&worker_block, std::time::Instant::now());
    let failure = json!({ "code": "network", "title": "Network error", "retryable": true });
    dispatch_apply(&state, Command::UpdateBlockMeta { block_id: worker_block.clone(), meta_patch: json!({ "agent:last_failure": failure }) }).await;
    // worker_notes's own note for this failure was admitted (delivered), not held.
    super::super::worker_failure_note::record_failure_admission(&worker_block, true);
    assert_eq!(note_for(&state.mstore, &worker_block, &lead.slug, heard_at), None, "worker_notes already told the lead");
    // The skip is specific to this worker's own leader: anyone else who messaged it still hears of its own
    // lost message, and so does the unresolved name a sender's own slug would be if it were deleted mid-flight.
    assert!(note_for(&state.mstore, &worker_block, &other.slug, heard_at).is_some(), "not this worker's leader");
    assert!(note_for(&state.mstore, &worker_block, "no-such-agent", heard_at).is_some());
}

/// Even to its own leader: if `worker_notes`'s own note for this failure was HELD by its one-note-a-minute
/// throttle rather than admitted, the leader learned nothing from it — so the sender's own note must still go
/// through, or this exact failure is told to nobody at all (#2629).
#[tokio::test]
async fn a_held_worker_note_does_not_silence_the_senders_own_note() {
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
    let worker_block = pane(&state, &tab, &worker_id).await;
    let heard_at = std::time::Instant::now();
    super::super::worker_failure_note::FAILED_AT.lock().record(&worker_block, std::time::Instant::now());
    let failure = json!({ "code": "rate_limited", "title": "x", "retryable": true });
    dispatch_apply(&state, Command::UpdateBlockMeta { block_id: worker_block.clone(), meta_patch: json!({ "agent:last_failure": failure }) }).await;
    // Nothing recorded yet (worker_notes never reached its admit line for this block) is the conservative
    // default: tell the sender rather than assume a note went out that didn't.
    assert!(
        !super::super::worker_notes::already_tells_its_leader(&state.mstore, &worker_block, &lead.slug),
        "no record at all: not suppressed"
    );
    // worker_notes's own note for this failure was held back (another of the worker's notes used its minute).
    super::super::worker_failure_note::record_failure_admission(&worker_block, false);
    assert!(note_for(&state.mstore, &worker_block, &lead.slug, heard_at).is_some(), "not suppressed: nothing else told the lead about this one");
}
