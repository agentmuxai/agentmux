// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers for the messaging bridge layer.
//!
//! Routes:
//!   GET  /api/messaging/status          — health of all active bridges
//!   POST /api/messaging/discord/send    — send a message via the Discord bridge
//!   POST /api/messaging/telegram/send   — send a message via the Telegram bridge
//!   POST /api/messaging/slack/send      — send a message via the Slack bridge

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::messaging::{EmbedField, MsgEmbed, MessagingBridge, OutboundMsg};
use crate::messaging::discord::DiscordBridge;
use crate::messaging::slack::SlackBridge;
use crate::messaging::telegram::TelegramBridge;

/// GET /api/messaging/status
pub(super) async fn handle_status(State(_state): State<AppState>) -> impl IntoResponse {
    let mut bridges = vec![];
    if let Some(b) = DiscordBridge::get() {
        bridges.push((b as &dyn MessagingBridge).health());
    }
    if let Some(b) = TelegramBridge::get() {
        bridges.push((b as &dyn MessagingBridge).health());
    }
    if let Some(b) = SlackBridge::get() {
        bridges.push((b as &dyn MessagingBridge).health());
    }
    Json(json!({ "bridges": bridges }))
}

/// POST /api/messaging/discord/send
#[derive(Deserialize)]
pub(super) struct DiscordSendRequest {
    /// Message text (may be empty when using embed only).
    #[serde(default)]
    pub text: String,
    /// Override channel. Empty → use the bridge's default channel.
    #[serde(default)]
    pub channel_id: String,
    /// Optional embed title.
    pub title: Option<String>,
    /// If present, creates a rich embed with this as the description.
    pub embed_description: Option<String>,
    /// Embed footer text (default: "via AgentMux").
    pub footer: Option<String>,
    /// Embed fields.
    #[serde(default)]
    pub fields: Vec<EmbedFieldRequest>,
}

#[derive(Deserialize)]
pub(super) struct EmbedFieldRequest {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

pub(super) async fn handle_discord_send(
    State(_state): State<AppState>,
    Json(req): Json<DiscordSendRequest>,
) -> impl IntoResponse {
    let bridge = match DiscordBridge::get() {
        Some(b) => b,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "discord bridge not initialized — set messaging:discord:enabled in settings"})),
            )
                .into_response();
        }
    };

    let embed = req.embed_description.map(|desc| MsgEmbed {
        title: req.title,
        description: desc,
        color: Some(5_763_719), // Discord green #57F287
        fields: req
            .fields
            .into_iter()
            .map(|f| EmbedField {
                name: f.name,
                value: f.value,
                inline: f.inline,
            })
            .collect(),
        footer: Some(req.footer.unwrap_or_else(|| "via AgentMux".to_string())),
    });

    let msg = OutboundMsg {
        text: req.text,
        channel_id: req.channel_id,
        reply_to: None,
        embed,
        edit_message_id: None,
        blocks: None,
    };

    match bridge.send(msg) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /api/messaging/telegram/send
#[derive(Deserialize)]
pub(super) struct TelegramSendRequest {
    #[serde(default)]
    pub text: String,
    /// Override chat. Empty → use the bridge's default chat
    /// (messaging:telegram:default_chat).
    #[serde(default)]
    pub chat_id: String,
    /// If set, edits this existing message instead of sending a new one
    /// (see spec §2.3 — streaming-output simulation).
    pub edit_message_id: Option<i64>,
}

pub(super) async fn handle_telegram_send(
    State(_state): State<AppState>,
    Json(req): Json<TelegramSendRequest>,
) -> impl IntoResponse {
    let bridge = match TelegramBridge::get() {
        Some(b) => b,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "telegram bridge not initialized — set messaging:telegram:enabled in settings"})),
            )
                .into_response();
        }
    };

    let msg = OutboundMsg {
        text: req.text,
        channel_id: req.chat_id,
        reply_to: None,
        embed: None,
        edit_message_id: req.edit_message_id,
        blocks: None,
    };

    match bridge.send(msg) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /api/messaging/slack/send
///
/// Dual path (spec §7): a raw Block Kit `blocks` escape hatch, a title/body
/// convenience path that synthesizes a minimal 2-block message server-side,
/// or plain `text` only. Block Kit doesn't map onto the shared `MsgEmbed`
/// struct (see `OutboundMsg.blocks` doc comment in `messaging/mod.rs`), so
/// this handler builds `blocks: Vec<serde_json::Value>` directly rather than
/// reusing the Discord embed request shape.
#[derive(Deserialize)]
pub(super) struct SlackSendRequest {
    /// Plain text — always sent as the top-level "text" field (required by
    /// Slack as the fallback/notification string even when blocks are
    /// present).
    #[serde(default)]
    pub text: String,
    /// Override channel. Empty → use the bridge's default channel.
    #[serde(default)]
    pub channel_id: String,
    /// Optional convenience path: a title + body rendered as a simple
    /// two-block Block Kit message (header + section).
    pub title: Option<String>,
    pub body: Option<String>,
    /// Escape hatch: raw Block Kit `blocks` array, passed through verbatim
    /// to chat.postMessage. If present, takes precedence over title/body.
    pub blocks: Option<Vec<serde_json::Value>>,
}

pub(super) async fn handle_slack_send(
    State(_state): State<AppState>,
    Json(req): Json<SlackSendRequest>,
) -> impl IntoResponse {
    let bridge = match SlackBridge::get() {
        Some(b) => b,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "slack bridge not initialized — set messaging:slack:enabled in settings"})),
            )
                .into_response();
        }
    };

    let blocks = if let Some(raw_blocks) = req.blocks {
        Some(raw_blocks)
    } else if req.title.is_some() || req.body.is_some() {
        let mut blocks = vec![];
        if let Some(title) = &req.title {
            blocks.push(json!({
                "type": "header",
                "text": { "type": "plain_text", "text": title }
            }));
        }
        if let Some(body) = &req.body {
            blocks.push(json!({
                "type": "section",
                "text": { "type": "mrkdwn", "text": body }
            }));
        }
        Some(blocks)
    } else {
        None
    };

    let text = if req.text.is_empty() {
        // Slack requires non-empty `text` as the fallback string even when
        // `blocks` carries the real content — synthesize one from
        // title/body so a title/body-only request doesn't fail Slack-side.
        match (&req.title, &req.body) {
            (Some(t), Some(b)) => format!("{t}: {b}"),
            (Some(t), None) => t.clone(),
            (None, Some(b)) => b.clone(),
            (None, None) => String::new(),
        }
    } else {
        req.text
    };

    let msg = OutboundMsg {
        text,
        channel_id: req.channel_id,
        reply_to: None,
        embed: None,
        edit_message_id: None,
        blocks,
    };

    match bridge.send(msg) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}
