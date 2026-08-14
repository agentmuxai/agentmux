// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Discord Gateway WebSocket client.
//!
//! Protocol flow: connect → HELLO → IDENTIFY (or RESUME) → READY → event loop.
//!
//! On disconnect:
//!   - If we have a stored session_id + last_seq, attempt RESUME using the
//!     `resume_gateway_url` from the prior READY payload.
//!   - On INVALID_SESSION(resumable=false) or RECONNECT, drop session and re-IDENTIFY.
//!
//! Heartbeat:
//!   - HELLO provides `heartbeat_interval` in milliseconds.
//!   - We send HEARTBEAT every interval, tracking ACK. If the prior heartbeat
//!     was not ACK'd, the connection is a zombie — close and reconnect.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::Message};

use crate::backend::reactive::handler::get_global_handler;
use crate::backend::reactive::types::InjectionRequest;
use crate::messaging::{BridgeHealth, BridgeStatus, OutboundMsg};

use super::rest;
use super::types::{
    opcode, GatewayPayload, HeartbeatPayload, IdentifyData, IdentifyPayload, IdentifyProperties,
    MessageCreate, ReadyEvent, ResumeData, ResumePayload, INTENTS,
};

const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const RECONNECT_DELAY_SECS: u64 = 5;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;

#[derive(Clone)]
struct Session {
    session_id: String,
    resume_url: String,
    last_seq: u64,
}

pub async fn run_gateway_loop(
    token: String,
    channel_id: String,
    target_agent: Option<String>,
    http: reqwest::Client,
    mut outbound_rx: mpsc::UnboundedReceiver<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
) {
    let mut delay_secs = RECONNECT_DELAY_SECS;
    let mut session: Option<Session> = None;

    loop {
        {
            let mut h = health.lock().unwrap();
            h.status = BridgeStatus::Connecting;
        }

        let connect_url = session
            .as_ref()
            .map(|s| {
                // Append gateway params if the resume URL doesn't include them.
                if s.resume_url.contains("?v=") {
                    s.resume_url.clone()
                } else {
                    format!("{}?v=10&encoding=json", s.resume_url)
                }
            })
            .unwrap_or_else(|| GATEWAY_URL.to_string());

        tracing::info!("discord_bridge: connecting to {}", connect_url);
        let session_start = std::time::Instant::now();

        match run_session(
            &token,
            &channel_id,
            &target_agent,
            &connect_url,
            &http,
            &mut outbound_rx,
            &health,
            session.take(),
        )
        .await
        {
            Ok(new_session) => {
                session = new_session;
                if session_start.elapsed().as_secs() > 30 {
                    delay_secs = RECONNECT_DELAY_SECS;
                }
                tracing::info!("discord_bridge: session ended cleanly");
            }
            Err(e) => {
                session = None;
                if session_start.elapsed().as_secs() > 30 {
                    delay_secs = RECONNECT_DELAY_SECS;
                }
                tracing::warn!("discord_bridge: session error: {e}");
                {
                    let mut h = health.lock().unwrap();
                    h.status = BridgeStatus::Error;
                    h.error = Some(e);
                    h.reconnect_count += 1;
                }
            }
        }

        {
            let mut h = health.lock().unwrap();
            if h.status != BridgeStatus::Error {
                h.status = BridgeStatus::Disconnected;
            }
        }

        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        delay_secs = (delay_secs * 2).min(MAX_RECONNECT_DELAY_SECS);
    }
}

/// Run a single Gateway session. Returns:
///   Ok(Some(session)) — clean disconnect, can attempt RESUME
///   Ok(None)          — clean disconnect, session is invalid (re-IDENTIFY)
///   Err(e)            — unexpected error, will re-IDENTIFY after backoff
async fn run_session(
    token: &str,
    channel_id: &str,
    target_agent: &Option<String>,
    connect_url: &str,
    http: &reqwest::Client,
    outbound_rx: &mut mpsc::UnboundedReceiver<OutboundMsg>,
    health: &Arc<Mutex<BridgeHealth>>,
    resume_session: Option<Session>,
) -> Result<Option<Session>, String> {
    use tokio_tungstenite::tungstenite::http::Request;

    let request = Request::builder()
        .uri(connect_url)
        .body(())
        .map_err(|e| format!("build ws request: {e}"))?;

    let (ws_stream, _) = connect_async_tls_with_config(request, None, false, None)
        .await
        .map_err(|e| format!("ws connect: {e}"))?;

    let (mut write, mut read) = ws_stream.split();

    // Heartbeat: starts as pending, replaced with a timed sleep on HELLO.
    // Using Box<dyn Future> avoids pinning complexity inside the select! arms.
    let mut hb_sleep: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        Box::pin(std::future::pending());
    let mut hb_interval_ms: u64 = 0;
    let mut last_hb_acked = true;

    let mut session: Option<Session> = None;
    let mut identified = false;

    loop {
        tokio::select! {
            // Heartbeat tick
            _ = &mut hb_sleep, if hb_interval_ms > 0 => {
                if !last_hb_acked {
                    tracing::warn!("discord_bridge: heartbeat zombie — reconnecting");
                    let _ = write.send(Message::Close(None)).await;
                    return Ok(session);
                }
                let seq = session.as_ref().map(|s| s.last_seq);
                let payload = serde_json::to_string(&HeartbeatPayload {
                    op: opcode::HEARTBEAT,
                    d: seq,
                })
                .unwrap_or_default();
                if let Err(e) = write.send(Message::Text(payload.into())).await {
                    return Err(format!("heartbeat send: {e}"));
                }
                last_hb_acked = false;
                hb_sleep = Box::pin(tokio::time::sleep(Duration::from_millis(hb_interval_ms)));
            }

            // Outbound message from agent → Discord REST
            msg = outbound_rx.recv() => {
                match msg {
                    None => return Ok(session),
                    Some(outbound) => {
                        let ch = if outbound.channel_id.is_empty() {
                            channel_id
                        } else {
                            outbound.channel_id.as_str()
                        };
                        if let Err(e) = rest::send_message(http, token, ch, &outbound).await {
                            tracing::warn!("discord_bridge: send failed: {e}");
                        }
                    }
                }
            }

            // Incoming Gateway frame
            frame = read.next() => {
                match frame {
                    None => return Ok(session),
                    Some(Err(e)) => return Err(format!("ws recv: {e}")),
                    Some(Ok(Message::Text(text))) => {
                        let payload: GatewayPayload = match serde_json::from_str(&text) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!("discord_bridge: parse error: {e}");
                                continue;
                            }
                        };

                        // Track sequence number on every dispatch
                        if let (Some(seq), Some(sess)) = (payload.seq, session.as_mut()) {
                            if seq > sess.last_seq {
                                sess.last_seq = seq;
                            }
                        }

                        match payload.opcode {
                            opcode::HELLO => {
                                hb_interval_ms = payload.data
                                    .as_ref()
                                    .and_then(|d| d.get("heartbeat_interval"))
                                    .and_then(Value::as_u64)
                                    .unwrap_or(41_250);

                                // First heartbeat fires after one full interval
                                hb_sleep = Box::pin(tokio::time::sleep(
                                    Duration::from_millis(hb_interval_ms),
                                ));
                                last_hb_acked = true;

                                if let Some(prev) = resume_session.as_ref().filter(|_| !identified) {
                                    let payload = serde_json::to_string(&ResumePayload {
                                        op: opcode::RESUME,
                                        d: ResumeData {
                                            token: token.to_string(),
                                            session_id: prev.session_id.clone(),
                                            seq: prev.last_seq,
                                        },
                                    })
                                    .unwrap_or_default();
                                    write.send(Message::Text(payload.into()))
                                        .await
                                        .map_err(|e| format!("resume send: {e}"))?;
                                    session = Some(prev.clone());
                                } else {
                                    let payload = serde_json::to_string(&IdentifyPayload {
                                        op: opcode::IDENTIFY,
                                        d: IdentifyData {
                                            token: token.to_string(),
                                            intents: INTENTS,
                                            properties: IdentifyProperties {
                                                os: std::env::consts::OS.to_string(),
                                                browser: "agentmux".to_string(),
                                                device: "agentmux".to_string(),
                                            },
                                        },
                                    })
                                    .unwrap_or_default();
                                    write.send(Message::Text(payload.into()))
                                        .await
                                        .map_err(|e| format!("identify send: {e}"))?;
                                }
                                identified = true;
                            }

                            opcode::HEARTBEAT_ACK => {
                                last_hb_acked = true;
                                let now = unix_secs();
                                let mut h = health.lock().unwrap();
                                h.status = BridgeStatus::Connected;
                                h.error = None;
                                h.last_event_at = Some(now);
                            }

                            opcode::HEARTBEAT => {
                                // Server-initiated heartbeat request
                                let seq = session.as_ref().map(|s| s.last_seq);
                                let payload = serde_json::to_string(&HeartbeatPayload {
                                    op: opcode::HEARTBEAT,
                                    d: seq,
                                })
                                .unwrap_or_default();
                                let _ = write.send(Message::Text(payload.into())).await;
                            }

                            opcode::RECONNECT => {
                                tracing::info!("discord_bridge: server requested reconnect (RESUME eligible)");
                                let _ = write.send(Message::Close(None)).await;
                                return Ok(session);
                            }

                            opcode::INVALID_SESSION => {
                                let resumable = payload.data
                                    .as_ref()
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false);
                                tracing::warn!(
                                    "discord_bridge: invalid session (resumable={resumable})"
                                );
                                let _ = write.send(Message::Close(None)).await;
                                return Ok(if resumable { session } else { None });
                            }

                            opcode::DISPATCH => {
                                handle_dispatch(
                                    payload.event_type.as_deref(),
                                    payload.data,
                                    channel_id,
                                    target_agent,
                                    &mut session,
                                    health,
                                );
                            }

                            op => {
                                tracing::debug!("discord_bridge: unhandled opcode {op}");
                            }
                        }
                    }

                    Some(Ok(Message::Ping(data))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) => return Ok(session),
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

fn handle_dispatch(
    event_type: Option<&str>,
    data: Option<Value>,
    channel_id: &str,
    target_agent: &Option<String>,
    session: &mut Option<Session>,
    health: &Arc<Mutex<BridgeHealth>>,
) {
    match event_type {
        Some("READY") => {
            if let Some(data) = data {
                if let Ok(ready) = serde_json::from_value::<ReadyEvent>(data) {
                    let username = ready
                        .user
                        .as_ref()
                        .map(|u| u.username.as_str())
                        .unwrap_or("unknown");
                    tracing::info!(
                        "discord_bridge: READY as {}, session {}",
                        username,
                        ready.session_id
                    );
                    *session = Some(Session {
                        session_id: ready.session_id.clone(),
                        resume_url: ready.resume_gateway_url.clone(),
                        last_seq: 0,
                    });
                    let mut h = health.lock().unwrap();
                    h.status = BridgeStatus::Connected;
                    h.error = None;
                }
            }
        }

        Some("RESUMED") => {
            tracing::info!("discord_bridge: session RESUMED");
            let mut h = health.lock().unwrap();
            h.status = BridgeStatus::Connected;
            h.error = None;
        }

        Some("MESSAGE_CREATE") => {
            let data = match data {
                Some(d) => d,
                None => return,
            };
            let msg: MessageCreate = match serde_json::from_value(data) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("discord_bridge: parse MESSAGE_CREATE: {e}");
                    return;
                }
            };

            // Ignore bot messages (including our own bot)
            if msg.author.bot.unwrap_or(false) {
                return;
            }

            // Only process messages from the configured channel
            if msg.channel_id != channel_id {
                return;
            }

            {
                let mut h = health.lock().unwrap();
                h.last_event_at = Some(unix_secs());
            }

            let Some(target) = target_agent else {
                tracing::debug!(
                    "discord_bridge: message from {} (no target agent configured)",
                    msg.author.username
                );
                return;
            };

            let envelope = format!("[Discord @{}]: {}", msg.author.username, msg.content);
            let handler = get_global_handler();
            let req = InjectionRequest {
                target_agent: target.clone(),
                message: envelope,
                source_agent: Some("discord".to_string()),
                request_id: Some(msg.id.clone()),
                priority: None,
                wait_for_idle: false,
                jekt_tier: None,
                delivery_tier: Some("wan".to_string()),
                forward_hops: 0,
                ..Default::default()
            };
            let result = handler.inject_message(req);
            if result.success {
                tracing::debug!(
                    "discord_bridge: injected msg from {} to agent {}",
                    msg.author.username,
                    target
                );
            } else {
                tracing::warn!(
                    "discord_bridge: inject to agent {} failed: {:?}",
                    target,
                    result.error
                );
            }
        }

        Some(t) => {
            tracing::debug!("discord_bridge: unhandled event {t}");
        }
        None => {}
    }
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
