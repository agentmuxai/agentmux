// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Slack messaging bridge — Socket Mode WebSocket receive + Web API send.
//!
//! Lifecycle:
//!   1. `SlackBridge::init_global(config, http)` — call once at startup.
//!   2. The Socket Mode loop (inbound) and the outbound send loop run in the
//!      background as two independent tokio tasks joined by
//!      `socket::run_bridge` — see that module's doc comment for why they're
//!      split rather than interleaved (mirrors `telegram/poller.rs`).
//!   3. `SlackBridge::get()` returns the singleton for HTTP handler use.
//!   4. `bridge.send(msg)` enqueues a message for the REST client.
//!   5. `bridge.health()` returns the current connection status.
//!
//! Unlike Discord's fixed Gateway URL, Slack's Socket Mode WS URL is
//! ephemeral and single-use — a fresh URL is fetched via
//! `apps.connections.open` before every connection attempt (initial connect
//! and every reconnect), inside the spawned background task. `init_global`
//! itself does no network I/O and returns immediately, matching Discord's
//! and Telegram's synchronous, non-blocking init.

mod socket;
pub mod rest;
pub mod types;

use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

use crate::messaging::{BridgeHealth, OutboundMsg};

// ── Config ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SlackConfig {
    /// Bot token (`xoxb-...`) — used for Web API calls (chat.postMessage).
    pub bot_token: String,
    /// App-level token (`xapp-...`) — used only for apps.connections.open.
    pub app_token: String,
    /// Default channel ID — inbound filter + default outbound target.
    pub channel_id: String,
    /// Agent ID to inject inbound Slack messages into via the reactive bus.
    /// If None, inbound messages are logged but not forwarded.
    pub target_agent: Option<String>,
}

// ── Bridge ─────────────────────────────────────────────────────────────────

pub struct SlackBridge {
    outbound_tx: mpsc::UnboundedSender<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
}

static GLOBAL_BRIDGE: OnceLock<SlackBridge> = OnceLock::new();

impl SlackBridge {
    /// Initialize the global Slack bridge and start the Socket Mode +
    /// outbound-send background tasks. No-op if already initialized.
    pub fn init_global(config: SlackConfig, http: reqwest::Client) {
        // Startup sanity check (spec §9b) — the two tokens have very
        // different blast radii (app-level token can only open Socket Mode
        // connections; bot token can actually read/post), so a swap
        // produces confusing auth errors. Warn, don't hard-fail.
        if !config.app_token.starts_with("xapp-") {
            tracing::warn!(
                "slack_bridge: messaging:slack:app_token does not start with 'xapp-' — check for a swapped bot/app token"
            );
        }
        if !config.bot_token.starts_with("xoxb-") {
            tracing::warn!(
                "slack_bridge: messaging:slack:bot_token does not start with 'xoxb-' — check for a swapped bot/app token"
            );
        }

        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<OutboundMsg>();
        let health = Arc::new(Mutex::new(BridgeHealth::connecting("slack")));

        let bridge = SlackBridge {
            outbound_tx,
            health: health.clone(),
        };

        if GLOBAL_BRIDGE.set(bridge).is_err() {
            return; // already initialized
        }

        let app_token = config.app_token.clone();
        let bot_token = config.bot_token.clone();
        let channel_id = config.channel_id.clone();
        let target_agent = config.target_agent.clone();

        tokio::spawn(async move {
            socket::run_bridge(
                app_token,
                bot_token,
                channel_id,
                target_agent,
                http,
                outbound_rx,
                health,
            )
            .await;
        });

        tracing::info!(
            "slack_bridge: initialized (channel={}, target={:?})",
            config.channel_id,
            config.target_agent
        );
    }

    pub fn get() -> Option<&'static SlackBridge> {
        GLOBAL_BRIDGE.get()
    }

    /// Enqueue a message for delivery via Slack's chat.postMessage.
    /// Returns error if the bridge task has exited (should not happen in normal operation).
    pub fn send(&self, msg: OutboundMsg) -> Result<(), String> {
        self.outbound_tx
            .send(msg)
            .map_err(|_| "slack_bridge: outbound channel closed".to_string())
    }

    pub fn health(&self) -> BridgeHealth {
        self.health.lock().unwrap().clone()
    }
}

impl crate::messaging::MessagingBridge for SlackBridge {
    fn health(&self) -> BridgeHealth {
        SlackBridge::health(self)
    }
}
