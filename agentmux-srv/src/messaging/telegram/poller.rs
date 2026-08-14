// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telegram long-polling loop: `getUpdates` → dispatch → offset advance;
//! outbound mpsc → REST send. Plays the same role as
//! `discord::gateway::run_gateway_loop`, but with no session/resume state —
//! long polling has no connection to lose beyond a single HTTP request, so
//! this is a plain retry-forever loop around one HTTP GET.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::backend::reactive::handler::get_global_handler;
use crate::backend::reactive::types::InjectionRequest;
use crate::messaging::{BridgeHealth, BridgeStatus, OutboundMsg};

use super::rest;
use super::types::Update;
use super::TelegramConfig;

const RECONNECT_DELAY_SECS: u64 = 5;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;

/// Minimum spacing between two outbound sends to the same chat (spec §2.5 —
/// "1 msg/second" limit, enforced with a bit of headroom).
const CHAT_MIN_SPACING: Duration = Duration::from_millis(1100);

/// Runs the inbound long-poll loop and the outbound send loop as two fully
/// independent tasks (`tokio::join!`, not `tokio::select!` on a shared loop
/// iteration).
///
/// The original design interleaved both in one `tokio::select!`: whichever
/// branch was constructed fresh each iteration. That meant an outbound send
/// arriving mid-poll would drop (cancel) the in-flight `getUpdates` future,
/// and — worse — any rate-limit backoff sleep inside outbound handling (up to
/// a Telegram-specified `retry_after`, potentially many seconds) blocked the
/// *entire* loop, including inbound polling, since both branches shared one
/// iteration. A burst of outbound sends (e.g. `edit_message_id` streaming
/// updates) could starve inbound message delivery for as long as the burst +
/// backoff lasted. Splitting into two independently-scheduled tasks removes
/// this coupling entirely: the inbound poll always runs on its own cadence
/// regardless of outbound activity.
pub async fn run_poll_loop(
    config: TelegramConfig,
    http: reqwest::Client,
    outbound_rx: mpsc::UnboundedReceiver<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
) {
    let inbound_config = config.clone();
    tokio::join!(
        run_inbound_loop(inbound_config, health),
        run_outbound_loop(config, http, outbound_rx),
    );
}

async fn run_inbound_loop(config: TelegramConfig, health: Arc<Mutex<BridgeHealth>>) {
    // Dedicated client for getUpdates: needs a longer per-request timeout than
    // the 30s `timeout` param Telegram itself waits on (spec §9). Sends use
    // a client with a short per-request timeout override instead (rest.rs),
    // so a stuck send can't wedge this loop — and now, since sends run on a
    // fully separate task, they couldn't wedge it even without that timeout.
    let poll_http = rest::build_poll_client();

    let mut offset: i64 = 0;
    let mut delay_secs = RECONNECT_DELAY_SECS;

    loop {
        match rest::get_updates(&poll_http, &config.token, offset).await {
            Ok(updates) => {
                delay_secs = RECONNECT_DELAY_SECS;
                {
                    let mut h = health.lock().unwrap();
                    h.status = BridgeStatus::Connected;
                    h.error = None;
                }

                // Offset advances past every update in the batch — including
                // ones that fail to parse — so a permanently-malformed update
                // can never wedge the poll loop (spec §9).
                let mut max_update_id = offset - 1;
                for raw in &updates {
                    let update_id = raw.get("update_id").and_then(|v| v.as_i64());
                    let Some(update_id) = update_id else {
                        tracing::warn!(
                            "telegram_bridge: update missing update_id, skipping (offset cannot advance past it)"
                        );
                        continue;
                    };
                    if update_id > max_update_id {
                        max_update_id = update_id;
                    }

                    match serde_json::from_value::<Update>(raw.clone()) {
                        Ok(update) => handle_update(&update, &config, &health),
                        Err(e) => {
                            tracing::warn!(
                                "telegram_bridge: malformed update {update_id}: {e} — skipping"
                            );
                        }
                    }
                }
                if max_update_id >= offset {
                    offset = max_update_id + 1;
                }
            }
            Err(e) => {
                tracing::warn!("telegram_bridge: getUpdates failed: {e}");
                {
                    let mut h = health.lock().unwrap();
                    h.status = BridgeStatus::Error;
                    h.error = Some(e);
                    h.reconnect_count += 1;
                }
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                delay_secs = (delay_secs * 2).min(MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

async fn run_outbound_loop(
    config: TelegramConfig,
    http: reqwest::Client,
    mut outbound_rx: mpsc::UnboundedReceiver<OutboundMsg>,
) {
    // Per-chat outbound rate limiting (spec §2.5, v1 scope: spacing + reactive
    // 429 backoff only, no full token-bucket accounting). Owned entirely by
    // this task now — never shared with the inbound loop.
    let mut chat_last_sent: HashMap<i64, Instant> = HashMap::new();
    let mut chat_backoff_until: HashMap<i64, Instant> = HashMap::new();
    let mut global_backoff_until: Option<Instant> = None;

    while let Some(msg) = outbound_rx.recv().await {
        handle_outbound(
            &http,
            &config,
            msg,
            &mut chat_last_sent,
            &mut chat_backoff_until,
            &mut global_backoff_until,
        )
        .await;
    }
}

/// Filters, envelopes, and injects one inbound update. Plays the same role as
/// `gateway.rs`'s `handle_dispatch` for `MESSAGE_CREATE`.
fn handle_update(update: &Update, config: &TelegramConfig, health: &Arc<Mutex<BridgeHealth>>) {
    let Some(message) = &update.message else {
        // callback_query (or another update kind we don't yet request) —
        // inline-keyboard handling is PR4 scope, not implemented here.
        tracing::debug!("telegram_bridge: unhandled update kind (update_id={})", update.update_id);
        return;
    };

    // Ignore messages from bots (including our own bot, e.g. echoes).
    if message.from.as_ref().map(|u| u.is_bot).unwrap_or(false) {
        return;
    }

    // Allowlist is the *only* gate for Telegram (spec §8.1) — a bot's
    // username is public, unlike Discord's invite-gated guild membership.
    // Silently drop, never reply, so an unlisted chat can't confirm the bot
    // is alive.
    if !config.allowed_chat_ids.contains(&message.chat.id) {
        tracing::debug!(
            "telegram_bridge: dropping message from non-allowlisted chat {}",
            message.chat.id
        );
        return;
    }

    {
        let mut h = health.lock().unwrap();
        h.last_event_at = Some(unix_secs());
    }

    let Some(target) = &config.target_agent else {
        tracing::debug!("telegram_bridge: message from chat {} (no target agent configured)", message.chat.id);
        return;
    };

    let username = message
        .from
        .as_ref()
        .and_then(|u| u.username.as_deref())
        .unwrap_or("unknown");
    let text = message.text.as_deref().unwrap_or("");
    let envelope = format!("[Telegram @{username}]: {text}");

    let handler = get_global_handler();
    let req = InjectionRequest {
        target_agent: target.clone(),
        message: envelope,
        source_agent: Some("telegram".to_string()),
        request_id: Some(update.update_id.to_string()),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: Some("wan".to_string()),
        forward_hops: 0,
        ..Default::default()
    };
    let result = handler.inject_message(req);
    if result.success {
        tracing::debug!("telegram_bridge: injected msg from {username} to agent {target}");
    } else {
        tracing::warn!("telegram_bridge: inject to agent {target} failed: {:?}", result.error);
    }
}

/// Resolves an `OutboundMsg.channel_id` (a stringified chat id, or empty for
/// "use the bridge's default") into a Telegram numeric chat id.
fn resolve_chat_id(channel_id: &str, default_chat_id: Option<i64>) -> Option<i64> {
    if channel_id.is_empty() {
        return default_chat_id;
    }
    channel_id.trim().parse::<i64>().ok().or(default_chat_id)
}

async fn handle_outbound(
    http: &reqwest::Client,
    config: &TelegramConfig,
    msg: OutboundMsg,
    chat_last_sent: &mut HashMap<i64, Instant>,
    chat_backoff_until: &mut HashMap<i64, Instant>,
    global_backoff_until: &mut Option<Instant>,
) {
    let Some(chat_id) = resolve_chat_id(&msg.channel_id, config.default_chat_id) else {
        tracing::warn!(
            "telegram_bridge: send dropped — no chat_id (empty channel_id and no messaging:telegram:default_chat configured)"
        );
        return;
    };

    if let Some(until) = *global_backoff_until {
        let now = Instant::now();
        if now < until {
            tokio::time::sleep(until - now).await;
        }
        *global_backoff_until = None;
    }

    if let Some(until) = chat_backoff_until.get(&chat_id).copied() {
        let now = Instant::now();
        if now < until {
            tokio::time::sleep(until - now).await;
        }
        chat_backoff_until.remove(&chat_id);
    }

    if let Some(last) = chat_last_sent.get(&chat_id) {
        let elapsed = last.elapsed();
        if elapsed < CHAT_MIN_SPACING {
            tokio::time::sleep(CHAT_MIN_SPACING - elapsed).await;
        }
    }

    match rest::send_or_edit(http, &config.token, chat_id, &msg.text, msg.edit_message_id).await {
        Ok(_) => {
            chat_last_sent.insert(chat_id, Instant::now());
        }
        Err(e) => {
            tracing::warn!("telegram_bridge: send failed: {e}");
            if let Some(retry_after) = e.retry_after {
                let backoff = Duration::from_secs(retry_after) + jitter();
                if e.chat_scoped {
                    chat_backoff_until.insert(chat_id, Instant::now() + backoff);
                } else {
                    *global_backoff_until = Some(Instant::now() + backoff);
                }
            }
        }
    }
}

/// Cheap jitter (0-300ms) without pulling in a `rand` dependency — derived
/// from the current time's sub-second nanoseconds.
fn jitter() -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    Duration::from_millis((nanos % 300) as u64)
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_chat_id_uses_channel_id_when_present() {
        assert_eq!(resolve_chat_id("123456", Some(999)), Some(123456));
    }

    #[test]
    fn resolve_chat_id_falls_back_to_default_when_empty() {
        assert_eq!(resolve_chat_id("", Some(999)), Some(999));
    }

    #[test]
    fn resolve_chat_id_none_when_empty_and_no_default() {
        assert_eq!(resolve_chat_id("", None), None);
    }

    #[test]
    fn resolve_chat_id_falls_back_to_default_on_unparseable_channel_id() {
        assert_eq!(resolve_chat_id("not-a-number", Some(999)), Some(999));
    }

    #[test]
    fn resolve_chat_id_trims_whitespace() {
        assert_eq!(resolve_chat_id("  42  ", None), Some(42));
    }
}
