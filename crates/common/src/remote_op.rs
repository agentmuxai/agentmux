// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A remote operation: a typed request one AgentMux sends another on behalf of
//! one of its agents, which the receiving AgentMux carries out itself
//! (`docs/specs/SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` §4.1).
//!
//! The envelope and its signature live here, beside the jekt signers, so the
//! sending and the receiving srv can't drift on the signed material. The
//! signature is Ed25519 with the requesting agent's LAN key, in a domain of its
//! own: a jekt's signature never verifies as a remote op, and the reverse.
//! It covers every field the receiver acts on, the `params` included, so a
//! request can't be re-aimed (another `audience`), re-attributed (another
//! `requester`), re-timed or given other parameters.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::jekt_sign::FIELD_SEP;

/// The signature's domain separator.
const REMOTE_OP_DOMAIN: &str = "amx-remote-op-v1";

/// The envelope version this build sends and accepts.
pub const REMOTE_OP_VERSION: u32 = 1;

/// How far from the receiver's clock a request's `ts` may be, in seconds.
pub const REMOTE_OP_MAX_SKEW_SECS: i64 = 60;

/// The longest `op` name, `nonce` and `reason`, and the largest serialized
/// `params`, a receiver reads.
pub const MAX_OP_LEN: usize = 64;
pub const MAX_NONCE_LEN: usize = 64;
pub const MAX_REASON_LEN: usize = 500;
pub const MAX_PARAMS_BYTES: usize = 16 * 1024;

/// The `audience` of a request for the LAN peer whose `lan_key` is `lan_key`: a label of the
/// key that peer advertises, so a request signed for one machine fails on another. The key is
/// readable by anything on the LAN, so this binds a request to a machine and proves nothing:
/// the signature is what proves the sender.
pub fn lan_audience(lan_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("amx-remote-op-audience{FIELD_SEP}{lan_key}").as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("lan-{hex}")
}

/// Who asks: the agent, and the install it runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requester {
    pub agent_uid: String,
    pub slug: String,
    /// The requesting install's id, as its certificate names it. Empty when the
    /// sender has none to give; a receiver that holds the agent's certificate
    /// checks the two agree.
    #[serde(default)]
    pub instance_id: String,
}

/// One request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteOp {
    pub v: u32,
    /// From the table in the spec's §4.3: `app.version`, `identity.switch`, ...
    pub op: String,
    #[serde(default)]
    pub params: Value,
    pub requester: Requester,
    /// The one machine this request is valid on.
    pub audience: String,
    pub ts: i64,
    /// Unique per request: a retry carries the same one and gets the first answer.
    pub nonce: String,
    /// The requester's own words, shown as theirs and never acted on.
    #[serde(default)]
    pub reason: String,
}

/// A request and its signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRemoteOp {
    #[serde(flatten)]
    pub op: RemoteOp,
    pub sig: String,
}

/// How a request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Done,
    Queued,
    /// A countdown with the person's veto is running on the target; the final
    /// outcome follows as a second result.
    PendingVeto,
    Refused,
    Failed,
}

/// The answer to one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteOpResult {
    pub status: OpStatus,
    #[serde(default)]
    pub result: Value,
    /// The receiving machine's audit row for this request.
    #[serde(default)]
    pub audit_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused_reason: Option<String>,
}

impl RemoteOpResult {
    pub fn done(result: Value) -> Self {
        Self { status: OpStatus::Done, result, audit_id: String::new(), refused_reason: None }
    }

    pub fn refused(reason: &str) -> Self {
        Self { status: OpStatus::Refused, result: Value::Null, audit_id: String::new(), refused_reason: Some(reason.to_string()) }
    }

    pub fn failed(error: &str) -> Self {
        Self { status: OpStatus::Failed, result: serde_json::json!({ "error": error }), audit_id: String::new(), refused_reason: None }
    }
}

/// `params` as one string that doesn't depend on key order or spacing, so the
/// sender and the receiver sign and check the same bytes whatever each side's
/// JSON library does with an object.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// A field that sits between separators must not hold one; the two free-text
/// fields are length-prefixed instead.
fn plain(field: &str) -> bool {
    !field.contains(FIELD_SEP)
}

/// `reason` cut to what [`RemoteOp::check_shape`] takes: at most [`MAX_REASON_LEN`] bytes, on a character boundary.
pub fn clip_reason(reason: &str) -> String {
    let reason = reason.trim();
    if reason.len() <= MAX_REASON_LEN {
        return reason.to_string();
    }
    let mut end = MAX_REASON_LEN;
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason[..end].to_string()
}

impl RemoteOp {
    /// Why this envelope can't be signed or acted on, if it can't: a field that
    /// would blur the signed material, one that is empty or over its limit, or a
    /// version this build doesn't speak.
    pub fn check_shape(&self) -> Result<(), &'static str> {
        if self.v != REMOTE_OP_VERSION {
            return Err("unsupported_version");
        }
        let required = [&self.op, &self.requester.agent_uid, &self.requester.slug, &self.audience, &self.nonce];
        if required.iter().any(|f| f.is_empty()) {
            return Err("malformed");
        }
        let separated = [&self.op, &self.requester.agent_uid, &self.requester.slug, &self.requester.instance_id, &self.audience, &self.nonce];
        if !separated.iter().all(|f| plain(f)) {
            return Err("malformed");
        }
        if self.op.len() > MAX_OP_LEN || self.nonce.len() > MAX_NONCE_LEN || self.reason.len() > MAX_REASON_LEN {
            return Err("too_long");
        }
        let mut params = String::new();
        canonical(&self.params, &mut params);
        if params.len() > MAX_PARAMS_BYTES {
            return Err("too_long");
        }
        Ok(())
    }

    fn signed_material(&self) -> String {
        let mut params = String::new();
        canonical(&self.params, &mut params);
        let sep = FIELD_SEP;
        format!(
            "{REMOTE_OP_DOMAIN}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}:{}{sep}{}:{}",
            self.v,
            self.op,
            self.requester.slug,
            self.requester.agent_uid,
            self.requester.instance_id,
            self.audience,
            self.ts,
            self.nonce,
            self.reason.len(),
            self.reason,
            params.len(),
            params,
        )
    }

    /// Sign with the requesting agent's LAN private key (the 32-byte seed).
    /// `None` for a key of another length or an envelope that fails
    /// [`check_shape`](Self::check_shape).
    pub fn sign(self, private_key: &[u8]) -> Option<SignedRemoteOp> {
        self.check_shape().ok()?;
        let seed: [u8; 32] = private_key.try_into().ok()?;
        let signature = SigningKey::from_bytes(&seed).sign(self.signed_material().as_bytes());
        Some(SignedRemoteOp { sig: BASE64.encode(signature.to_bytes()), op: self })
    }
}

impl SignedRemoteOp {
    /// Whether `sig` is the claimed requester's signature over this envelope,
    /// under `public_key`. `false` for anything malformed, never a panic.
    pub fn verify(&self, public_key: &[u8]) -> bool {
        if self.op.check_shape().is_err() {
            return false;
        }
        let Ok(key) = <[u8; 32]>::try_from(public_key) else { return false };
        let Ok(key) = VerifyingKey::from_bytes(&key) else { return false };
        let Ok(bytes) = BASE64.decode(&self.sig) else { return false };
        let Ok(bytes) = <[u8; 64]>::try_from(bytes.as_slice()) else { return false };
        key.verify_strict(self.op.signed_material().as_bytes(), &Signature::from_bytes(&bytes)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jekt_sign::generate_lan_keypair;
    use serde_json::json;

    fn op() -> RemoteOp {
        RemoteOp {
            v: REMOTE_OP_VERSION,
            op: "app.version".into(),
            params: json!({ "b": 1, "a": [true, "x"] }),
            requester: Requester { agent_uid: "uid-lazo".into(), slug: "lazo".into(), instance_id: "inst-1".into() },
            audience: "lan-aaaa".into(),
            ts: 1_790_000_000,
            nonce: "n-1".into(),
            reason: "checking the version".into(),
        }
    }

    fn keys() -> ([u8; 32], [u8; 32]) {
        generate_lan_keypair([7u8; 32])
    }

    #[test]
    fn a_signed_request_verifies_under_the_signers_key_only() {
        let (public, private) = keys();
        let signed = op().sign(&private).unwrap();
        assert!(signed.verify(&public));
        let (other_public, _) = generate_lan_keypair([8u8; 32]);
        assert!(!signed.verify(&other_public), "another agent's key");
    }

    #[test]
    fn every_field_the_receiver_acts_on_is_covered() {
        let (public, private) = keys();
        let signed = op().sign(&private).unwrap();
        let tamper: Vec<(&str, Box<dyn Fn(&mut RemoteOp)>)> = vec![
            ("op", Box::new(|o| o.op = "app.health".into())),
            ("params", Box::new(|o| o.params = json!({ "a": [true, "y"], "b": 1 }))),
            ("slug", Box::new(|o| o.requester.slug = "manoz".into())),
            ("uid", Box::new(|o| o.requester.agent_uid = "uid-x".into())),
            ("install", Box::new(|o| o.requester.instance_id = "inst-2".into())),
            ("audience", Box::new(|o| o.audience = "lan-bbbb".into())),
            ("ts", Box::new(|o| o.ts += 1)),
            ("nonce", Box::new(|o| o.nonce = "n-2".into())),
            ("reason", Box::new(|o| o.reason = "other".into())),
        ];
        for (name, change) in tamper {
            let mut forged = signed.clone();
            change(&mut forged.op);
            assert!(!forged.verify(&public), "a changed {name} still verified");
        }
    }

    #[test]
    fn key_order_and_spacing_of_params_do_not_matter() {
        let (public, private) = keys();
        let signed = op().sign(&private).unwrap();
        let mut reordered = signed.clone();
        reordered.op.params = serde_json::from_str(r#"{ "a" : [ true , "x" ] , "b" : 1 }"#).unwrap();
        assert!(reordered.verify(&public));
    }

    #[test]
    fn a_jekt_signature_never_verifies_as_a_remote_op() {
        let (public, private) = keys();
        let o = op();
        // The jekt signer over the same fields, as a jekt's material would lay them out.
        let jekt = crate::jekt_sign::sign_lan_jekt(&private, &o.nonce, &o.requester.slug, &o.audience, o.ts, &o.op).unwrap();
        let forged = SignedRemoteOp { op: o, sig: jekt };
        assert!(!forged.verify(&public));
    }

    #[test]
    fn a_separator_in_a_field_is_refused_not_blurred() {
        let (_, private) = keys();
        let mut o = op();
        o.requester.slug = format!("la{FIELD_SEP}zo");
        assert!(o.sign(&private).is_none());
        let mut o = op();
        o.v = 2;
        assert_eq!(o.check_shape(), Err("unsupported_version"));
        let mut o = op();
        o.nonce = String::new();
        assert_eq!(o.check_shape(), Err("malformed"));
    }

    #[test]
    fn free_text_cannot_move_a_boundary() {
        // reason "ab" + params "c" must not sign the same as reason "a" + params "bc".
        let (public, private) = keys();
        let mut a = op();
        a.reason = "x".repeat(3);
        let signed = a.sign(&private).unwrap();
        let mut moved = signed.clone();
        moved.op.reason = "xx".into();
        moved.op.params = json!("x");
        assert!(!moved.verify(&public));
    }

    #[test]
    fn oversize_and_malformed_inputs_are_not_verified() {
        let (public, private) = keys();
        let signed = op().sign(&private).unwrap();
        assert!(!signed.verify(&[1, 2, 3]));
        let mut bad = signed.clone();
        bad.sig = "not base64!!".into();
        assert!(!bad.verify(&public));
        let mut big = op();
        big.params = json!({ "k": "z".repeat(MAX_PARAMS_BYTES) });
        assert_eq!(big.check_shape(), Err("too_long"));
        assert!(op().sign(&private[..31]).is_none());
    }

    #[test]
    fn a_long_reason_in_any_script_is_clipped_to_a_signable_length() {
        let (_, private) = keys();
        for text in ["x".repeat(2000), "é".repeat(2000), "日本語".repeat(500), format!("  {}  ", "ü".repeat(MAX_REASON_LEN))] {
            let clipped = clip_reason(&text);
            assert!(clipped.len() <= MAX_REASON_LEN && !clipped.is_empty());
            let mut o = op();
            o.reason = clipped;
            assert!(o.sign(&private).is_some());
        }
        assert_eq!(clip_reason("  short  "), "short");
    }

    #[test]
    fn a_lan_audience_names_one_key() {
        assert_eq!(lan_audience("key-a"), lan_audience("key-a"));
        assert_ne!(lan_audience("key-a"), lan_audience("key-b"));
        assert!(lan_audience("key-a").starts_with("lan-") && lan_audience("key-a").len() == 20);
    }

    #[test]
    fn the_wire_shape_round_trips() {
        let (public, private) = keys();
        let signed = op().sign(&private).unwrap();
        let wire = serde_json::to_string(&signed).unwrap();
        let back: SignedRemoteOp = serde_json::from_str(&wire).unwrap();
        assert!(back.verify(&public));
        assert_eq!(back, signed);
    }
}
