// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Messaging bridge framework — common types shared by all platform bridges.
//!
//! Each bridge (Discord, Telegram, Slack, …) runs as a background tokio task.
//! Inbound messages are injected into the reactive bus; outbound messages
//! arrive via HTTP endpoint or MCP tool.

pub mod discord;

use serde::{Deserialize, Serialize};

// ── Common message types ───────────────────────────────────────────────────

/// A message received from a messaging platform, normalized for reactive bus injection.
/// Constructed in Phase 2 when per-message metadata is surfaced to the frontend.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMsg {
    pub platform: String,
    pub platform_msg_id: String,
    pub from_id: String,
    pub from_name: String,
    pub channel_id: String,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<String>,
    pub received_at: u64,
}

/// A message to send to a messaging platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMsg {
    pub text: String,
    /// Target channel. Empty → use the bridge's default channel.
    #[serde(default)]
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed: Option<MsgEmbed>,
}

/// Rich embed for platforms that support structured output (Discord, Slack).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgEmbed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    /// Platform-specific color integer (Discord: 0xRRGGBB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(default)]
    pub fields: Vec<EmbedField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

// ── Bridge health ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeStatus {
    Connected,
    Connecting,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeHealth {
    pub platform: String,
    pub status: BridgeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<u64>,
    pub reconnect_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BridgeHealth {
    pub fn connecting(platform: &str) -> Self {
        BridgeHealth {
            platform: platform.to_string(),
            status: BridgeStatus::Connecting,
            latency_ms: None,
            last_event_at: None,
            reconnect_count: 0,
            error: None,
        }
    }
}
