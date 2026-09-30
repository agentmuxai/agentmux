// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `transcript_request`/`transcript_response` wire payload — `muxspect`
//! Phase B/C's LAN/WAN conversation-visibility protocol
//! (docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md,
//! jekt tier rules confirmed in
//! docs/specs/SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md).
//!
//! There is no structured `msg_type`/content-type field anywhere in the
//! jekt wire format — every jekt is one opaque `message: String`, and that
//! field is part of the SIGNED material (`jekt_sign::signed_material`).
//! Adding a top-level field would mean touching every signer/verifier and
//! every existing marker-formatting call site for a feature that's
//! otherwise fully layerable on top. Instead, this payload is JSON-encoded
//! and carried AS the jekt `message` — the receiving side sniffs for it
//! (`parse_transcript_request`/`parse_transcript_response`, both `None` for
//! any ordinary free-text message, so this can never misfire on normal
//! jekt traffic) before falling through to ordinary message handling.
//!
//! Shared between `agentmux-mcp` (constructs a request, sends it as an
//! ordinary signed jekt via the existing `/agentmux/reactive/inject` path —
//! no new transport) and `agentmux-srv` (detects an incoming request/
//! response payload in `Handler::inject_message_inner` and routes it to the
//! conversation-visibility auto-resolution logic instead of ordinary
//! delivery).

use serde::{Deserialize, Serialize};

pub const TRANSCRIPT_REQUEST_TYPE: &str = "transcript_request";
pub const TRANSCRIPT_RESPONSE_TYPE: &str = "transcript_response";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptRequest {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: String,
    pub max_lines: usize,
}

impl TranscriptRequest {
    pub fn new(request_id: impl Into<String>, max_lines: usize) -> Self {
        Self {
            msg_type: TRANSCRIPT_REQUEST_TYPE.to_string(),
            request_id: request_id.into(),
            max_lines,
        }
    }

    pub fn to_message(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum TranscriptResponseStatus {
    Ok { lines: Vec<String> },
    Denied,
    Error { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptResponse {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: String,
    #[serde(flatten)]
    pub status: TranscriptResponseStatus,
}

impl TranscriptResponse {
    pub fn ok(request_id: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            msg_type: TRANSCRIPT_RESPONSE_TYPE.to_string(),
            request_id: request_id.into(),
            status: TranscriptResponseStatus::Ok { lines },
        }
    }

    pub fn denied(request_id: impl Into<String>) -> Self {
        Self {
            msg_type: TRANSCRIPT_RESPONSE_TYPE.to_string(),
            request_id: request_id.into(),
            status: TranscriptResponseStatus::Denied,
        }
    }

    pub fn error(request_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            msg_type: TRANSCRIPT_RESPONSE_TYPE.to_string(),
            request_id: request_id.into(),
            status: TranscriptResponseStatus::Error { error: error.into() },
        }
    }

    pub fn to_message(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Sniff a jekt's raw `message` for a `transcript_request` payload. `None`
/// for anything else — malformed JSON, valid JSON that isn't this shape, or
/// ordinary free text — so this can never misclassify normal jekt traffic.
/// Deliberately checks the `type` discriminant BEFORE fully deserializing
/// (a message that happens to parse as arbitrary JSON but isn't a
/// transcript_request must return `None`, not an error).
pub fn parse_transcript_request(message: &str) -> Option<TranscriptRequest> {
    let value: serde_json::Value = serde_json::from_str(message).ok()?;
    if value.get("type")?.as_str()? != TRANSCRIPT_REQUEST_TYPE {
        return None;
    }
    serde_json::from_value(value).ok()
}

/// Sniff a jekt's raw `message` for a `transcript_response` payload. Same
/// "never misfire" contract as [`parse_transcript_request`].
pub fn parse_transcript_response(message: &str) -> Option<TranscriptResponse> {
    let value: serde_json::Value = serde_json::from_str(message).ok()?;
    if value.get("type")?.as_str()? != TRANSCRIPT_RESPONSE_TYPE {
        return None;
    }
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_message_encoding() {
        let req = TranscriptRequest::new("req-1", 100);
        let message = req.to_message();
        let parsed = parse_transcript_request(&message).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn ordinary_free_text_never_parses_as_a_request() {
        assert!(parse_transcript_request("just chatting with you").is_none());
    }

    #[test]
    fn arbitrary_json_that_isnt_this_shape_never_parses_as_a_request() {
        assert!(parse_transcript_request(r#"{"type":"something_else","foo":"bar"}"#).is_none());
        assert!(parse_transcript_request(r#"{"foo":"bar"}"#).is_none());
        assert!(parse_transcript_request("[1,2,3]").is_none());
    }

    #[test]
    fn malformed_json_never_parses_as_a_request_and_never_panics() {
        assert!(parse_transcript_request("{not json").is_none());
    }

    #[test]
    fn ok_response_round_trips() {
        let resp = TranscriptResponse::ok("req-1", vec!["line 1".to_string(), "line 2".to_string()]);
        let message = resp.to_message();
        let parsed = parse_transcript_response(&message).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn denied_response_round_trips() {
        let resp = TranscriptResponse::denied("req-1");
        let message = resp.to_message();
        let parsed = parse_transcript_response(&message).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn error_response_round_trips() {
        let resp = TranscriptResponse::error("req-1", "something broke");
        let message = resp.to_message();
        let parsed = parse_transcript_response(&message).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn a_request_never_parses_as_a_response_and_vice_versa() {
        let req = TranscriptRequest::new("req-1", 100);
        assert!(parse_transcript_response(&req.to_message()).is_none());
        let resp = TranscriptResponse::denied("req-1");
        assert!(parse_transcript_request(&resp.to_message()).is_none());
    }
}
