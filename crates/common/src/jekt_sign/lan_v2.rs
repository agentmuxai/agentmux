//! LAN v2: bound to the sender's UID (identity M4d-6,
//! `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10). Made with
//! the sender's UID-keyed key over the LAN material plus its UID. LAN peers are
//! unauthenticated, so a receiver records the UID only as *claimed* by that
//! LAN path, never as verified attribution. Its own domain: v1 LAN signs the
//! bare material, and neither version's signature verifies as the other.

use super::{BASE64, FIELD_SEP};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

const LAN_V2_DOMAIN: &str = "amx-jekt-lan-v2";

fn lan_v2_signed_material(
    msgid: &str,
    source_agent: &str,
    source_uid: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
) -> String {
    format!(
        "{LAN_V2_DOMAIN}{FIELD_SEP}{msgid}{FIELD_SEP}{source_agent}{FIELD_SEP}{source_uid}{FIELD_SEP}\
         {target_agent}{FIELD_SEP}{ts_secs}{FIELD_SEP}{message}"
    )
}

/// Sign a LAN jekt as `source_uid`, with that UID's own LAN private key.
/// `None` for a key that isn't 32 bytes, like the other signers.
pub fn sign_lan_jekt_v2(
    private_key: &[u8],
    msgid: &str,
    source_agent: &str,
    source_uid: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
) -> Option<String> {
    let seed: [u8; 32] = private_key.try_into().ok()?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let material = lan_v2_signed_material(msgid, source_agent, source_uid, target_agent, ts_secs, message);
    let signature = ed25519_dalek::Signer::sign(&signing_key, material.as_bytes());
    Some(BASE64.encode(signature.to_bytes()))
}

/// Verify a LAN v2 signature against the UID key the answering peer gave.
/// `false`, never a panic, for anything malformed.
#[allow(clippy::too_many_arguments)]
pub fn verify_lan_jekt_v2(
    public_key: &[u8],
    msgid: &str,
    source_agent: &str,
    source_uid: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
    sig_b64: &str,
) -> bool {
    let Ok(pubkey_arr) = <[u8; 32]>::try_from(public_key) else { return false };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pubkey_arr) else { return false };
    let Ok(sig_bytes) = BASE64.decode(sig_b64) else { return false };
    let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else { return false };
    let signature = Signature::from_bytes(&sig_arr);
    let material = lan_v2_signed_material(msgid, source_agent, source_uid, target_agent, ts_secs, message);
    verifying_key.verify(material.as_bytes(), &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jekt_sign::{generate_lan_keypair, sign_lan_jekt, verify_lan_jekt};

    const TS: i64 = 1_790_000_000;

    #[test]
    fn a_lan_v2_signature_verifies_only_for_the_uid_it_binds() {
        let (public, private) = generate_lan_keypair([3u8; 32]);
        let sig = sign_lan_jekt_v2(&private, "m1", "aria", "uid-aria", "lark", TS, "hello").unwrap();
        assert!(verify_lan_jekt_v2(&public, "m1", "aria", "uid-aria", "lark", TS, "hello", &sig));
        assert!(!verify_lan_jekt_v2(&public, "m1", "aria", "uid-other", "lark", TS, "hello", &sig), "another UID");
        assert!(!verify_lan_jekt_v2(&public, "m1", "bob", "uid-aria", "lark", TS, "hello", &sig), "another name");
        assert!(!verify_lan_jekt_v2(&public, "m1", "aria", "uid-aria", "lark", TS, "edited", &sig), "another message");
    }

    #[test]
    fn neither_lan_version_verifies_as_the_other() {
        let (public, private) = generate_lan_keypair([3u8; 32]);
        let v1 = sign_lan_jekt(&private, "m1", "aria", "lark", TS, "hello").unwrap();
        assert!(!verify_lan_jekt_v2(&public, "m1", "aria", "", "lark", TS, "hello", &v1));
        let v2 = sign_lan_jekt_v2(&private, "m1", "aria", "", "lark", TS, "hello").unwrap();
        assert!(!verify_lan_jekt(&public, "m1", "aria", "lark", TS, "hello", &v2));
    }

    #[test]
    fn lan_and_channel_v2_never_verify_as_each_other() {
        let (public, private) = generate_lan_keypair([3u8; 32]);
        let lan = sign_lan_jekt_v2(&private, "m1", "aria", "uid-aria", "lark", TS, "hello").unwrap();
        assert!(!crate::jekt_sign::verify_channel_jekt_v2(
            &public, "m1", "aria", "uid-aria", "", "lark", TS, "hello", &lan
        ));
    }
}
