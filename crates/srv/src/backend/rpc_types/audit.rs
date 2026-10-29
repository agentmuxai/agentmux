// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for `audit.query` (`backend::audit_query`): the Audit view's one merged, filterable list over the
//! jekt audit file and the agent segment ledger. An entry carries no message text.

use serde::{Deserialize, Serialize};

/// `audit.query`'s request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditQueryReq {
    /// Only entries of this agent (its slug); a jekt matches at either end.
    #[serde(default)]
    #[ts(optional)]
    pub agent_id: Option<String>,
    /// `jekt`, `action`, `registry`, `agent_start`, `agent_end`, `tool` or `thought`. None: all but `tool` and `thought`.
    #[serde(default)]
    #[ts(optional)]
    pub kinds: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub since_ms: Option<u64>,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub until_ms: Option<u64>,
    /// Entries whose summary, agents, outcome or reason contain this, ignoring case.
    #[serde(default)]
    #[ts(optional)]
    pub text: Option<String>,
    /// `next_cursor` of the page before.
    #[serde(default)]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// At most this many (default 50, at most 200).
    #[serde(default)]
    #[ts(optional)]
    pub limit: Option<u32>,
    /// Also answer `totals` over every entry the filter matches, not only this page.
    #[serde(default)]
    #[ts(optional)]
    pub totals: Option<bool>,
}

/// Where an entry can be opened in the agent's history: `HistoryNavigator.locate({ at_ms | at_byte })`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditHistoryLink {
    /// The agent's slug.
    pub agent_id: String,
    /// The agent's definition id, when known (what `history.agents` lists).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub definition_id: Option<String>,
    /// The transcript zone a byte position is in, when the entry has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub zone: Option<String>,
    #[ts(type = "number")]
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub at_byte: Option<u64>,
}

/// A live pane that can be revealed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditPaneLink {
    pub block_id: String,
}

/// The links an entry (or one end of a jekt) has: each only when it can work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditLinks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub history: Option<AuditHistoryLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pane: Option<AuditPaneLink>,
}

/// One end of a jekt, or the target of an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditEnd {
    pub agent_id: String,
    pub links: AuditLinks,
}

/// What an entry cost, in tokens. Estimated from the message's length unless `basis` says measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditCost {
    #[ts(type = "number")]
    pub input: u64,
    #[ts(type = "number")]
    pub cache_read: u64,
    #[ts(type = "number")]
    pub output: u64,
    /// Absent when there is no price to estimate it by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub usd: Option<f64>,
    /// `measured` or `estimated`.
    pub basis: String,
}

/// One row of the Audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditEntry {
    /// Stable: the same entry has the same id in every page.
    pub id: String,
    #[ts(type = "number")]
    pub ts_ms: u64,
    /// `jekt`, `action`, `registry`, `agent_start`, `agent_end`, `tool` or `thought`.
    pub kind: String,
    /// The agent the entry is about (every kind but `jekt`, which has `from` and `to`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<AuditEnd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<AuditEnd>,
    /// One line saying what happened. Never a message's text.
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub outcome: Option<String>,
    /// A note on an action; for `agent_end`, why the process went (`exited`, `crashed`, `closed`, `restarted`, `stopped`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cost: Option<AuditCost>,
    /// For entries about one agent; a jekt's are on `from` and `to`.
    #[serde(default)]
    pub links: AuditLinks,
}

/// The cost of every entry a filter matches (all of them, not a page).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditTotals {
    /// Entries that match.
    #[ts(type = "number")]
    pub entries: u64,
    #[ts(type = "number")]
    pub input: u64,
    #[ts(type = "number")]
    pub cache_read: u64,
    #[ts(type = "number")]
    pub output: u64,
    /// Absent when no matching entry has a price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub usd: Option<f64>,
    /// Some of what is summed is estimated.
    pub estimated: bool,
}

/// What `audit.query` answers: newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditQueryResult {
    pub entries: Vec<AuditEntry>,
    /// Pass back as `cursor` for the next page; absent at the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_cursor: Option<String>,
    /// Requested kinds that aren't recorded yet (`tool`, `thought`).
    #[serde(default)]
    pub not_recorded: Vec<String>,
    /// When asked for (`totals`): over everything the filter matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub totals: Option<AuditTotals>,
}
