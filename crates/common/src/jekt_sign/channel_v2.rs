//! Cross-channel v2: bound to the sender's UID (identity M4d-6,
//! `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10). v1 proves
//! only that the sender holds *some* key published under the claimed name,
//! and one name can belong to different agents in different channels. v2 is
//! made with the sender's UID-keyed key and binds its UID, so a receiver that
//! verifies it against the key published for that UID knows which agent sent
//! it. Its own domain, so neither version's signature verifies as the other.

use super::{BASE64, FIELD_SEP};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

const CHANNEL_V2_DOMAIN: &str = "amx-jekt-channel-v2";

#[allow(clippy::too_many_arguments)]
fn channel_v2_signed_material(
    msgid: &str,
    source_agent: &str,
    source_uid: &str,
    source_channel: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
) -> String {
    format!(
        "{CHANNEL_V2_DOMAIN}{FIELD_SEP}{msgid}{FIELD_SEP}{source_agent}{FIELD_SEP}{source_uid}{FIELD_SEP}\
         {source_channel}{FIELD_SEP}{target_agent}{FIELD_SEP}{ts_secs}{FIELD_SEP}{message}"
    )
}

/// Sign a cross-channel jekt as `source_uid`, with that UID's own LAN private
/// key. `None` for a key that isn't 32 bytes, like the other signers.
#[allow(clippy::too_many_arguments)]
pub fn sign_channel_jekt_v2(
    private_key: &[u8],
    msgid: &str,
    source_agent: &str,
    source_uid: &str,
    source_channel: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
) -> Option<String> {
    let seed: [u8; 32] = private_key.try_into().ok()?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let material = channel_v2_signed_material(
        msgid,
        source_agent,
        source_uid,
        source_channel,
        target_agent,
        ts_secs,
        message,
    );
    let signature = ed25519_dalek::Signer::sign(&signing_key, material.as_bytes());
    Some(BASE64.encode(signature.to_bytes()))
}

/// Verify a v2 cross-channel signature against the public key published for
/// `source_uid`. `false`, never a panic, for anything malformed.
#[allow(clippy::too_many_arguments)]
pub fn verify_channel_jekt_v2(
    public_key: &[u8],
    msgid: &str,
    source_agent: &str,
    source_uid: &str,
    source_channel: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
    sig_b64: &str,
) -> bool {
    let Ok(pubkey_arr) = <[u8; 32]>::try_from(public_key) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pubkey_arr) else {
        return false;
    };
    let Ok(sig_bytes) = BASE64.decode(sig_b64) else {
        return false;
    };
    let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_arr);
    let material = channel_v2_signed_material(
        msgid,
        source_agent,
        source_uid,
        source_channel,
        target_agent,
        ts_secs,
        message,
    );
    verifying_key
        .verify(material.as_bytes(), &signature)
        .is_ok()
}

#[cfg(test)]
mod channel_v2_tests {
    use super::*;
    use crate::jekt_sign::{generate_lan_keypair, sign_channel_jekt, verify_channel_jekt};

    const TS: i64 = 1_790_000_000;

    fn sign(private: &[u8], uid: &str) -> String {
        sign_channel_jekt_v2(private, "m1", "aria", uid, "chan-a", "lark", TS, "hello").unwrap()
    }

    #[test]
    fn a_v2_signature_verifies_only_for_the_uid_it_binds() {
        let (public, private) = generate_lan_keypair([9u8; 32]);
        let sig = sign(&private, "uid-aria");
        assert!(verify_channel_jekt_v2(
            &public, "m1", "aria", "uid-aria", "chan-a", "lark", TS, "hello", &sig
        ));
        assert!(
            !verify_channel_jekt_v2(
                &public,
                "m1",
                "aria",
                "uid-other",
                "chan-a",
                "lark",
                TS,
                "hello",
                &sig
            ),
            "another UID"
        );
        assert!(
            !verify_channel_jekt_v2(
                &public, "m1", "aria", "uid-aria", "chan-b", "lark", TS, "hello", &sig
            ),
            "another channel"
        );
        assert!(
            !verify_channel_jekt_v2(
                &public, "m1", "aria", "uid-aria", "chan-a", "lark", TS, "edited", &sig
            ),
            "another message"
        );
    }

    #[test]
    fn neither_version_verifies_as_the_other() {
        let (public, private) = generate_lan_keypair([9u8; 32]);
        let v1 = sign_channel_jekt(&private, "m1", "aria", "chan-a", "lark", TS, "hello").unwrap();
        assert!(!verify_channel_jekt_v2(
            &public, "m1", "aria", "", "chan-a", "lark", TS, "hello", &v1
        ));
        let v2 = sign(&private, "");
        assert!(!verify_channel_jekt(
            &public, "m1", "aria", "chan-a", "lark", TS, "hello", &v2
        ));
    }
}
