// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's sync rules applied to the store: an entry's link ids and the activity lines other machines made. The rules
//! are `sync_links` and `sync_activity`; this is what reads and writes the tables they decide for.

use super::sync_activity::{self, ActivityBody};
use super::sync_links::{self, LinkOwners, Links};
use super::sync_merge::Decision;
use crate::backend::storage::store::Store;

/// The store as the one that knows which entry holds an id. A store that cannot answer is taken to hold the id, so an id
/// is never given away on a failed read.
impl LinkOwners for Store {
    fn owner_of(&self, id: &str) -> Option<String> {
        self.vault_link_holder(id).unwrap_or_else(|_| Some(String::new()))
    }
    fn retired(&self, id: &str) -> bool {
        self.vault_link_retired(id).unwrap_or(true)
    }
}

/// The link ids `entry_id` has here: the one this machine gave it, and its aliases.
pub fn links_of(store: &Store, entry_id: &str) -> Result<Links, String> {
    Ok(Links { primary: store.vault_link_primary(entry_id).map_err(|e| e.to_string())?, aliases: store.vault_link_aliases(entry_id).map_err(|e| e.to_string())? })
}

/// Takes the link ids a record for `entry_id` carries (`EntryBody::link_ids`): what this machine had plus the ids it did
/// not, except those another entry holds or a removed entry had. Returns the ids the entry has now.
pub fn apply_links(store: &Store, entry_id: &str, incoming: &[String]) -> Result<Links, String> {
    let local = links_of(store, entry_id)?;
    let settled = sync_links::settle(store, entry_id, &local, incoming);
    if settled != local {
        store.vault_links_set(entry_id, settled.primary.as_deref(), &settled.aliases).map_err(|e| e.to_string())?;
    }
    links_of(store, entry_id)
}

/// Adds the activity line machine `machine` made as its number `seq`, unless this machine already has it. The line goes
/// in the table of other machines' lines, outside this machine's hash chain.
pub fn apply_activity(store: &Store, machine: &str, seq: i64, body: &ActivityBody) -> Result<Decision, String> {
    let have = store.vault_activity_remote_has(machine, seq).map_err(|e| e.to_string())?;
    let decision = sync_activity::merge(have);
    if decision == Decision::Applied {
        store.vault_activity_remote_add(machine, seq, &body.into_row(seq)).map_err(|e| e.to_string())?;
    }
    Ok(decision)
}

#[cfg(test)]
#[path = "sync_store_tests.rs"]
mod tests;
