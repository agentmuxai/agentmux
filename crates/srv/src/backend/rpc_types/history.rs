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
