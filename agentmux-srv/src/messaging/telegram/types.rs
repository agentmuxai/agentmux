// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telegram Bot API wire types (getUpdates / sendMessage / editMessageText).
//!
//! `CallbackQuery` is parsed for wire compatibility (it can arrive in the same
//! `getUpdates` batch as `message` updates) but is not acted on in v1 — inline
//! keyboards / callback-query handling is explicitly out of scope for this PR
//! (see spec §2.4, §10 PR 4).

use serde::{Deserialize, Serialize};

// ── Inbound (getUpdates) ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    #[allow(dead_code)]
    pub callback_query: Option<CallbackQuery>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub chat: Chat,
    #[serde(default)]
    pub from: Option<User>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub date: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub type_: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    #[allow(dead_code)]
    pub id: i64,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub is_bot: bool,
}

/// Inbound callback-query update (button tap). Not processed in v1 — see
/// module doc comment.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub message: Option<Message>,
}

// ── Bot API response envelope ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
// Explicit deserialize bound: without this, serde-derive's naive bound
// inference for the `#[serde(default)]` field below adds `T: Default` even
// though the field type is `Option<T>` (whose `Default` never needs `T:
// Default`) — a known serde-derive quirk. Override to the bound we actually need.
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct TelegramResponse<T> {
    pub ok: bool,
    #[serde(default)]
    pub result: Option<T>,
    #[serde(default)]
    #[allow(dead_code)]
    pub error_code: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<ResponseParameters>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseParameters {
    /// Present on 429 responses: seconds to wait before retrying.
    #[serde(default)]
    pub retry_after: Option<u64>,
    /// Bot API 7.8+: `"chat"` or `"user"` scope for a 429. Absent on older
    /// responses / globally-scoped limits — treat missing as global scope.
    #[serde(default)]
    pub scope: Option<String>,
}

// ── Outbound (sendMessage / editMessageText) ────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SendMessageBody {
    pub chat_id: i64,
    pub text: String,
    pub parse_mode: &'static str,
}

#[derive(Debug, Serialize)]
pub struct EditMessageTextBody {
    pub chat_id: i64,
    pub message_id: i64,
    pub text: String,
    pub parse_mode: &'static str,
}
