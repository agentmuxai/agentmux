// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WhatsApp Cloud API messaging bridge — inbound webhook receiver + outbound
//! Graph API send. See
//! `docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md`.
//!
//! Lifecycle:
//!   1. `WhatsAppBridge::init_global(config, http)` — call once at startup.
//!   2. The outbound send loop runs in a background tokio task, reading from
//!      an mpsc channel — same shape as Discord/Telegram/Slack, so this
//!      bridge implements the real (sync) `MessagingBridge::send` exactly
//!      like its siblings, rather than the async-trait alternative the spec
//!      sketched before the trait had actually shipped (see §5.1's "open
//!      decision point" — resolved here in favor of matching the merged
//!      Discord/Telegram/Slack convention instead of relitigating it).
//!   3. Inbound is passive: Meta POSTs to `GET`/`POST /webhook/whatsapp`,
//!      handled by ordinary axum request tasks (`webhook.rs`), not a loop
//!      this module drives. That's already fully concurrent with the
//!      outbound send loop — one lives on axum's request-handling tasks, the
//!      other on its own `tokio::spawn`ed task — so there's no interleaving
//!      hazard to split apart the way `telegram/poller.rs` and
//!      `slack/socket.rs` had to (see those files' doc comments for the
//!      review finding that motivated their inbound/outbound task split).
//!   4. `WhatsAppBridge::get()` returns the singleton for HTTP handler /
//!      webhook handler use.
//!   5. `bridge.send(msg)` enqueues a message for the Graph API sender.
//!   6. `bridge.health()` returns the current status.
//!
//! ## Scope deviation from the spec: no automated tunnel management
//!
//! The spec (§4) designs a `TunnelManager` that spawns and supervises a
//! `cloudflared` subprocess, auto-detects its connected URL, and sequences
//! bridge startup around it. That subsystem cannot be meaningfully built or
//! verified in this environment (no live Cloudflare/Meta credentials, no way
//! to exercise a real tunnel handshake), so it is **not implemented in this
//! PR**. v1 assumes the operator has already stood up their own tunnel
//! (Cloudflare Tunnel, ngrok, or otherwise) pointed at this instance's
//! webhook port, and has manually registered the callback URL + verify token
//! in Meta's App Dashboard — both one-time, out-of-band steps independent of
//! `agentmux-srv`'s process lifecycle. `messaging:whatsapp:tunnel_domain` is
//! kept as a config field purely so the startup log can print the full
//! callback URL as a convenience reminder; nothing reads it to manage a
//! subprocess. `messaging/tunnel.rs` (the spec's shared `TunnelManager`) is
//! not created. This scoping decision means `BridgeHealth` reflects only
//! "the bridge is initialized and the outbound sender is running", not
//! "Meta can currently reach us" — the latter depends on infrastructure this
//! process doesn't own or observe.

mod webhook;
pub mod rest;
pub mod types;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

pub use webhook::{handle_inbound, handle_verify};

use crate::messaging::{BridgeHealth, BridgeStatus, OutboundMsg};

// ── Config ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    /// WhatsApp Business phone number ID (Meta App Dashboard > WhatsApp > API Setup).
    pub phone_number_id: String,
    /// System User access token (permanent). Treat as a secret — do not log.
    pub access_token: String,
    /// Meta App Secret, used to validate `X-Hub-Signature-256` on inbound
    /// webhooks. Treat as a secret — do not log.
    pub app_secret: String,
    /// Verify token used in the `GET /webhook/whatsapp` handshake. Treat as
    /// a secret — do not log.
    pub webhook_verify_token: String,
    /// Agent ID to inject inbound WhatsApp messages into via the reactive
    /// bus. If `None`, inbound messages are logged but not forwarded.
    pub target_agent: Option<String>,
    /// Template name used for outbound sends outside the 24h customer
    /// service window (spec §3.3/§3.4). If `None` and the window has
    /// expired, `send()` fails fast rather than attempting delivery.
    pub fallback_template: Option<String>,
    /// Template language code (BCP-47), e.g. "en_US".
    pub fallback_template_lang: String,
}

// ── Bridge ─────────────────────────────────────────────────────────────────

pub struct WhatsAppBridge {
    outbound_tx: mpsc::UnboundedSender<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
    app_secret: String,
    webhook_verify_token: String,
    target_agent: Option<String>,
    /// Per-sender (WhatsApp phone number) last-inbound timestamp (unix ms),
    /// for 24h customer service window tracking (spec §3.3). Not persisted
    /// to disk in v1 — a restart means the bridge falls back to templates
    /// until the next inbound message re-opens the window, an accepted
    /// degradation per the spec.
    window_state: Arc<Mutex<HashMap<String, u64>>>,
}

static GLOBAL_BRIDGE: OnceLock<WhatsAppBridge> = OnceLock::new();

impl WhatsAppBridge {
    /// Initialize the global WhatsApp bridge and start the outbound send
    /// background task. No-op if already initialized.
    ///
    /// Unlike Discord/Telegram/Slack, there is no inbound connection loop to
    /// spawn here — inbound delivery is passive HTTP handled by
    /// `webhook::handle_verify`/`handle_inbound`, already registered
    /// unconditionally on the main axum router (see `server/mod.rs`)
    /// regardless of whether `init_global` has run. Those handlers return
    /// `503` via `WhatsAppBridge::get()` returning `None` until this runs.
    pub fn init_global(config: WhatsAppConfig, http: reqwest::Client) {
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<OutboundMsg>();
        let health = Arc::new(Mutex::new(BridgeHealth::connecting("whatsapp")));
        let window_state: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));

        let bridge = WhatsAppBridge {
            outbound_tx,
            health: health.clone(),
            app_secret: config.app_secret.clone(),
            webhook_verify_token: config.webhook_verify_token.clone(),
            target_agent: config.target_agent.clone(),
            window_state: window_state.clone(),
        };

        if GLOBAL_BRIDGE.set(bridge).is_err() {
            return; // already initialized
        }

        // No tunnel subprocess to wait on in this PR's scope (see module
        // doc comment). The bridge is "connected" in the sense that the
        // outbound sender and webhook handlers are live; whether Meta can
        // actually reach the webhook route depends on infra this process
        // doesn't manage, so this status should be read as "ready", not as
        // proof of end-to-end reachability.
        {
            let mut h = health.lock().unwrap();
            h.status = BridgeStatus::Connected;
        }

        let target_agent = config.target_agent.clone();
        tokio::spawn(async move {
            run_outbound_loop(config, http, window_state, outbound_rx).await;
        });

        tracing::info!(
            "whatsapp_bridge: initialized (target={:?}) — ensure a tunnel is pointed at this \
             instance's webhook port and the callback URL is registered in Meta App Dashboard > \
             WhatsApp > Configuration (v1 does not manage the tunnel itself)",
            target_agent
        );
    }

    pub fn get() -> Option<&'static WhatsAppBridge> {
        GLOBAL_BRIDGE.get()
    }

    /// Enqueue a message for delivery via the Graph API.
    /// Returns error if the bridge task has exited (should not happen in normal operation).
    pub fn send(&self, msg: OutboundMsg) -> Result<(), String> {
        self.outbound_tx
            .send(msg)
            .map_err(|_| "whatsapp_bridge: outbound channel closed".to_string())
    }

    pub fn health(&self) -> BridgeHealth {
        self.health.lock().unwrap().clone()
    }

    pub(crate) fn app_secret(&self) -> &str {
        &self.app_secret
    }

    pub(crate) fn verify_token(&self) -> &str {
        &self.webhook_verify_token
    }

    pub(crate) fn target_agent(&self) -> Option<&String> {
        self.target_agent.as_ref()
    }

    /// Called by `webhook::handle_inbound` on each valid inbound message, to
    /// update the 24h window state (spec §3.3) and the health snapshot's
    /// `last_event_at`.
    ///
    /// Keys on `rest::normalize_phone(from_id)`, not the raw id, so this
    /// matches whatever normalized form `rest::send_message` looks the
    /// number up under — Meta's inbound `from` has no `+` prefix, but a
    /// caller sending via `handle_whatsapp_send` may supply one (E.164);
    /// without a shared normalization the window lookup always missed
    /// (found via review).
    pub(crate) fn record_inbound(&self, from_id: &str) {
        let now = now_ms();
        self.window_state
            .lock()
            .unwrap()
            .insert(rest::normalize_phone(from_id), now);
        let mut h = self.health.lock().unwrap();
        h.last_event_at = Some(now / 1000);
    }
}

impl crate::messaging::MessagingBridge for WhatsAppBridge {
    fn health(&self) -> BridgeHealth {
        WhatsAppBridge::health(self)
    }
}

/// Drains the outbound mpsc queue and calls the Graph API for each message.
/// Runs as its own `tokio::spawn`ed task, fully independent of the axum
/// request tasks servicing `webhook::handle_inbound` — a slow or
/// rate-limited send here cannot block the webhook route from accepting
/// Meta's next delivery (see module doc comment).
async fn run_outbound_loop(
    config: WhatsAppConfig,
    http: reqwest::Client,
    window_state: Arc<Mutex<HashMap<String, u64>>>,
    mut outbound_rx: mpsc::UnboundedReceiver<OutboundMsg>,
) {
    while let Some(msg) = outbound_rx.recv().await {
        if let Err(e) = rest::send_message(&http, &config, &window_state, &msg).await {
            tracing::warn!("whatsapp_bridge: send failed: {e}");
        }
    }
}

fn now_ms() -> u64 {
    agentmux_common::time::now_ms_u64()
}
