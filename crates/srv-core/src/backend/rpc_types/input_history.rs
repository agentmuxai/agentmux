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

/// `agent.input-history-delete`: forget one entry. It waits in the trash, and
/// can be restored for a while.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryDeleteData {
    #[ts(type = "number")]
    pub id: i64,
}

/// `agent.input-history-clear`: forget an agent's whole history, into the
/// trash as one batch.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryClearData {
    pub agent_id: String,
}

/// How many entries a delete or clear forgot, and the trash batch holding
/// them (none when there was nothing to forget).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryRemovedResult {
    pub removed: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub batch_id: Option<String>,
}

/// `agent.input-history-trash`: what was forgotten and can still be restored,
/// one agent's or every agent's, most recently forgotten first.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryTrashData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent_id: Option<String>,
}

/// One forgetting in the trash: a single entry, or an agent's whole history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryTrashItem {
    pub batch_id: String,
    pub agent_id: String,
    #[ts(type = "number")]
    pub deleted_at_ms: i64,
    /// The agent's slug when an agent forgot it, `armory-ui` for the AgentMux
    /// window.
    pub deleted_by: String,
    pub count: u32,
    /// The oldest entry's text, to say what it was.
    pub sample: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryTrashResult {
    pub batches: Vec<InputHistoryTrashItem>,
    /// How long a forgotten entry stays restorable.
    #[ts(type = "number")]
    pub retention_ms: i64,
}

/// `agent.input-history-restore`: bring a trash batch back into its agent's
/// history.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandInputHistoryRestoreData {
    pub batch_id: String,
}

/// How many entries came back (0 when the batch was already restored or has
/// aged out).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct InputHistoryRestoredResult {
    pub restored: u32,
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
    /// `history`, `draft`, `backfill` (older entries came in, from the
    /// agent's past conversations) or `restored` (forgotten entries came back,
    /// so a pane reloads rather than reading only the newer ones).
    pub what: String,
}
