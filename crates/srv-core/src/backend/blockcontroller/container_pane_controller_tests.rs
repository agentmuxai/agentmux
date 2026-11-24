// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The controller a container agent's pane may have: its turns run in its container, so none that starts the
//! agent's CLI on this computer is started for it (`resync_steps::effective_controller_type`).

use super::resync_steps::effective_controller_type;
use super::*;

fn pane(mode: &str) -> Block {
    let meta = [("agentMode".to_string(), serde_json::json!(mode))].into_iter().collect();
    Block { oid: "b-container-controller".to_string(), meta, ..Default::default() }
}

#[test]
fn a_container_agents_pane_is_given_no_controller_that_starts_its_cli_on_this_computer() {
    for controller in [BLOCK_CONTROLLER_ACP, BLOCK_CONTROLLER_MSP, BLOCK_CONTROLLER_APP_SERVER] {
        let refused = effective_controller_type(&pane("container"), controller.to_string()).unwrap_err();
        assert!(refused.contains("runs in its container") && refused.contains(controller), "{controller}: {refused}");
    }
}

#[test]
fn a_container_agents_pane_keeps_its_per_turn_controller_and_a_stale_persistent_one_becomes_it() {
    assert_eq!(effective_controller_type(&pane("container"), BLOCK_CONTROLLER_SUBPROCESS.to_string()).as_deref(), Ok(BLOCK_CONTROLLER_SUBPROCESS));
    assert_eq!(effective_controller_type(&pane("container"), BLOCK_CONTROLLER_PERSISTENT.to_string()).as_deref(), Ok(BLOCK_CONTROLLER_SUBPROCESS));
}

#[test]
fn a_host_agents_pane_keeps_every_controller_it_asks_for() {
    for controller in [
        BLOCK_CONTROLLER_SHELL,
        BLOCK_CONTROLLER_CMD,
        BLOCK_CONTROLLER_SUBPROCESS,
        BLOCK_CONTROLLER_PERSISTENT,
        BLOCK_CONTROLLER_ACP,
        BLOCK_CONTROLLER_MSP,
        BLOCK_CONTROLLER_APP_SERVER,
    ] {
        assert_eq!(effective_controller_type(&pane("host"), controller.to_string()).as_deref(), Ok(controller), "{controller}");
    }
}
