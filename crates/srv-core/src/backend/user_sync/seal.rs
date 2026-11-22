// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Sealing a record's body (SPEC_USER_DATA_SYNC_LAYER §3.1). A body leaves a machine sealed with the collection's key
//! at the record's key epoch: ChaCha20-Poly1305, with the collection id, record id, version, base version and key
//! epoch as associated data, so a body cannot be replayed under another record, version, base or epoch, nor moved into
//! another collection. The seal is the Vault's (`vault::sync_seal`) under this module's associated-data prefix, so
//! there is one implementation. The layer treats a body as opaque bytes: the caller seals before a record is written
//! ([`sealer`] with `log::append_local_with`) and opens after one is applied ([`open`]).
//!
//! The ids in the associated data are letters, digits and `._-` only (at most 128 characters): a collection or record
//! id with anything else is refused, so no two places can have the same associated data.

use std::collections::BTreeMap;

use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use super::hlc::Version;
use super::log::SealAt;
use super::record::Header;
use crate::backend::vault::seal::Key;
use crate::backend::vault::sync_seal::{self, Place};

/// The associated data's first part for a Kanban board's records.
pub const AAD_PREFIX: &str = "agentmux-sync-v1";

/// A new random collection key, made on the machine that founds a collection or rotates it.
pub fn generate_key() -> Option<Key> {
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).ok()?;
    Some(Zeroizing::new(bytes))
}

/// The keys a member holds for one collection, one per epoch it has been in, so it opens earlier records too.
#[derive(Default)]
pub struct KeyRing {
    keys: BTreeMap<u32, Key>,
}

impl KeyRing {
    pub fn new() -> KeyRing {
        KeyRing::default()
    }

    pub fn insert(&mut self, epoch: u32, key: Key) {
        self.keys.insert(epoch, key);
    }

    pub fn get(&self, epoch: u32) -> Option<&Key> {
        self.keys.get(&epoch)
    }

    /// The newest epoch this ring has a key for.
    pub fn latest(&self) -> Option<u32> {
        self.keys.keys().next_back().copied()
    }
}

/// Why a body would not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The ring has no key for the record's epoch.
    NoKey,
    /// Malformed, altered, or sealed for another place (collection, record, version, base or epoch); or an id the
    /// seal does not take.
    Failed(String),
}

fn old_version(v: &Version) -> sync_seal::Version {
    sync_seal::Version::new(v.wall_ms, v.counter, &v.machine)
}

/// `body` sealed for the place `at` names, under the key the ring holds for that epoch.
pub fn seal(ring: &KeyRing, at: &SealAt, body: &[u8]) -> Result<Vec<u8>, String> {
    let key = ring.get(at.epoch).ok_or_else(|| format!("no key for epoch {}", at.epoch))?;
    let (version, base) = (old_version(&at.version), at.base.as_ref().map(old_version));
    let place = Place { vault: &at.collection, record: &at.record, version: &version, base: base.as_ref(), epoch: at.epoch };
    sync_seal::seal_raw_with(AAD_PREFIX, key, &place, body)
}

/// The body of a record, opened with the key its header's epoch names. The header must be the verified one: its
/// fields are the associated data, so a body altered, or moved to another record, version, base or epoch, fails.
pub fn open(ring: &KeyRing, header: &Header, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, OpenError> {
    let key = ring.get(header.epoch).ok_or(OpenError::NoKey)?;
    let (version, base) = (old_version(&header.version), header.base.as_ref().map(old_version));
    let place = Place { vault: &header.collection, record: &header.record, version: &version, base: base.as_ref(), epoch: header.epoch };
    sync_seal::open_raw_with(AAD_PREFIX, key, &place, sealed).map_err(OpenError::Failed)
}

/// For `log::append_local_with`: seals `body` under the ring's key at the epoch the record will carry.
pub fn sealer<'a>(ring: &'a KeyRing, body: &'a [u8]) -> impl FnOnce(&SealAt) -> Result<Vec<u8>, String> + 'a {
    move |at| seal(ring, at, body)
}
