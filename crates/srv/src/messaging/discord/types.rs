// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Discord Gateway and REST API wire types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Gateway intents bitmask: GUILDS (1<<0) | GUILD_MESSAGES (1<<9) | MESSAGE_CONTENT (1<<15).
/// MESSAGE_CONTENT is a privileged intent — enable it in the Discord Developer Portal
/// for private bots under 100 servers (no review required).
pub const INTENTS: u32 = (1 << 0) | (1 << 9) | (1 << 15);

// ── Gateway opcodes ──────────────────────────────────────────────────────

pub mod opcode {
    pub const DISPATCH: u8 = 0;
    pub const HEARTBEAT: u8 = 1;
    pub const IDENTIFY: u8 = 2;
    pub const RESUME: u8 = 6;
    pub const RECONNECT: u8 = 7;
    pub const INVALID_SESSION: u8 = 9;
    pub const HELLO: u8 = 10;
    pub const HEARTBEAT_ACK: u8 = 11;
}

// ── Gateway wire types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GatewayPayload {
    #[serde(rename = "op")]
    pub opcode: u8,
    #[serde(rename = "d")]
    pub data: Option<Value>,
    #[serde(rename = "s")]
    pub seq: Option<u64>,
    #[serde(rename = "t")]
    pub event_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IdentifyPayload {
    pub op: u8,
    pub d: IdentifyData,
}

#[derive(Debug, Serialize)]
pub struct IdentifyData {
    pub token: String,
    pub intents: u32,
    pub properties: IdentifyProperties,
}

#[derive(Debug, Serialize)]
pub struct IdentifyProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatPayload {
    pub op: u8,
    pub d: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ResumePayload {
    pub op: u8,
    pub d: ResumeData,
}

#[derive(Debug, Serialize)]
pub struct ResumeData {
    pub token: String,
    pub session_id: String,
    pub seq: u64,
}

// ── READY event ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReadyEvent {
    pub session_id: String,
    pub resume_gateway_url: String,
    #[serde(default)]
    pub user: Option<DiscordUser>,
}

// ── User ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct DiscordUser {
    #[allow(dead_code)]
    pub id: String,
    pub username: String,
    /// Present and true on bot accounts, absent on humans.
    #[serde(default)]
    pub bot: Option<bool>,
}

// ── MESSAGE_CREATE event ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MessageCreate {
    pub id: String,
    pub channel_id: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub content: String,
    pub author: DiscordUser,
}

// ── REST send types ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SendMessageBody {
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<DiscordEmbed>,
}

#[derive(Debug, Serialize)]
pub struct DiscordEmbed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<DiscordEmbedField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<DiscordEmbedFooter>,
}

#[derive(Debug, Serialize)]
pub struct DiscordEmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

#[derive(Debug, Serialize)]
pub struct DiscordEmbedFooter {
    pub text: String,
}
