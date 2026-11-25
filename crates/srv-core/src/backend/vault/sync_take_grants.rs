// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A site grant from another machine, taken into the store: the half of [`super::sync_take`] for the records
//! [`super::sync_capture_kinds`] writes.
//!
//! A grant record names its entry by the record the entry is written under, so it finds the entry here by any of the entry's
//! link ids. It applies only to the entry it was made for (`GrantBody::applies_to`: the same `created_at`), so a re-made entry
//! of the same name does not take the site. A grant that arrives before its entry waits, and is applied when the entry is
//! taken. A revoke is a tombstone of the grant's record, which says neither entry nor site: the grants here that have that
//! record id, under any id of their entry, are the ones it ends.

use super::sync_capture_kinds as kinds;
use super::sync_grant::{self, GrantBody};
use super::sync_store::links_of;
use super::sync_take::{open_body, Effect, Taker};
use crate::backend::user_sync::log;
use crate::backend::user_sync::record::Record;

impl Taker<'_> {
    /// A grant record, or the revoke of one, that is now the record's current state.
    pub(super) fn grant(&self, rec: &Record) -> Result<Effect, String> {
        let h = &rec.header;
        if h.tombstone {
            return self.revoke(rec);
        }
        let body: GrantBody = open_body(self.ring, rec)?;
        if body.record_id() != h.record {
            return Ok(Effect::Skipped("a grant whose id is not its entry's and its site's".to_string()));
        }
        // Only a site is granted from another machine: a key that names an agent or a setting is this machine's own, and no
        // record makes one.
        if !kinds::is_site(&body.origin) {
            return Ok(Effect::Skipped("a grant for something that is not a site".to_string()));
        }
        self.apply_grant(&h.collection, &h.record, &body)
    }

    /// Makes the grant here if its entry is here and is the entry it was made for; waits for the entry if it is not here.
    fn apply_grant(&self, vault: &str, record: &str, body: &GrantBody) -> Result<Effect, String> {
        let store = self.store;
        let entry = store.vault_entry_of_link(&body.entry_id).map_err(|e| e.to_string())?;
        let row = match entry {
            Some(id) => store.vault_get(&id).map_err(|e| e.to_string())?,
            None => None,
        };
        // An entry that is not here, or is in another Vault here, may yet arrive (or move) here.
        let Some(row) = row.filter(|r| r.vault_id == vault) else {
            kinds::wait_grant(store, vault, record, &body.entry_id)?;
            return Ok(Effect::Skipped("a grant waits until its entry is here".to_string()));
        };
        if !body.applies_to(Some(row.created_at)) {
            kinds::forget_grant(store, vault, record)?;
            return Ok(Effect::Skipped("a grant made for another entry of that name".to_string()));
        }
        store.vault_grant_add(&row.id, &body.origin, body.granted_at, body.entry_created_at).map_err(|e| e.to_string())?;
        if !store.vault_grant_check(&row.id, &body.origin).map_err(|e| e.to_string())? {
            return Err("the entry changed while its grant was taken: it will be taken again when the record is next sent".to_string());
        }
        // What the log holds for this record is what the store holds: a capture does not write it back as a new version.
        kinds::mark_grant(store, vault, record, &row.id, body.granted_at, body.entry_created_at)?;
        Ok(Effect::Granted { entry: row.id })
    }

    /// `rec`, a tombstone, ends every grant here that is that record.
    fn revoke(&self, rec: &Record) -> Result<Effect, String> {
        let h = &rec.header;
        let mut ended = None;
        for (entry, origin, _) in self.store.vault_grant_rows().map_err(|e| e.to_string())? {
            // A grant that is not a site's is never written, so no record ends it.
            if !kinds::is_site(&origin) {
                continue;
            }
            let in_vault = self.store.vault_get(&entry).map_err(|e| e.to_string())?.is_some_and(|r| r.vault_id == h.collection);
            if in_vault && links_of(self.store, &entry)?.ids().iter().any(|id| sync_grant::record_id(id, &origin) == h.record) {
                self.store.vault_grant_revoke(&entry, &origin).map_err(|e| e.to_string())?;
                ended = Some(entry);
            }
        }
        kinds::forget_grant(self.store, &h.collection, &h.record)?;
        Ok(match ended {
            Some(entry) => Effect::Revoked { entry },
            None => Effect::None,
        })
    }

    /// The grants that were waiting for `entry_id`, now that it is here.
    pub(super) fn apply_waiting(&self, entry_id: &str, vault: &str) -> Result<(), String> {
        let links = links_of(self.store, entry_id)?.ids();
        for record in kinds::waiting_for(self.store, vault, &links)? {
            let current = log::current(&self.store.conn().lock().unwrap_or_else(|e| e.into_inner()), vault, &record).map_err(|e| e.to_string())?;
            let Some(rec) = current.filter(|r| !r.header.tombstone) else {
                kinds::forget_grant(self.store, vault, &record)?;
                continue;
            };
            let body: GrantBody = open_body(self.ring, &rec)?;
            self.apply_grant(vault, &record, &body)?;
        }
        Ok(())
    }
}
