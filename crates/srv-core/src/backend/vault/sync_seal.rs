// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Vault record as it leaves the machine: sealed with the Vault's collection key, so a LAN peer's log, the relay and
//! the cloud store see only the clear header (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! section 6.1) and never a label, a kind, a site or a value.
//!
//! The seal is ChaCha20-Poly1305 with a random nonce and the associated data
//! `agentmux-vault-sync-v1:<vault>:<record>:<version>:<epoch>:<base>`, so a record replayed under another Vault, record, version
//! key epoch or base does not open, and an old version can't be put back under the same id. This is a different seal from
//! `seal.rs`, which keeps a value on one machine under that machine's own key: the record is opened with the collection
//! key on arrival and sealed again with the machine key.
//!
//! Every function here is pure: bytes in, bytes out. Nothing reads a store, a clock or the network, so the layer that
//! carries the records (and the cloud service that stores them) needs none of this to be built first.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use serde::de::DeserializeOwned;
use serde::Serialize;
use zeroize::Zeroizing;

use super::seal::Key;

/// The associated data's first part: what the seal is for and its format.
pub const AAD_PREFIX: &str = "agentmux-vault-sync-v1";
/// The first byte of every sealed record: its format.
const FORMAT_V1: u8 = 1;
/// A sealed record travels in pieces of at most this size. A record in AWS Secrets Manager is limited to 64 KB, and the
/// pieces leave room for the framing around them.
pub const MAX_CHUNK: usize = 48 * 1024;
/// The most a sealed record may weigh: a Vault part is at most 1 MiB, and a file entry goes as content-addressed chunks
/// of its own, so this bounds the record and not the file.
pub const MAX_SEALED: usize = 8 * 1024 * 1024;

/// A record's version: the hybrid logical clock the layer gives it, ordered by wall time, then counter, then machine, so
/// any two versions are ordered and a machine with a fast clock cannot make two of its own versions equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub wall_ms: i64,
    pub counter: u32,
    pub machine: String,
}

impl Version {
    pub fn new(wall_ms: i64, counter: u32, machine: &str) -> Self {
        Self { wall_ms, counter, machine: machine.to_string() }
    }

    /// The version as the associated data writes it.
    pub fn text(&self) -> String {
        format!("{}.{}.{}", self.wall_ms, self.counter, self.machine)
    }
}

/// Where a sealed record belongs: the Vault, the record, the version and the key epoch it was sealed at. All four are in
/// the associated data, so a record only opens at the place it was sealed for.
#[derive(Debug, Clone, Copy)]
pub struct Place<'a> {
    pub vault: &'a str,
    pub record: &'a str,
    pub version: &'a Version,
    /// The version the change replaced on the machine that made it, `None` for a first version. It is in the associated
    /// data too: the merge decides from it whether two changes were made apart, so a relay that altered it in the clear
    /// header would otherwise hide a conflict.
    pub base: Option<&'a Version>,
    pub epoch: u32,
}

/// A Vault, record or machine id as it may appear in the associated data: no `:` to run two parts into one.
fn plain_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

impl Place<'_> {
    /// The associated data under a prefix: the Vault's is [`AAD_PREFIX`], and another kind of collection seals its
    /// records the same way under its own (see `user_sync::seal`).
    fn aad_with(&self, prefix: &str) -> Result<Vec<u8>, String> {
        let base_machine = self.base.map(|b| ("machine", b.machine.as_str()));
        for (what, id) in [("Vault", self.vault), ("record", self.record), ("machine", self.version.machine.as_str())].into_iter().chain(base_machine) {
            if !plain_id(id) {
                return Err(format!("a synced record's {what} id is letters, digits and ._- only, at most 128 characters"));
            }
        }
        let base = self.base.map_or_else(|| "-".to_string(), Version::text);
        Ok(format!("{prefix}:{}:{}:{}:{}:{base}", self.vault, self.record, self.version.text(), self.epoch).into_bytes())
    }
}

fn aead_key(key: &Key) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key[..]).expect("a 32-byte key"))
}

const CANT_OPEN: &str = "a synced record can't be opened: it was changed, it was sealed for another place, or it was sealed with another key";

/// Seals `plain` for the place `at`. The plaintext is copied into a buffer that is wiped when this returns.
pub fn seal_raw(key: &Key, at: &Place<'_>, plain: &[u8]) -> Result<Vec<u8>, String> {
    seal_raw_with(AAD_PREFIX, key, at, plain)
}

/// [`seal_raw`] under another associated-data prefix (another kind of collection; `user_sync::seal`).
pub fn seal_raw_with(prefix: &str, key: &Key, at: &Place<'_>, plain: &[u8]) -> Result<Vec<u8>, String> {
    let aad = at.aad_with(prefix)?;
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce).map_err(|_| "the system's random source failed".to_string())?;
    let mut buf = Zeroizing::new(plain.to_vec());
    aead_key(key).seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut *buf).map_err(|_| "sealing a record failed".to_string())?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + buf.len());
    out.push(FORMAT_V1);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&buf);
    if out.len() > MAX_SEALED {
        return Err(format!("a synced record is at most {} MiB sealed", MAX_SEALED / (1024 * 1024)));
    }
    Ok(out)
}

/// Opens what [`seal_raw`] sealed for `at`. Fails, with one sentence for every cause, on any change to the record, on a
/// record sealed for another Vault, record, version or epoch, and on another key. What it returns is wiped when dropped.
pub fn open_raw(key: &Key, at: &Place<'_>, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    open_raw_with(AAD_PREFIX, key, at, sealed)
}

/// [`open_raw`] under another associated-data prefix.
pub fn open_raw_with(prefix: &str, key: &Key, at: &Place<'_>, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    if sealed.len() < 1 + NONCE_LEN || sealed.len() > MAX_SEALED || sealed[0] != FORMAT_V1 {
        return Err(CANT_OPEN.into());
    }
    let aad = at.aad_with(prefix)?;
    let nonce = Nonce::try_assume_unique_for_key(&sealed[1..1 + NONCE_LEN]).map_err(|_| CANT_OPEN.to_string())?;
    let mut buf = Zeroizing::new(sealed[1 + NONCE_LEN..].to_vec());
    let plain = aead_key(key).open_in_place(nonce, Aad::from(aad), &mut *buf).map_err(|_| CANT_OPEN.to_string())?;
    Ok(Zeroizing::new(plain.to_vec()))
}

/// [`seal_raw`] for a body: its JSON, sealed.
pub fn seal_record<T: Serialize>(key: &Key, at: &Place<'_>, body: &T) -> Result<Vec<u8>, String> {
    let json = Zeroizing::new(serde_json::to_vec(body).map_err(|e| e.to_string())?);
    seal_raw(key, at, &json)
}

/// [`open_raw`] for a body. The body's own `Drop` wipes what it holds.
pub fn open_record<T: DeserializeOwned>(key: &Key, at: &Place<'_>, sealed: &[u8]) -> Result<T, String> {
    let plain = open_raw(key, at, sealed)?;
    serde_json::from_slice(&plain).map_err(|_| CANT_OPEN.to_string())
}

/// A tombstone sealed for `at`: it holds nothing, so no value and no label can live on in it, but the seal still
/// vouches for its place, its version and its base, so a relay can neither forge a delete nor change what it follows.
pub fn seal_tombstone(key: &Key, at: &Place<'_>) -> Result<Vec<u8>, String> {
    seal_raw(key, at, &[])
}

/// Opens a tombstone sealed by [`seal_tombstone`]. A record that holds anything is not a tombstone and is refused.
pub fn open_tombstone(key: &Key, at: &Place<'_>, sealed: &[u8]) -> Result<(), String> {
    if open_raw(key, at, sealed)?.is_empty() {
        Ok(())
    } else {
        Err(CANT_OPEN.into())
    }
}

/// Seals again a record under a new key and epoch, for the background pass that follows a member's removal: the new
/// key is sealed to the members who remain, and every record is opened with the old key at the old place and sealed
/// with the new at the new one. The version is the record's own and does not change.
pub fn reseal(old: &Key, from: &Place<'_>, new: &Key, to: &Place<'_>, sealed: &[u8]) -> Result<Vec<u8>, String> {
    let plain = open_raw(old, from, sealed)?;
    seal_raw(new, to, &plain)
}

/// A sealed record in pieces of at most [`MAX_CHUNK`], in order.
pub fn split(sealed: &[u8]) -> Vec<Vec<u8>> {
    sealed.chunks(MAX_CHUNK).map(<[u8]>::to_vec).collect()
}

/// The sealed record back from its pieces. A piece over [`MAX_CHUNK`], a short piece before the last, and a total over
/// [`MAX_SEALED`] are refused: a peer that sends them is not sending what [`split`] makes.
pub fn join(chunks: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let last = chunks.len().saturating_sub(1);
    let odd = chunks.iter().enumerate().any(|(i, c)| c.is_empty() || c.len() > MAX_CHUNK || (i < last && c.len() != MAX_CHUNK));
    if chunks.is_empty() || odd {
        return Err("the pieces of a synced record are not the sizes a record is split into".into());
    }
    let total: usize = chunks.iter().map(Vec::len).sum();
    if total > MAX_SEALED {
        return Err(format!("a synced record is at most {} MiB sealed", MAX_SEALED / (1024 * 1024)));
    }
    Ok(chunks.concat())
}

#[cfg(test)]
#[path = "sync_seal_tests.rs"]
mod tests;
