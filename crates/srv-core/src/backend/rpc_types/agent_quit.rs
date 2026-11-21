// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for ending an agent from the person's window (`agentquit`, `agentshutdown`).

use serde::{Deserialize, Serialize};

/// `agentquit`: the `/shutdown` (`/quit`, `/exit`) of the agent in pane `blockid`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAgentQuitData {
    pub blockid: String,
}

/// `agentshutdown`: Shut down from My agents or a pane's menu, with the person's answer to "stop its background
/// tasks too?".
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAgentShutdownData {
    pub blockid: String,
    #[serde(default)]
    #[ts(optional)]
    pub stop_background: Option<bool>,
}
