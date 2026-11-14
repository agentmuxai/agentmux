// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The wire shape of the shutdown events and the names they are sent under. The frontend reads these
//! field names and the event names, so they are pinned here whichever module holds the types.

use serde_json::json;

use super::close_pane::{ShutdownExtra, ShutdownLineEvent, ShutdownProcess, EVENT_AGENT_SHUTDOWN};
use super::pending_shutdown::{
    BatchBlock, BlockOutcome, PendingClearedEvent, PendingView, EVENT_SHUTDOWN_PENDING, EVENT_SHUTDOWN_PENDING_CLEARED,
};

fn view() -> PendingView {
    PendingView {
        request_id: "req-1".into(),
        block_id: "blk-1".into(),
        by: "qtest".into(),
        target: "lark".into(),
        via: "ClosePane".into(),
        reason: "retire it".into(),
        effect: "close_pane",
        deadline_ms: 1234,
        status: "pending",
        error: None,
        blocks: None,
        outcomes: None,
        joined: true,
    }
}

#[test]
fn the_event_names_are_what_the_frontend_listens_for() {
    assert_eq!(EVENT_AGENT_SHUTDOWN, "agent:shutdown");
    assert_eq!(EVENT_SHUTDOWN_PENDING, "agent:shutdown-pending");
    assert_eq!(EVENT_SHUTDOWN_PENDING_CLEARED, "agent:shutdown-pending-cleared");
}

#[test]
fn a_pending_view_omits_what_is_absent_and_never_sends_joined() {
    assert_eq!(
        serde_json::to_value(view()).unwrap(),
        json!({
            "request_id": "req-1", "block_id": "blk-1", "by": "qtest", "target": "lark", "via": "ClosePane",
            "reason": "retire it", "effect": "close_pane", "deadline_ms": 1234, "status": "pending",
        })
    );
}

#[test]
fn a_pending_view_of_a_batch_carries_its_blocks_and_outcomes() {
    let mut v = view();
    v.status = "failed";
    v.error = Some("could not stop".into());
    v.blocks = Some(vec![BatchBlock { block_id: "b2".into(), name: "A".into() }]);
    v.outcomes = Some(vec![
        BlockOutcome { block_id: "b3".into(), name: "B".into(), outcome: "shut_down", error: None },
        BlockOutcome { block_id: "b4".into(), name: "C".into(), outcome: "failed", error: Some("boom".into()) },
    ]);
    let j = serde_json::to_value(v).unwrap();
    assert_eq!(j["error"], "could not stop");
    assert_eq!(j["blocks"], json!([{ "block_id": "b2", "name": "A" }]));
    assert_eq!(
        j["outcomes"],
        json!([
            { "block_id": "b3", "name": "B", "outcome": "shut_down" },
            { "block_id": "b4", "name": "C", "outcome": "failed", "error": "boom" },
        ])
    );
}

#[test]
fn a_cleared_event_names_the_request_and_how_it_ended() {
    let e = PendingClearedEvent { block_id: "blk-1".into(), request_id: "req-1".into(), outcome: "kept_by_user".into() };
    assert_eq!(serde_json::to_value(e).unwrap(), json!({ "block_id": "blk-1", "request_id": "req-1", "outcome": "kept_by_user" }));
}

#[test]
fn a_shutdown_line_flattens_what_its_step_adds_and_omits_the_rest() {
    let plain = ShutdownLineEvent { block_id: "b".into(), seq: 7, step: "exit".into(), text: "lark — exited".into(), extra: ShutdownExtra::default() };
    assert_eq!(serde_json::to_value(plain).unwrap(), json!({ "block_id": "b", "seq": 7, "step": "exit", "text": "lark — exited" }));

    let full = ShutdownLineEvent {
        block_id: "b".into(),
        seq: 8,
        step: "kill".into(),
        text: "killed".into(),
        extra: ShutdownExtra {
            forced_after_ms: Some(5000),
            error: Some("stuck".into()),
            container: Some("db".into()),
            outcome: Some("left running".into()),
            process: Some(ShutdownProcess { pid: 42, name: "node".into(), outcome: "killed".into() }),
        },
    };
    assert_eq!(
        serde_json::to_value(full).unwrap(),
        json!({
            "block_id": "b", "seq": 8, "step": "kill", "text": "killed",
            "forced_after_ms": 5000, "error": "stuck", "container": "db", "outcome": "left running",
            "process": { "pid": 42, "name": "node", "outcome": "killed" },
        })
    );
}
