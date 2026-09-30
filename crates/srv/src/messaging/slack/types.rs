// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Slack Socket Mode + Web API wire types.
//!
//! Block Kit is represented as raw `serde_json::Value` rather than a typed
//! enum tree (spec §4.4, §7, §11.2) — Slack's block/element/object surface
//! is large and evolving; a raw escape hatch is the deliberate v1 choice.

use serde::{Deserialize, Serialize};

// ── Socket Mode inbound envelope ────────────────────────────────────────────

/// Tagged on the wire `"type"` field. `SlashCommands` is parsed for forward
/// compatibility (an envelope of this kind can arrive if the app has any
/// slash command registered) but its payload is not modeled/acted on — slash
/// command handling is explicitly out of scope for this PR (spec §2.5, §10
/// Phase 4).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SocketEnvelope {
    /// Content unused today — only the variant tag matters (first-frame
    /// check in `socket.rs::open_and_await_hello`, reconnect-ack log line in
    /// `handle_frame`).
    #[allow(dead_code)]
    Hello(HelloFrame),
    EventsApi(EventsApiEnvelope),
    Disconnect(DisconnectFrame),
    SlashCommands(SlashCommandsEnvelope),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct HelloFrame {
    #[serde(default)]
    #[allow(dead_code)]
    pub num_connections: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct EventsApiEnvelope {
    pub envelope_id: String,
    #[serde(default)]
    pub payload: Option<EventsApiPayload>,
}

#[derive(Debug, Deserialize)]
pub struct EventsApiPayload {
    #[serde(default)]
    pub event: Option<SlackEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackEvent {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub ts: Option<String>,
    /// Present on bot-authored events, including this bridge's own posts —
    /// used for self-message loop prevention (spec §9.4).
    #[serde(default)]
    pub bot_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DisconnectFrame {
    #[serde(default)]
    pub reason: Option<String>,
}

/// Present for wire-compatibility only — not routed/acted on (see module doc).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SlashCommandsEnvelope {
    pub envelope_id: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

// ── Socket Mode outbound (ACK) ──────────────────────────────────────────────

/// Sent back on the same socket a `events_api`/`slash_commands` envelope
/// arrived on, within 3 seconds of receipt (spec §2.2 — hard correctness
/// requirement).
#[derive(Debug, Serialize)]
pub struct AckFrame {
    pub envelope_id: String,
}

// ── Web API: apps.connections.open ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OpenConnectionResponse {
    pub ok: bool,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

// ── Web API: chat.postMessage ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PostMessageBody {
    pub channel: String,
    /// Always sent, even alongside `blocks` — Slack uses it as the
    /// notification/accessibility fallback (spec §2.4).
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
pub struct PostMessageResponse {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hello_frame() {
        let json = r#"{"type": "hello", "num_connections": 1, "connection_info": {"app_id": "A1"}}"#;
        let env: SocketEnvelope = serde_json::from_str(json).unwrap();
        assert!(matches!(env, SocketEnvelope::Hello(_)));
    }

    #[test]
    fn parses_disconnect_warning_frame() {
        let json = r#"{"type": "disconnect", "reason": "warning"}"#;
        let env: SocketEnvelope = serde_json::from_str(json).unwrap();
        match env {
            SocketEnvelope::Disconnect(d) => assert_eq!(d.reason.as_deref(), Some("warning")),
            other => panic!("expected Disconnect, got {other:?}"),
        }
    }

    #[test]
    fn parses_events_api_envelope_with_message_event() {
        let json = r#"{
            "envelope_id": "95869",
            "type": "events_api",
            "accepts_response_payload": false,
            "payload": {
                "event": {"type": "message", "channel": "C1", "user": "U1", "text": "hi", "ts": "123.4"}
            }
        }"#;
        let env: SocketEnvelope = serde_json::from_str(json).unwrap();
        match env {
            SocketEnvelope::EventsApi(e) => {
                assert_eq!(e.envelope_id, "95869");
                let event = e.payload.unwrap().event.unwrap();
                assert_eq!(event.type_, "message");
                assert_eq!(event.channel.as_deref(), Some("C1"));
                assert_eq!(event.text.as_deref(), Some("hi"));
                assert!(event.bot_id.is_none());
            }
            other => panic!("expected EventsApi, got {other:?}"),
        }
    }

    #[test]
    fn parses_events_api_envelope_with_bot_id() {
        let json = r#"{
            "envelope_id": "1",
            "type": "events_api",
            "payload": {"event": {"type": "message", "channel": "C1", "bot_id": "B1"}}
        }"#;
        let env: SocketEnvelope = serde_json::from_str(json).unwrap();
        match env {
            SocketEnvelope::EventsApi(e) => {
                let event = e.payload.unwrap().event.unwrap();
                assert_eq!(event.bot_id.as_deref(), Some("B1"));
            }
            other => panic!("expected EventsApi, got {other:?}"),
        }
    }

    #[test]
    fn unknown_envelope_type_does_not_error() {
        let json = r#"{"type": "some_future_type", "foo": "bar"}"#;
        let env: SocketEnvelope = serde_json::from_str(json).unwrap();
        assert!(matches!(env, SocketEnvelope::Unknown));
    }

    #[test]
    fn ack_frame_serializes_envelope_id_only() {
        let ack = AckFrame { envelope_id: "95869".to_string() };
        let json = serde_json::to_string(&ack).unwrap();
        assert_eq!(json, r#"{"envelope_id":"95869"}"#);
    }

    #[test]
    fn post_message_body_omits_blocks_when_none() {
        let body = PostMessageBody {
            channel: "C1".to_string(),
            text: "hi".to_string(),
            blocks: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(!json.contains("blocks"));
    }
}
