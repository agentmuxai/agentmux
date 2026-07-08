// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers for the messaging bridge layer.
//!
//! Routes:
//!   GET  /api/messaging/status          — health of all active bridges
//!   POST /api/messaging/discord/send    — send a message via the Discord bridge
//!   POST /api/messaging/telegram/send   — send a message via the Telegram bridge

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
