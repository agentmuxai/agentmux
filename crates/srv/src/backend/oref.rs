// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! ORef: typed object reference in "otype:oid" string format.
//! Custom serde: serializes as a JSON string, not an object.


use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use uuid::Uuid;

use super::obj::VALID_OTYPES;

/// Object reference combining a type name and UUID.
/// Wire format: `"block:550e8400-e29b-41d4-a716-446655440000"`
///
/// `#[ts(type = "string")]` is load-bearing, not decoration. This struct has a
/// hand-written `Serialize`/`Deserialize` pair (below) that emits and parses a
/// single `"otype:oid"` STRING — the Rust shape and the wire shape deliberately
/// disagree. Without this attribute ts-rs would faithfully generate
/// `{ otype: string, oid: string }` from the struct, which is not what any
/// client ever receives: it would be a confidently wrong binding, worse than
/// none, and the compiler could not catch it because TypeScript would simply
/// believe it. ts-rs documents this attribute for exactly this case — "when you
/// have a custom serializer and deserializer".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ORef {
    pub otype: String,
    pub oid: String,
}

impl ORef {
    #[allow(dead_code)]
    pub fn new(otype: impl Into<String>, oid: impl Into<String>) -> Self {
        Self {
            otype: otype.into(),
            oid: oid.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.otype.is_empty() || self.oid.is_empty()
    }

    /// Parse an ORef from the "otype:oid" string format.
    pub fn parse(s: &str) -> Result<Self, ORefParseError> {
        if s.is_empty() {
            return Ok(Self {
                otype: String::new(),
                oid: String::new(),
            });
        }

        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(ORefParseError::InvalidFormat(s.to_string()));
        }

        let otype = parts[0];
        let oid = parts[1];

        // Validate otype: must be lowercase ascii letters only
        if otype.is_empty() || !otype.chars().all(|c| c.is_ascii_lowercase()) {
            return Err(ORefParseError::InvalidOType(otype.to_string()));
        }

        if !VALID_OTYPES.contains(&otype) {
            return Err(ORefParseError::UnknownOType(otype.to_string()));
        }

        // Validate OID is a valid UUID
        Uuid::parse_str(oid).map_err(|_| ORefParseError::InvalidOID(oid.to_string()))?;

        Ok(Self {
            otype: otype.to_string(),
            oid: oid.to_string(),
        })
    }
}

impl fmt::Display for ORef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return write!(f, "");
        }
        write!(f, "{}:{}", self.otype, self.oid)
    }
}

/// Errors from parsing an ORef string.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ORefParseError {
    #[error("invalid object reference: {0:?}")]
    InvalidFormat(String),
    #[error("invalid object type: {0:?}")]
    InvalidOType(String),
    #[error("unknown object type: {0:?}")]
    UnknownOType(String),
    #[error("invalid object id: {0:?}")]
    InvalidOID(String),
}

// ts-rs binding, kept beside the serde impls it has to agree with.
//
// Written by hand rather than derived, because a derive cannot express this.
// ts-rs emits a *reference* to a type's name() at every use site, so for a
// field holding an ORef to come out as `string`, name() itself has to be
// "string". Neither `#[ts(as = "String")]` nor `#[ts(type = "string")]` on the
// container does that: both leave name() as "ORef" and a consumer still
// generates `{ oref: ORef }`. That is not a guess — the test below was written
// first and failed against both attributes before this impl existed.
//
// This mirrors what ts-rs does internally for foreign string-like types
// (`impl_primitives! { uuid::Uuid => "string" }`) and for the same reason: the
// Rust shape and the wire shape differ, and the wire is what a client sees.
//
// decl()/inline_flattened() panic exactly as they do for every ts-rs primitive:
// ORef is never declared as its own TypeScript type, and cannot be flattened.
impl ts_rs::TS for ORef {
    type WithoutGenerics = Self;
    fn name() -> String {
        "string".to_owned()
    }
    fn inline() -> String {
        <Self as ts_rs::TS>::name()
    }
    fn inline_flattened() -> String {
        panic!("ORef cannot be flattened")
    }
    fn decl() -> String {
        panic!("ORef cannot be declared")
    }
    fn decl_concrete() -> String {
        panic!("ORef cannot be declared")
    }
}

// Custom serde: serialize as "otype:oid" string, not as an object.

impl Serialize for ORef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ORef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        ORef::parse(&s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The binding ts-rs emits for ORef must be `string`, because that is what
    /// the wire carries. Both halves are asserted together on purpose: if
    /// either the TS name or the serialized form drifts, the other pins it.
    ///
    /// Why this needs guarding at all: ts-rs derives from the STRUCT shape, and
    /// the `serde-compat` feature (see agentmux-srv/Cargo.toml) only reads
    /// serde ATTRIBUTES — rename_all, skip_serializing_if and friends. It
    /// cannot see a hand-written `impl Serialize`, which is exactly what ORef
    /// has. Without `#[ts(type = "string")]` the generated type would be
    /// `{ otype: string, oid: string }`: a shape no client ever receives, that
    /// TypeScript would nonetheless believe, and that no compiler on either
    /// side could catch.
    #[test]
    fn ts_binding_is_a_string_because_the_wire_is_a_string() {
        // Assert what a CONSUMER generates, not ORef::name(). `#[ts(as)]`
        // changes what is inlined at use sites; the type keeps its own name,
        // so asserting name() would test the wrong thing (it returns "ORef").
        // A field is what any real command actually has.
        #[derive(ts_rs::TS)]
        #[allow(dead_code)]
        struct Consumer {
            oref: ORef,
        }
        let decl = <Consumer as ts_rs::TS>::decl();
        assert!(
            decl.contains("oref: string"),
            "a field holding an ORef must generate `string`; got: {decl}"
        );
        assert!(
            !decl.contains("otype"),
            "the struct shape must not leak into the binding; got: {decl}"
        );

        let oref = ORef::new("block", "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(
            serde_json::to_string(&oref).unwrap(),
            "\"block:550e8400-e29b-41d4-a716-446655440000\"",
            "the wire form the binding above claims to describe"
        );
    }

    #[test]
    fn test_oref_roundtrip() {
        let oref = ORef::new("block", "550e8400-e29b-41d4-a716-446655440000");
        let json = serde_json::to_string(&oref).unwrap();
        assert_eq!(json, r#""block:550e8400-e29b-41d4-a716-446655440000""#);

        let parsed: ORef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, oref);
    }

    #[test]
    fn test_oref_empty() {
        let oref = ORef::parse("").unwrap();
        assert!(oref.is_empty());
        let json = serde_json::to_string(&oref).unwrap();
        assert_eq!(json, r#""""#);
    }

    #[test]
    fn test_oref_all_valid_types() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        for otype in VALID_OTYPES {
            let s = format!("{otype}:{uuid}");
            let oref = ORef::parse(&s).unwrap();
            assert_eq!(oref.otype, *otype);
            assert_eq!(oref.oid, uuid);
        }
    }

    #[test]
    fn test_oref_invalid_otype() {
        let result = ORef::parse("BLOCK:550e8400-e29b-41d4-a716-446655440000");
        assert!(result.is_err());
        assert!(matches!(result, Err(ORefParseError::InvalidOType(_))));
    }

    #[test]
    fn test_oref_unknown_otype() {
        let result = ORef::parse("foobar:550e8400-e29b-41d4-a716-446655440000");
        assert!(result.is_err());
        assert!(matches!(result, Err(ORefParseError::UnknownOType(_))));
    }

    #[test]
    fn test_oref_invalid_uuid() {
        let result = ORef::parse("block:not-a-uuid");
        assert!(result.is_err());
        assert!(matches!(result, Err(ORefParseError::InvalidOID(_))));
    }

    #[test]
    fn test_oref_no_colon() {
        let result = ORef::parse("blockuuid");
        assert!(result.is_err());
        assert!(matches!(result, Err(ORefParseError::InvalidFormat(_))));
    }

    #[test]
    fn test_oref_display() {
        let oref = ORef::new("tab", "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(oref.to_string(), "tab:550e8400-e29b-41d4-a716-446655440000");
    }
}
