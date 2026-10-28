// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A signed notice that one agent's WAN key was retired.
//!
//! An install replaces an agent's key when the old one should no longer be
//! trusted. The install signs a
//! [`WanKeyRevocation`] naming the key (instance, agent, channel,
//! fingerprint) with its instance key, the same key that certified the agent
//! key in the first place, so only that install can retire it and an agent
//! never can. The directory serves the notice, and a receiver that holds it
//! refuses any message signed by that fingerprint.
//!
//! The byte format must match the cloud's (`wan-keys.ts` there); the fixed
//! vector in the tests below is the contract between the two.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use crate::jekt_sign::{decode_public_key_b64, ed25519_sign_b64, ed25519_verify_b64, wan_instance_id, FIELD_SEP};

/// Domain separator for a key revocation. Distinct from the certificate's and
/// the instance revocation's, so none of their signatures can stand in for one.
const WAN_KEY_REVOKE_DOMAIN: &str = "amx-wan-agent-key-revoke-v1";

fn wan_key_revoke_material(instance_id: &str, agent_id: &str, channel: &str, key_fp: &str, revoked_at: i64) -> String {
    format!(
        "{WAN_KEY_REVOKE_DOMAIN}{FIELD_SEP}{instance_id}{FIELD_SEP}{agent_id}{FIELD_SEP}{channel}{FIELD_SEP}{key_fp}{FIELD_SEP}{revoked_at}"
    )
}

/// One agent key retired by the instance that minted it. Every field is
/// untrusted until [`WanKeyRevocation::verify`] passes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WanKeyRevocation {
    pub instance_id: String,
    pub instance_pubkey: String,
    pub agent_id: String,
    pub channel: String,
    pub key_fp: String,
    pub revoked_at: i64,
    pub sig: String,
}

impl WanKeyRevocation {
    /// Retire the key with fingerprint `key_fp`, signing with the instance
    /// key. `None` for a malformed instance private key; never panics.
    pub fn sign(instance_private_key: &[u8], agent_id: &str, channel: &str, key_fp: &str, revoked_at: i64) -> Option<Self> {
        let seed: [u8; 32] = instance_private_key.try_into().ok()?;
        let instance_public_key = ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        let instance_id = wan_instance_id(&instance_public_key);
        let agent_id = agent_id.to_ascii_lowercase();
        let material = wan_key_revoke_material(&instance_id, &agent_id, channel, key_fp, revoked_at);
        Some(Self {
            instance_id,
            instance_pubkey: BASE64.encode(instance_public_key),
            agent_id,
            channel: channel.to_string(),
            key_fp: key_fp.to_string(),
            revoked_at,
            sig: ed25519_sign_b64(&seed, &material)?,
        })
    }

    /// The instance id is the hash of the instance key, the agent id is
    /// lowercase, and the instance key signed this key's retirement.
    pub fn verify(&self) -> bool {
        let Some(instance_public_key) = decode_public_key_b64(&self.instance_pubkey) else { return false };
        wan_instance_id(&instance_public_key) == self.instance_id
            && self.agent_id == self.agent_id.to_ascii_lowercase()
            && ed25519_verify_b64(
                &instance_public_key,
                &wan_key_revoke_material(&self.instance_id, &self.agent_id, &self.channel, &self.key_fp, self.revoked_at),
                &self.sig,
            )
    }

    /// Whether this names exactly the key a message or record carries
    /// (the agent compared case-insensitively, as every W3-S identifier is).
    pub fn names(&self, instance_id: &str, agent_id: &str, channel: &str, key_fp: &str) -> bool {
        self.instance_id == instance_id.to_ascii_lowercase()
            && self.agent_id == agent_id.to_ascii_lowercase()
            && self.channel == channel
            && self.key_fp == key_fp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cross-language vector, the same as `V_*` in the cloud's wan-keys.test.ts:
    // instance seed [1; 32], and the fingerprint of agent seed [2; 32].
    const V_INSTANCE_PUBKEY: &str = "iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w=";
    const V_INSTANCE_ID: &str = "gr2q7gf5lh6pzfdnurnkvputhm";
    const V_KEY_FP: &str = "ajgD1fBZkCocba-8m6RykhL3yqwIY0zDrnaydSnwOCc";
    const V_KEY_REVOKE_SIG: &str =
        "8OewRgBH2AN0tNh6jVIkB5qX+T0BWkUCezup79Vyaff8vw/VXo9oLdryeNs3a5N/YwKumN89MGpyQXbPuiv2BA==";

    fn vector() -> WanKeyRevocation {
        WanKeyRevocation {
            instance_id: V_INSTANCE_ID.into(),
            instance_pubkey: V_INSTANCE_PUBKEY.into(),
            agent_id: "camper".into(),
            channel: "stable".into(),
            key_fp: V_KEY_FP.into(),
            revoked_at: 1_790_000_600,
            sig: V_KEY_REVOKE_SIG.into(),
        }
    }

    #[test]
    fn signing_gives_the_signature_the_cloud_checks() {
        let signed = WanKeyRevocation::sign(&[1; 32], "Camper", "stable", V_KEY_FP, 1_790_000_600).unwrap();
        assert_eq!(signed, vector());
        assert!(signed.verify());
    }

    #[test]
    fn the_cloud_signed_vector_verifies() {
        assert!(vector().verify());
    }

    #[test]
    fn anything_changed_after_signing_fails() {
        let changes: [(&str, fn(&mut WanKeyRevocation)); 8] = [
            ("agent", |r| r.agent_id = "lark".into()),
            ("uppercased agent", |r| r.agent_id = "Camper".into()),
            ("channel", |r| r.channel = "beta".into()),
            ("fingerprint", |r| r.key_fp = "x".repeat(43)),
            ("time", |r| r.revoked_at += 1),
            ("signature", |r| r.sig = V_KEY_REVOKE_SIG.replace('8', "9")),
            ("instance key", |r| r.instance_pubkey = BASE64.encode([9u8; 32])),
            ("instance id", |r| r.instance_id = "a".repeat(26)),
        ];
        for (what, change) in changes {
            let mut r = vector();
            change(&mut r);
            assert!(!r.verify(), "a changed {what} must not verify");
        }
    }

    #[test]
    fn another_instances_key_cannot_sign_for_this_one() {
        let mut forged = WanKeyRevocation::sign(&[9; 32], "camper", "stable", V_KEY_FP, 1_790_000_600).unwrap();
        forged.instance_id = V_INSTANCE_ID.into();
        assert!(!forged.verify(), "the id is not the hash of the signing key");
        forged.instance_pubkey = V_INSTANCE_PUBKEY.into();
        assert!(!forged.verify(), "the key given is not the one that signed");
    }

    #[test]
    fn a_certificate_or_an_instance_revocation_does_not_stand_in() {
        let cert_sig = "ighbn1y/kyisIolSHwNyRtBqC2u95zQEBtGc3wxwksBUBHrDoUvXmz8yEXY20TFzOBC6IDEsG51WzTNJCrgNAA==";
        assert!(!WanKeyRevocation { sig: cert_sig.into(), ..vector() }.verify());
    }

    #[test]
    fn a_malformed_private_key_signs_nothing() {
        assert!(WanKeyRevocation::sign(&[1; 31], "camper", "stable", V_KEY_FP, 1).is_none());
    }

    #[test]
    fn it_names_exactly_one_key() {
        let r = vector();
        assert!(r.names(V_INSTANCE_ID, "Camper", "stable", V_KEY_FP));
        assert!(!r.names(V_INSTANCE_ID, "camper", "beta", V_KEY_FP));
        assert!(!r.names(V_INSTANCE_ID, "lark", "stable", V_KEY_FP));
        assert!(!r.names("a".repeat(26).as_str(), "camper", "stable", V_KEY_FP));
        assert!(!r.names(V_INSTANCE_ID, "camper", "stable", &"x".repeat(43)));
    }
}
