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
use crate::backend::user_sync::seal::{self as user_seal, KeyRing};

/// How many times a take reads the entry again when a change made here lands between its read and its write.
const TRIES: usize = 3;

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
    /// A site grant another machine made was made here, for the entry.
    Granted { entry: String },
    /// A site grant another machine revoked was ended here, for the entry.
    Revoked { entry: String },
    /// The record was not taken into the store, and why (no value in it).
    Skipped(String),
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

/// The entry a record is about.
enum Found {
    None,
    One(String),
    /// The record's ids belong to two entries here.
    Two,
}

/// This machine's side of the sync layer for the Vault: its store, the Vault's keys, this machine's own key, and its profiles.
pub struct Taker<'a> {
    /// This machine's id in the layer (the origin of its own records): the machine a version of its own is "changed on".
    pub machine: &'a str,
    pub store: &'a Store,
    pub ring: &'a KeyRing,
    /// This machine's own key, which seals the values it keeps.
    pub key: &'a Key,
    pub profiles: &'a dyn Profiles,
}

/// The body of `rec`, opened with the Vault's key at the place its header names.
pub(super) fn open_body<T: serde::de::DeserializeOwned>(ring: &KeyRing, rec: &Record) -> Result<T, String> {
    let plain = user_seal::open(ring, &rec.header, &rec.body).map_err(|e| format!("a record that does not open: {e:?}"))?;
    serde_json::from_slice(&plain).map_err(|_| "a record whose body is not what its id says".to_string())
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
    /// Hands `rec` to the layer, and takes its result into the store. Errors from the layer (a record that is refused, out of
    /// sequence or at another epoch) and from the store come back as text, and change nothing in the store.
    pub fn receive(&self, rec: &Record, pins: &Pins, clock: &mut Clock, now_ms: i64) -> Result<Taken, String> {
        let h = &rec.header;
        // Everything that reads the store comes before the log is locked: the log is the store's own connection.
        let holder = match Kind::of(&h.record) {
            Kind::Entry => match self.find(&self.ids_of(rec))? {
                Found::One(id) => Some(id),
                _ => None,
            },
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
        match decision {
            Decision::KeptLocal => Ok(none(outcome)),
            Decision::Applied => {
                let effect = self.current(rec, None, now_ms)?;
                Ok(Taken { outcome, effect, kept: None })
            }
            Decision::Conflict { current: Chose::Incoming, kept, .. } => {
                let label = kept.label();
                let effect = self.current(rec, Some(&kept), now_ms)?;
                Ok(Taken { outcome, effect, kept: Some(label) })
            }
            Decision::Conflict { current: Chose::Local, kept, .. } => {
                let label = kept.label();
                let effect = self.keep_incoming(rec, &kept, now_ms)?;
                Ok(Taken { outcome, effect, kept: Some(label) })
            }
        }
    }

    /// The ids a record names its entry by: its own, then the ones its body says the entry has (a body that does not open, or
    /// a tombstone, names only its own).
    fn ids_of(&self, rec: &Record) -> Vec<String> {
        let mut ids = vec![rec.header.record.clone()];
        if !rec.header.tombstone {
            if let Ok(body) = open_body::<EntryBody>(self.ring, rec) {
                ids.extend(body.link_ids.iter().filter(|i| sync_links::is_link_id(i) && **i != rec.header.record).cloned());
            }
        }
        ids
    }

    /// The entry here that has any of `ids`. Ids that belong to two entries here are not an entry: they are told apart, and
    /// a record that names both is not taken (it would make one entry of two).
    fn find(&self, ids: &[String]) -> Result<Found, String> {
        let mut found: Vec<String> = Vec::new();
        for id in ids {
            if let Some(entry) = self.store.vault_entry_of_link(id).map_err(|e| e.to_string())? {
                if !found.contains(&entry) {
                    found.push(entry);
                }
            }
        }
        Ok(match found.len() {
            0 => Found::None,
            1 => Found::One(found.remove(0)),
            _ => Found::Two,
        })
    }

    /// Whether the entry has a rotation waiting to be confirmed (spec 6.3).
    fn rotation_waits(&self, entry_id: &str) -> Result<bool, String> {
        let Some(row) = self.store.vault_get(entry_id).map_err(|e| e.to_string())? else { return Ok(false) };
        Ok(self.store.vault_versions(&row.id, row.created_at).map_err(|e| e.to_string())?.iter().any(|v| v.state == versions::PENDING))
    }

    /// `rec` is now the record's current state: the entry takes its value, or is deleted. `lost` is the other side of a
    /// conflict, which this machine held, and which is kept.
    fn current(&self, rec: &Record, lost: Option<&Kept>, now_ms: i64) -> Result<Effect, String> {
        match Kind::of(&rec.header.record) {
            Kind::Activity => return self.line(rec),
            Kind::Grant => return self.grant(rec),
            Kind::Entry => {}
        }
        if !sync_links::is_link_id(&rec.header.record) {
            return Ok(Effect::Skipped("a record whose id is not an entry's".to_string()));
        }
        if rec.header.tombstone {
            return self.delete(rec, lost);
        }
        let body: EntryBody = open_body(self.ring, rec)?;
        if let Some(why) = refused(&body) {
            return Ok(Effect::Skipped(why));
        }
        for _ in 0..TRIES {
            if let Some(effect) = self.write(rec, &body, lost, now_ms)? {
                // Grants that came before the entry did are made now.
                if let Effect::Wrote { entry } = &effect {
                    self.apply_waiting(entry, &rec.header.collection)?;
                }
                return Ok(effect);
            }
        }
        Err("the entry kept changing while its record was taken: it will be taken again when the record is next sent".to_string())
    }

    /// One try at writing `body` as the entry's value. `None` if the entry changed here between the read and the write.
    fn write(&self, rec: &Record, body: &EntryBody, lost: Option<&Kept>, now_ms: i64) -> Result<Option<Effect>, String> {
        let h = &rec.header;
        let store = self.store;
        let mut ids = vec![h.record.clone()];
        ids.extend(body.link_ids.iter().filter(|i| **i != h.record).cloned());
        let existing = match self.find(&ids)? {
            Found::None => None,
            Found::One(id) => store.vault_get(&id).map_err(|e| e.to_string())?,
            Found::Two => return Ok(Some(Effect::Skipped("its ids belong to two entries here".to_string()))),
        };
        // One entry is one record. An entry that is written under another record than this one has two: the smaller id's is
        // the one every machine follows, so the larger's is not taken (it names the entry's ids, so a link made from it
        // opens it), and this machine's own larger one is left for the smaller (below).
        let mut adopting: Option<(String, String)> = None;
        if let Some(old) = &existing {
            if let Some((vault, mine)) = sync_capture::identity_of(store, &old.id)? {
                if mine != h.record {
                    if h.record > mine {
                        let local = links_of(store, &old.id)?;
                        let settled = sync_links::settle(store, &old.id, &local, &ids);
                        if settled != local {
                            store.vault_links_set(&old.id, settled.primary.as_deref(), &settled.aliases).map_err(|e| e.to_string())?;
                        }
                        return Ok(Some(Effect::Skipped("another record of an entry here: the one with the smaller id is followed".to_string())));
                    }
                    adopting = Some((vault, mine));
                }
            }
        }
        let entry_id = existing.as_ref().map_or_else(|| uuid::Uuid::new_v4().to_string(), |r| r.id.clone());
        // The ids: the record's own, then the ones the sender says the entry has. A new entry takes the record's as its own.
        let local = if existing.is_some() { links_of(store, &entry_id)? } else { Links::default() };
        let links = sync_links::settle(&Reviving { store, record: &h.record }, &entry_id, &local, &ids);
        if existing.is_none() && links.primary.as_deref() != Some(h.record.as_str()) {
            return Ok(Some(Effect::Skipped("its id is held by another entry here".to_string())));
        }
        let sealed = seal::seal(self.key, &entry_id, &Parts(body.parts.clone()))?;
        let mut row = body.into_row(&entry_id, &h.collection, sealed, existing.as_ref(), self.profiles);
        let machine = h.version.machine.as_str();
        // This machine's own larger record is left for the smaller: if what the entry holds here is not what that record
        // says, this machine's value is kept beside it as a version of its own, as for a change made at the same time.
        let adopt_kept = Kept::SameTime { machine: self.machine.to_string() };
        let lost = match (&adopting, &existing) {
            (Some(_), Some(old)) if lost.is_none() && self.differs(old, &row, body) => Some(&adopt_kept),
            _ => lost,
        };
        let mut kept = Vec::new();
        match (&existing, lost) {
            // A conflict: the value this machine held is kept as it was, and the tags are the two sides' together.
            (Some(old), Some(why)) => {
                kept.push(version(&old.id, old.created_at, old.sealed.clone(), format!("{KEPT_PREFIX}{}", why.label()), machine, now_ms));
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
            (None, Some(why)) => kept.push(version(&entry_id, row.created_at, Vec::new(), format!("{KEPT_PREFIX}{}", why.label()), machine, now_ms)),
            (None, None) => {}
        }
        let write = TakeWrite {
            row: &row,
            was: existing.as_ref().map(|r| r.sealed.as_slice()),
            primary: links.primary.as_deref(),
            aliases: &links.aliases,
            revive: existing.is_none().then_some(h.record.as_str()),
            kept: &kept,
        };
        if !store.vault_take_write(&write).map_err(|e| e.to_string())? {
            return Ok(None);
        }
        let stored = store.vault_get(&entry_id).map_err(|e| e.to_string())?.ok_or_else(|| "the entry taken is not in the store".to_string())?;
        sync_capture::mark_seen(store, &stored, &h.record, self.key, self.profiles)?;
        // The record this machine wrote it under is ended by its next capture, so a machine that has only that one (a
        // new member) does not make a second entry of it.
        if let Some((vault, mine)) = adopting.filter(|(vault, _)| !vault.is_empty()) {
            sync_capture::end_record(store, &vault, &mine)?;
        }
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
        let mut body: ActivityBody = open_body(self.ring, rec)?;
        // A line names its entry by its record; here the entry has an id of its own, if it is here.
        if let Some(entry) = self.store.vault_entry_of_link(&body.entry_id).map_err(|e| e.to_string())? {
            body.entry_id = entry;
        }
        Ok(match apply_activity(self.store, machine, seq, &body)? {
            Decision::Applied => Effect::Line { machine: machine.to_string(), seq },
            _ => Effect::None,
        })
    }

    /// `rec`, a tombstone, is now the record's current state. A conflict's other side is the entry's last value, which the
    /// delete takes with it: it stays in the log.
    fn delete(&self, rec: &Record, lost: Option<&Kept>) -> Result<Effect, String> {
        let h = &rec.header;
        let Some(id) = self.store.vault_entry_of_link(&h.record).map_err(|e| e.to_string())? else { return Ok(Effect::None) };
        let Some(row) = self.store.vault_get(&id).map_err(|e| e.to_string())? else { return Ok(Effect::None) };
        // An entry that moved to another Vault is not deleted by its old Vault's tombstone.
        if row.vault_id != h.collection {
            return Ok(Effect::None);
        }
        // Nor by the end of a record it is no longer written under (it follows another's, see `write`).
        if sync_capture::identity_of(self.store, &id)?.is_some_and(|(_, mine)| mine != h.record) {
            return Ok(Effect::None);
        }
        self.store.vault_delete(&id).map_err(|e| e.to_string())?;
        sync_capture::forget(self.store, &id)?;
        Ok(match lost {
            Some(why) => Effect::KeptInLog(why.label()),
            None => Effect::Deleted { entry: id },
        })
    }

    /// Whether `old`, the entry as it is stored, differs from `new`, the row `body` makes of it.
    fn differs(&self, old: &VaultRow, new: &VaultRow, body: &EntryBody) -> bool {
        let same_parts = seal::open(self.key, &old.id, &old.sealed).is_ok_and(|p| p.0 == body.parts);
        old.kind != new.kind || old.label != new.label || old.fields != new.fields || old.tags != new.tags || !same_parts
    }

    /// `rec` lost a conflict: this machine's value stays, and `rec` is kept beside it as a version, with its tags.
    fn keep_incoming(&self, rec: &Record, why: &Kept, now_ms: i64) -> Result<Effect, String> {
        let h = &rec.header;
        let label = why.label();
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
        for _ in 0..TRIES {
            let Some(row) = self.store.vault_get(&id).map_err(|e| e.to_string())? else { return Ok(Effect::KeptInLog(label)) };
            let mut next = row.clone();
            if let Some(theirs) = &theirs {
                next.tags = tags_json(&sync_merge::union_tags(&tags_of(&row), theirs));
            }
            let links = links_of(self.store, &id)?;
            let kept = [version(&id, row.created_at, sealed.clone(), state.clone(), machine, now_ms)];
            let write = TakeWrite { row: &next, was: Some(row.sealed.as_slice()), primary: links.primary.as_deref(), aliases: &links.aliases, revive: None, kept: &kept };
            if self.store.vault_take_write(&write).map_err(|e| e.to_string())? {
                let stored = self.store.vault_get(&id).map_err(|e| e.to_string())?.ok_or_else(|| "the entry is not in the store".to_string())?;
                sync_capture::mark_seen(self.store, &stored, &h.record, self.key, self.profiles)?;
                return Ok(Effect::Wrote { entry: id });
            }
        }
        Err("the entry kept changing while its record was taken: it will be taken again when the record is next sent".to_string())
    }
}

#[cfg(test)]
#[path = "sync_take_tests.rs"]
mod tests;
