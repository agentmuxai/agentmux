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
    /// `channel`: only what this srv's channel wrote (an entry that says nothing of its channel is this one's);
    /// `machine` (the default): what every channel on this computer wrote.
    #[serde(default)]
    #[ts(optional)]
    pub scope: Option<String>,
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

/// Where an entry came from and how it travelled, and for a jekt which of its two halves it is. Old lines have little
/// of it: a tier only where the line itself proves it (`tier_inferred`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AuditProvenance {
    /// The computer that wrote the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    /// The channel that wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub channel: Option<String>,
    /// The version of the srv that wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// `host`, `channel`, `lan` or `cloud`. Absent on an old line that does not prove it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tier: Option<String>,
    /// The tier was worked out from the line (a cloud verdict, a LAN-claimed UID), not recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tier_inferred: Option<bool>,
    /// How far the sender was proven (the marker's `TRUST=` label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub trust: Option<String>,
    /// The message's id, the same on both of its halves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub msg_id: Option<String>,
    /// On a joined row, when the sender handed the message over (`ts_ms` is when it was delivered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub sent_ms: Option<u64>,
    /// The row is the sender's half alone: the other end was not seen (the peer is off, or not paired).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sent_only: Option<bool>,
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
    /// Where it came from and how it travelled. Absent for a row that has no such record (an agent start, a tool call).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub provenance: Option<AuditProvenance>,
    /// The dialog's text as read, for a sensitive-op row (`os_prompt_answer`). Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prompt: Option<String>,
    /// The exact value given. Never a password or other secret value (the guard refuses those).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub answer: Option<String>,
    /// The op's risk level: `low`, `medium` or `high`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub level: Option<String>,
    /// `true` when a remembered grant allowed it, with no fresh countdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub standing: Option<bool>,
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
    /// The channel that answered: what an entry with no channel of its own belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub channel: Option<String>,
}
