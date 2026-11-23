// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A collection's keys on a machine, and how they reach another member (SPEC_USER_DATA_SYNC_LAYER §3.1). A collection
//! has one key for each key epoch it has been in. A key is made on the machine that founds the collection or removes a
//! member, kept by a [`KeyStore`], and handed to another member sealed to that member's box key (the key its install
//! offers for sealing, derived from its WAN seed and signed into the membership chain), one sealed key for each epoch,
//! so a new member can open what was written before it joined. A key is never sent unsealed, and a machine that is not
//! a current member is given none.
//!
//! The Kanban's keys are kept in the shared store beside the board they protect ([`SqlKeys`]). A collection with
//! secrets of its own (a Vault) supplies a [`KeyStore`] that keeps its keys under its own protection.

use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use agentmux_common::jekt_sign::wan_key_fingerprint;

use super::members::Subject;
use super::ops::KeyDelivery;
use super::seal::{self, KeyRing};
use crate::backend::jekt_seal::{self, Recipient};
use crate::backend::vault::seal::Key;

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_sync_keys (
        collection TEXT NOT NULL,
        epoch      INTEGER NOT NULL,
        key        BLOB NOT NULL,
        PRIMARY KEY (collection, epoch)
    );";

/// Where a machine keeps the keys of the collections it is a member of.
pub trait KeyStore {
    fn get(&self, collection: &str, epoch: u32) -> Option<Key>;
    /// Keeps the key for an epoch. An epoch that already has a key keeps it: false if it could not be kept.
    fn put(&self, collection: &str, epoch: u32, key: &Key) -> bool;
    /// The epochs a key is held for, lowest first.
    fn epochs(&self, collection: &str) -> Vec<u32>;
    /// Forgets the key of an epoch.
    fn forget(&self, collection: &str, epoch: u32);
}

/// Keys kept in the shared store, for collections whose data is in that store anyway.
pub struct SqlKeys<'a>(pub &'a Connection);

impl KeyStore for SqlKeys<'_> {
    fn get(&self, collection: &str, epoch: u32) -> Option<Key> {
        let bytes: Option<Vec<u8>> =
            self.0.query_row("SELECT key FROM db_sync_keys WHERE collection = ?1 AND epoch = ?2", params![collection, epoch], |r| r.get(0)).optional().ok()?;
        let array: [u8; 32] = bytes?.try_into().ok()?;
        Some(Zeroizing::new(array))
    }

    fn put(&self, collection: &str, epoch: u32, key: &Key) -> bool {
        self.0
            .execute("INSERT OR IGNORE INTO db_sync_keys (collection, epoch, key) VALUES (?1, ?2, ?3)", params![collection, epoch, key.as_slice()])
            .is_ok()
    }

    fn forget(&self, collection: &str, epoch: u32) {
        let _ = self.0.execute("DELETE FROM db_sync_keys WHERE collection = ?1 AND epoch = ?2", params![collection, epoch]);
    }

    fn epochs(&self, collection: &str) -> Vec<u32> {
        let Ok(mut stmt) = self.0.prepare("SELECT epoch FROM db_sync_keys WHERE collection = ?1 ORDER BY epoch") else { return Vec::new() };
        let rows = stmt.query_map(params![collection], |r| r.get::<_, u32>(0));
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }
}

/// The keys a machine holds for a collection, one per epoch, to seal and open records with.
pub fn ring(store: &dyn KeyStore, collection: &str) -> KeyRing {
    let mut ring = KeyRing::new();
    for epoch in store.epochs(collection) {
        if let Some(key) = store.get(collection, epoch) {
            ring.insert(epoch, key);
        }
    }
    ring
}

/// Makes and keeps the key for an epoch if there is none: false if it could not be made or kept.
pub fn mint(store: &dyn KeyStore, collection: &str, epoch: u32) -> bool {
    if store.get(collection, epoch).is_some() {
        return true;
    }
    seal::generate_key().is_some_and(|key| store.put(collection, epoch, &key))
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    super::members::unhex32(s)
}

fn place(epoch: u32) -> String {
    format!("epoch-{epoch}")
}

/// `key` sealed to a member's box key, for this collection and epoch only: it opens nowhere else.
pub fn seal_to(key: &Key, collection: &str, epoch: u32, to: &Subject) -> Option<String> {
    let recipient = Recipient { key: unhex32(&to.box_key)?, key_fp: wan_key_fingerprint(&to.public()?) };
    jekt_seal::wrap(key, &recipient, collection, &place(epoch))
}

/// The key a sealed one holds, opened with this install's WAN seed; none if it was sealed to another key, for another
/// collection or epoch, or altered.
pub fn open_sealed(sealed: &str, wan_public: &[u8; 32], wan_seed: &[u8; 32], collection: &str, epoch: u32) -> Option<Key> {
    jekt_seal::unwrap(sealed, wan_public, wan_seed, collection, &place(epoch)).ok().map(Zeroizing::new)
}

/// This install's side of key delivery over a [`KeyStore`]: seals what it holds to a member, makes the key of a new
/// epoch, and opens keys another member sealed to it.
pub struct StoreKeys<'a> {
    pub store: &'a dyn KeyStore,
    pub wan_public: [u8; 32],
    pub wan_seed: [u8; 32],
}

impl KeyDelivery for StoreKeys<'_> {
    fn keys_for(&self, collection: &str, to: &Subject) -> Vec<(u32, String)> {
        self.store.epochs(collection).into_iter().filter_map(|e| Some((e, seal_to(&self.store.get(collection, e)?, collection, e, to)?))).collect()
    }

    fn mint(&self, collection: &str, epoch: u32) -> bool {
        mint(self.store, collection, epoch)
    }

    fn discard(&self, collection: &str, epoch: u32) {
        self.store.forget(collection, epoch);
    }

    fn take(&self, collection: &str, keys: &[(u32, String)]) -> usize {
        keys.iter()
            .filter(|(epoch, sealed)| {
                open_sealed(sealed, &self.wan_public, &self.wan_seed, collection, *epoch).is_some_and(|key| self.store.put(collection, *epoch, &key))
            })
            .count()
    }
}
