// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One agent's runtime state as a viewer sees it (`server/agent_runtime.rs` builds the rows),
//! kept below the server so `mps_events` can name the `agentruntime:changed:` payload.

use serde::{Deserialize, Serialize};

/// The state of one agent as a viewer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeState {
    /// No process: never started, ended, or its pane closed.
    Stopped,
    /// A process is coming up and has not reported a state yet.
    Starting,
    Idle,
    Working,
    /// A turn is running and a question waits for the user.
    WaitingOnUser,
    /// Out of quota; `resets_at` when the provider said when it lifts.
    Limited,
    SignedOut,
    /// Its process ended with an error.
    Error,
}

/// Whether the agent should be running (spec rule 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum AgentDesiredState {
    Running,
    Stopped,
}

/// One agent's runtime state, with what a row needs to show it before it has a pane. The payload of
/// `agentruntime:changed:<agent id>` and a row of `agentruntime.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentRuntime {
    /// The agent's UID, the id of `agentidentities:changed:<agent id>`.
    pub agent_id: String,
    pub state: AgentRuntimeState,
    /// Why, for a state that has a reason: `usage_limit`, `needs_login` or `starts_on_first_message`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// When a limit lifts (unix ms), when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub resets_at: Option<u64>,
    /// The block the agent runs in, shown in a pane or parked; absent for an agent that has never had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub block_id: Option<String>,
    pub desired: AgentDesiredState,
    pub name: String,
    pub provider: String,
    pub icon: String,
    /// The colour its pane shows, once it has had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<String>,
}
