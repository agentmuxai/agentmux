// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What srv says about the messages it holds for agents (`server::jekt_held_summary`): per agent, how many and why,
//! as the `jekt.held.summary` answer, and the `jektheld` event that tells a window the answer changed. Counts and reasons
//! only: no message body, no sender and no message id leave the hold table.

use serde::Serialize;

/// The event published when the held counts change. A signal, carrying only the totals: a window reads
/// `jekt.held.summary` for who and why.
pub const EVENT_JEKT_HELD: &str = "jektheld";

/// Why an agent's messages are held, in the words the hold answer uses (`held_reason`).
pub const REASON_NOT_RUNNING: &str = "not_running";
pub const REASON_STARTING: &str = "starting";
pub const REASON_NEEDS_LOGIN: &str = "needs_login";
pub const REASON_USAGE_LIMIT: &str = "usage_limit";

/// One agent's held messages for one reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JektHeldAgent {
    /// The agent's UID: `AgentRuntime.agent_id`.
    pub agent_id: String,
    pub agent_name: String,
    /// `not_running`, `starting`, `needs_login`, `usage_limit`, `idle`, `quota`, `api_offline` or `peer_offline`.
    pub reason: String,
    #[ts(type = "number")]
    pub count: u32,
    /// When the oldest of them was sent (unix ms).
    #[ts(type = "number")]
    pub since_ms: u64,
}

/// `jekt.held.summary`: the held messages, by agent and reason, the agent with the oldest message first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JektHeldSummaryResult {
    pub agents: Vec<JektHeldAgent>,
}

/// `jektheld`: the held counts changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JektHeldEvent {
    /// Messages held in all.
    #[ts(type = "number")]
    pub messages: u32,
    /// Agents that have any.
    #[ts(type = "number")]
    pub agents: u32,
}
