// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent.send`, called as `agent_io::agent_send(&state, cmd)` without the engine.

use super::agent_io::agent_send;
use crate::backend::obj;
use crate::backend::rpc_types::CommandAgentSendData;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use std::sync::Arc;

fn send_cmd(block_id: &str) -> CommandAgentSendData {
    CommandAgentSendData { block_id: block_id.to_string(), message: "hello".to_string() }
}

fn subprocess_controller(block_id: &str) -> Arc<crate::backend::blockcontroller::subprocess::SubprocessController> {
    Arc::new(crate::backend::blockcontroller::subprocess::SubprocessController::new(
        "tab-send".to_string(),
        block_id.to_string(),
        None,
        None,
        None,
        None,
        None,
        Arc::from("test-boot"),
    ))
}

fn insert_agent_block(state: &crate::server::AppState, block_id: &str, agent_id: &str) {
    let mut meta = obj::MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("agent"));
    meta.insert("agentId".to_string(), serde_json::json!(agent_id));
    let mut block = crate::backend::obj::Block {
        oid: block_id.to_string(),
        parentoref: String::new(),
        version: 0,
        runtimeopts: None,
        stickers: None,
        meta,
        subblockids: None,
    };
    state.mstore.insert(&mut block).unwrap();
}

/// No controller for the block: nothing is running to receive the message.
#[tokio::test]
async fn agent_send_to_a_block_with_no_controller_is_not_running() {
    let state = test_state();
    let err = agent_send(&state, send_cmd("send-no-ctrl")).await.unwrap_err();
    assert_eq!(err, "NOT_RUNNING: no controller for block send-no-ctrl");
}

/// A controller with no block row behind it is reported as a missing block.
#[tokio::test]
async fn agent_send_with_a_controller_but_no_block_row_is_block_not_found() {
    let state = test_state();
    crate::backend::blockcontroller::register_controller("send-no-row", subprocess_controller("send-no-row"));
    let err = agent_send(&state, send_cmd("send-no-row")).await.unwrap_err();
    assert_eq!(err, "BLOCK_NOT_FOUND: send-no-row");
}

/// An OAuth-class agent with no bound account is refused before any spawn.
#[tokio::test]
async fn agent_send_refuses_at_the_identity_gate_before_spawning() {
    let state = test_state();
    let mut def = test_agent_def("send-gate-agent", "SendGate", "claude", "agent", 1, "");
    def.slug = "sendgate".to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    insert_agent_block(&state, "send-gate-block", "send-gate-agent");
    crate::backend::blockcontroller::register_controller("send-gate-block", subprocess_controller("send-gate-block"));

    let err = agent_send(&state, send_cmd("send-gate-block")).await.unwrap_err();
    assert!(err.starts_with("identity spawn gate: "), "{err}");
    assert!(err.contains("no claude account is linked to this agent"), "{err}");
}
