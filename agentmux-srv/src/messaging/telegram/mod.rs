// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telegram messaging bridge — long-polling `getUpdates` + REST send.
//!
//! Lifecycle:
//!   1. `TelegramBridge::init_global(config, http)` — call once at startup.
//!   2. The poll loop runs in a background tokio task (retries forever).
//!   3. `TelegramBridge::get()` returns the singleton for HTTP handler use.
//!   4. `bridge.send(msg)` enqueues a message for the REST client.
//!   5. `bridge.health()` returns the current connection status.

mod poller;
pub mod rest;
pub mod types;

use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

use crate::messaging::{BridgeHealth, OutboundMsg};

// ── Config ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub token: String,
    /// Allowlisted chat IDs. Inbound updates from chats not in this list are
    /// silently dropped (no reply, no injection, no log-level above debug).
    pub allowed_chat_ids: Vec<i64>,
    /// Default chat ID for outbound sends when `OutboundMsg.channel_id` is empty.
    pub default_chat_id: Option<i64>,
    /// Agent ID to inject inbound Telegram messages into via the reactive bus.
    /// If None, inbound messages are logged but not forwarded.
    pub target_agent: Option<String>,
}

// ── Bridge ─────────────────────────────────────────────────────────────────

pub struct TelegramBridge {
    outbound_tx: mpsc::UnboundedSender<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
}

static GLOBAL_BRIDGE: OnceLock<TelegramBridge> = OnceLock::new();

impl TelegramBridge {
    /// Initialize the global Telegram bridge and start the poll background
    /// task. No-op if already initialized.
    pub fn init_global(config: TelegramConfig, http: reqwest::Client) {
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<OutboundMsg>();
        let health = Arc::new(Mutex::new(BridgeHealth::connecting("telegram")));

        let bridge = TelegramBridge {
            outbound_tx,
            health: health.clone(),
        };

        if GLOBAL_BRIDGE.set(bridge).is_err() {
            return; // already initialized
        }

        let allowed_chat_ids = config.allowed_chat_ids.clone();
        let target_agent = config.target_agent.clone();

        tokio::spawn(async move {
            poller::run_poll_loop(config, http, outbound_rx, health).await;
        });

        tracing::info!(
            "telegram_bridge: initialized (allowed_chats={:?}, target={:?})",
            allowed_chat_ids,
            target_agent
        );
    }

    pub fn get() -> Option<&'static TelegramBridge> {
        GLOBAL_BRIDGE.get()
    }

    /// Enqueue a message for delivery via Telegram REST.
    /// Returns error if the bridge task has exited (should not happen in normal operation).
    pub fn send(&self, msg: OutboundMsg) -> Result<(), String> {
        self.outbound_tx
            .send(msg)
            .map_err(|_| "telegram_bridge: outbound channel closed".to_string())
    }

    pub fn health(&self) -> BridgeHealth {
        self.health.lock().unwrap().clone()
    }
}

impl crate::messaging::MessagingBridge for TelegramBridge {
    fn health(&self) -> BridgeHealth {
        TelegramBridge::health(self)
    }
}
