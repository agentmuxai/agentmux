// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Signed widget packages (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §2).
//!
//! A package may carry `widget.sig`: an Ed25519 signature by its publisher
//! over its id, version and content hash. The content hash leaves
//! `widget.sig` out, so signing doesn't change what was approved. srv pins
//! each publisher (the part of the id before the dot) to the first key the
//! user approved for it, and reports any later package of that publisher
//! signed by another key, or not signed, as `key_changed`.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SIG_FILE: &str = "widget.sig";
const SIG_VERSION: u32 = 1;
const CONTEXT: &[u8] = b"agentmux-widget-sig-v1\0";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigFile {
    sig_version: u32,
    algorithm: String,
    public_key: String,
    signature: String,
}

/// The bytes a publisher signs.
pub fn signed_message(id: &str, version: &str, hash: &str) -> Vec<u8> {
    let mut m = CONTEXT.to_vec();
    m.extend_from_slice(id.as_bytes());
    m.push(0);
    m.extend_from_slice(version.as_bytes());
    m.push(0);
    m.extend_from_slice(hash.as_bytes());
    m
}

/// `widget.sig` in `dir`: `None` when there is none, the publisher's public
/// key (base64) when it verifies, or why it doesn't.
pub fn check(dir: &Path, id: &str, version: &str, hash: &str) -> Option<Result<String, String>> {
    let text = match std::fs::read_to_string(dir.join(SIG_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(format!("can't read {SIG_FILE}: {e}"))),
    };
    Some(verify(&text, id, version, hash))
}

fn verify(text: &str, id: &str, version: &str, hash: &str) -> Result<String, String> {
    let sig: SigFile = serde_json::from_str(text).map_err(|e| format!("{SIG_FILE}: {e}"))?;
    if sig.sig_version != SIG_VERSION {
        return Err(format!("{SIG_FILE} is version {}, this AgentMux reads {SIG_VERSION}", sig.sig_version));
    }
    if sig.algorithm != "ed25519" {
        return Err(format!("{SIG_FILE} uses {:?}, this AgentMux reads ed25519", sig.algorithm));
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let key: [u8; 32] = b64
        .decode(sig.public_key.trim())
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or("publicKey isn't a 32-byte base64 key")?;
    let signature: [u8; 64] = b64
        .decode(sig.signature.trim())
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or("signature isn't 64 bytes of base64")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "publicKey isn't a valid Ed25519 key")?;
    key.verify(&signed_message(id, version, hash), &Signature::from_bytes(&signature))
        .map_err(|_| "its signature doesn't match its files".to_string())?;
    Ok(b64.encode(key.as_bytes()))
}

/// A key as people compare it: the first 10 bytes of its SHA-256, base32,
/// in groups of four (`K7Q2-MZ4D-PX3A-9TWE`).
pub fn fingerprint(public_key_b64: &str) -> String {
    let Ok(key) = base64::engine::general_purpose::STANDARD.decode(public_key_b64) else {
        return String::new();
    };
    let digest = Sha256::digest(&key);
    let text = base32(&digest[..10]);
    text.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
}

/// RFC 4648 base32, no padding.
fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The publisher part of a package id (`acme` of `acme.pr-dashboard`).
pub fn publisher_of(id: &str) -> &str {
    id.split_once('.').map_or(id, |(p, _)| p)
}

// ── What the UI sees ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum SignatureState {
    /// Signed by the key this publisher is pinned to.
    Signed,
    /// Signed; the first package of this publisher here.
    SignedNew,
    Unsigned,
    /// Signed by another key than the pinned one, or not signed while the
    /// publisher is pinned.
    KeyChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetSignatureInfo {
    pub state: SignatureState,
    pub publisher: String,
    /// The fingerprint of the key that signed it.
    pub fingerprint: Option<String>,
    /// The fingerprint of the key this publisher is pinned to.
    pub pinned: Option<String>,
}

/// A publisher pinned to a key, for Settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetPublisherPin {
    pub publisher: String,
    pub fingerprint: String,
}

/// publisher → public key (base64).
pub type Pins = BTreeMap<String, String>;

pub fn describe(id: &str, signer: Option<&str>, pins: &Pins) -> WidgetSignatureInfo {
    let publisher = publisher_of(id).to_string();
    let pinned = pins.get(&publisher);
    let state = match (signer, pinned) {
        (Some(k), Some(p)) if k == p => SignatureState::Signed,
        (Some(_), Some(_)) | (None, Some(_)) => SignatureState::KeyChanged,
        (Some(_), None) => SignatureState::SignedNew,
        (None, None) => SignatureState::Unsigned,
    };
    WidgetSignatureInfo { state, publisher, fingerprint: signer.map(fingerprint), pinned: pinned.map(|p| fingerprint(p)) }
}

pub fn read_pins(path: &Path) -> Pins {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_pins(path: &Path, pins: &Pins) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(pins).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
pub mod test_keys {
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};

    /// A `widget.sig` for `id`/`version`/`hash`, signed by the key from `seed`.
    pub fn sig_file(seed: u8, id: &str, version: &str, hash: &str) -> String {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let b64 = base64::engine::general_purpose::STANDARD;
        let sig = key.sign(&super::signed_message(id, version, hash));
        serde_json::json!({
            "sigVersion": 1,
            "algorithm": "ed25519",
            "publicKey": b64.encode(key.verifying_key().as_bytes()),
            "signature": b64.encode(sig.to_bytes()),
        })
        .to_string()
    }

    pub fn public_key(seed: u8) -> String {
        base64::engine::general_purpose::STANDARD.encode(SigningKey::from_bytes(&[seed; 32]).verifying_key().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_verifies_only_for_its_own_id_version_and_hash() {
        let sig = test_keys::sig_file(7, "acme.x", "1.0.0", "abc");
        assert_eq!(verify(&sig, "acme.x", "1.0.0", "abc").unwrap(), test_keys::public_key(7));
        assert!(verify(&sig, "acme.x", "1.0.1", "abc").is_err());
        assert!(verify(&sig, "acme.x", "1.0.0", "abd").is_err());
        assert!(verify(&sig, "evil.x", "1.0.0", "abc").is_err());
        assert!(verify("{}", "acme.x", "1.0.0", "abc").is_err());
    }

    /// The fixture the SDK's `agentmux-widget sign` signed with the same test
    /// seed: both sides must agree on its hash, its signature and its
    /// fingerprint (frontend/app/block/widget-sign.test.ts checks the SDK's).
    #[test]
    fn the_sdk_signed_fixture_verifies_here() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/widget-sdk/fixtures/acme.fixture");
        let files = crate::backend::widget_packages::hash_files(&dir).unwrap();
        assert!(!files.contains_key(SIG_FILE));
        let hash = crate::backend::widget_packages::package_hash(&files);
        assert_eq!(hash, "2f480223aad83d3e220179f1e67c8bd660d5bec197cd334b71f514661018b202");
        assert_eq!(check(&dir, "acme.fixture", "1.0.0", &hash).unwrap().unwrap(), test_keys::public_key(7));
        assert_eq!(fingerprint(&test_keys::public_key(7)), "72AS-YEXT-VNGO-NLC5");
    }

    #[test]
    fn fingerprints_are_grouped_base32() {
        let f = fingerprint(&test_keys::public_key(7));
        assert_eq!(f.len(), 19);
        assert_eq!(f.matches('-').count(), 3);
        assert!(f.chars().all(|c| c == '-' || c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
        assert_ne!(f, fingerprint(&test_keys::public_key(8)));
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn a_pinned_publisher_flags_another_key_or_no_signature() {
        let (k1, k2) = (test_keys::public_key(1), test_keys::public_key(2));
        let mut pins = Pins::new();
        assert_eq!(describe("acme.x", Some(&k1), &pins).state, SignatureState::SignedNew);
        assert_eq!(describe("acme.x", None, &pins).state, SignatureState::Unsigned);
        pins.insert("acme".into(), k1.clone());
        assert_eq!(describe("acme.y", Some(&k1), &pins).state, SignatureState::Signed);
        assert_eq!(describe("acme.y", Some(&k2), &pins).state, SignatureState::KeyChanged);
        assert_eq!(describe("acme.y", None, &pins).state, SignatureState::KeyChanged);
        assert_eq!(describe("other.y", None, &pins).state, SignatureState::Unsigned);
    }
}
