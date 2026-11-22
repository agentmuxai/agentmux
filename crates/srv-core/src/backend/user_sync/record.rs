// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A record as it leaves a machine (SPEC_USER_DATA_SYNC_LAYER §3.2): a fixed, signed header in the clear and a
//! sealed body. The header is signed by the origin machine's key over every field and a digest of the body, so
//! no other member and no relay can write a record "from" a machine. The clear header is a fixed list
//! ([`CLEAR_FIELDS`]); a collection's name and everything descriptive lives in the sealed body.

use agentmux_common::jekt_sign::{ed25519_sign_b64, ed25519_verify_b64};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::hlc::Version;

/// Every field a record carries in the clear, and no other.
pub const CLEAR_FIELDS: [&str; 10] = ["collection", "record", "version", "base", "origin", "seq", "epoch", "tombstone", "size", "sig"];

/// Separates the fields of the signed material; a field holding it is refused.
const SEP: char = '\u{1f}';
/// The most a record's sealed body may hold (a collection's own setting may lower it).
pub const MAX_RECORD_BYTES: u64 = 25 * 1024 * 1024;
const DOMAIN: &str = "agentmux-sync-record-v1";

/// The header of a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub collection: String,
    /// A stable id made where the record was born, never reused.
    pub record: String,
    pub version: Version,
    /// The version this change replaced on the machine that made it (`None` for a record's first version). It
    /// lets an adapter tell an edit made on top of what a receiver holds from one made apart, from headers alone.
    pub base: Option<Version>,
    /// The machine that wrote this entry of its log (its pinned instance identifier).
    pub origin: String,
    /// The origin's own sequence number for this collection.
    pub seq: u64,
    pub epoch: u32,
    pub tombstone: bool,
    /// The length of the sealed body.
    pub size: u64,
}

/// A record: its header, the origin's signature over it, and the sealed body (empty for a tombstone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub header: Header,
    pub sig: String,
    pub body: Vec<u8>,
}

/// Why a record was not accepted as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadRecord {
    /// A field holds the separator, the size does not match the body, or a tombstone has a body.
    Malformed,
    /// The version names a different machine than the origin.
    VersionNotFromOrigin,
    /// The signature does not verify against the origin's key.
    Signature,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Header {
    /// The bytes the origin signs: every field, and a digest of the sealed body.
    fn material(&self, body: &[u8]) -> String {
        let digest = hex(&Sha256::digest(body));
        let base = match &self.base {
            None => ["0".to_string(), String::new(), String::new(), String::new()],
            Some(b) => ["1".to_string(), b.wall_ms.to_string(), b.counter.to_string(), b.machine.clone()],
        };
        [
            DOMAIN.to_string(),
            self.collection.clone(),
            self.record.clone(),
            self.version.wall_ms.to_string(),
            self.version.counter.to_string(),
            self.version.machine.clone(),
        ]
        .into_iter()
        .chain(base)
        .chain([
            self.origin.clone(),
            self.seq.to_string(),
            self.epoch.to_string(),
            (self.tombstone as u8).to_string(),
            self.size.to_string(),
            digest,
        ])
        .collect::<Vec<String>>()
        .join(&SEP.to_string())
    }

    fn well_formed(&self, body: &[u8]) -> bool {
        let clean = |s: &str| !s.is_empty() && !s.contains(SEP);
        clean(&self.collection)
            && clean(&self.record)
            && clean(&self.origin)
            && clean(&self.version.machine)
            && self.base.as_ref().map_or(true, |b| clean(&b.machine))
            && self.size == body.len() as u64
            && self.size <= MAX_RECORD_BYTES
            && (!self.tombstone || body.is_empty())
    }
}

impl Record {
    /// A record signed by `origin_private` (the 32-byte seed of the origin machine's signing key).
    pub fn sign(origin_private: &[u8], header: Header, body: Vec<u8>) -> Option<Record> {
        if !header.well_formed(&body) || header.version.machine != header.origin {
            return None;
        }
        let sig = ed25519_sign_b64(origin_private, &header.material(&body))?;
        Some(Record { header, sig, body })
    }

    /// Checks the record against the origin's pinned public key.
    pub fn verify(&self, origin_public: &[u8; 32]) -> Result<(), BadRecord> {
        if !self.header.well_formed(&self.body) {
            return Err(BadRecord::Malformed);
        }
        if self.header.version.machine != self.header.origin {
            return Err(BadRecord::VersionNotFromOrigin);
        }
        if ed25519_verify_b64(origin_public, &self.header.material(&self.body), &self.sig) {
            Ok(())
        } else {
            Err(BadRecord::Signature)
        }
    }

    /// What this record shows in the clear, as it is sent and stored: exactly [`CLEAR_FIELDS`].
    pub fn clear_fields(&self) -> Map<String, Value> {
        let h = &self.header;
        let Value::Object(map) = json!({
            "collection": h.collection,
            "record": h.record,
            "version": { "wall_ms": h.version.wall_ms, "counter": h.version.counter, "machine": h.version.machine },
            "base": h.base.as_ref().map(|b| json!({ "wall_ms": b.wall_ms, "counter": b.counter, "machine": b.machine })),
            "origin": h.origin,
            "seq": h.seq,
            "epoch": h.epoch,
            "tombstone": h.tombstone,
            "size": h.size,
            "sig": self.sig,
        }) else {
            return Map::new();
        };
        map
    }
}
