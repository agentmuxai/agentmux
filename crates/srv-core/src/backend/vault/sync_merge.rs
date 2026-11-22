// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Vault does when a record arrives from another machine (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! sections 6.2 to 6.5): the answer the sync layer asks of an adapter, `Applied`, `KeptLocal` or `Conflict(both)`.
//!
//! A value is never merged and never lost to a clock. When two machines change an entry before either has seen the
//! other's change, the later version becomes current and the other is kept, as a version called "changed on <machine> at
//! the same time" that is not pruned after 7 days. A delete is a version like any other: apart from a change, the later
//! of the two is current and the other is kept, so the answer is the same on every machine in whatever order the records
//! arrive. A rotation in progress is never overwritten, nor deleted until it ends. Only the tags are merged, as a union.
//!
//! Every function is pure over versions and tags: no store, no clock, no network. The records' bodies stay sealed in
//! [`super::sync_seal`]; what is decided here needs only their versions.

use super::sync_seal::Version;

/// A record arriving from another machine, as far as the merge needs to know it.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub version: Version,
    /// The version this change replaced on the machine that made it. It travels in the clear beside the version and is
    /// also in the record's associated data (`sync_seal::Place::base`), so the caller takes it from a record that has
    /// opened: a relay cannot change it to hide a conflict. Even then it only lets a newer record take the place of the
    /// current one, never an older (see [`merge`]).
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
    /// A delete made on `machine` of a value whose rotation has not ended here: it waits too, because the rotation may
    /// already have changed the credential at its source, and deleting the entry would lose the new value.
    DeleteWaitsForRotation { machine: String },
    /// A change made on `machine` apart from a delete of the entry, and the one the delete is later than: the entry ends
    /// deleted on every machine, and the change is kept as a version the person can restore.
    ChangedWhileDeleted { machine: String },
    /// A delete made on `machine` apart from a change that is later than it: the entry ends as changed on every machine,
    /// and the delete is kept, so the person can see it and delete the entry again.
    Deleted { machine: String },
    /// Two deletes of the entry made apart on `machine` and elsewhere: the later is current and the earlier is kept, so
    /// a change made between them is compared with the same delete on every machine.
    DeletedAlso { machine: String },
}

impl Kept {
    /// What the Vault pane shows beside the version.
    pub fn label(&self) -> String {
        match self {
            Kept::SameTime { machine } => format!("changed on {machine} at the same time"),
            Kept::WaitsForRotation { machine } => format!("changed on {machine} while this was being rotated"),
            Kept::DeleteWaitsForRotation { machine } => format!("deleted on {machine} while this was being rotated"),
            Kept::ChangedWhileDeleted { machine } => format!("changed on {machine} while it was deleted elsewhere"),
            Kept::Deleted { machine } => format!("deleted on {machine} while it was changed elsewhere"),
            Kept::DeletedAlso { machine } => format!("deleted on {machine} as well"),
        }
    }
}

/// The adapter's answer to the layer (spec 6.3).
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// The incoming record replaces the current one.
    Applied,
    /// The incoming record is one this machine already has, or is older than what it holds: nothing changes.
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
        return if h.pending_rotation && !h.deleted {
            let machine = change.version.machine.clone();
            let kept = if value_in.is_some() { Kept::WaitsForRotation { machine } } else { Kept::DeleteWaitsForRotation { machine } };
            Decision::Conflict { current: Side::Local, kept, tags: h.tags.clone() }
        } else {
            Decision::Applied
        };
    }
    // Not on top of what this machine holds: made apart from it, or after versions this machine has not been sent yet.
    // The later version is the current one, whatever the two are (a value or a delete), so every machine reaches the same
    // answer in whatever order the records arrive, and the other is kept as a version for the person. Which came after
    // which on the wall clock is not what decides it, only the versions: a late ancestor is older than what is held and
    // never replaces it, and a record that is on top of one this machine has not been sent yet (a restore after a delete,
    // edited again) ends current when the earlier ones arrive, because it is the latest.
    let incoming_later = change.version > *current;
    let rotating = h.pending_rotation && !h.deleted;
    let side = if rotating || !incoming_later { Side::Local } else { Side::Incoming };
    let (local_machine, incoming_machine) = (current.machine.clone(), change.version.machine.clone());
    match (h.deleted, value_in) {
        // Two deletes of the same record: it is deleted either way, but which delete is current is still decided by the
        // versions, because a change made apart from them is compared with the current one.
        (true, None) if side == Side::Local => Decision::KeptLocal,
        (true, None) => Decision::Conflict { current: Side::Incoming, kept: Kept::DeletedAlso { machine: local_machine }, tags: h.tags.clone() },
        (false, Some(theirs)) => {
            let kept = match side {
                Side::Local if rotating => Kept::WaitsForRotation { machine: incoming_machine },
                Side::Local => Kept::SameTime { machine: incoming_machine },
                Side::Incoming => Kept::SameTime { machine: local_machine },
            };
            let tags = match side {
                Side::Local => union_tags(&h.tags, theirs),
                Side::Incoming => union_tags(theirs, &h.tags),
            };
            Decision::Conflict { current: side, kept, tags }
        }
        // A delete against a value: the later is current, and the other is kept.
        (deleted_here, _) => {
            let kept = match (side, deleted_here) {
                (Side::Local, false) if rotating => Kept::DeleteWaitsForRotation { machine: incoming_machine },
                // Here a value, a delete arrives and is older: the delete is the one kept.
                (Side::Local, false) => Kept::Deleted { machine: incoming_machine },
                // Here a delete, a value arrives and is older: the value is the one kept.
                (Side::Local, true) => Kept::ChangedWhileDeleted { machine: incoming_machine },
                // The incoming is later and takes the place of what is here: what was here is the one kept.
                (Side::Incoming, false) => Kept::ChangedWhileDeleted { machine: local_machine },
                (Side::Incoming, true) => Kept::Deleted { machine: local_machine },
            };
            Decision::Conflict { current: side, kept, tags: h.tags.clone() }
        }
    }
}

/// After a full resync: the records only this machine has (`mine`, against `theirs`, the ids every other member holds).
/// They are shown to the person as "Only on this machine: keep or delete", never brought back or deleted silently.
pub fn only_here(mine: &[String], theirs: &[String]) -> Vec<String> {
    mine.iter().filter(|id| !theirs.contains(id)).cloned().collect()
}

#[cfg(test)]
#[path = "sync_merge_tests.rs"]
mod tests;
