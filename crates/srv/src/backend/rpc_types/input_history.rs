// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the user typed to an agent (`storage/agent_input_history.rs`,
//! `docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`): the composer's ArrowUp
//! recall and draft, read and written by the UI.

use serde::{Deserialize, Serialize};

/// One entry, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryItem {
    #[ts(type = "number")]
    pub id: i64,
    pub agent_id: String,
    #[ts(type = "number")]
    pub sent_at_ms: i64,
    pub text: String,
    /// `message`, `slash` or `bang`.
    pub kind: String,
    /// `composer`, `swarm` or `backfill`.
    pub source: String,
    /// How many times in a row it was sent.
    #[ts(type = "number")]
    pub sends: i64,
}

/// `agent.input-history`: an agent's entries, newest first; the `limit` (at
/// most 500, default 200) newest, or those before entry `before_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryData {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub before_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// The entries a read or a search found, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryResult {
    pub entries: Vec<InputHistoryItem>,
}

/// `agent.input-history-append`: the user sent `text` to agent `agent_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryAppendData {
    pub agent_id: String,
    pub text: String,
    /// `message`, `slash` or `bang`.
    pub kind: String,
    /// `composer` (the default) or `swarm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source: Option<String>,
    /// Names of what was attached; never the contents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}

/// The entry the send went into, or none when nothing was kept (an unknown
/// agent, an empty or oversized text).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryAppendResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entry: Option<InputHistoryItem>,
}

/// `agent.input-history-search`: entries holding `query`, one agent's or
/// every agent's, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistorySearchData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent_id: Option<String>,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// `agent.input-history-delete`: remove one entry.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryDeleteData {
    #[ts(type = "number")]
    pub id: i64,
}

/// `agent.input-history-clear`: remove an agent's whole history.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryClearData {
    pub agent_id: String,
}

/// How many entries a delete or clear removed.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryRemovedResult {
    pub removed: u32,
}

/// `agent.input-draft`: the agent's unsent draft; with `text`, keep that one
/// (empty removes it) and answer it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputDraftData {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputDraftResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
}

/// `agent:inputhistory`: an agent's history or draft changed, so another
/// pane showing it reads it again.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryChangedEvent {
    pub agent_id: String,
    /// `history` or `draft`.
    pub what: String,
}
