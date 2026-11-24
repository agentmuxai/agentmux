// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's merge rule as the sync layer's adapter (`user_sync::adapter`): the layer hands over the record's current
//! header and the incoming one, and this answers `Applied`, `KeptLocal` or `Conflict { current }` by the rules of
//! [`super::sync_merge`]. It sees headers only, never a body: which side is current is decided by versions, what a
//! change replaced (`Header::base`, signed by the origin and sealed into the body) and whether the entry is being rotated
//! here. The tags of a conflicted entry, a union of both sides, are put together afterwards from the two bodies.
//!
//! The record's other versions are read from the layer's own log, so a late record that this machine already has an
//! ancestor of is not taken for a change made apart.

use rusqlite::{params, Connection};

use super::sync_activity;
use super::sync_grant;
use super::sync_merge::{self, Change, Decision, Held, Side as Chose};
use super::sync_seal::Version;
use crate::backend::user_sync::adapter::{Adapter, Merge, Side};
use crate::backend::user_sync::record::Header;

/// Whether an entry is being rotated on this machine: a new value is waiting to be confirmed, and no sync may overwrite
/// it (spec 6.3). The Vault's store answers this; the layer does not know.
pub trait Rotations {
    fn rotating(&self, record: &str) -> bool;
}

/// What a record of a Vault's collection is, by its id. One collection holds all three, and each has its own rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An entry (and anything whose id is not one of the other two): keep-both on a change made apart.
    Entry,
    /// An activity line, `a.<machine>.<seq>` (`sync_activity`): never changes, so never conflicts.
    Activity,
    /// A site grant, `g-<hex>` (`sync_grant`): the later version, a grant or its revoking, is the one to follow.
    Grant,
}

impl Kind {
    pub fn of(record: &str) -> Kind {
        if sync_activity::parse_record_id(record).is_some() {
            Kind::Activity
        } else if sync_grant::is_record_id(record) {
            Kind::Grant
        } else {
            Kind::Entry
        }
    }
}

/// The Vault's adapter, over the layer's log (`conn`) and the Vault's rotations.
pub struct VaultAdapter<'a> {
    pub conn: &'a Connection,
    pub rotations: &'a dyn Rotations,
}

/// Every version of `record` this machine knows, from any origin: the ones its log holds, and the ones those say they
/// replaced (the signed `base` of each). A version only known as a base has not arrived yet, but it is known to be
/// superseded, so it is not taken for a change made apart when it does. The record being decided, `(origin, seq)`, is
/// not counted: the layer has already written it into the log.
fn versions_held(conn: &Connection, incoming: &Header) -> rusqlite::Result<Vec<Version>> {
    let mut stmt = conn.prepare(
        "SELECT wall_ms, counter, machine FROM db_sync_log WHERE collection = ?1 AND record = ?2 AND NOT (origin = ?3 AND seq = ?4) \
         UNION SELECT base_wall, base_counter, base_machine FROM db_sync_log WHERE collection = ?1 AND record = ?2 AND NOT (origin = ?3 AND seq = ?4) AND base_wall IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![incoming.collection, incoming.record, incoming.origin, incoming.seq as i64], |r| {
        Ok(Version { wall_ms: r.get(0)?, counter: r.get(1)?, machine: r.get(2)? })
    })?;
    rows.collect()
}

impl VaultAdapter<'_> {
    /// The whole answer for `incoming`, with what is kept and why (the layer is told only `Merge`; taking the record into
    /// the store needs the rest, see `sync_take`).
    pub fn decide(&self, local: Option<&Header>, incoming: &Header) -> Decision {
        match Kind::of(&incoming.record) {
            Kind::Activity => return sync_activity::merge(local.is_some()),
            Kind::Grant => {
                let had = versions_held(self.conn, incoming).unwrap_or_default();
                return sync_grant::merge(local.map(|l| &l.version), &had, &incoming.version);
            }
            Kind::Entry => {}
        }
        let held = local.map(|l| {
            // The current version first, then the others it had. If the log can't be read, only the current one is known,
            // which can only turn a late ancestor into a kept version, never lose a change.
            let mut versions = vec![l.version.clone()];
            for v in versions_held(self.conn, incoming).unwrap_or_default() {
                if !versions.contains(&v) {
                    versions.push(v);
                }
            }
            Held { versions, deleted: l.tombstone, pending_rotation: self.rotations.rotating(&l.record), tags: Vec::new() }
        });
        let change = Change { version: incoming.version.clone(), base: incoming.base.clone(), tags: (!incoming.tombstone).then(Vec::new) };
        sync_merge::merge(held.as_ref(), &change)
    }
}

/// What the layer is told of a [`Decision`].
pub fn merge_of(decision: &Decision) -> Merge {
    match decision {
        Decision::Applied => Merge::Applied,
        Decision::KeptLocal => Merge::KeptLocal,
        Decision::Conflict { current, .. } => Merge::Conflict { current: if *current == Chose::Local { Side::Local } else { Side::Incoming } },
    }
}

impl Adapter for VaultAdapter<'_> {
    fn merge(&self, local: Option<&Header>, incoming: &Header) -> Merge {
        merge_of(&self.decide(local, incoming))
    }
}

#[cfg(test)]
#[path = "sync_adapter_tests.rs"]
mod tests;
