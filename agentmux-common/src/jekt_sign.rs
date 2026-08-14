// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Host-tier jekt sender signing/verification (HMAC-SHA256).
//!
//! Shared between `agentmux-mcp` (the `SendMessage` tool signs an outgoing
//! jekt with the calling agent's own key, read from its `AGENTMUX_JEKT_KEY`
//! process env var) and `agentmux-srv` (the reactive handler verifies an
//! incoming jekt's claimed signature against the claimed sender's key,
//! looked up server-side by agent_id — see
//! `agentmux-srv/src/backend/storage/agent_jekt_keys.rs`). Living here, not
//! in either binary, is what guarantees the two can never independently
//! drift on the signed-material format.
//!
//! See docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2 and
//! `docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §5.3 (the
//! original spec's never-built "Phase 5" this module implements).
//!
//! The signed material binds sender, target, message id, and timestamp
//! together (not just msgid+payload) so a valid signature can't be replayed
//! against a different target or reattributed to a different claimed sender.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Field separator inside the signed material — a control character, not a
/// character any of the individual fields can plausibly contain, so
/// concatenation can't be ambiguated by a field that happens to contain the
/// separator (msgid/agent ids are validated elsewhere to be alnum/_/-; the
/// message body is free text, but shifting the separator's *position* within
/// the body can't produce a colliding signature for a different logical
/// (source, target, msgid, ts) tuple, which is what actually matters here).
const FIELD_SEP: char = '\u{1}';

fn signed_material(msgid: &str, source_agent: &str, target_agent: &str, ts_secs: i64, message: &str) -> String {
    format!("{msgid}{FIELD_SEP}{source_agent}{FIELD_SEP}{target_agent}{FIELD_SEP}{ts_secs}{FIELD_SEP}{message}")
}

/// Decode a base64-encoded signing key (e.g. from the `AGENTMUX_JEKT_KEY`
/// process env var). Returns `None` on malformed input rather than erroring
/// — callers should treat that the same as "no key available" (sign
/// nothing, deliver unsigned rather than fail the send). Exposed here so
/// `agentmux-mcp` doesn't need its own `base64` dependency just for this.
pub fn decode_key(b64: &str) -> Option<Vec<u8>> {
    BASE64.decode(b64).ok()
}

/// Sign a jekt with the sender's key. Returns a base64-encoded HMAC-SHA256
/// tag. `key` of any length is accepted (HMAC's own key-hashing handles
/// that) but `agent_jekt_keys::agent_jekt_key_ensure` always mints 32 bytes.
pub fn sign_jekt(key: &[u8], msgid: &str, source_agent: &str, target_agent: &str, ts_secs: i64, message: &str) -> String {
    let material = signed_material(msgid, source_agent, target_agent, ts_secs, message);
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(material.as_bytes());
    BASE64.encode(mac.finalize().into_bytes())
}

/// Verify a claimed signature against the sender's stored key. Constant-time
/// comparison via `Mac::verify_slice` (not a manual `==`), so this can't leak
/// timing information about how much of the tag matched.
pub fn verify_jekt(
    key: &[u8],
    msgid: &str,
    source_agent: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
    sig_b64: &str,
) -> bool {
    let Ok(sig) = BASE64.decode(sig_b64) else { return false };
    let material = signed_material(msgid, source_agent, target_agent, ts_secs, message);
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else { return false };
    mac.update(material.as_bytes());
    mac.verify_slice(&sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Vec<u8> {
        vec![7u8; 32]
    }

    #[test]
    fn a_correctly_signed_message_verifies() {
        let k = key();
        let sig = sign_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "hello");
        assert!(verify_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "hello", &sig));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let sig = sign_jekt(&key(), "msg-1", "agentx", "agenty", 1_000, "hello");
        let other_key = vec![9u8; 32];
        assert!(!verify_jekt(&other_key, "msg-1", "agentx", "agenty", 1_000, "hello", &sig));
    }

    #[test]
    fn tampered_message_fails_verification() {
        let k = key();
        let sig = sign_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "hello");
        assert!(!verify_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "goodbye", &sig));
    }

    #[test]
    fn signature_cannot_be_replayed_against_a_different_target() {
        // The whole point of binding target_agent into the signed material:
        // a signature legitimately produced for agenty must not verify when
        // replayed claiming a different target.
        let k = key();
        let sig = sign_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "hello");
        assert!(!verify_jekt(&k, "msg-1", "agentx", "agentz", 1_000, "hello", &sig));
    }

    #[test]
    fn signature_cannot_be_reattributed_to_a_different_claimed_sender() {
        let k = key();
        let sig = sign_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "hello");
        assert!(!verify_jekt(&k, "msg-1", "agentz", "agenty", 1_000, "hello", &sig));
    }

    #[test]
    fn different_msgid_fails_verification() {
        let k = key();
        let sig = sign_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "hello");
        assert!(!verify_jekt(&k, "msg-2", "agentx", "agenty", 1_000, "hello", &sig));
    }

    #[test]
    fn different_timestamp_fails_verification() {
        let k = key();
        let sig = sign_jekt(&k, "msg-1", "agentx", "agenty", 1_000, "hello");
        assert!(!verify_jekt(&k, "msg-1", "agentx", "agenty", 1_001, "hello", &sig));
    }

    #[test]
    fn malformed_base64_signature_fails_verification_not_panics() {
        assert!(!verify_jekt(&key(), "msg-1", "agentx", "agenty", 1_000, "hello", "not-valid-base64!!"));
    }

    #[test]
    fn empty_signature_fails_verification() {
        assert!(!verify_jekt(&key(), "msg-1", "agentx", "agenty", 1_000, "hello", ""));
    }
}
