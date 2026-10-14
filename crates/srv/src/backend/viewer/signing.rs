// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A paired device's key, and the signature its send requests carry
//! (agentmux-mobile's SPEC_SEND_FROM_THE_PHONE_2026_10_13 §4, §5.2.1).
//!
//! Sending makes the phone the operator for an agent, so the viewer token
//! alone isn't enough: a request also carries a signature by a key made in
//! the phone's hardware key store at pairing, which never leaves it. srv
//! keeps the public half (`db_viewer_devices.device_key`) and checks:
//!
//! ```text
//! amxv-sig-v1
//! <METHOD>
//! <path as sent, no query>
//! <X-Amxv-Timestamp, unix ms>
//! <lowercase hex SHA-256 of the body>
//! ```
//!
//! joined by `\n`. The phone's key is ECDSA P-256 (the one curve Android's
//! Keystore and iOS's Secure Enclave both hold in hardware), sent as the
//! 65-byte uncompressed point, signing SHA-256 with a DER signature. A
//! 32-byte key is Ed25519, the shape pairing accepted first; it verifies too.

use base64::Engine as _;
use sha2::{Digest, Sha256};

/// The first line of the signed text; a new format gets a new tag.
pub const SIG_VERSION: &str = "amxv-sig-v1";
/// The header carrying the signing time, unix ms.
pub const TIMESTAMP_HEADER: &str = "x-amxv-timestamp";
/// The header carrying the signature, standard base64.
pub const SIGNATURE_HEADER: &str = "x-amxv-signature";
/// How far a request's timestamp may be from srv's clock, either way.
pub const MAX_SKEW_MS: i64 = 60_000;

/// A device's public key, parsed from its stored base64.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceKey {
    /// The 65-byte uncompressed SEC1 point `0x04 ‖ X ‖ Y`.
    P256(Vec<u8>),
    Ed25519([u8; 32]),
}

impl DeviceKey {
    /// Parse standard base64 of a 65-byte P-256 point or a 32-byte Ed25519
    /// key.
    pub fn parse(b64: &str) -> Result<Self, &'static str> {
        const SHAPE: &str = "device_key must be standard base64 of a 65-byte P-256 point or a 32-byte Ed25519 key";
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| SHAPE)?;
        match bytes.len() {
            // ring checks the point is on the curve when it verifies: a
            // device that paired with a bad point can never send.
            65 if bytes[0] == 0x04 => Ok(Self::P256(bytes)),
            32 => {
                let key: [u8; 32] = bytes.try_into().map_err(|_| SHAPE)?;
                ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|_| "device_key is not an Ed25519 key")?;
                Ok(Self::Ed25519(key))
            }
            _ => Err(SHAPE),
        }
    }

    /// Whether `signature` (standard base64) signs `message` with this key.
    pub fn verifies(&self, message: &[u8], signature_b64: &str) -> bool {
        let Ok(sig) = base64::engine::general_purpose::STANDARD.decode(signature_b64.trim()) else {
            return false;
        };
        match self {
            Self::P256(point) => ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point)
                .verify(message, &sig)
                .is_ok(),
            Self::Ed25519(key) => {
                let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(key) else {
                    return false;
                };
                let Ok(sig) = ed25519_dalek::Signature::from_slice(&sig) else {
                    return false;
                };
                key.verify_strict(message, &sig).is_ok()
            }
        }
    }
}

/// The text a request's signature covers.
pub fn signed_text(method: &str, path: &str, timestamp_ms: i64, body: &[u8]) -> String {
    let body_hash = hex::encode(Sha256::digest(body));
    format!("{SIG_VERSION}\n{method}\n{path}\n{timestamp_ms}\n{body_hash}")
}

/// Whether a request signed at `timestamp_ms` is fresh at `now_ms`.
pub fn fresh(timestamp_ms: i64, now_ms: i64) -> bool {
    (now_ms - timestamp_ms).abs() <= MAX_SKEW_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn p256() -> EcdsaKeyPair {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap()
    }

    #[test]
    fn a_p256_signature_over_the_signed_text_verifies() {
        let pair = p256();
        let key = DeviceKey::parse(&b64(pair.public_key().as_ref())).unwrap();
        assert!(matches!(key, DeviceKey::P256(_)));
        let text = signed_text("POST", "/agentmux/viewer/agents/Clamk/input", 1_791_963_024_123, br#"{"message":"hi"}"#);
        let sig = pair.sign(&SystemRandom::new(), text.as_bytes()).unwrap();
        assert!(key.verifies(text.as_bytes(), &b64(sig.as_ref())));
        // Any change to what was signed fails.
        let other = signed_text("POST", "/agentmux/viewer/agents/Clamk/input", 1_791_963_024_123, br#"{"message":"rm"}"#);
        assert!(!key.verifies(other.as_bytes(), &b64(sig.as_ref())));
        // Another device's key fails.
        let stranger = DeviceKey::parse(&b64(p256().public_key().as_ref())).unwrap();
        assert!(!stranger.verifies(text.as_bytes(), &b64(sig.as_ref())));
        assert!(!key.verifies(text.as_bytes(), "not base64!"));
    }

    #[test]
    fn an_ed25519_signature_verifies() {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let key = DeviceKey::parse(&b64(signing.verifying_key().as_bytes())).unwrap();
        let text = signed_text("POST", "/p", 1, b"{}");
        use ed25519_dalek::Signer as _;
        let sig = signing.sign(text.as_bytes());
        assert!(key.verifies(text.as_bytes(), &b64(&sig.to_bytes())));
        assert!(!key.verifies(b"other", &b64(&sig.to_bytes())));
    }

    #[test]
    fn keys_of_the_wrong_shape_are_refused() {
        assert!(DeviceKey::parse("").is_err());
        assert!(DeviceKey::parse("%%%").is_err());
        assert!(DeviceKey::parse(&b64(&[4u8; 64])).is_err(), "64 bytes is neither shape");
        let mut compressed = vec![0x02];
        compressed.extend([1u8; 32]);
        assert!(DeviceKey::parse(&b64(&compressed)).is_err(), "compressed points are not accepted");
        // A point off the curve parses, and never verifies.
        let mut off = vec![0x04];
        off.extend([1u8; 64]);
        let off = DeviceKey::parse(&b64(&off)).unwrap();
        let pair = p256();
        let sig = pair.sign(&SystemRandom::new(), b"x").unwrap();
        assert!(!off.verifies(b"x", &b64(sig.as_ref())));
    }

    #[test]
    fn the_signed_text_is_five_lines_with_the_body_hash_last() {
        let text = signed_text("POST", "/agentmux/viewer/agents/A%20B/input", 42, b"");
        assert_eq!(
            text,
            "amxv-sig-v1\nPOST\n/agentmux/viewer/agents/A%20B/input\n42\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_timestamp_is_fresh_within_a_minute_either_way() {
        assert!(fresh(100_000, 160_000));
        assert!(fresh(160_000, 100_000));
        assert!(!fresh(100_000, 160_001));
        assert!(!fresh(160_001, 100_000));
    }
}
