// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The JSON `agent.send/stop/status/list/output/open` returned before they moved to
// `register_typed`: every key, and `null` for an unset `Option`.

use crate::backend::rpc_types::{
    AgentListEntry, AgentListResult, AgentOpenResult, AgentOutputResult, AgentSendResult,
    AgentStatusResult, AgentStopResult,
};
use serde_json::json;

fn wire<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn agent_send_result_keeps_its_wire() {
    let r = AgentSendResult { block_id: "b1".into(), status: "running".into(), session_id: None };
    assert_eq!(wire(&r), json!({"block_id": "b1", "status": "running", "session_id": null}));
}

#[test]
fn agent_stop_result_keeps_its_wire() {
    let r = AgentStopResult { block_id: "b1".into(), status: "done".into(), exit_code: None };
    assert_eq!(wire(&r), json!({"block_id": "b1", "status": "done", "exit_code": null}));
}

#[test]
fn agent_status_result_keeps_its_wire() {
    let r = AgentStatusResult {
        block_id: "b1".into(),
        agent_id: "a1".into(),
        provider: "claude".into(),
        controller_type: "persistent".into(),
        status: "done".into(),
        session_id: Some("s1".into()),
        pid: None,
        exit_code: Some(0),
    };
    assert_eq!(
        wire(&r),
        json!({
            "block_id": "b1",
            "agent_id": "a1",
            "provider": "claude",
            "controller_type": "persistent",
            "status": "done",
            "session_id": "s1",
            "pid": null,
            "exit_code": 0
        })
    );
}

#[test]
fn agent_list_result_keeps_its_wire() {
    let r = AgentListResult {
        agents: vec![AgentListEntry {
            block_id: "b1".into(),
            tab_id: "t1".into(),
            agent_id: "a1".into(),
            provider: "codex".into(),
            status: "init".into(),
            session_id: None,
        }],
    };
    assert_eq!(
        wire(&r),
        json!({"agents": [{
            "block_id": "b1",
            "tab_id": "t1",
            "agent_id": "a1",
            "provider": "codex",
            "status": "init",
            "session_id": null
        }]})
    );
}

#[test]
fn agent_output_result_keeps_its_wire() {
    let r = AgentOutputResult {
        block_id: "b1".into(),
        lines: vec!["one".into(), "two".into()],
        total_lines: 5,
        has_more: true,
    };
    assert_eq!(
        wire(&r),
        json!({"block_id": "b1", "lines": ["one", "two"], "total_lines": 5, "has_more": true})
    );
}

#[test]
fn agent_open_result_keeps_its_wire() {
    let r = AgentOpenResult {
        block_id: "b1".into(),
        tab_id: "t1".into(),
        agent_id: "a1".into(),
        provider: "claude".into(),
        controller_type: "persistent".into(),
        status: "running".into(),
        created: true,
    };
    assert_eq!(
        wire(&r),
        json!({
            "block_id": "b1",
            "tab_id": "t1",
            "agent_id": "a1",
            "provider": "claude",
            "controller_type": "persistent",
            "status": "running",
            "created": true
        })
    );
}
