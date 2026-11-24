// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Request-shape tests for the eight websocket-hosted block commands migrated
// in this slice.

use super::*;
use serde_json::json;

#[test]
fn the_single_field_commands_accept_their_payloads() {
    serde_json::from_value::<CommandDeleteBlockData>(json!({"blockid": "b1"}))
        .expect("deletesubblock");
    serde_json::from_value::<CommandListBackgroundTasksData>(json!({"blockid": "b1"}))
        .expect("listbackgroundtasks");
    serde_json::from_value::<CommandAgentCancelData>(
        json!({"blockid": "b1", "tool_use_id": "t1"}),
    )
    .expect("agentcancel");
    serde_json::from_value::<CommandAgentShutdownKeepData>(
        json!({"blockid": "b1", "request_id": "r1"}),
    )
    .expect("agentshutdownkeep");
}

// `outcome` and `scope` are Rust `String`s carrying closed sets. The
// generated binding keeps the unions via #[ts(type = ...)], but the SERVER
// is the thing that actually enforces them -- the handler rejects an
// unknown outcome explicitly. Pin that the wire type stays permissive
// (a bad value deserializes, then fails with a domain error) so the
// union in TypeScript is understood as a caller-side aid, not the guard.
#[test]
fn tool_decision_parses_any_outcome_string_and_leaves_validation_to_the_handler() {
    let good: CommandToolDecisionData = serde_json::from_value(
        json!({"blockid": "b1", "request_id": "r1", "outcome": "allow", "scope": "once"}),
    )
    .expect("tooldecision must accept the payload the stub sends");
    assert!(good.feedback.is_none(), "feedback is optional");

    serde_json::from_value::<CommandToolDecisionData>(
        json!({"blockid": "b1", "request_id": "r1", "outcome": "bogus", "scope": "once"}),
    )
    .expect("an unknown outcome still DESERIALIZES; the handler rejects it, not serde");
}

#[test]
fn ambient_narrate_accepts_the_payload_the_stub_sends() {
    serde_json::from_value::<CommandAmbientNarrateData>(json!({
        "blockid": "b1", "kind": "k", "context": "c", "dedupe_key": "d",
    }))
    .expect("ambientnarrate");
}

// `timestamp` / `run_in_background` are Options with no
// skip_serializing_if. serde treats a missing Option field as None, so the
// generated binding marks them optional -- which is what the hand-written
// declarations already said. Both halves matter, so pin both.
#[test]
fn dock_node_status_accepts_the_payload_with_and_without_its_optionals() {
    let bare: CommandDockNodeStatusData = serde_json::from_value(
        json!({"blockid": "b1", "node_id": "n1", "tool_name": "t", "status": "s"}),
    )
    .expect("docknodestatus must accept the bare payload");
    assert!(bare.timestamp.is_none() && bare.run_in_background.is_none());

    let full: CommandDockNodeStatusData = serde_json::from_value(json!({
        "blockid": "b1", "node_id": "n1", "tool_name": "t", "status": "s",
        "timestamp": 1_789_000_000_000_i64, "run_in_background": true,
    }))
    .expect("docknodestatus must accept the full payload");
    assert_eq!(full.timestamp, Some(1_789_000_000_000));
    assert_eq!(full.run_in_background, Some(true));
}

#[test]
fn background_task_commands_accept_their_payloads() {
    let done: CommandBackgroundTaskCompletionData =
        serde_json::from_value(json!({"blockid": "b1", "node_id": "n1", "status": "done"}))
            .expect("backgroundtaskcompletion, timestamp omitted");
    assert!(done.timestamp.is_none());

    let pid: CommandBackgroundTaskPidData =
        serde_json::from_value(json!({"blockid": "b1", "node_id": "n1", "pid": 4321}))
            .expect("backgroundtaskpid");
    assert_eq!(pid.pid, 4321);

    // `pid` is a u32, so a negative must be rejected rather than wrapping
    // into a huge process id.
    assert!(
        serde_json::from_value::<CommandBackgroundTaskPidData>(
            json!({"blockid": "b1", "node_id": "n1", "pid": -1}),
        )
        .is_err(),
        "a negative pid must fail loudly, not wrap"
    );
}
