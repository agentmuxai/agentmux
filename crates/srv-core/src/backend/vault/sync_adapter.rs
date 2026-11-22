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

use super::sync_merge::{self, Change, Decision, Held, Side as Chose};
use super::sync_seal::Version;
use crate::backend::user_sync::adapter::{Adapter, Merge, Side};
use crate::backend::user_sync::record::Header;

/// Whether an entry is being rotated on this machine: a new value is waiting to be confirmed, and no sync may overwrite
/// it (spec 6.3). The Vault's store answers this; the layer does not know.
pub trait Rotations {
    fn rotating(&self, record: &str) -> bool;
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

impl Adapter for VaultAdapter<'_> {
    fn merge(&self, local: Option<&Header>, incoming: &Header) -> Merge {
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
        match sync_merge::merge(held.as_ref(), &change) {
            Decision::Applied => Merge::Applied,
            Decision::KeptLocal => Merge::KeptLocal,
            Decision::Conflict { current, .. } => Merge::Conflict { current: if current == Chose::Local { Side::Local } else { Side::Incoming } },
        }
    }
}

#[cfg(test)]
#[path = "sync_adapter_tests.rs"]
mod tests;
