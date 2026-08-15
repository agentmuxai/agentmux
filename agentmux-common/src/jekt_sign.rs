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
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
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

// ---- WAN-tier sender signing (Ed25519) ----
//
// Distinct signing scheme from host-tier's HMAC above, and deliberately so:
// host-tier's key is minted per-instance and never leaves the srv process
// that owns it, so a symmetric secret shared only between "sign" and
// "verify" on the very same machine is the right tool. A WAN-tier service
// sender (e.g. the GitHub review-notification consumer, "reagent") is
// verified by *every* AgentMux instance on the network, not just its own
// account — an HMAC key distributed that widely is no longer meaningfully
// secret. Asymmetric signing lets the private key stay in exactly one
// place (agentmux-cloud's Secrets Manager) while every client ships only
// the public key, openly. See
// docs/specs/SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md §6.2 addendum.
//
// Reuses the exact same `signed_material` construction as host-tier HMAC —
// same replay/reattribution-resistance properties (msgid+source+target+ts
// bound into what's signed), just a different signature algorithm over it.

/// Pinned Ed25519 public keys for AgentMux's own WAN-tier service senders.
/// Keyed by `key_id` (carried alongside the signature in the wire format)
/// so a future key rotation can add a new entry without invalidating
/// verification of messages already in flight when signed under the old
/// key. The matching private key is never present in this repo — it lives
/// only in agentmux-cloud's Secrets Manager, held by the signing service.
///
/// `reagent-v1-dev` was a placeholder minted during initial implementation
/// (its private half was generated in a local shell for wiring/testing
/// purposes and was treated as already-exposed from the moment it was
/// generated) — kept registered here, but agentmux-cloud's signer
/// (`muxbus/consumers/github/handler.ts`'s `REAGENT_KEY_ID`) no longer signs
/// under it, so no genuine production traffic uses it. Left in place only
/// so any message that happened to be signed under it before the rotation
/// still verifies, matching this map's whole "add, don't replace" design.
///
/// `reagent-v1` (2026-08-14) is the real production key: generated via
/// `crypto.generateKeyPairSync('ed25519', ...)` in a one-off Node script
/// whose output was piped directly into `aws secretsmanager put-secret-value`
/// without the private key ever being printed, logged, or otherwise
/// appearing in any transcript — the private half lives only in
/// agentmux-cloud's Secrets Manager (`services/infra`'s
/// `reagent-jekt-signing-key` field), consistent with the exposure lesson
/// `reagent-v1-dev` was kept around specifically to document.
fn reagent_public_key(key_id: &str) -> Option<[u8; 32]> {
    match key_id {
        "reagent-v1-dev" => Some([
            104, 98, 241, 120, 99, 50, 230, 185, 228, 117, 241, 110, 130, 85, 252, 75, 141, 152,
            224, 92, 125, 154, 123, 40, 96, 149, 214, 172, 94, 120, 116, 200,
        ]),
        "reagent-v1" => Some([
            185, 72, 99, 0, 119, 36, 15, 50, 117, 0, 134, 93, 66, 39, 65, 21, 64, 229, 3, 157,
            238, 188, 79, 40, 74, 42, 99, 108, 28, 101, 243, 205,
        ]),
        _ => None,
    }
}

/// The one `key_id` whose matching private key is believed secret today —
/// i.e. the only key a caller should treat as *authorization to relax
/// `TIER=sensitive`* for a WAN jekt via rule 1b (the SIG=verified
/// exception), as opposed to merely "the signature checks out
/// cryptographically." `reagent-v1-dev` also verifies successfully via
/// `verify_reagent_jekt` (its entry stays in `reagent_public_key` so
/// already-in-flight dev-signed messages don't break), but its private half
/// is documented above as exposed since the moment it was generated —
/// anyone holding it can mint a signature that verifies. `reagent_verified
/// == Some(true)` alone therefore answers "did a registered key sign this,"
/// not "did a key nobody else has sign this"; callers making a trust
/// decision for rule 1b specifically (not just rendering `SIG=` in a
/// marker) must additionally check the key_id against this function.
///
/// As of `docs/specs/SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md`,
/// failing this check is no longer itself grounds to force
/// `TIER=sensitive` (reagentx P0 on PR #2576 addressed a stricter, now-
/// superseded default — see `agentmux-srv/src/backend/reactive/handler.rs`'s
/// tier-escalation block, which gates forcing on an ACTIVE verification
/// failure, `reagent_verified == Some(false)`, not on failing this
/// trusted-key check). A dev-key-signed message that fails this check
/// simply doesn't qualify for rule 1b's relaxation — it falls through to
/// the declared tier like any other unverified sender, same as a WAN jekt
/// with no signature at all.
pub fn is_reagent_trusted_signing_key(key_id: &str) -> bool {
    key_id == "reagent-v1"
}

/// Verify a WAN-tier jekt's claimed `reagent_sig` against the pinned public
/// key for `key_id`. Returns `false` (never panics) for an unknown
/// `key_id`, malformed base64, a malformed/wrong-length signature, or a
/// signature that simply doesn't verify — callers should treat all of
/// these identically: "not verified," rendering `SIG=invalid` rather than
/// `SIG=verified` in the marker (see `wrap_jekt_message`'s doc comment).
///
/// This function alone does NOT tell you whether the message is safe to
/// treat as more trusted than an ordinary WAN jekt — see
/// `is_reagent_trusted_signing_key`'s doc comment. A message signed under
/// `reagent-v1-dev` verifies here (`true`) exactly the same as one signed
/// under the real production key; only `is_reagent_trusted_signing_key`
/// tells the two apart, and (as of
/// `docs/specs/SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md`) that
/// distinction no longer feeds into whether `TIER=sensitive` is forced —
/// it still matters for whatever future feature wants to lean on genuinely
/// proven reagent identity, just not that decision.
pub fn verify_reagent_jekt(
    key_id: &str,
    msgid: &str,
    source_agent: &str,
    target_agent: &str,
    ts_secs: i64,
    message: &str,
    sig_b64: &str,
) -> bool {
    let Some(pubkey_bytes) = reagent_public_key(key_id) else { return false };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pubkey_bytes) else { return false };
    let Ok(sig_bytes) = BASE64.decode(sig_b64) else { return false };
    let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else { return false };
    let signature = Signature::from_bytes(&sig_arr);
    let material = signed_material(msgid, source_agent, target_agent, ts_secs, message);
    verifying_key.verify(material.as_bytes(), &signature).is_ok()
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

    // ---- verify_reagent_jekt (Ed25519) ----
    //
    // Fixture signature produced offline against the "reagent-v1-dev" pinned
    // public key's matching private key, over signed_material("msg-1",
    // "github-consumer", "agentx", 1000, "hello") — i.e. the exact material
    // `signed_material()` above constructs for those arguments. Exercises the
    // real pinned key path end to end rather than a separate test-only key.
    const FIXTURE_SIG_B64: &str =
        "QehidZjJa2jYLPIPYSsVxUlm86W5Fdbr9PV3P4HJyZwJ68/HZR9EaAL0MpcVtTuZJW2+MMGebc0RH9HITNJGCw==";

    #[test]
    fn a_correctly_signed_reagent_message_verifies() {
        assert!(verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    // Same fixture material, signed under the real production key
    // (`reagent-v1`, provisioned 2026-08-14 — see this key's own doc comment
    // on `reagent_public_key` for how the private half was generated and
    // provisioned without ever appearing in a transcript). Proves the
    // production key is genuinely wired up and verifiable end to end, not
    // just registered as an unused map entry.
    const PROD_FIXTURE_SIG_B64: &str =
        "FCFjcvAzla329a39u8fFxOvRWaH1R2fUn8RsGtP9RaIbLbaS3aXgAQ7YB4ssWV5TDvAeGrwSkHfoeGi11iCPBg==";

    #[test]
    fn a_correctly_signed_reagent_message_verifies_under_the_production_key() {
        assert!(verify_reagent_jekt(
            "reagent-v1",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            PROD_FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn the_dev_and_prod_reagent_keys_are_not_interchangeable() {
        // The prod fixture must NOT verify against the dev key id, and
        // vice versa -- proves these are genuinely two distinct keys, not
        // the same key registered twice under different names.
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            PROD_FIXTURE_SIG_B64,
        ));
        assert!(!verify_reagent_jekt(
            "reagent-v1",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    // ---- is_reagent_trusted_signing_key ----
    //
    // The dev key verifies signatures fine (it's a real registered key) but
    // must never be treated as authorization to relax TIER=sensitive — its
    // private half is documented as exposed since generation. Only the
    // production key is "trusted" in that stronger sense.

    #[test]
    fn the_production_key_is_trusted_for_tier_relaxation() {
        assert!(is_reagent_trusted_signing_key("reagent-v1"));
    }

    #[test]
    fn the_exposed_dev_key_is_not_trusted_for_tier_relaxation_even_though_it_verifies() {
        assert!(!is_reagent_trusted_signing_key("reagent-v1-dev"));
        // Sanity check: it's not untrusted because it's unregistered — it
        // genuinely verifies signatures, it's just not the trusted one.
        assert!(verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn an_unregistered_key_id_is_not_trusted_for_tier_relaxation() {
        assert!(!is_reagent_trusted_signing_key("reagent-v2-does-not-exist"));
    }

    #[test]
    fn reagent_unknown_key_id_fails_verification() {
        assert!(!verify_reagent_jekt(
            "reagent-v2-does-not-exist",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn reagent_tampered_message_fails_verification() {
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "goodbye",
            FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn reagent_wrong_target_fails_verification() {
        // Same reattribution/replay resistance as host-tier HMAC — the
        // signature is bound to target_agent, so it can't be replayed
        // claiming delivery to a different agent.
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agenty",
            1_000,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn reagent_wrong_source_fails_verification() {
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "someone-else",
            "agentx",
            1_000,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn reagent_wrong_timestamp_fails_verification() {
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_001,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn reagent_wrong_msgid_fails_verification() {
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-2",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            FIXTURE_SIG_B64,
        ));
    }

    #[test]
    fn reagent_malformed_base64_signature_fails_verification_not_panics() {
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            "not-valid-base64!!",
        ));
    }

    #[test]
    fn reagent_wrong_length_signature_fails_verification_not_panics() {
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            "dG9vc2hvcnQ=", // "tooshort" base64 — decodes fine, wrong length
        ));
    }

    #[test]
    fn reagent_empty_signature_fails_verification() {
        assert!(!verify_reagent_jekt(
            "reagent-v1-dev",
            "msg-1",
            "github-consumer",
            "agentx",
            1_000,
            "hello",
            "",
        ));
    }
}
