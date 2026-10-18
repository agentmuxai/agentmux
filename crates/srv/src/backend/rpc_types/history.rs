// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the history total in the status bar and its panel
//! (`history.usage`, `backend::history_usage`).

use serde::{Deserialize, Serialize};

/// `history.usage`'s request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryUsageReq {
    /// Measure again now, instead of answering from the last measurement.
    #[serde(default)]
    pub refresh: bool,
}

/// One place agent history is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryUsageStore {
    /// `provider-files`, `pane-history`, `channel-copies`, `archives` or `audit`.
    pub key: String,
    pub label: String,
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub files: u64,
}

/// An agent and the size of its pane history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryUsageAgent {
    pub definition_id: String,
    pub name: String,
    #[ts(type = "number")]
    pub bytes: u64,
}

/// What `history.usage` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryUsage {
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub stores: Vec<HistoryUsageStore>,
    /// The agents with the most pane history, largest first.
    pub agents: Vec<HistoryUsageAgent>,
    /// When this was measured, in Unix milliseconds.
    #[ts(type = "number")]
    pub computed_at_ms: u64,
}

/// `history.agents`'s request: Warden's History section lists every agent
/// with history (`backend::history_registry`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryAgentsReq {}

/// An agent and the history the global transcript store keeps for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryAgent {
    pub definition_id: String,
    /// Empty when the agent was deleted.
    pub name: String,
    pub provider: String,
    /// The agent it was forked from, by definition id; empty when it isn't a fork.
    pub fork_of: String,
    pub fork_of_name: String,
    /// The agent no longer exists; its history is kept.
    pub deleted: bool,
    /// Its current conversation's transcript.
    #[ts(type = "number")]
    pub current_bytes: u64,
    /// Conversations archived by "New conversation".
    pub archives: u32,
    #[ts(type = "number")]
    pub archive_bytes: u64,
    /// When its history last changed, in Unix milliseconds.
    #[ts(type = "number")]
    pub last_active_ms: u64,
}

/// What `history.agents` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryAgents {
    /// Most recently active first.
    pub agents: Vec<HistoryAgent>,
}

/// `history.agent_sessions`'s request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryAgentSessionsReq {
    pub definition_id: String,
}

/// A session in an agent's record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistorySession {
    pub session_id: String,
    /// The working directory it reported; empty when unknown.
    pub cwd: String,
    /// Receive times of its first and last lines, in Unix milliseconds; 0 = unknown.
    #[ts(type = "number")]
    pub first_ms: u64,
    #[ts(type = "number")]
    pub last_ms: u64,
    /// Transcript bytes it holds.
    #[ts(type = "number")]
    pub bytes: u64,
    /// All of it is in archived conversations.
    pub archived: bool,
}

/// What `history.agent_sessions` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HistoryAgentSessions {
    /// Most recent first.
    pub sessions: Vec<HistorySession>,
    /// What couldn't be read, if anything.
    pub problems: Vec<String>,
}
