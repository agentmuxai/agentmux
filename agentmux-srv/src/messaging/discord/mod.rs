// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Discord messaging bridge — Gateway WebSocket + REST send.
//!
//! Lifecycle:
//!   1. `DiscordBridge::init_global(config, http)` — call once at startup.
//!   2. The gateway loop runs in a background tokio task (auto-reconnects).
//!   3. `DiscordBridge::get()` returns the singleton for HTTP handler use.
//!   4. `bridge.send(msg)` enqueues a message for the REST client.
//!   5. `bridge.health()` returns the current connection status.

mod gateway;
pub mod rest;
pub mod types;

use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

use crate::messaging::{BridgeHealth, OutboundMsg};

// ── Config ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DiscordConfig {
    /// Discord bot token (obtain from discord.com/developers/applications).
    pub token: String,
    /// Default channel ID — inbound messages are filtered to this channel;
    /// outbound messages target it when `OutboundMsg.channel_id` is empty.
    pub channel_id: String,
    /// Agent ID to inject inbound Discord messages into via the reactive bus.
    /// If None, inbound messages are logged but not forwarded.
    pub target_agent: Option<String>,
    /// Guild ID for guild-scoped slash command registration (Phase 2).
    #[allow(dead_code)]
    pub guild_id: Option<String>,
}

// ── Bridge ─────────────────────────────────────────────────────────────────

pub struct DiscordBridge {
    outbound_tx: mpsc::UnboundedSender<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
}

static GLOBAL_BRIDGE: OnceLock<DiscordBridge> = OnceLock::new();

impl DiscordBridge {
    /// Initialize the global Discord bridge and start the Gateway background task.
    /// No-op if already initialized.
    pub fn init_global(config: DiscordConfig, http: reqwest::Client) {
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<OutboundMsg>();
        let health = Arc::new(Mutex::new(BridgeHealth::connecting("discord")));

        let bridge = DiscordBridge {
            outbound_tx,
            health: health.clone(),
        };

        if GLOBAL_BRIDGE.set(bridge).is_err() {
            return; // already initialized
        }

        let token = config.token.clone();
        let channel_id = config.channel_id.clone();
        let target_agent = config.target_agent.clone();

        tokio::spawn(async move {
            gateway::run_gateway_loop(token, channel_id, target_agent, http, outbound_rx, health)
                .await;
        });

        tracing::info!(
            "discord_bridge: initialized (channel={}, target={:?})",
            config.channel_id,
            config.target_agent
        );
    }

    pub fn get() -> Option<&'static DiscordBridge> {
        GLOBAL_BRIDGE.get()
    }

    /// Enqueue a message for delivery via Discord REST.
    /// Returns error if the bridge task has exited (should not happen in normal operation).
    pub fn send(&self, msg: OutboundMsg) -> Result<(), String> {
        self.outbound_tx
            .send(msg)
            .map_err(|_| "discord_bridge: outbound channel closed".to_string())
    }

    pub fn health(&self) -> BridgeHealth {
        self.health.lock().unwrap().clone()
    }
}

impl crate::messaging::MessagingBridge for DiscordBridge {
    fn health(&self) -> BridgeHealth {
        DiscordBridge::health(self)
    }
}
