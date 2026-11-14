// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Audit's index of what agents do (docs/specs/SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.2).
//!
//! Every output line of every agent is appended in one place (`transcript::append`), and its tool calls, thinking
//! blocks and turn ends are already in it. This module reads those lines as they pass and writes one small row for
//! each to `activity-YYYY-MM-DD.jsonl`, one file a day, so a day can be dropped by date. The row holds the agent,
//! the kind, a scrubbed one-line summary and where the line landed in the agent's history; the transcript keeps the
//! full call. Nothing is recorded twice, only indexed.
//!
//! What is kept: the agent, the kind, a summary that never copies a raw call (see [`scrub`]), times, outcome, cost
//! and byte positions. Never: a Vault value, a secret, a thought's text, a tool's result.
//!
//! The rows go to the same place the jekt audit file does ([`reactive::audit_store::write_path`]'s folder): the
//! shared `audit` folder, or the srv's own while it does not use the shared one.

pub mod line;
pub mod scrub;
pub mod store;

use serde::{Deserialize, Serialize};

pub use store::{day_files, install_for_this_srv, record, DayFile};

use crate::backend::rpc_types::AuditCost;

/// How long a day's file is kept (days). The transcript still holds the full call, so what goes is only the index.
pub const KEEP_DAYS: u64 = 30;

/// One indexed action. `agent` is the agent's UID (the id in its transcript zone), turned into a name when listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityRow {
    /// Unix ms the line was written.
    pub ts: u64,
    /// `tool`, `turn` or `thought`.
    pub kind: String,
    pub agent: String,
    pub block: String,
    /// The transcript zone the line is in, and where in it (`at_byte` of the History link).
    pub zone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte: Option<u64>,
    /// One line, scrubbed.
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// A turn's length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<AuditCost>,
    /// The channel that wrote the row; set by the writer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// Indexes the transcript lines `records` (newline-terminated, as written) that landed in `zone` from byte `at`.
/// Called for every transcript write; a line that is not a tool call, a thinking block or a turn end costs a
/// substring test.
pub fn note_records(block_id: &str, zone: &str, records: &[u8], at: Option<u64>) {
    let now = agentmux_common::time::now_ms_u64();
    for row in rows_of(block_id, zone, records, at, now) {
        record(row);
    }
}

/// The rows in `records`, which were written to `zone` from byte `at` at time `now`.
fn rows_of(block_id: &str, zone: &str, records: &[u8], at: Option<u64>, now: u64) -> Vec<ActivityRow> {
    let Some(agent) = agent_of_zone(zone) else { return Vec::new() };
    let mut rows = Vec::new();
    let mut offset = at;
    for raw in records.split_inclusive(|b| *b == b'\n') {
        let start = offset;
        offset = offset.map(|o| o + raw.len() as u64);
        let Ok(text) = std::str::from_utf8(raw) else { continue };
        for parsed in line::parse(text.trim_end()) {
            rows.push(ActivityRow {
                ts: now,
                kind: parsed.kind.to_string(),
                agent: agent.to_string(),
                block: block_id.to_string(),
                zone: zone.to_string(),
                byte: start,
                summary: parsed.summary,
                tool: parsed.tool,
                outcome: parsed.outcome,
                ms: parsed.ms,
                cost: parsed.cost,
                channel: None,
            });
        }
    }
    rows
}

/// The agent a transcript zone belongs to: `agent:<id>:current`, for a defined agent. A quick-launch provider's zone
/// is shared by every pane of that provider, so no one agent made what is in it.
fn agent_of_zone(zone: &str) -> Option<&str> {
    let id = zone.strip_prefix("agent:")?.strip_suffix(":current")?;
    (crate::backend::providers::get_provider(id).is_none()).then_some(id)
}

#[cfg(test)]
mod tests;
