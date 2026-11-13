// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A cloud file's key, sealed to the install that receives it
//! (`docs/specs/SPEC_JEKT_FILE_TRANSFER_2026_10_05.md` §7, Phase 5).
//!
//! Without sealing, a cloud manifest carries each file's AES key as plain
//! base64, so the relay, which stores the manifest with the message, could
//! read every file. Here each key is sealed to the receiving agent on the
//! install that runs it:
//!
//! - **The key.** An X25519 key derived from the agent's WAN seed with HKDF,
//!   so it is not the signing key itself (the curve library's advice), and
//!   nothing new is stored: the same seed always gives the same key.
//! - **The offer.** The install signs that key with the agent's WAN key and
//!   offers it in the agent's lease ([`offer`]). The relay only passes it on.
//! - **The check.** The sender fetches the agent's record from the account's
//!   key directory, whose chain the install's instance key anchors, and
//!   checks the offer's signature against it ([`check_offer`]). The relay
//!   can't substitute a key without that install's instance key.
//! - **The seal.** Per file: an ephemeral X25519 key, HKDF-SHA256 over the
//!   shared secret, both public keys, the file's hash and its blob id, and
//!   AES-256-GCM over the file key ([`wrap`], [`unwrap`]).
//!
//! What stays readable: an older receiver's lease offers no key, and its
//! files go as before. A cloud that actively adds an install to the
//! account's directory could be offered files, the residual WAN verification
//! already accepts (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.6).

use agentmux_common::jekt_files::SEALED_KEY_PREFIX;
use agentmux_common::jekt_sign::{self, WanKeyRecord};
use base64::Engine;
use curve25519_dalek::montgomery::MontgomeryPoint;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};

/// Salt for deriving the sealing key from a WAN seed, and the leading field
/// of the offer's signed material.
const SEAL_KEY_DOMAIN: &str = "amx-jekt-files-seal-key-v1";
/// Salt for each file's wrapping key.
const WRAP_DOMAIN: &str = "amx-jekt-files-seal-v1";
const FIELD_SEP: char = '\u{1}';
const TAG_LEN: usize = 16;
/// Ephemeral public key, wrapped key, tag.
const SEALED_LEN: usize = 32 + 32 + TAG_LEN;

const STD: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// The key an install wants one agent's file keys sealed to, as its lease
/// carries it. `instance` and `key_fp` find the agent's record in the key
/// directory; `key` is the X25519 public key and `sig` the agent's WAN key
/// over it, both standard base64.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SealOffer {
    pub instance: String,
    pub key_fp: String,
    pub key: String,
    pub sig: String,
}

/// Where a sender seals to: the receiver's X25519 key, checked, and the
/// fingerprint of the WAN key it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recipient {
    pub key: [u8; 32],
    pub key_fp: String,
}

struct Len32;

impl hkdf::KeyType for Len32 {
    fn len(&self) -> usize {
        32
    }
}

/// The X25519 secret for an agent's WAN seed.
fn seal_secret(wan_seed: &[u8; 32]) -> Option<[u8; 32]> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, SEAL_KEY_DOMAIN.as_bytes()).extract(wan_seed);
    let mut out = [0u8; 32];
    prk.expand(&[b"x25519"], Len32).ok()?.fill(&mut out).ok()?;
    Some(out)
}

fn offer_material(agent_id: &str, instance: &str, key_fp: &str, key_b64: &str) -> String {
    let agent_id = agent_id.to_ascii_lowercase();
    format!("{SEAL_KEY_DOMAIN}{FIELD_SEP}{agent_id}{FIELD_SEP}{instance}{FIELD_SEP}{key_fp}{FIELD_SEP}{key_b64}")
}

/// This install's offer for `agent_id`, whose WAN keypair is `wan_public`
/// and `wan_seed`; `None` for a malformed key.
pub(crate) fn offer(instance_id: &str, agent_id: &str, wan_public: &[u8; 32], wan_seed: &[u8; 32]) -> Option<SealOffer> {
    let signing = ed25519_dalek::SigningKey::from_bytes(wan_seed);
    if signing.verifying_key().to_bytes() != *wan_public {
        return None;
    }
    let key = STD.encode(MontgomeryPoint::mul_base_clamped(seal_secret(wan_seed)?).to_bytes());
    let key_fp = jekt_sign::wan_key_fingerprint(wan_public);
    let material = offer_material(agent_id, instance_id, &key_fp, &key);
    let sig = STD.encode(ed25519_dalek::Signer::sign(&signing, material.as_bytes()).to_bytes());
    Some(SealOffer { instance: instance_id.to_string(), key_fp, key, sig })
}

/// The key `offer` names, if `record` (the agent's entry in the key
/// directory) proves it: the record's chain holds, it is `agent_id`'s record
/// for the offer's install and key, and that key signed the offer. `Err`
/// says why not, to follow the agent's name.
pub(crate) fn check_offer(offer: &SealOffer, record: &WanKeyRecord, agent_id: &str) -> Result<Recipient, &'static str> {
    let wan_public = record.check_chain().ok_or("has a key record whose certificate doesn't check out")?;
    if record.instance_id != offer.instance || record.key_fp != offer.key_fp || !record.agent_id.eq_ignore_ascii_case(agent_id) {
        return Err("offered a sealing key for a different key record");
    }
    let verifying = ed25519_dalek::VerifyingKey::from_bytes(&wan_public).map_err(|_| "has a malformed WAN key")?;
    let sig: [u8; 64] = STD
        .decode(&offer.sig)
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or("offered a malformed sealing key signature")?;
    let material = offer_material(agent_id, &offer.instance, &offer.key_fp, &offer.key);
    verifying
        .verify_strict(material.as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig))
        .map_err(|_| "offered a sealing key its WAN key didn't sign")?;
    let key: [u8; 32] = STD
        .decode(&offer.key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or("offered a malformed sealing key")?;
    Ok(Recipient { key, key_fp: offer.key_fp.clone() })
}

/// The AES key that wraps one file's key: HKDF over the shared secret and
/// everything the seal is for.
fn wrapping_key(shared: &[u8; 32], ephemeral: &[u8; 32], recipient: &[u8; 32], sha256: &str, blob_id: &str) -> Option<LessSafeKey> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, WRAP_DOMAIN.as_bytes()).extract(shared);
    let sep = [FIELD_SEP as u8];
    let info: [&[u8]; 7] = [ephemeral, &sep, recipient, &sep, sha256.as_bytes(), &sep, blob_id.as_bytes()];
    let okm = prk.expand(&info, &AES_256_GCM).ok()?;
    Some(LessSafeKey::new(UnboundKey::from(okm)))
}

/// X25519, refusing the all-zero result a low-order public key gives.
fn agree(secret: [u8; 32], public: &[u8; 32]) -> Option<[u8; 32]> {
    let shared = MontgomeryPoint(*public).mul_clamped(secret).to_bytes();
    (shared != [0u8; 32]).then_some(shared)
}

/// `file_key` sealed to `to`, for the file `sha256` stored as `blob_id`.
pub(crate) fn wrap(file_key: &[u8; 32], to: &Recipient, sha256: &str, blob_id: &str) -> Option<String> {
    let mut secret = [0u8; 32];
    SystemRandom::new().fill(&mut secret).ok()?;
    let ephemeral = MontgomeryPoint::mul_base_clamped(secret).to_bytes();
    let shared = agree(secret, &to.key)?;
    // Each wrapping key comes from a fresh ephemeral key, so it seals once:
    // a fixed nonce is safe.
    let mut sealed = file_key.to_vec();
    wrapping_key(&shared, &ephemeral, &to.key, sha256, blob_id)?
        .seal_in_place_append_tag(Nonce::assume_unique_for_key([0u8; NONCE_LEN]), Aad::empty(), &mut sealed)
        .ok()?;
    let mut body = ephemeral.to_vec();
    body.extend_from_slice(&sealed);
    Some(format!("{SEALED_KEY_PREFIX}{}.{}", to.key_fp, URL.encode(body)))
}

/// Why a sealed key didn't open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnwrapError {
    /// Sealed to another key: another install's copy of this agent, after
    /// its lease moved.
    OtherKey,
    /// Malformed, altered, or for another file.
    Failed,
}

/// The file key `sealed` holds, opened with the agent's WAN seed.
pub(crate) fn unwrap(sealed: &str, wan_public: &[u8; 32], wan_seed: &[u8; 32], sha256: &str, blob_id: &str) -> Result<[u8; 32], UnwrapError> {
    let (key_fp, body) = sealed
        .strip_prefix(SEALED_KEY_PREFIX)
        .and_then(|rest| rest.split_once('.'))
        .ok_or(UnwrapError::Failed)?;
    if key_fp != jekt_sign::wan_key_fingerprint(wan_public) {
        return Err(UnwrapError::OtherKey);
    }
    let body = URL.decode(body).map_err(|_| UnwrapError::Failed)?;
    if body.len() != SEALED_LEN {
        return Err(UnwrapError::Failed);
    }
    let (ephemeral, wrapped) = body.split_at(32);
    let ephemeral: [u8; 32] = ephemeral.try_into().map_err(|_| UnwrapError::Failed)?;
    let secret = seal_secret(wan_seed).ok_or(UnwrapError::Failed)?;
    let own = MontgomeryPoint::mul_base_clamped(secret).to_bytes();
    let shared = agree(secret, &ephemeral).ok_or(UnwrapError::Failed)?;
    let mut buf = wrapped.to_vec();
    let key = wrapping_key(&shared, &ephemeral, &own, sha256, blob_id)
        .ok_or(UnwrapError::Failed)?
        .open_in_place(Nonce::assume_unique_for_key([0u8; NONCE_LEN]), Aad::empty(), &mut buf)
        .map_err(|_| UnwrapError::Failed)?;
    key.try_into().map_err(|_| UnwrapError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "2c17a5b7c1f2e3d4c5b6a79881726354a5b6c7d8e9f0a1b2c3d4e5f60718293a";
    const BLOB: &str = "blob-0123abcd-0000-4000-8000-0123456789ab";

    fn agent(seed: u8) -> ([u8; 32], [u8; 32]) {
        jekt_sign::generate_wan_keypair([seed; 32])
    }

    /// The directory record an install with instance seed `instance` would
    /// publish for `agent_id`'s key.
    fn record(instance: u8, agent_id: &str, wan_public: &[u8; 32]) -> WanKeyRecord {
        WanKeyRecord::certify(&[instance; 32], agent_id, "stable", wan_public, "starpower", 1_792_000_000).unwrap()
    }

    fn recipient(seed: u8) -> (Recipient, [u8; 32], [u8; 32]) {
        let (public, private) = agent(seed);
        let rec = record(9, "jf-mac", &public);
        let offered = offer(&rec.instance_id, "jf-mac", &public, &private).unwrap();
        (check_offer(&offered, &rec, "JF-Mac").unwrap(), public, private)
    }

    #[test]
    fn a_sealed_key_opens_only_for_its_agent_and_its_file() {
        let (to, public, private) = recipient(1);
        let file_key = [7u8; 32];
        let sealed = wrap(&file_key, &to, SHA, BLOB).unwrap();
        assert!(agentmux_common::jekt_files::is_sealed_key(&sealed), "{sealed}");
        assert_eq!(unwrap(&sealed, &public, &private, SHA, BLOB), Ok(file_key));

        let (other_public, other_private) = agent(2);
        assert_eq!(unwrap(&sealed, &other_public, &other_private, SHA, BLOB), Err(UnwrapError::OtherKey));
        // The right fingerprint but not the key behind it.
        assert_eq!(unwrap(&sealed, &public, &other_private, SHA, BLOB), Err(UnwrapError::Failed));
        assert_eq!(unwrap(&sealed, &public, &private, &SHA.replace('2', "3"), BLOB), Err(UnwrapError::Failed), "another file");
        assert_eq!(unwrap(&sealed, &public, &private, SHA, &BLOB.replace('1', "2")), Err(UnwrapError::Failed), "another blob");
        let mut tampered = sealed.clone().into_bytes();
        let at = tampered.len() - 5;
        tampered[at] = if tampered[at] == b'A' { b'B' } else { b'A' };
        assert_eq!(unwrap(std::str::from_utf8(&tampered).unwrap(), &public, &private, SHA, BLOB), Err(UnwrapError::Failed));
        assert_eq!(unwrap("A".repeat(43).as_str(), &public, &private, SHA, BLOB), Err(UnwrapError::Failed));
    }

    #[test]
    fn every_seal_is_fresh() {
        let (to, ..) = recipient(1);
        assert_ne!(wrap(&[7; 32], &to, SHA, BLOB), wrap(&[7; 32], &to, SHA, BLOB));
    }

    #[test]
    fn the_sealing_key_is_not_the_signing_key() {
        let (to, public, _) = recipient(1);
        let signing_as_x25519 = ed25519_dalek::VerifyingKey::from_bytes(&public).unwrap().to_montgomery().to_bytes();
        assert_ne!(to.key, signing_as_x25519);
        assert_eq!(recipient(1).0, to, "the same seed always gives the same key");
    }

    #[test]
    fn an_offer_is_taken_only_with_the_record_that_proves_it() {
        let (public, private) = agent(1);
        let rec = record(9, "jf-mac", &public);
        let offered = offer(&rec.instance_id, "jf-mac", &public, &private).unwrap();
        assert!(check_offer(&offered, &rec, "jf-mac").is_ok());

        // A relay swapping in its own key: signed by a key the record doesn't name.
        let (_, relay_private) = agent(3);
        let (relay_public, _) = agent(3);
        let forged = offer(&rec.instance_id, "jf-mac", &relay_public, &relay_private).unwrap();
        let forged = SealOffer { key_fp: offered.key_fp.clone(), ..forged };
        assert!(check_offer(&forged, &rec, "jf-mac").unwrap_err().contains("didn't sign"));
        let swapped_key = SealOffer { key: forged.key.clone(), ..offered.clone() };
        assert!(check_offer(&swapped_key, &rec, "jf-mac").unwrap_err().contains("didn't sign"));

        // Another agent's, another install's, or a broken record.
        assert!(check_offer(&offered, &rec, "lark").is_err());
        let other_install = record(8, "jf-mac", &public);
        assert!(check_offer(&offered, &other_install, "jf-mac").unwrap_err().contains("different key record"));
        let mut broken = rec.clone();
        broken.host_hint = "elsewhere".into();
        assert!(check_offer(&offered, &broken, "jf-mac").unwrap_err().contains("certificate"));

        // An offer signed for one agent can't be replayed as another's.
        let lark_rec = record(9, "lark", &public);
        assert!(check_offer(&offered, &lark_rec, "lark").unwrap_err().contains("didn't sign"));
    }

    #[test]
    fn an_offer_needs_a_matching_keypair() {
        let (public, _) = agent(1);
        let (_, other_private) = agent(2);
        assert_eq!(offer("i", "jf-mac", &public, &other_private), None);
    }

    #[test]
    fn a_low_order_key_is_refused() {
        let to = Recipient { key: [0u8; 32], key_fp: "x".repeat(43) };
        assert_eq!(wrap(&[7; 32], &to, SHA, BLOB), None);
    }
}
