// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of `controller_backstop`: which agent blocks it registers a controller for.

use super::*;
use crate::sagas::park_agents_tests::{dispatch_apply, seed_agent_block, seed_block, seed_workspace};
use crate::server::app_api::agent_attach::{set_desired, DESIRED_STOPPED};
use crate::server::tests::test_state;
use agentmux_common::ipc::Command;

fn set_meta(state: &AppState, block_id: &str, meta: serde_json::Value) {
    let map: obj::MetaMapType = serde_json::from_value(meta).unwrap();
    crate::server::service::update_object_meta(&state.mstore, &format!("block:{block_id}"), &map).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_pane_with_no_controller_is_registered_and_the_rest_are_left_alone() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let wanted = seed_agent_block(&state, &tabs[0], "ag-wanted").await;
    let stopped = seed_agent_block(&state, &tabs[0], "ag-stopped").await;
    set_desired(&state.mstore, &stopped, DESIRED_STOPPED);
    let signed_out = seed_agent_block(&state, &tabs[0], "ag-signed-out").await;
    set_meta(
        &state,
        &signed_out,
        serde_json::json!({ "agent:last_failure": { "code": "auth", "title": "t", "detail": "d", "retryable": false } }),
    );
    let in_terminal = seed_agent_block(&state, &tabs[0], "ag-term").await;
    set_meta(&state, &in_terminal, serde_json::json!({ "agentProvider": "aider" }));
    let parked = seed_agent_block(&state, &tabs[0], "ag-parked").await;
    dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: parked }).await;
    seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;
    seed_block(&state, &tabs[0], serde_json::json!({ "view": "agent", "agentId": "deleted-agent" })).await;

    assert_eq!(candidates(&state), vec![(wanted, tabs[0].clone())]);
}
