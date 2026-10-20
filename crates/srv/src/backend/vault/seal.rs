// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's encryption at rest
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.5).
//!
//! One random 256-bit Vault key lives in `secret_store` (the OS keychain, or
//! the headless file store); each entry's secret parts are sealed with it
//! (ChaCha20-Poly1305, a random nonce each time, the entry id as associated
//! data, so a sealed value moved to another row doesn't open) and kept in
//! the database. One keychain item rather than one per value: Windows'
//! Credential Manager caps an item at 1280 characters, and macOS asks the
//! person about every item separately.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::{Zeroize, Zeroizing};

use crate::identity::secret_store;

/// The Vault key's id in `secret_store`.
const KEY_ID: &str = "vault-key";
const KEY_LEN: usize = 32;
/// The first byte of every sealed value: its format.
const FORMAT_V1: u8 = 1;

/// An entry's secret parts by name. Values are wiped when it is dropped.
#[derive(Default, serde::Deserialize)]
#[serde(transparent)]
pub struct Parts(pub BTreeMap<String, String>);

impl Drop for Parts {
    fn drop(&mut self) {
        for v in self.0.values_mut() {
            v.zeroize();
        }
    }
}

impl Parts {
    pub fn get(&self, part: &str) -> Option<&str> {
        self.0.get(part).map(String::as_str)
    }
}

pub type Key = Zeroizing<[u8; KEY_LEN]>;

/// The key, read once and then kept, so the keychain asks at most once.
static CACHED: Mutex<Option<Key>> = Mutex::new(None);

/// The Vault key. `create` makes one when the keychain has none, which is
/// right only while the Vault has no entries: a new key can't open what an
/// old one sealed.
pub fn key(create: bool) -> Result<Key, String> {
    let mut cached = CACHED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(k) = cached.as_ref() {
        return Ok(k.clone());
    }
    let key = match secret_store::get_optional(KEY_ID)? {
        Some(hex_key) => decode_key(&hex_key)?,
        None if create => {
            let mut k: Key = Zeroizing::new([0u8; KEY_LEN]);
            SystemRandom::new().fill(&mut k[..]).map_err(|_| "the system's random source failed".to_string())?;
            let encoded = Zeroizing::new(hex::encode(&k[..]));
            secret_store::put(KEY_ID, &encoded)?;
            tracing::info!(backend = secret_store::backend_name(), "vault: created the Vault key");
            k
        }
        None => {
            return Err(format!(
                "the Vault key is missing from the {}: the Vault's entries can't be opened",
                secret_store::backend_name()
            ))
        }
    };
    *cached = Some(key.clone());
    Ok(key)
}

fn decode_key(hex_key: &str) -> Result<Key, String> {
    let bytes = Zeroizing::new(hex::decode(hex_key.trim()).map_err(|_| "the stored Vault key is damaged".to_string())?);
    if bytes.len() != KEY_LEN {
        return Err("the stored Vault key is damaged".into());
    }
    let mut k: Key = Zeroizing::new([0u8; KEY_LEN]);
    k.copy_from_slice(&bytes);
    Ok(k)
}

fn aead_key(key: &Key) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key[..]).expect("a 32-byte key"))
}

fn aad(entry_id: &str) -> Vec<u8> {
    format!("agentmux-vault-v1:{entry_id}").into_bytes()
}

/// Seals `parts` for the entry `entry_id`.
pub fn seal(key: &Key, entry_id: &str, parts: &Parts) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce).map_err(|_| "the system's random source failed".to_string())?;
    let mut buf = Zeroizing::new(serde_json::to_vec(&parts.0).map_err(|e| e.to_string())?);
    aead_key(key)
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad(entry_id)), &mut *buf)
        .map_err(|_| "sealing failed".to_string())?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + buf.len());
    out.push(FORMAT_V1);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&buf);
    Ok(out)
}

/// Opens what [`seal`] sealed for `entry_id`. Fails on any change to it,
/// and on a value sealed for another entry.
pub fn open(key: &Key, entry_id: &str, sealed: &[u8]) -> Result<Parts, String> {
    let damaged = || format!("entry {entry_id} can't be opened: it was changed, or sealed with another Vault key");
    if sealed.len() < 1 + NONCE_LEN || sealed[0] != FORMAT_V1 {
        return Err(damaged());
    }
    let nonce = Nonce::try_assume_unique_for_key(&sealed[1..1 + NONCE_LEN]).map_err(|_| damaged())?;
    let mut buf = Zeroizing::new(sealed[1 + NONCE_LEN..].to_vec());
    let plain = aead_key(key).open_in_place(nonce, Aad::from(aad(entry_id)), &mut *buf).map_err(|_| damaged())?;
    let map: BTreeMap<String, String> = serde_json::from_slice(plain).map_err(|_| damaged())?;
    Ok(Parts(map))
}

#[cfg(test)]
pub(crate) fn test_key() -> Key {
    Zeroizing::new([7u8; KEY_LEN])
}

/// Tests use [`test_key`] instead of the keychain.
#[cfg(test)]
pub(crate) fn use_test_key() {
    *CACHED.lock().unwrap_or_else(|p| p.into_inner()) = Some(test_key());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(pairs: &[(&str, &str)]) -> Parts {
        Parts(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    #[test]
    fn a_sealed_entry_opens_and_holds_no_plaintext() {
        let key = test_key();
        let sealed = seal(&key, "e1", &parts(&[("value", "123-45-6789")])).unwrap();
        assert!(!sealed.windows(11).any(|w| w == b"123-45-6789"));
        assert_eq!(open(&key, "e1", &sealed).unwrap().get("value"), Some("123-45-6789"));
    }

    #[test]
    fn sealing_twice_differs() {
        let key = test_key();
        let p = parts(&[("value", "x")]);
        assert_ne!(seal(&key, "e1", &p).unwrap(), seal(&key, "e1", &p).unwrap());
    }

    #[test]
    fn a_value_moved_to_another_entry_or_changed_doesnt_open() {
        let key = test_key();
        let mut sealed = seal(&key, "e1", &parts(&[("value", "secret")])).unwrap();
        assert!(open(&key, "e2", &sealed).is_err(), "bound to its entry");
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(open(&key, "e1", &sealed).is_err(), "tampering is caught");
        let other: Key = Zeroizing::new([8u8; KEY_LEN]);
        let sealed = seal(&key, "e1", &parts(&[("value", "secret")])).unwrap();
        assert!(open(&other, "e1", &sealed).is_err(), "another key can't open it");
    }

    #[test]
    fn a_damaged_stored_key_is_refused() {
        assert!(decode_key("zz").is_err());
        assert!(decode_key(&"ab".repeat(16)).is_err(), "16 bytes is too short");
        assert!(decode_key(&"ab".repeat(32)).is_ok());
    }
}
