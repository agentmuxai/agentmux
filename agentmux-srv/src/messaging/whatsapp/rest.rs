// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WhatsApp Cloud API REST client — outbound message send only.
//!
//! Mirrors Discord's `rest.rs` shape (a single `send_message` function, no
//! client struct), with one addition Discord doesn't need: the 24-hour
//! customer service window (spec §3.3/§3.4). Outside the window, Meta
//! rejects a free-form text send (error code 131047); rather than round-trip
//! to the API to discover that, `send_message` checks the local window state
//! first and fails fast (or falls back to a configured template) before
//! making the HTTP call at all.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::messaging::OutboundMsg;

use super::types::SendBody;
use super::WhatsAppConfig;

const GRAPH_API_VERSION: &str = "v25.0";
const TWENTY_FOUR_HOURS_MS: u64 = 24 * 60 * 60 * 1000;

/// POST /{phone_number_id}/messages
pub async fn send_message(
    http: &reqwest::Client,
    config: &WhatsAppConfig,
    window_state: &Mutex<HashMap<String, u64>>,
    msg: &OutboundMsg,
) -> Result<(), String> {
    // WhatsApp has no channel/room concept — `OutboundMsg.channel_id` is
    // repurposed as "recipient phone number" (spec §5.3). There is no
    // bridge-level default to fall back to the way Discord/Slack have a
    // default channel, so an empty recipient is a caller error.
    //
    // Normalized (digits only) before anything else: `handle_whatsapp_send`
    // documents `to` as "E.164 preferred" (e.g. "+14155552671"), but Meta's
    // inbound webhook `from` field has no `+` (e.g. "14155552671"). Without
    // normalizing both to the same form, the window-state lookup below
    // key-misses every caller following the documented `+`-prefixed format,
    // always falling through to "24h window expired" even when Meta's real
    // window is open. Digits-only also matches the format the Graph API's
    // own `to` field expects, so the normalized value is used for the send
    // body too, not just the lookup.
    let to = normalize_phone(msg.channel_id.trim());
    if to.is_empty() {
        return Err(
            "whatsapp: OutboundMsg.channel_id (recipient phone number) is required".to_string(),
        );
    }

    let last_inbound = window_state.lock().unwrap().get(&to).copied();
    let within_window = last_inbound
        .map(|last| window_ok(last, now_ms()))
        .unwrap_or(false);

    let text = flatten_text(msg);

    let body = if within_window {
        SendBody::text(&to, &text)
    } else {
        match config.fallback_template.as_deref().filter(|t| !t.is_empty()) {
            Some(template) => SendBody::template(&to, template, &config.fallback_template_lang),
            None => {
                return Err(
                    "whatsapp: 24h window expired and no fallback template configured"
                        .to_string(),
                );
            }
        }
    };

    let url = format!(
        "https://graph.facebook.com/{GRAPH_API_VERSION}/{}/messages",
        config.phone_number_id
    );

    let resp = http
        .post(&url)
        .bearer_auth(&config.access_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("whatsapp rest: http error: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Graph API error bodies are `{ "error": { "message", "type", "code",
        // "error_subcode", "fbtrace_id" } }` — never contain the access
        // token or app secret, so passing the text through is safe (unlike
        // request header content on the inbound webhook path, which is
        // deliberately never logged — see webhook.rs).
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("whatsapp rest: {status}: {text}"));
    }

    Ok(())
}

/// WhatsApp's Cloud API has no rich embed support (spec §5.3) — if
/// `OutboundMsg.embed` is set, flatten it to plain text rather than dropping
/// it silently.
fn flatten_text(msg: &OutboundMsg) -> String {
    let Some(embed) = &msg.embed else {
        return msg.text.clone();
    };

    let mut parts = Vec::new();
    if let Some(title) = &embed.title {
        parts.push(title.clone());
    }
    parts.push(embed.description.clone());
    for f in &embed.fields {
        parts.push(format!("{}: {}", f.name, f.value));
    }
    if let Some(footer) = &embed.footer {
        parts.push(footer.clone());
    }
    let flattened = parts.join("\n\n");

    if msg.text.is_empty() {
        flattened
    } else {
        format!("{}\n\n{}", msg.text, flattened)
    }
}

/// True if `now_ms` is within 24h of `last_inbound_ms`. Pulled out as a pure
/// function so the boundary condition (spec §10's "24h window edge case") is
/// unit-testable without a clock or network call.
fn window_ok(last_inbound_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(last_inbound_ms) <= TWENTY_FOUR_HOURS_MS
}

/// Strips everything but ASCII digits, so the same phone number in different
/// formats — Meta's inbound `from` (no `+`) vs. a caller-supplied `to` in
/// E.164 (`+`-prefixed) or with spaces/dashes — normalizes to one 24h-window
/// state key. `WhatsAppBridge::record_inbound` (mod.rs) applies this same
/// normalization when storing, so the two call sites always agree.
pub(crate) fn normalize_phone(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn now_ms() -> u64 {
    agentmux_common::time::now_ms_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{EmbedField, MsgEmbed};

    #[test]
    fn window_ok_within_bound() {
        let last = 1_000_000u64;
        assert!(window_ok(last, last + TWENTY_FOUR_HOURS_MS));
    }

    #[test]
    fn window_ok_exactly_at_boundary_is_inclusive() {
        let last = 1_000_000u64;
        assert!(window_ok(last, last + TWENTY_FOUR_HOURS_MS));
    }

    #[test]
    fn window_ok_one_ms_past_boundary_expires() {
        let last = 1_000_000u64;
        assert!(!window_ok(last, last + TWENTY_FOUR_HOURS_MS + 1));
    }

    #[test]
    fn window_ok_now_before_last_does_not_panic() {
        // Clock skew / stale entry — saturating_sub prevents underflow panic.
        assert!(window_ok(2_000_000, 1_000_000));
    }

    #[test]
    fn normalize_phone_strips_plus_and_formatting() {
        assert_eq!(normalize_phone("+14155552671"), "14155552671");
        assert_eq!(normalize_phone("1-415-555-2671"), "14155552671");
        assert_eq!(normalize_phone("(415) 555-2671"), "4155552671");
    }

    #[test]
    fn normalize_phone_is_noop_on_already_digits_only() {
        // Meta's inbound `from` format — no `+`, no separators.
        assert_eq!(normalize_phone("14155552671"), "14155552671");
    }

    #[test]
    fn normalize_phone_produces_matching_keys_for_meta_inbound_and_e164_outbound() {
        // Regression for the window-state key mismatch a review caught:
        // Meta's inbound `from` and a caller's documented E.164 `to` must
        // normalize to the identical map key.
        let meta_inbound_from = "14155552671";
        let caller_supplied_to = "+14155552671";
        assert_eq!(normalize_phone(meta_inbound_from), normalize_phone(caller_supplied_to));
    }

    #[test]
    fn flatten_text_plain_text_only() {
        let msg = OutboundMsg {
            text: "hello".to_string(),
            channel_id: "15551234567".to_string(),
            reply_to: None,
            embed: None,
            edit_message_id: None,
            blocks: None,
        };
        assert_eq!(flatten_text(&msg), "hello");
    }

    #[test]
    fn flatten_text_embed_flattens_title_description_fields_footer() {
        let msg = OutboundMsg {
            text: String::new(),
            channel_id: "15551234567".to_string(),
            reply_to: None,
            embed: Some(MsgEmbed {
                title: Some("Title".to_string()),
                description: "Desc".to_string(),
                color: None,
                fields: vec![EmbedField {
                    name: "Status".to_string(),
                    value: "OK".to_string(),
                    inline: false,
                }],
                footer: Some("Footer".to_string()),
            }),
            edit_message_id: None,
            blocks: None,
        };
        assert_eq!(flatten_text(&msg), "Title\n\nDesc\n\nStatus: OK\n\nFooter");
    }

    #[test]
    fn flatten_text_prepends_text_before_flattened_embed() {
        let msg = OutboundMsg {
            text: "lead-in".to_string(),
            channel_id: "15551234567".to_string(),
            reply_to: None,
            embed: Some(MsgEmbed {
                title: None,
                description: "Desc".to_string(),
                color: None,
                fields: vec![],
                footer: None,
            }),
            edit_message_id: None,
            blocks: None,
        };
        assert_eq!(flatten_text(&msg), "lead-in\n\nDesc");
    }
}
