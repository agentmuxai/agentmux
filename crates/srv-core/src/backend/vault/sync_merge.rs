// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Vault does when a record arrives from another machine (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! sections 6.2 to 6.5): the answer the sync layer asks of an adapter, `Applied`, `KeptLocal` or `Conflict(both)`.
//!
//! A value is never merged and never lost to a clock. When two machines change an entry before either has seen the
//! other's change, the later version becomes current and the other is kept, as a version called "changed on <machine> at
//! the same time" that is not pruned after 7 days. A delete made without seeing an edit loses to the edit. A rotation in
//! progress is never overwritten. Only the tags are merged, as a union.
//!
//! Every function is pure over versions and tags: no store, no clock, no network. The records' bodies stay sealed in
//! [`super::sync_seal`]; what is decided here needs only their versions.

use super::sync_seal::Version;

/// How long a tombstone is kept at most (spec 6.4): the Vault's own horizon, longer than the layer's default of 30 days.
pub const TOMBSTONE_HORIZON_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// A record arriving from another machine, as far as the merge needs to know it.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub version: Version,
    /// The version this change replaced on the machine that made it. It travels in the clear beside the version, so a
    /// relay could alter it: it only ever lets a newer record take the place of the current one, never an older (see
    /// [`merge`]), and a false one can only show up as a conflict, which keeps both.
    pub base: Option<Version>,
    /// The tags of the new value; `None` for a tombstone, which carries only its id and versions.
    pub tags: Option<Vec<String>>,
}

/// What this machine holds of the same record.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Held {
    /// Every version of the record this machine has had, the current one first. A tombstone is the current one of a
    /// deleted record. Empty when the machine has never heard of the record.
    pub versions: Vec<Version>,
    pub deleted: bool,
    /// A rotation is under way here: its new value is waiting to be confirmed, and no sync may overwrite it.
    pub pending_rotation: bool,
    /// The current value's tags.
    pub tags: Vec<String>,
}

/// Which of two versions is the one every machine uses from now on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Local,
    Incoming,
}

/// The state of the version that did not become current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kept {
    /// Made on `machine` at the same time as the current one, neither having seen the other.
    SameTime { machine: String },
    /// Made on `machine` on top of a value whose rotation has not ended here: it waits as a version, and the person
    /// settles it when the rotation is over.
    WaitsForRotation { machine: String },
}

impl Kept {
    /// What the Vault pane shows beside the version.
    pub fn label(&self) -> String {
        match self {
            Kept::SameTime { machine } => format!("changed on {machine} at the same time"),
            Kept::WaitsForRotation { machine } => format!("changed on {machine} while this was being rotated"),
        }
    }
}

/// The adapter's answer to the layer (spec 6.3).
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// The incoming record replaces the current one.
    Applied,
    /// The incoming record is one this machine already has, or is older than what it holds, or is a delete that did not
    /// see an edit: nothing changes.
    KeptLocal,
    /// Neither replaces the other. `current` is the one every machine uses until the person chooses, `kept` is the state
    /// of the other (kept as a version, never dropped), and `tags` is the union of both's tags, which goes on the
    /// current one.
    Conflict { current: Side, kept: Kept, tags: Vec<String> },
}

/// `a`'s tags, then those of `b` that `a` does not have.
pub fn union_tags(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = a.to_vec();
    for t in b {
        if !out.contains(t) {
            out.push(t.clone());
        }
    }
    out
}

/// What to do with `change` when this machine holds `held` (`None`, or no versions, if it has never had the record).
pub fn merge(held: Option<&Held>, change: &Change) -> Decision {
    let Some(h) = held.filter(|h| !h.versions.is_empty()) else {
        return Decision::Applied;
    };
    // One this machine has had: a repeat, or a version already replaced.
    if h.versions.contains(&change.version) {
        return Decision::KeptLocal;
    }
    let current = &h.versions[0];
    let value_in = change.tags.as_deref();
    // The change was made on top of what this machine holds, and is later than it: it follows on. A change that only
    // claims to follow, but is not later, is treated as made at the same time, so an old record replayed with a false
    // base cannot take the place of a new one.
    if change.base.as_ref() == Some(current) && change.version > *current {
        return if h.pending_rotation && !h.deleted && value_in.is_some() {
            Decision::Conflict { current: Side::Local, kept: Kept::WaitsForRotation { machine: change.version.machine.clone() }, tags: h.tags.clone() }
        } else {
            Decision::Applied
        };
    }
    // Made without seeing the current version, or after one this machine never saw.
    match (h.deleted, value_in) {
        // Two deletes of the same record.
        (true, None) => Decision::KeptLocal,
        // An edit made without seeing the delete wins: nothing is lost to a delete the editor didn't know of. The person
        // deletes it again if they meant to.
        (true, Some(_)) => Decision::Applied,
        // A delete made without seeing an edit loses to the edit, for the same reason.
        (false, None) => Decision::KeptLocal,
        (false, Some(theirs)) => {
            let incoming_later = change.version > *current;
            let side = if h.pending_rotation || !incoming_later { Side::Local } else { Side::Incoming };
            let kept = match side {
                Side::Local if h.pending_rotation => Kept::WaitsForRotation { machine: change.version.machine.clone() },
                Side::Local => Kept::SameTime { machine: change.version.machine.clone() },
                Side::Incoming => Kept::SameTime { machine: current.machine.clone() },
            };
            let tags = match side {
                Side::Local => union_tags(&h.tags, theirs),
                Side::Incoming => union_tags(theirs, &h.tags),
            };
            Decision::Conflict { current: side, kept, tags }
        }
    }
}

/// Whether a tombstone made at `version` can be dropped: every member has acknowledged it, or it is older than the
/// horizon. Until then it stays, so a machine that was off does not bring the entry back.
pub fn tombstone_due(version: &Version, now_ms: i64, acknowledged_by_all: bool) -> bool {
    acknowledged_by_all || now_ms.saturating_sub(version.wall_ms) >= TOMBSTONE_HORIZON_MS
}

/// Whether a machine last in contact at `last_contact_ms` has been away so long that tombstones it never saw are gone,
/// so it must take the whole Vault again from scratch rather than catch up.
pub fn needs_resync(last_contact_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(last_contact_ms) >= TOMBSTONE_HORIZON_MS
}

/// After a full resync: the records only this machine has (`mine`, against `theirs`, the ids every other member holds).
/// They are shown to the person as "Only on this machine: keep or delete", never brought back or deleted silently.
pub fn only_here(mine: &[String], theirs: &[String]) -> Vec<String> {
    mine.iter().filter(|id| !theirs.contains(id)).cloned().collect()
}

#[cfg(test)]
#[path = "sync_merge_tests.rs"]
mod tests;
