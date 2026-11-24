// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A record from another machine, taken into the Vault (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! sections 6.2 to 6.5): the other half of [`super::sync_capture`], which writes this machine's changes into the log.
//!
//! [`Taker::receive`] hands a record to the sync layer with the Vault's adapter ([`super::sync_adapter`]), and when the layer
//! makes it, or keeps it as the other side of a conflict, puts the result in the store:
//! - A change that follows on from what this machine holds replaces the entry's value. The value it replaces is kept for 7 days
//!   as a version, like an edit made here.
//! - A change made apart from what this machine holds is a conflict, and a value is never merged: the later version is the
//!   entry's value on every machine, and the other is kept as a version that says "changed on <machine> at the same time"
//!   (`versions::KEPT_PREFIX`) and is not pruned. Only the tags are merged, as a union.
//! - A tombstone deletes the entry, its versions and its grants, unless the entry has moved to another Vault, or a rotation of it
//!   is waiting to be confirmed (then the delete is the kept side of a conflict, and the entry stays).
//! - A change to an entry this machine deleted brings the entry back when it is the later of the two, under the id the record
//!   names, and the delete is kept as a version that holds no value.
//!
//! A record names an entry by one of its link ids (`sync_links`), and never makes a second entry for an id that one entry
//! has. A new entry is made with the ids the record carries, the record's own first, in the same transaction as its row, so a
//! capture can never give a taken entry an id of its own in between.
//!
//! What the person keeps to themselves on this machine stays: the agents an entry is shared with and when it was last used are
//! not replaced (`EntryBody::into_row`). What arrives is checked first: a kind this machine does not know, or a secret part the
//! kind does not have, is not taken.
//!
//! Where a conflict keeps nothing in the store, the side that lost is still in the log, sealed: when the entry is deleted (a
//! change made apart from a delete has nowhere to be kept once the entry is gone), and when the entry is not here to attach it to.
//! A pane that lists conflicts reads it from there; this module only says so ([`Effect::KeptInLog`]).
//!
//! Taking and capturing run one at a time on one worker: both read the store, then write it, and neither locks the other out.

use std::cell::RefCell;

use super::kinds;
use super::seal::{self, Key, Parts};
use super::sync_activity::{self, ActivityBody};
use super::sync_adapter::{merge_of, Kind, Rotations, VaultAdapter};
use super::sync_capture;
use super::sync_links::{self, LinkOwners, Links};
use super::sync_merge::{self, Decision, Kept, Side as Chose};
use super::sync_record::{EntryBody, Profiles};
use super::sync_seal::{self, Place};
use super::sync_store::{apply_activity, links_of};
use super::versions::{self, KEPT_PREFIX, PREVIOUS};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::backend::storage::vault_sync::TakeWrite;
use crate::backend::storage::vault_versions::VaultVersionRow;
use crate::backend::user_sync::adapter::{Adapter, Merge};
use crate::backend::user_sync::hlc::Clock;
use crate::backend::user_sync::log::{self, Outcome, Pins};
use crate::backend::user_sync::record::{Header, Record};
use crate::backend::user_sync::seal::KeyRing;

/// What taking a record did to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Nothing: the record was held, older than what is here, void, or about an entry this machine does not have (a delete
    /// of one it never had, or of one that moved to another Vault).
    None,
    /// The entry was made or its value replaced.
    Wrote { entry: String },
    /// The entry was deleted.
    Deleted { entry: String },
    /// An activity line another machine made was added to the lines shown with a Machine column.
    Line { machine: String, seq: i64 },
    /// The record was not taken into the store, and why (no value in it), and it is not going to be.
    Skipped(String),
    /// The record was not taken into the store yet, and why: a kind this version does not know, or an id another entry holds.
    /// The log holds it and does not send it again, so [`Taker::retake`] takes it again from the log.
    Deferred(String),
    /// The side of a conflict that lost has nowhere in the store to be kept: it stays in the log, sealed.
    KeptInLog(String),
}

/// What [`Taker::receive`] did: what the layer made of the record, what the store did, and the version a conflict kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub outcome: Outcome,
    pub effect: Effect,
    /// Why a version was kept ("changed on <machine> at the same time"), for a conflict.
    pub kept: Option<String>,
}

/// Whether a rotation of the entry is waiting here, read before the log is locked (the store and the log are one connection).
struct Rotating(bool);

impl Rotations for Rotating {
    fn rotating(&self, _record: &str) -> bool {
        self.0
    }
}

/// The Vault's adapter, keeping its last answer: the layer is told `Merge`, the take needs what was kept and why.
struct Recording<'a> {
    inner: VaultAdapter<'a>,
    last: RefCell<Option<Decision>>,
}

impl Adapter for Recording<'_> {
    fn merge(&self, local: Option<&Header>, incoming: &Header) -> Merge {
        let decision = self.inner.decide(local, incoming);
        let merge = merge_of(&decision);
        *self.last.borrow_mut() = Some(decision);
        merge
    }
}

/// The ids the record's own entry may take: a retired one is allowed for `record`, which is a later change to the entry that
/// had it (see `Store::vault_take_write`). Every other id is as `Store` answers.
struct Reviving<'a> {
    store: &'a Store,
    record: &'a str,
}

impl LinkOwners for Reviving<'_> {
    fn owner_of(&self, id: &str) -> Option<String> {
        self.store.owner_of(id)
    }
    fn retired(&self, id: &str) -> bool {
        id != self.record && self.store.retired(id)
    }
}

/// This machine's side of the sync layer for the Vault: its store, the Vault's keys, this machine's own key, and its profiles.
pub struct Taker<'a> {
    pub store: &'a Store,
    pub ring: &'a KeyRing,
    /// This machine's own key, which seals the values it keeps.
    pub key: &'a Key,
    pub profiles: &'a dyn Profiles,
}

/// The body of `rec`, opened with the Vault's key at the place its header names.
fn open_body<T: serde::de::DeserializeOwned>(ring: &KeyRing, rec: &Record) -> Result<T, String> {
    let h = &rec.header;
    let key = ring.get(h.epoch).ok_or_else(|| format!("no key for epoch {}", h.epoch))?;
    let place = Place { vault: &h.collection, record: &h.record, version: &h.version, base: h.base.as_ref(), epoch: h.epoch };
    sync_seal::open_record(key, &place, &rec.body)
}

/// Why a body is not taken, if it is not: it would make an entry the Vault cannot show or use.
fn refused(body: &EntryBody) -> Option<String> {
    let Some(kind) = kinds::kind(&body.kind) else {
        return Some(format!("a kind this version does not know: {}", body.kind));
    };
    body.parts.keys().find(|p| !kind.parts.contains(&p.as_str())).map(|p| format!("a part the kind {} does not have: {p}", kind.name))
}

fn tags_of(row: &VaultRow) -> Vec<String> {
    serde_json::from_str(&row.tags).unwrap_or_default()
}

fn tags_json(tags: &[String]) -> String {
    serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string())
}

/// A version this machine keeps of what a take replaced.
fn version(entry_id: &str, created_at: i64, sealed: Vec<u8>, state: String, machine: &str, now_ms: i64) -> VaultVersionRow {
    VaultVersionRow { seq: 0, entry_id: entry_id.to_string(), entry_created_at: created_at, sealed, replaced_at: now_ms, replaced_by: machine.to_string(), how: "synced".to_string(), state }
}

impl Taker<'_> {
    /// Hands `rec` to the layer, and takes its result into the store. An error from the layer (a record that is refused, out of
    /// sequence or at another epoch) comes back as text and the log does not hold the record. An error from the store, or a
    /// record the store cannot take yet ([`Effect::Deferred`]), comes back after the log holds the record, which the layer does
    /// not hand over again: so it is noted, and [`Taker::retake`] takes it again from the log.
    pub fn receive(&self, rec: &Record, pins: &Pins, clock: &mut Clock, now_ms: i64) -> Result<Taken, String> {
        let h = &rec.header;
        // Everything that reads the store comes before the log is locked: the log is the store's own connection.
        let holder = match Kind::of(&h.record) {
            Kind::Entry => self.store.vault_entry_of_link(&h.record).map_err(|e| e.to_string())?,
            _ => None,
        };
        let rotating = match &holder {
            Some(id) => self.rotation_waits(id)?,
            None => false,
        };
        let (outcome, decision) = {
            let conn = self.store.conn().lock().unwrap_or_else(|e| e.into_inner());
            let adapter = Recording { inner: VaultAdapter { conn: &conn, rotations: &Rotating(rotating) }, last: RefCell::new(None) };
            let outcome = log::apply_remote(&conn, rec, pins, clock, now_ms, &adapter).map_err(|e| format!("{e:?}"))?;
            (outcome, adapter.last.into_inner())
        };
        let none = |outcome| Taken { outcome, effect: Effect::None, kept: None };
        // The adapter is not asked about a record the layer already holds or voids.
        let Some(decision) = decision else { return Ok(none(outcome)) };
        // Why the other side of a conflict is kept, when this record won one: a retake keeps it the same way.
        let lost = match &decision {
            Decision::Conflict { current: Chose::Incoming, kept, .. } => Some(kept.label()),
            _ => None,
        };
        let result = match decision {
            Decision::KeptLocal => return Ok(none(outcome)),
            Decision::Applied => self.current(rec, None, false, now_ms).map(|effect| Taken { outcome, effect, kept: None }),
            Decision::Conflict { current: Chose::Incoming, kept, .. } => {
                let label = kept.label();
                self.current(rec, Some(&label), false, now_ms).map(|effect| Taken { outcome, effect, kept: Some(label) })
            }
            Decision::Conflict { current: Chose::Local, kept, .. } => {
                let label = kept.label();
                self.keep_incoming(rec, &label, now_ms).map(|effect| Taken { outcome, effect, kept: Some(label) })
            }
        };
        // The log holds the record now. What the store could not take of it is noted, so a record that is never sent again is
        // still taken; what it took, or is not going to, is not.
        let (vault, record) = (&h.collection, &h.record);
        match &result {
            Err(_) | Ok(Taken { effect: Effect::Deferred(_), .. }) => {
                sync_capture::retake_mark(self.store, vault, record, lost.as_deref()).map_err(|e| format!("the log holds the record, and it could not be noted for `retake`: {e}"))?;
            }
            // Clearing a mark that is not needed any more fails harmlessly: the record is taken again, and the take is the same.
            Ok(_) => drop(sync_capture::retake_clear(self.store, vault, record)),
        }
        result.map_err(|e| format!("{e}: the log holds the record, and `retake` takes it again"))
    }

    /// Takes again the records of `vault` that the log holds and the store did not take (an error, a key not here yet, a kind
    /// this version does not know), from the log's current state of each, as a plain change that follows on: the side of a
    /// conflict that lost stays in the log. A record taken, or no longer in the log, is not noted any more; one that fails again
    /// stays, with the conflict it won (so does one that finds the entry changed here since it was read, which is never written over). Run it after a
    /// capture, at the start of a sync round, and after an upgrade: a change made here is in the log by then.
    pub fn retake(&self, vault: &str, now_ms: i64) -> Result<Vec<(String, Result<Effect, String>)>, String> {
        let mut out = Vec::new();
        for (record, lost) in sync_capture::retake_pending(self.store, vault)? {
            let current = log::current(&self.store.conn().lock().unwrap_or_else(|e| e.into_inner()), vault, &record).map_err(|e| e.to_string())?;
            let Some(rec) = current else {
                sync_capture::retake_clear(self.store, vault, &record)?;
                continue;
            };
            let effect = self.current(&rec, lost.as_deref(), true, now_ms);
            if !matches!(effect, Err(_) | Ok(Effect::Deferred(_))) {
                sync_capture::retake_clear(self.store, vault, &record)?;
            }
            out.push((record, effect));
        }
        Ok(out)
    }

    /// Whether the entry has a rotation waiting to be confirmed (spec 6.3).
    fn rotation_waits(&self, entry_id: &str) -> Result<bool, String> {
        let Some(row) = self.store.vault_get(entry_id).map_err(|e| e.to_string())? else { return Ok(false) };
        Ok(self.store.vault_versions(&row.id, row.created_at).map_err(|e| e.to_string())?.iter().any(|v| v.state == versions::PENDING))
    }

    /// `rec` is now the record's current state: the entry takes its value, or is deleted. `lost` is the other side of a
    /// conflict, which this machine held, and which is kept.
    fn current(&self, rec: &Record, lost: Option<&str>, settled: bool, now_ms: i64) -> Result<Effect, String> {
        match Kind::of(&rec.header.record) {
            Kind::Activity => return self.line(rec),
            Kind::Grant => return Ok(Effect::Skipped("a site grant is not taken into the store yet".to_string())),
            Kind::Entry => {}
        }
        if !sync_links::is_link_id(&rec.header.record) {
            return Ok(Effect::Skipped("a record whose id is not an entry's".to_string()));
        }
        if rec.header.tombstone {
            return self.delete(rec, lost, settled, now_ms);
        }
        let body: EntryBody = open_body(self.ring, rec)?;
        if let Some(why) = refused(&body) {
            return Ok(Effect::Deferred(why));
        }
        // One try: what was decided was decided against the entry as it was read. An entry changed here since is not written
        // over, and the decision is not run again against it: the change here is sent by the next capture, and `retake` takes
        // the record's state then.
        match self.write(rec, &body, lost, now_ms)? {
            Some(effect) => Ok(effect),
            None => Err("the entry was changed here while its record was taken, so nothing was overwritten".to_string()),
        }
    }

    /// One try at writing `body` as the entry's value. `None` if the entry changed here between the read and the write.
    fn write(&self, rec: &Record, body: &EntryBody, lost: Option<&str>, now_ms: i64) -> Result<Option<Effect>, String> {
        let h = &rec.header;
        let store = self.store;
        let existing = match store.vault_entry_of_link(&h.record).map_err(|e| e.to_string())? {
            Some(id) => store.vault_get(&id).map_err(|e| e.to_string())?,
            None => None,
        };
        // The entry is in another Vault here, and that Vault's record of it is live and later than this one: this record is the
        // entry's past (the source Vault's, taken after the destination's), and does not move it back.
        if let Some(old) = existing.as_ref().filter(|r| r.vault_id != h.collection) {
            let there = log::current(&store.conn().lock().unwrap_or_else(|e| e.into_inner()), &old.vault_id, &h.record).map_err(|e| e.to_string())?;
            if there.is_some_and(|t| !t.header.tombstone && t.header.version > h.version) {
                return Ok(Some(Effect::None));
            }
        }
        let entry_id = existing.as_ref().map_or_else(|| uuid::Uuid::new_v4().to_string(), |r| r.id.clone());
        // The ids: the record's own, then the ones the sender says the entry has. A new entry takes the record's as its own.
        let mut ids = vec![h.record.clone()];
        ids.extend(body.link_ids.iter().filter(|i| **i != h.record).cloned());
        let local = if existing.is_some() { links_of(store, &entry_id)? } else { Links::default() };
        let links = sync_links::settle(&Reviving { store, record: &h.record }, &entry_id, &local, &ids);
        if existing.is_none() && links.primary.as_deref() != Some(h.record.as_str()) {
            return Ok(Some(Effect::Deferred("its id is held by another entry here".to_string())));
        }
        let sealed = seal::seal(self.key, &entry_id, &Parts(body.parts.clone()))?;
        let mut row = body.into_row(&entry_id, &h.collection, sealed, existing.as_ref(), self.profiles);
        let machine = h.version.machine.as_str();
        let mut kept = Vec::new();
        match (&existing, lost) {
            // A conflict: the value this machine held is kept as it was, and the tags are the two sides' together.
            (Some(old), Some(why)) => {
                kept.push(version(&old.id, old.created_at, old.sealed.clone(), format!("{KEPT_PREFIX}{why}"), machine, now_ms));
                row.tags = tags_json(&sync_merge::union_tags(&body.tags, &tags_of(old)));
            }
            // A change that follows on: the value it replaces is kept for 7 days, if it differs.
            (Some(old), None) => {
                let same = seal::open(self.key, &old.id, &old.sealed).is_ok_and(|p| p.0 == body.parts);
                if !same {
                    kept.push(version(&old.id, old.created_at, old.sealed.clone(), PREVIOUS.to_string(), machine, now_ms));
                }
            }
            // The entry was deleted here, and this change is later than the delete: it comes back, and the delete is kept.
            (None, Some(why)) => kept.push(version(&entry_id, row.created_at, Vec::new(), format!("{KEPT_PREFIX}{why}"), machine, now_ms)),
            (None, None) => {}
        }
        let write = TakeWrite {
            row: &row,
            was: existing.as_ref(),
            primary: links.primary.as_deref(),
            aliases: &links.aliases,
            revive: existing.is_none().then_some(h.record.as_str()),
            kept: &kept,
        };
        if !store.vault_take_write(&write).map_err(|e| e.to_string())? {
            return Ok(None);
        }
        // What was written is what is marked as seen, not what the store holds by now: a change made here after the write is
        // not in the log, and the next capture has to send it.
        sync_capture::mark_seen(store, &row, &h.record, self.key, self.profiles)?;
        Ok(Some(Effect::Wrote { entry: entry_id }))
    }

    /// An activity line another machine made: added once, to the lines of other machines. A line names the machine that made
    /// it, and only that machine may make it; a line never changes, so a delete of one is not taken.
    fn line(&self, rec: &Record) -> Result<Effect, String> {
        let h = &rec.header;
        let Some((machine, seq)) = sync_activity::parse_record_id(&h.record) else { return Ok(Effect::Skipped("not an activity line".to_string())) };
        if h.tombstone {
            return Ok(Effect::Skipped("an activity line is never deleted".to_string()));
        }
        if machine != h.origin {
            return Ok(Effect::Skipped("an activity line named for another machine".to_string()));
        }
        let body: ActivityBody = open_body(self.ring, rec)?;
        Ok(match apply_activity(self.store, machine, seq, &body)? {
            Decision::Applied => Effect::Line { machine: machine.to_string(), seq },
            _ => Effect::None,
        })
    }

    /// `rec`, a tombstone, is now the record's current state. A conflict's other side is the entry's last value, which the
    /// delete takes with it: it stays in the log.
    ///
    /// A tombstone in a Vault is also what a move to another Vault leaves behind, and the two Vaults' records come separately,
    /// in either order. An entry that moved is not deleted by its old Vault's tombstone: not when the new Vault's record is
    /// already in the log, and not on first sight either, when it may still come in this round. The delete waits one round
    /// (`settled` is false then, true when [`Taker::retake`] takes it again), and by then the new Vault's record has moved the
    /// entry, which keeps its grants, versions and ids; if it has not, the entry is deleted.
    fn delete(&self, rec: &Record, lost: Option<&str>, settled: bool, now_ms: i64) -> Result<Effect, String> {
        let h = &rec.header;
        let Some(id) = self.store.vault_entry_of_link(&h.record).map_err(|e| e.to_string())? else { return Ok(Effect::None) };
        let Some(row) = self.store.vault_get(&id).map_err(|e| e.to_string())? else { return Ok(Effect::None) };
        // An entry that moved to another Vault is not deleted by its old Vault's tombstone.
        if row.vault_id != h.collection {
            return Ok(Effect::None);
        }
        let moved = log::live_elsewhere(&self.store.conn().lock().unwrap_or_else(|e| e.into_inner()), &h.collection, &h.record).map_err(|e| e.to_string())?;
        if moved {
            return Ok(Effect::None);
        }
        // A rotation may have started since the tombstone came (a rotation holds a pending value beside the entry, and is not a
        // change to it): the delete does not take the pending value with it, and is kept beside the entry, as a delete that
        // came while the entry was being rotated is.
        if self.rotation_waits(&id)? {
            return self.keep_incoming(rec, &format!("deleted on {} while this was being rotated", h.version.machine), now_ms);
        }
        if !settled {
            return Ok(Effect::Deferred("deleted in this Vault: it waits one round, in case the entry moved to another Vault whose record has not come yet".to_string()));
        }
        self.store.vault_delete(&id).map_err(|e| e.to_string())?;
        sync_capture::forget(self.store, &id)?;
        Ok(match lost {
            Some(why) => Effect::KeptInLog(why.to_string()),
            None => Effect::Deleted { entry: id },
        })
    }

    /// `rec` lost a conflict: this machine's value stays, and `rec` is kept beside it as a version, with its tags.
    fn keep_incoming(&self, rec: &Record, label: &str, now_ms: i64) -> Result<Effect, String> {
        let h = &rec.header;
        let label = label.to_string();
        let Some(id) = self.store.vault_entry_of_link(&h.record).map_err(|e| e.to_string())? else { return Ok(Effect::KeptInLog(label)) };
        let machine = h.version.machine.as_str();
        let state = format!("{KEPT_PREFIX}{label}");
        // A delete that lost holds no value; a value that lost is sealed under this machine's key, like the entry's own.
        let (sealed, theirs) = if h.tombstone {
            (Vec::new(), None)
        } else {
            let body: EntryBody = open_body(self.ring, rec)?;
            if let Some(why) = refused(&body) {
                return Ok(Effect::Skipped(why));
            }
            (seal::seal(self.key, &id, &Parts(body.parts.clone()))?, Some(body.tags.clone()))
        };
        // One try, as in `current`: an entry changed here since it was read is not written over.
        let Some(row) = self.store.vault_get(&id).map_err(|e| e.to_string())? else { return Ok(Effect::KeptInLog(label)) };
        let mut next = row.clone();
        if let Some(theirs) = &theirs {
            next.tags = tags_json(&sync_merge::union_tags(&tags_of(&row), theirs));
        }
        let links = links_of(self.store, &id)?;
        let kept = [version(&id, row.created_at, sealed, state, machine, now_ms)];
        let write = TakeWrite { row: &next, was: Some(&row), primary: links.primary.as_deref(), aliases: &links.aliases, revive: None, kept: &kept };
        if !self.store.vault_take_write(&write).map_err(|e| e.to_string())? {
            return Err("the entry was changed here while its record was taken, so nothing was overwritten".to_string());
        }
        sync_capture::mark_seen(self.store, &next, &h.record, self.key, self.profiles)?;
        Ok(Effect::Wrote { entry: id })
    }
}

#[cfg(test)]
#[path = "sync_take_tests.rs"]
mod tests;
