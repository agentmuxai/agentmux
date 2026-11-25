// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's activity as sync records (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! section 6.7): append-only per machine, so lines from different machines never conflict.
//!
//! A line is named by its machine and its sequence number there, and never changes. A machine keeps its own hash chain
//! over its own lines (`storage::vault::activity_hash`); the lines of other machines are kept beside them, without that
//! chain, and shown with a Machine column. A line holds no value, but its label and target are sealed like any record.

use serde::{Deserialize, Serialize};

use super::sync_merge::Decision;
use crate::backend::storage::vault::VaultActivityRow;

/// An activity line as a sync record carries it. It leaves out what is the machine's own bookkeeping: the order in its
/// table and its place in its hash chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityBody {
    pub at: i64,
    /// In a record, the record the entry is written under (its link id), the same on every machine; in a row, the id the entry
    /// has here.
    pub entry_id: String,
    pub entry_label: String,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub outcome: String,
    pub version: i64,
    pub actor_kind: String,
    pub host: String,
    pub reason: String,
    pub refs: String,
}

impl ActivityBody {
    pub fn of_row(r: &VaultActivityRow) -> Self {
        Self {
            at: r.at,
            entry_id: r.entry_id.clone(),
            entry_label: r.entry_label.clone(),
            actor: r.actor.clone(),
            action: r.action.clone(),
            target: r.target.clone(),
            outcome: r.outcome.clone(),
            version: r.version,
            actor_kind: r.actor_kind.clone(),
            host: r.host.clone(),
            reason: r.reason.clone(),
            refs: r.refs.clone(),
        }
    }

    /// The row a machine shows for a line another machine made. It has no place in this machine's chain: `seq` is the
    /// sequence number on the machine that made it, and the hashes are empty.
    pub fn into_row(&self, seq: i64) -> VaultActivityRow {
        VaultActivityRow {
            seq,
            at: self.at,
            entry_id: self.entry_id.clone(),
            entry_label: self.entry_label.clone(),
            actor: self.actor.clone(),
            action: self.action.clone(),
            target: self.target.clone(),
            outcome: self.outcome.clone(),
            version: self.version,
            actor_kind: self.actor_kind.clone(),
            host: self.host.clone(),
            reason: self.reason.clone(),
            refs: self.refs.clone(),
            prev_hash: String::new(),
            hash: String::new(),
        }
    }
}

/// The id of the line `seq` made on `machine`: `a.<machine>.<seq>`, letters, digits and `._-` as a sealed record's id may
/// hold. The sequence has no dot, so the id reads back one way only.
pub fn record_id(machine: &str, seq: i64) -> String {
    format!("a.{machine}.{seq}")
}

/// The machine and sequence an activity record id names, or `None` for any other id.
pub fn parse_record_id(id: &str) -> Option<(&str, i64)> {
    let rest = id.strip_prefix("a.")?;
    let (machine, seq) = rest.rsplit_once('.')?;
    let seq: i64 = seq.parse().ok().filter(|n| *n > 0)?;
    (!machine.is_empty()).then_some((machine, seq))
}

/// What to do with a line that arrives: a line this machine already has changes nothing, and any other is added. A line
/// never conflicts, because no two machines make the same (machine, sequence).
pub fn merge(already_have: bool) -> Decision {
    if already_have {
        Decision::KeptLocal
    } else {
        Decision::Applied
    }
}

#[cfg(test)]
#[path = "sync_activity_tests.rs"]
mod tests;
