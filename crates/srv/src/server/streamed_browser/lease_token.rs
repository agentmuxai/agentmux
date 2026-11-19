// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The lease token a streamed-browser host accepts: who may open this browser, signed by a key the host only holds the
//! public half of.
//!
//!   lease1.<key id>.<lease id>.<workspace id>.<task id>.<address>.<expires, unix seconds>.<token id>.<signature>
//!
//! The signature is Ed25519 over everything before it (base64url, 86 characters). The address is the browser's own,
//! with dashes for dots. The host checks the signature against its public keys, the lease and workspace it was started
//! for, the expiry, and that the token id was not used before: a token opens one connection.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine;
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};

const PREFIX: &str = "lease1";
const MAX_LEN: usize = 320;
/// DER prefix of an Ed25519 public key in SubjectPublicKeyInfo form; the 32 key bytes follow.
const SPKI_PREFIX: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];

/// Why a token was refused, in a word the host logs (never the token).
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Malformed,
    UnknownKey,
    Signature,
    Lease,
    Workspace,
    Expired,
    Used,
}

fn id_ok(s: &str, min: usize, max: usize, extra: &[char]) -> bool {
    (min..=max).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

/// The keys a host is given, `{"<key id>": "<base64 of the SPKI DER>"}`.
pub fn parse_public_keys(json: &str) -> Result<HashMap<String, VerifyingKey>, String> {
    let map: HashMap<String, String> = serde_json::from_str(json).map_err(|e| format!("the public keys are not a JSON object: {e}"))?;
    let mut keys = HashMap::new();
    for (kid, b64) in map {
        if !id_ok(&kid, 1, 8, &[]) {
            return Err(format!("bad key id: {kid}"));
        }
        let der = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| format!("key {kid}: {e}"))?;
        let raw: [u8; 32] = match der.strip_prefix(&SPKI_PREFIX).map(<[u8; 32]>::try_from) {
            Some(Ok(raw)) => raw,
            _ => return Err(format!("key {kid} is not an Ed25519 public key")),
        };
        let key = VerifyingKey::from_bytes(&raw).map_err(|e| format!("key {kid}: {e}"))?;
        keys.insert(kid, key);
    }
    if keys.is_empty() {
        return Err("no public key".to_string());
    }
    Ok(keys)
}

/// What a host checks a token against.
pub struct LeaseGate {
    keys: HashMap<String, VerifyingKey>,
    lease: String,
    workspace: String,
    /// Token ids already accepted, each kept until its token would have expired.
    used: Mutex<HashMap<String, u64>>,
}

impl LeaseGate {
    pub fn new(keys: HashMap<String, VerifyingKey>, lease: String, workspace: String) -> Self {
        Self { keys, lease, workspace, used: Mutex::new(HashMap::new()) }
    }

    /// Accept a token once: `Ok(expires)` marks its id used.
    pub fn accept(&self, token: &str, now: u64) -> Result<u64, Refusal> {
        if token.len() > MAX_LEN {
            return Err(Refusal::Malformed);
        }
        let parts: Vec<&str> = token.split('.').collect();
        let [prefix, kid, lease, workspace, task, address, exp, jti, sig] = parts[..] else { return Err(Refusal::Malformed) };
        let shape_ok = prefix == PREFIX
            && id_ok(kid, 1, 8, &[])
            && id_ok(lease, 1, 64, &['-'])
            && id_ok(workspace, 1, 64, &['-'])
            && id_ok(task, 1, 64, &['-'])
            && id_ok(address, 7, 15, &['-'])
            && id_ok(exp, 1, 12, &[])
            && id_ok(jti, 8, 32, &['-', '_']);
        if !shape_ok {
            return Err(Refusal::Malformed);
        }
        let key = self.keys.get(kid).ok_or(Refusal::UnknownKey)?;
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(sig).map_err(|_| Refusal::Malformed)?;
        let sig = Signature::from_slice(&sig).map_err(|_| Refusal::Malformed)?;
        let signed = &token[..token.len() - parts[8].len() - 1];
        key.verify(signed.as_bytes(), &sig).map_err(|_| Refusal::Signature)?;
        if lease != self.lease {
            return Err(Refusal::Lease);
        }
        if workspace != self.workspace {
            return Err(Refusal::Workspace);
        }
        let expires: u64 = exp.parse().map_err(|_| Refusal::Malformed)?;
        if expires <= now {
            return Err(Refusal::Expired);
        }
        let mut used = self.used.lock().unwrap();
        used.retain(|_, until| *until > now);
        if used.insert(jti.to_string(), expires).is_some() {
            return Err(Refusal::Used);
        }
        Ok(expires)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key (seed of seven bytes of 7) and a token it signed; the same vector is checked from the signing side.
    const KEYS: &str = r#"{"k1":"MCowBQYDK2VwAyEA6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw="}"#;
    const BODY: &str = "lease1.k1.lse-abc123.abcdefghijkl.task-xyz.10-78-1-5.4000000000.nonce12345678";
    const SIG: &str = "19ST-r3qBXo_kn_VjHSVE-nHkfdTm0N_ikXs30W79j4lQfgK-0mpD7SWT9IEnDnTxId_TTt5V1DDYAOHSl5ECg";

    fn verifier() -> LeaseGate {
        LeaseGate::new(parse_public_keys(KEYS).unwrap(), "lse-abc123".into(), "abcdefghijkl".into())
    }

    fn token() -> String {
        format!("{BODY}.{SIG}")
    }

    #[test]
    fn a_signed_token_for_this_lease_opens_once() {
        let v = verifier();
        assert_eq!(v.accept(&token(), 1_000), Ok(4_000_000_000));
        assert_eq!(v.accept(&token(), 1_001), Err(Refusal::Used), "a token opens one connection");
    }

    #[test]
    fn a_token_for_another_lease_or_workspace_is_refused() {
        let other_lease = LeaseGate::new(parse_public_keys(KEYS).unwrap(), "lse-other".into(), "abcdefghijkl".into());
        assert_eq!(other_lease.accept(&token(), 1_000), Err(Refusal::Lease));
        let other_workspace = LeaseGate::new(parse_public_keys(KEYS).unwrap(), "lse-abc123".into(), "mnopqrstuvwx".into());
        assert_eq!(other_workspace.accept(&token(), 1_000), Err(Refusal::Workspace));
    }

    #[test]
    fn an_expired_token_is_refused_and_does_not_use_its_id() {
        let v = verifier();
        assert_eq!(v.accept(&token(), 4_000_000_000), Err(Refusal::Expired));
        assert_eq!(v.accept(&token(), 1_000), Ok(4_000_000_000));
    }

    #[test]
    fn a_changed_token_is_refused() {
        let v = verifier();
        let tampered = token().replace("lse-abc123", "lse-abc124");
        assert_eq!(v.accept(&tampered, 1_000), Err(Refusal::Signature));
        let bad_key = token().replace(".k1.", ".k2.");
        assert_eq!(v.accept(&bad_key, 1_000), Err(Refusal::UnknownKey));
        let mut flipped = token();
        flipped.replace_range(flipped.len() - 2.., "AA");
        assert_eq!(v.accept(&flipped, 1_000), Err(Refusal::Signature));
    }

    #[test]
    fn anything_that_is_not_a_token_is_refused() {
        let v = verifier();
        for bad in ["", "lease1", "v2.a.b.c.d.e", &"a.".repeat(200), &format!("{BODY}.short"), &format!("lease2{}", &token()[6..])] {
            assert_eq!(v.accept(bad, 1_000), Err(Refusal::Malformed), "{bad}");
        }
    }

    #[test]
    fn the_public_keys_must_be_ed25519_keys() {
        assert!(parse_public_keys("{}").is_err());
        assert!(parse_public_keys("[]").is_err());
        assert!(parse_public_keys(r#"{"k1":"AAAA"}"#).is_err());
        assert!(parse_public_keys(&KEYS.replace("k1", "toolongkeyid")).is_err());
    }
}
