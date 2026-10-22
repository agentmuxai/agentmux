// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bitwarden's password-protected JSON export
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.4). The file is
//!
//! ```json
//! { "encrypted": true, "passwordProtected": true, "salt": "…", "kdfType": 0 or 1,
//!   "kdfIterations": n, "kdfMemory": MiB, "kdfParallelism": n,
//!   "encKeyValidation_DO_NOT_EDIT": "2.iv|ct|mac", "data": "2.iv|ct|mac" }
//! ```
//!
//! and `data` is the ordinary unencrypted export, encrypted. As Bitwarden's own clients write it
//! (`base-vault-export.service.ts`, `default-key-generation.service.ts`, and the SDK's
//! `bitwarden-crypto` for the KDFs):
//! - the key material is the password's UTF-8 bytes through the KDF with the salt's UTF-8 bytes:
//!   PBKDF2-HMAC-SHA256 (`kdfType` 0), or Argon2id (`kdfType` 1; version 0x13, memory `kdfMemory`
//!   MiB, `kdfIterations` passes, `kdfParallelism` lanes, 32 bytes) with the *SHA-256 of the salt*
//!   as its salt;
//! - that 32-byte key is stretched with HKDF-Expand-SHA256 into an encryption key (info `enc`) and
//!   a MAC key (info `mac`), 32 bytes each;
//! - an EncString of type 2 is `2.` and the base64 of the IV, the ciphertext and the MAC, joined by
//!   `|`: AES-256-CBC with PKCS7 padding, the MAC being HMAC-SHA256 over IV then ciphertext.
//!   `encKeyValidation_DO_NOT_EDIT` holds a GUID encrypted the same way, so a wrong password is
//!   known before the data is touched.
//!
//! The cost a file asks for is read first and capped (§8.7), before any password is tried. Keys
//! and the decrypted text are wiped when dropped.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::ReadError;

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type HmacSha256 = Hmac<Sha256>;

/// PBKDF2 rounds a file may ask for. Bitwarden allows up to 2,000,000 and defaults to 600,000.
pub const MAX_PBKDF2_ROUNDS: u32 = 5_000_000;
/// Argon2id: memory in MiB (1 GiB), passes and lanes, as for a KeePass database.
pub const MAX_ARGON_MEMORY_MIB: u32 = 1024;
pub const MAX_ARGON_ITERATIONS: u32 = 100;
pub const MAX_ARGON_LANES: u32 = 64;

/// What Bitwarden's clients write when a file has no `kdfType`: PBKDF2, 100,000 rounds.
const OLD_PBKDF2_ROUNDS: u32 = 100_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Protected {
    #[serde(default)]
    pub password_protected: bool,
    #[serde(default)]
    salt: String,
    kdf_type: Option<u32>,
    kdf_iterations: Option<u32>,
    kdf_memory: Option<u32>,
    kdf_parallelism: Option<u32>,
    #[serde(rename = "encKeyValidation_DO_NOT_EDIT", default)]
    validation: String,
    #[serde(default)]
    data: String,
}

enum Kdf {
    Pbkdf2 { rounds: u32 },
    Argon2id { memory_mib: u32, iterations: u32, lanes: u32 },
}

impl Protected {
    fn kdf(&self) -> Result<Kdf, String> {
        match self.kdf_type.unwrap_or(0) {
            0 => {
                let rounds = self.kdf_iterations.unwrap_or(OLD_PBKDF2_ROUNDS);
                if rounds == 0 || rounds > MAX_PBKDF2_ROUNDS {
                    return Err(too_heavy());
                }
                Ok(Kdf::Pbkdf2 { rounds })
            }
            1 => {
                let (Some(memory_mib), Some(iterations), Some(lanes)) = (self.kdf_memory, self.kdf_iterations, self.kdf_parallelism) else {
                    return Err(DAMAGED.to_string());
                };
                if memory_mib == 0 || iterations == 0 || lanes == 0 || memory_mib > MAX_ARGON_MEMORY_MIB || iterations > MAX_ARGON_ITERATIONS || lanes > MAX_ARGON_LANES {
                    return Err(too_heavy());
                }
                Ok(Kdf::Argon2id { memory_mib, iterations, lanes })
            }
            _ => Err("this Bitwarden export uses a key derivation AgentMux doesn't know".to_string()),
        }
    }
}

const DAMAGED: &str = "this Bitwarden export is damaged or isn't one";

fn too_heavy() -> String {
    "this Bitwarden export asks for more key-stretching to open than AgentMux will spend; open it in Bitwarden and export it unencrypted instead".to_string()
}

/// The 64 bytes Bitwarden's clients derive from a password: an encryption key, then a MAC key.
type StretchedKey = Zeroizing<[u8; 64]>;

fn master_key(password: &[u8], salt: &[u8], kdf: &Kdf) -> Result<Zeroizing<[u8; 32]>, String> {
    let mut out = Zeroizing::new([0u8; 32]);
    match kdf {
        Kdf::Pbkdf2 { rounds } => pbkdf2::pbkdf2_hmac::<Sha256>(password, salt, *rounds, &mut out[..]),
        Kdf::Argon2id { memory_mib, iterations, lanes } => {
            let config = argon2::Config {
                variant: argon2::Variant::Argon2id,
                version: argon2::Version::Version13,
                mem_cost: memory_mib * 1024,
                time_cost: *iterations,
                lanes: *lanes,
                hash_length: 32,
                ..argon2::Config::default()
            };
            let hashed_salt = Sha256::digest(salt);
            let raw = Zeroizing::new(argon2::hash_raw(password, &hashed_salt, &config).map_err(|_| "this Bitwarden export's key derivation couldn't run".to_string())?);
            if raw.len() != 32 {
                return Err(DAMAGED.to_string());
            }
            out.copy_from_slice(&raw);
        }
    }
    Ok(out)
}

/// HKDF-Expand with SHA-256 (RFC 5869): `prk` is already a key, not to be extracted.
fn hkdf_expand(prk: &[u8], info: &[u8], out: &mut [u8]) -> Result<(), String> {
    let hk = hkdf::Hkdf::<Sha256>::from_prk(prk).map_err(|_| DAMAGED.to_string())?;
    hk.expand(info, out).map_err(|_| DAMAGED.to_string())
}

fn stretch(master: &[u8; 32]) -> Result<StretchedKey, String> {
    let mut key: StretchedKey = Zeroizing::new([0u8; 64]);
    let (enc, mac) = key.split_at_mut(32);
    hkdf_expand(master, b"enc", enc)?;
    hkdf_expand(master, b"mac", mac)?;
    Ok(key)
}

/// What an EncString failed on.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failed {
    /// Not a type 2 string, or not well formed.
    Form,
    /// The MAC didn't match: the wrong key, or the data was changed.
    Mac,
}

/// Decrypts a type 2 EncString (`2.iv|ciphertext|mac`) with a 64-byte key (encryption key, MAC key).
pub(super) fn decrypt_enc_string(enc: &str, key: &[u8; 64]) -> Result<Zeroizing<Vec<u8>>, Failed> {
    let body = enc.strip_prefix("2.").ok_or(Failed::Form)?;
    let mut parts = body.split('|');
    let (Some(iv), Some(ct), Some(mac), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { return Err(Failed::Form) };
    let b64 = base64::engine::general_purpose::STANDARD;
    let (iv, ct, mac) = (b64.decode(iv).map_err(|_| Failed::Form)?, b64.decode(ct).map_err(|_| Failed::Form)?, b64.decode(mac).map_err(|_| Failed::Form)?);
    if iv.len() != 16 || ct.is_empty() || ct.len() % 16 != 0 || mac.len() != 32 {
        return Err(Failed::Form);
    }
    let (enc_key, mac_key) = key.split_at(32);
    let mut hmac = HmacSha256::new_from_slice(mac_key).map_err(|_| Failed::Form)?;
    hmac.update(&iv);
    hmac.update(&ct);
    // Constant time: `verify_slice` is.
    hmac.verify_slice(&mac).map_err(|_| Failed::Mac)?;
    let mut buf = Zeroizing::new(ct);
    let n = Aes256CbcDec::new_from_slices(enc_key, &iv).map_err(|_| Failed::Form)?.decrypt_padded_mut::<Pkcs7>(&mut buf).map_err(|_| Failed::Form)?.len();
    buf.truncate(n);
    Ok(buf)
}

/// The export inside, as text, or why not.
pub fn open(protected: &Protected, password: &str) -> Result<Zeroizing<String>, ReadError> {
    let kdf = protected.kdf()?;
    if password.is_empty() {
        return Err(ReadError::NeedsPassphrase);
    }
    if protected.salt.is_empty() || protected.data.is_empty() {
        return Err(DAMAGED.into());
    }
    let master = master_key(password.as_bytes(), protected.salt.as_bytes(), &kdf)?;
    let key = stretch(&master)?;
    // The validation string first: a wrong password is told from a damaged file.
    match decrypt_enc_string(&protected.validation, &key) {
        Ok(_) => {}
        Err(Failed::Mac) => return Err("that password didn't open the Bitwarden export".into()),
        Err(Failed::Form) => return Err(unsupported(&protected.validation)),
    }
    let plain = match decrypt_enc_string(&protected.data, &key) {
        Ok(p) => p,
        Err(Failed::Mac) => return Err("this Bitwarden export is damaged: its data didn't pass its check".into()),
        Err(Failed::Form) => return Err(unsupported(&protected.data)),
    };
    let text = String::from_utf8(plain.to_vec()).map_err(|_| ReadError::from(DAMAGED))?;
    Ok(Zeroizing::new(text))
}

/// An EncString of a type this doesn't read (a newer Bitwarden writes others), or one that isn't one.
fn unsupported(enc: &str) -> ReadError {
    match enc.split('.').next() {
        Some(t) if t.len() == 1 && t != "2" => ReadError::from("this Bitwarden export uses an encryption AgentMux doesn't read yet: export it again unencrypted from Bitwarden"),
        _ => ReadError::from(DAMAGED),
    }
}

#[cfg(test)]
#[path = "bitwarden_encrypted_tests.rs"]
mod tests;
