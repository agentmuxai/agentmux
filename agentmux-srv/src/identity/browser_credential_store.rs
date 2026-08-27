// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! OS-keychain-backed storage for browser-pane HTTP Basic-Auth credentials,
//! scoped per identity so one agent's saved site credential is never
//! reachable through another identity's browsing session.
//!
//! Reuses [`crate::identity::secret_store`]'s existing `keyring`-crate
//! wrapper (already backing `SecretRef::Keychain` for Armory API keys) —
//! this module only builds the right account-key string and JSON-encodes
//! the `{username, password}` pair; no new keychain code, no filesystem
//! path involved (so `agentmux_common::data_paths::sanitize_path_segment`
//! genuinely doesn't apply here — nothing is written to disk).
//!
//! See docs/status/majestic-painting-minsky plan (credential-isolated
//! browser-pane auto-fill) and
//! docs/specs/SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md (the v1 spec
//! that named "Persisted credential store (OS keyring)" as deferred future
//! work — this module is that follow-up).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::identity::secret_store;

/// A saved browser credential. `Zeroize`s on drop so a decoded value
/// doesn't linger in memory longer than the caller actually needs it —
/// mirrors `secret_store::get`'s own `Zeroizing<String>` discipline for
/// the raw keychain read.
#[derive(Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
pub struct BrowserCredential {
    pub username: String,
    pub password: String,
}

/// Build the keychain account string for one (identity, protection-space)
/// pair. Deliberately NOT naive `:`-concatenation of the raw fields —
/// `origin` itself contains `:` (`https://host:8443`) and `realm` is
/// server-supplied, effectively attacker-influenced, free text, so two
/// different (origin, realm) pairs could otherwise concatenate to the same
/// string (e.g. `origin="https://a:1"` + `realm="b"` vs
/// `origin="https://a"` + `realm="1:b"`). Hashing a NUL-separated tuple
/// removes any field-boundary ambiguity — NUL can't appear in either field
/// in practice, but even if it could, the hash input's own delimiter
/// collision risk is the same NUL-in-a-field edge case for both operands,
/// which cancels out (unlike `:`, which is valid content in `origin`).
///
/// `identity_id` is kept as a literal prefix (not folded into the hash)
/// so a corrupted/missing hash can't accidentally resolve under a
/// different identity, and so the account string stays legible in
/// diagnostic contexts (e.g. `keyring`-crate error messages) without
/// revealing the origin/realm themselves — only which identity owns the
/// entry.
fn account_key(identity_id: &str, origin: &str, realm: &str, is_proxy: bool) -> String {
    let mut hasher = Sha256::new();
    hasher.update(origin.as_bytes());
    hasher.update([0u8]);
    hasher.update(realm.as_bytes());
    hasher.update([0u8]);
    hasher.update(if is_proxy { b"1" } else { b"0" });
    let digest_hex = hex::encode(hasher.finalize());
    format!("browser-auth:{identity_id}:{digest_hex}")
}

/// Store (or overwrite) `username`/`password` for this identity +
/// protection-space. Unbounded (no timeout) — see `secret_store`'s doc
/// comment for why a write must not race a timed-out-but-later-completing
/// keychain call.
pub fn save(
    identity_id: &str,
    origin: &str,
    realm: &str,
    is_proxy: bool,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let key = account_key(identity_id, origin, realm, is_proxy);
    let cred = BrowserCredential {
        username: username.to_string(),
        password: password.to_string(),
    };
    let json = serde_json::to_string(&cred).map_err(|e| format!("encode credential: {e}"))?;
    secret_store::put(&key, &json)
}

/// Look up a stored credential. `Ok(None)` means "nothing saved for this
/// identity + protection-space" (not an error) — same contract as
/// `secret_store::get_optional`, which this wraps directly so a transient
/// keychain failure (locked keychain, no Secret Service daemon, permission
/// denied) is distinguishable from "genuinely never saved" at the call
/// site, the same distinction `secret_store`'s own doc comment argues for.
pub fn load(
    identity_id: &str,
    origin: &str,
    realm: &str,
    is_proxy: bool,
) -> Result<Option<BrowserCredential>, String> {
    let key = account_key(identity_id, origin, realm, is_proxy);
    let Some(json) = secret_store::get_optional(&key)? else {
        return Ok(None);
    };
    let cred: BrowserCredential =
        serde_json::from_str(&json).map_err(|e| format!("decode credential: {e}"))?;
    Ok(Some(cred))
}

/// Delete the stored credential for this identity + protection-space.
/// Idempotent (a missing entry is success) — same contract as
/// `secret_store::delete`. Called when a stored credential turns out to be
/// wrong (a re-challenge shortly after an auto-fill — see the
/// credential_broker orchestration in agentmux-cef).
pub fn delete(identity_id: &str, origin: &str, realm: &str, is_proxy: bool) -> Result<(), String> {
    let key = account_key(identity_id, origin, realm, is_proxy);
    secret_store::delete(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_key_is_namespaced_by_identity() {
        let a = account_key("identity-a", "https://example.com", "realm", false);
        let b = account_key("identity-b", "https://example.com", "realm", false);
        assert_ne!(a, b, "different identities must never share an account key");
        assert!(a.starts_with("browser-auth:identity-a:"));
        assert!(b.starts_with("browser-auth:identity-b:"));
    }

    #[test]
    fn account_key_is_stable_for_identical_inputs() {
        let a = account_key("id", "https://example.com", "realm", true);
        let b = account_key("id", "https://example.com", "realm", true);
        assert_eq!(a, b);
    }

    #[test]
    fn account_key_distinguishes_is_proxy() {
        let a = account_key("id", "https://example.com", "realm", true);
        let b = account_key("id", "https://example.com", "realm", false);
        assert_ne!(a, b, "proxy-auth and origin-server-auth caches must not collide");
    }

    /// The exact bug naive `format!("{origin}:{realm}")` concatenation
    /// would have: `origin` itself contains `:` (`https://host:port`), so
    /// two different (origin, realm) pairs can produce the same
    /// concatenated string. The NUL-separated-then-hashed scheme must not
    /// have this problem.
    #[test]
    fn account_key_does_not_collide_across_delimiter_boundaries() {
        let a = account_key("id", "https://a:1", "b", false);
        let b = account_key("id", "https://a", "1:b", false);
        assert_ne!(
            a, b,
            "origin/realm field-boundary shift must not collide (naive ':' concat would fail this)"
        );
    }

    #[test]
    fn account_key_distinguishes_origin() {
        let a = account_key("id", "https://example.com", "realm", false);
        let b = account_key("id", "https://other.com", "realm", false);
        assert_ne!(a, b);
    }

    #[test]
    fn account_key_distinguishes_realm() {
        let a = account_key("id", "https://example.com", "realm-a", false);
        let b = account_key("id", "https://example.com", "realm-b", false);
        assert_ne!(a, b);
    }

    #[test]
    fn browser_credential_json_round_trips() {
        let cred = BrowserCredential {
            username: "user".to_string(),
            password: "pass".to_string(),
        };
        let json = serde_json::to_string(&cred).unwrap();
        let decoded: BrowserCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.username, "user");
        assert_eq!(decoded.password, "pass");
    }
}
