// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `subprocessspawn`, `agentinput` and `agentstop` handlers, called without the RPC engine.

use super::*;
use crate::server::tests::test_state;

fn input(block_id: &str) -> CommandAgentInputData {
    serde_json::from_value(serde_json::json!({ "blockid": block_id, "message": "hello" })).unwrap()
}

#[tokio::test]
async fn an_agent_connection_cannot_type_into_a_composer() {
    let state = test_state();
    let deps = AgentTurnDeps::from_state(&state);
    let agent = RpcContext { agent_id: "SomeAgent".into(), ..Default::default() };
    let err = agent_input(&deps, &agent, input("blk-agent-input")).await.err().unwrap();
    assert!(err.starts_with("FORBIDDEN: only the AgentMux window"), "{err}");
}

#[tokio::test]
async fn the_window_typing_into_a_pane_with_no_controller_is_told_so() {
    let state = test_state();
    let deps = AgentTurnDeps::from_state(&state);
    let err = agent_input(&deps, &RpcContext::operator_for_tests(), input("blk-no-controller")).await.err().unwrap();
    assert_eq!(err, "no controller for block blk-no-controller");
}

#[test]
fn stopping_a_block_with_no_controller_is_a_no_op_not_an_error() {
    let cmd: CommandAgentStopData = serde_json::from_value(serde_json::json!({ "blockid": "blk-never-started" })).unwrap();
    assert!(agent_stop(cmd).is_ok());
}
