// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WhatsApp Cloud API wire types — webhook payload (inbound) and Graph API
//! send body (outbound). Hand-rolled serde structs over `reqwest` +
//! `serde_json`, matching Discord's `types.rs` precedent (no SDK crate).

use serde::{Deserialize, Serialize};

// ── Webhook payload (inbound) ───────────────────────────────────────────────
//
// Meta's nested envelope: `entry[].changes[].value.messages[]`. Deserialized
// permissively (unknown fields ignored by default with serde) — `statuses[]`
// (delivery receipts) is intentionally not modeled at all, per spec §5.4;
// any change payload that carries only `statuses` and no `messages` simply
// yields zero `ExtractedMessage`s.

#[derive(Debug, Default, Deserialize)]
pub struct WebhookPayload {
    #[serde(default)]
    pub entry: Vec<WebhookEntry>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WebhookEntry {
    #[serde(default)]
    pub changes: Vec<WebhookChange>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WebhookChange {
    #[serde(default)]
    pub value: WebhookValue,
}

#[derive(Debug, Default, Deserialize)]
pub struct WebhookValue {
    #[serde(default)]
    pub messages: Vec<WebhookMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookMessage {
    /// Sender's WhatsApp phone number (no `+`, e.g. "14155552671").
    pub from: String,
    pub id: String,
    #[serde(default)]
    pub text: Option<WebhookText>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookText {
    pub body: String,
}

/// A flattened, normalized inbound WhatsApp message extracted from the
/// nested webhook envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedMessage {
    pub from: String,
    pub id: String,
    pub text: String,
}

impl WebhookPayload {
    /// Flattens `entry[].changes[].value.messages[]`. Only text messages are
    /// surfaced in v1 (spec §5.2/§5.4 scope) — messages of other types
    /// (image, reaction, location, …) or `statuses` entries are skipped, not
    /// errored on, since a webhook batch legitimately mixes message types
    /// and delivery-receipt updates.
    pub fn extract_messages(&self) -> Vec<ExtractedMessage> {
        self.entry
            .iter()
            .flat_map(|e| e.changes.iter())
            .flat_map(|c| c.value.messages.iter())
            .filter_map(|m| {
                let text = m.text.as_ref()?.body.clone();
                Some(ExtractedMessage {
                    from: m.from.clone(),
                    id: m.id.clone(),
                    text,
                })
            })
            .collect()
    }
}

// ── Graph API send types (outbound) ────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SendBody {
    pub messaging_product: &'static str,
    pub to: String,
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<SendText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<SendTemplate>,
}

#[derive(Debug, Serialize)]
pub struct SendText {
    pub body: String,
}

#[derive(Debug, Serialize)]
pub struct SendTemplate {
    pub name: String,
    pub language: SendTemplateLanguage,
}

#[derive(Debug, Serialize)]
pub struct SendTemplateLanguage {
    pub code: String,
}

impl SendBody {
    pub fn text(to: &str, body: &str) -> Self {
        SendBody {
            messaging_product: "whatsapp",
            to: to.to_string(),
            msg_type: "text",
            text: Some(SendText {
                body: body.to_string(),
            }),
            template: None,
        }
    }

    pub fn template(to: &str, name: &str, lang: &str) -> Self {
        SendBody {
            messaging_product: "whatsapp",
            to: to.to_string(),
            msg_type: "template",
            text: None,
            template: Some(SendTemplate {
                name: name.to_string(),
                language: SendTemplateLanguage {
                    code: lang.to_string(),
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_messages_flattens_nested_envelope_and_skips_non_text() {
        let json = r#"{
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [
                            { "from": "15551234567", "id": "wamid.1", "type": "text", "text": { "body": "hi" } },
                            { "from": "15551234567", "id": "wamid.2", "type": "reaction" }
                        ]
                    }
                }]
            }]
        }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let msgs = payload.extract_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "15551234567");
        assert_eq!(msgs[0].id, "wamid.1");
        assert_eq!(msgs[0].text, "hi");
    }

    #[test]
    fn extract_messages_ignores_statuses_only_payload() {
        // `statuses` isn't modeled at all — a change with only delivery
        // receipts yields an empty `messages` vec (default), so extraction
        // returns zero results without error.
        let json = r#"{
            "entry": [{
                "changes": [{
                    "value": {
                        "statuses": [{ "id": "wamid.1", "status": "delivered" }]
                    }
                }]
            }]
        }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert!(payload.extract_messages().is_empty());
    }

    #[test]
    fn extract_messages_empty_entry_list() {
        let payload: WebhookPayload = serde_json::from_str(r#"{"entry": []}"#).unwrap();
        assert!(payload.extract_messages().is_empty());
    }

    #[test]
    fn send_body_text_serializes_expected_shape() {
        let body = SendBody::text("15551234567", "hello");
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["messaging_product"], "whatsapp");
        assert_eq!(v["to"], "15551234567");
        assert_eq!(v["type"], "text");
        assert_eq!(v["text"]["body"], "hello");
        assert!(v.get("template").is_none());
    }

    #[test]
    fn send_body_template_serializes_expected_shape() {
        let body = SendBody::template("15551234567", "hello_world", "en_US");
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["type"], "template");
        assert_eq!(v["template"]["name"], "hello_world");
        assert_eq!(v["template"]["language"]["code"], "en_US");
        assert!(v.get("text").is_none());
    }
}
