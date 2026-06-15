// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cloud push subscription — replaces per-agent polling with a single
//! sidecar-level WebSocket to muxbus.agentmux.ai.
//!
//! When MUXBUS_TOKEN is present, the subscriber:
//!   1. Opens wss://muxbus.agentmux.ai/ws with the bearer token.
//!   2. Sends { type: "subscribe", agents: [...all known agents...] }.
//!   3. Listens for { type: "inject", ... } pushes and delivers via ReactiveHandler.
//!   4. Sends { type: "ack", id } after delivery.
//!   5. Reconnects with exponential back-off on any disconnect.
//!
//! Agents register/unregister via add_agent()/remove_agent().
//! No MUXBUS_TOKEN → subscriber stays idle.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::Message};

use crate::backend::reactive::handler::get_global_handler;
use crate::backend::reactive::types::InjectionRequest;
use crate::backend::storage::store::Store;

const MUXBUS_WS_URL: &str = "wss://muxbus.agentmux.ai/ws";
const RECONNECT_DELAY_SECS: u64 = 5;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;
// Application-level keepalive. Without it, a silently half-open WS (NAT/firewall
// idle drop, server stall with no FIN/RST) leaves read.next() blocked forever:
// the subscriber looks connected but receives no pushes and never reconnects.
// We send a WS Ping every PING_INTERVAL and force a reconnect if no frame of any
// kind (data, ping, or pong) arrives within IDLE_TIMEOUT.
const PING_INTERVAL_SECS: u64 = 30;
const IDLE_TIMEOUT_SECS: u64 = 90;

// ── Wire protocol ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Subscribe { agents: Vec<String> },
    #[serde(rename = "subscribe:add")]
    SubscribeAdd { agents: Vec<String> },
    #[serde(rename = "subscribe:remove")]
    SubscribeRemove { agents: Vec<String> },
    Ack { id: String },
    Ping,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    Inject {
        id: String,
        target_agent: String,
        message: String,
        source_agent: Option<String>,
        priority: Option<String>,
    },
    Subscribed {
        #[allow(dead_code)]
        agents: Vec<String>,
    },
    Pong,
    Error {
        message: String,
    },
    #[serde(other)]
    Unknown,
}

// ── Control channel ───────────────────────────────────────────────────────────

enum CtrlMsg {
    AddAgent(String),
    RemoveAgent(String),
    ReloadToken,
}

// ── Public struct ─────────────────────────────────────────────────────────────

pub struct CloudSubscriber {
    ctrl_tx: mpsc::UnboundedSender<CtrlMsg>,
    /// In-memory copy of subscribed agents, kept in sync with the WS loop
    /// so add_agent/remove_agent can be called before the WS connects.
    agents: Arc<Mutex<HashSet<String>>>,
}

static GLOBAL_SUBSCRIBER: OnceLock<CloudSubscriber> = OnceLock::new();

pub fn get_global_subscriber() -> Option<&'static CloudSubscriber> {
    GLOBAL_SUBSCRIBER.get()
}

impl CloudSubscriber {
    /// Initialize the global subscriber and start the background WS loop.
    /// Should be called once at startup. No-op if already initialized.
    pub fn init_global(wstore: Arc<Store>) {
        let (ctrl_tx, ctrl_rx) = mpsc::unbounded_channel::<CtrlMsg>();
        let agents = Arc::new(Mutex::new(HashSet::<String>::new()));
        let subscriber = CloudSubscriber {
            ctrl_tx,
            agents: agents.clone(),
        };
        if GLOBAL_SUBSCRIBER.set(subscriber).is_err() {
            return; // already initialized
        }
        tokio::spawn(run_loop(wstore, agents, ctrl_rx));
    }

    /// Notify the WS loop that a new agent is registered locally.
    pub fn add_agent(&self, agent_id: &str) {
        self.agents.lock().unwrap().insert(agent_id.to_string());
        let _ = self.ctrl_tx.send(CtrlMsg::AddAgent(agent_id.to_string()));
    }

    /// Notify the WS loop that an agent has been unregistered.
    pub fn remove_agent(&self, agent_id: &str) {
        self.agents.lock().unwrap().remove(agent_id);
        let _ = self.ctrl_tx.send(CtrlMsg::RemoveAgent(agent_id.to_string()));
    }

    /// Called after muxbus.login completes — trigger a fresh WS connection
    /// with the newly stored token.
    pub fn reload_token(&self) {
        let _ = self.ctrl_tx.send(CtrlMsg::ReloadToken);
    }
}

// ── Background loop ───────────────────────────────────────────────────────────

async fn run_loop(
    wstore: Arc<Store>,
    agents: Arc<Mutex<HashSet<String>>>,
    mut ctrl_rx: mpsc::UnboundedReceiver<CtrlMsg>,
) {
    let http = reqwest::Client::new();
    let mut delay_secs = RECONNECT_DELAY_SECS;

    loop {
        // Load token — refresh if expired.
        // Two distinct None cases:
        //   a) No credentials in DB at all → wait for muxbus.login ReloadToken signal
        //   b) Credentials exist but refresh failed transiently → back off and retry
        let has_stored_creds = wstore.muxbus_load().ok().flatten().is_some();
        let token = match load_valid_token(&wstore, &http).await {
            // Do NOT reset back-off here: a valid token loads on every iteration,
            // so resetting on load would pin the delay at the base value and
            // defeat exponential back-off for the common "valid token, WS endpoint
            // unreachable" failure. Back-off is reset only after a *healthy*
            // connection — see the clean-disconnect and >30s-session branches below.
            Some(t) => t,
            None if !has_stored_creds => {
                // No credentials at all — wait for muxbus.login to signal us
                while let Some(msg) = ctrl_rx.recv().await {
                    if matches!(msg, CtrlMsg::ReloadToken) { break; }
                    // AddAgent / RemoveAgent already updated `agents` mutex
                }
                continue;
            }
            None => {
                // Credentials present but refresh failed transiently — back off and retry
                tracing::warn!(
                    "cloud_subscriber: token refresh failed, retrying in {}s",
                    delay_secs
                );
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                delay_secs = (delay_secs * 2).min(MAX_RECONNECT_DELAY_SECS);
                continue;
            }
        };

        tracing::info!("cloud_subscriber: connecting to {}", MUXBUS_WS_URL);
        let session_start = std::time::Instant::now();

        match connect_and_run(&token, agents.clone(), &mut ctrl_rx, &wstore, &http).await {
            Ok(()) => {
                // Clean disconnect — reset back-off
                delay_secs = RECONNECT_DELAY_SECS;
                tracing::info!("cloud_subscriber: disconnected cleanly");
            }
            Err(e) => {
                // If the session was healthy for >30s before the error, treat it as
                // network instability rather than a persistent failure and reset back-off.
                if session_start.elapsed().as_secs() > 30 {
                    delay_secs = RECONNECT_DELAY_SECS;
                }
                tracing::warn!("cloud_subscriber: error: {e}, reconnecting in {}s", delay_secs);
            }
        }

        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        delay_secs = (delay_secs * 2).min(MAX_RECONNECT_DELAY_SECS);
    }
}

async fn connect_and_run(
    token: &str,
    agents: Arc<Mutex<HashSet<String>>>,
    ctrl_rx: &mut mpsc::UnboundedReceiver<CtrlMsg>,
    wstore: &Arc<Store>,
    _http: &reqwest::Client,
) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::http::Request;

    // Only set Authorization — tungstenite adds Connection/Upgrade/Sec-WebSocket-*
    // automatically. Setting them manually causes duplicate headers in the handshake.
    let request = Request::builder()
        .uri(MUXBUS_WS_URL)
        .header("Authorization", format!("Bearer {}", token))
        .body(())
        .map_err(|e| format!("build request: {e}"))?;

    let (ws_stream, _) = connect_async_tls_with_config(request, None, false, None)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    tracing::info!("cloud_subscriber: WebSocket connected");
    let (mut write, mut read) = ws_stream.split();

    // Initial subscription for all currently-registered agents
    let initial_agents: Vec<String> = agents.lock().unwrap().iter().cloned().collect();
    let sub_msg = serde_json::to_string(&ClientMsg::Subscribe { agents: initial_agents })
        .map_err(|e| format!("serialize subscribe: {e}"))?;
    write.send(Message::Text(sub_msg.into()))
        .await
        .map_err(|e| format!("send subscribe: {e}"))?;

    let mut keepalive = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECS));
    // The first tick fires immediately; skip it so we don't ping before any idle.
    keepalive.tick().await;
    let mut last_activity = std::time::Instant::now();

    loop {
        tokio::select! {
            // Incoming WebSocket message from cloud
            msg = read.next() => {
                // Any frame — data, ping, or pong — proves the link is alive.
                last_activity = std::time::Instant::now();
                match msg {
                    None => return Ok(()), // server closed
                    Some(Err(e)) => return Err(format!("ws recv: {e}")),
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(server_msg) = serde_json::from_str::<ServerMsg>(&text) {
                            if let Err(e) = handle_server_msg(server_msg, &mut write, wstore).await {
                                tracing::warn!("cloud_subscriber: handle msg error: {e}");
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) => return Ok(()),
                    Some(Ok(_)) => {} // pong / binary frames etc — ignore (already counted as activity)
                }
            }

            // Keepalive tick: detect a half-open connection and ping to keep NAT open.
            _ = keepalive.tick() => {
                if last_activity.elapsed() >= Duration::from_secs(IDLE_TIMEOUT_SECS) {
                    return Err(format!(
                        "keepalive timeout: no frames for {}s", IDLE_TIMEOUT_SECS
                    ));
                }
                if let Err(e) = write.send(Message::Ping(Vec::<u8>::new().into())).await {
                    return Err(format!("keepalive ping failed: {e}"));
                }
            }

            // Control messages from register/unregister calls
            ctrl = ctrl_rx.recv() => {
                match ctrl {
                    None => return Ok(()), // channel closed
                    Some(CtrlMsg::AddAgent(id)) => {
                        let msg = serde_json::to_string(&ClientMsg::SubscribeAdd { agents: vec![id] })
                            .unwrap_or_default();
                        let _ = write.send(Message::Text(msg.into())).await;
                    }
                    Some(CtrlMsg::RemoveAgent(id)) => {
                        let msg = serde_json::to_string(&ClientMsg::SubscribeRemove { agents: vec![id] })
                            .unwrap_or_default();
                        let _ = write.send(Message::Text(msg.into())).await;
                    }
                    Some(CtrlMsg::ReloadToken) => {
                        // Close and let the outer loop reconnect with the new token
                        let _ = write.send(Message::Close(None)).await;
                        return Ok(());
                    }
                }
            }
        }
    }
}

async fn handle_server_msg(
    msg: ServerMsg,
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    _wstore: &Arc<Store>,
) -> Result<(), String> {
    match msg {
        ServerMsg::Inject { id, target_agent, message, source_agent, priority } => {
            let req = InjectionRequest {
                target_agent: target_agent.clone(),
                message,
                source_agent,
                request_id: Some(id.clone()),
                priority,
                wait_for_idle: false,
            };
            let resp = get_global_handler().inject_message(req);
            tracing::debug!(
                injection_id = %id,
                target_agent = %target_agent,
                success = resp.success,
                "cloud_subscriber: delivered injection"
            );
            // Always ACK — if delivery failed (agent not local), the cloud
            // should not retry since the sidecar confirmed it received the message.
            let ack = serde_json::to_string(&ClientMsg::Ack { id })
                .map_err(|e| format!("serialize ack: {e}"))?;
            write.send(Message::Text(ack.into()))
                .await
                .map_err(|e| format!("send ack: {e}"))?;
        }
        ServerMsg::Error { message } => {
            tracing::warn!("cloud_subscriber: server error: {message}");
        }
        ServerMsg::Pong | ServerMsg::Subscribed { .. } | ServerMsg::Unknown => {}
    }
    Ok(())
}

/// Load a valid (non-expired) access token, refreshing via refresh_token if needed.
/// Saves refreshed credentials back to the store. Returns None if no credentials
/// are stored, the token is expired and refresh fails, or the refresh_token is absent.
async fn load_valid_token(wstore: &Store, http: &reqwest::Client) -> Option<String> {
    let creds = match wstore.muxbus_load() {
        Ok(Some(c)) if !c.access_token.is_empty() => c,
        _ => return None,
    };
    // Gate on nearly_expired (300s buffer), not is_valid (exact expiry): a token
    // with only seconds of life would otherwise open a long-lived WS that the
    // cloud drops right after the handshake, causing reconnect churn. Refresh
    // proactively while there's still a comfortable margin.
    if !creds.nearly_expired() {
        return Some(creds.access_token);
    }
    // Token expired or within the refresh buffer — try refresh
    tracing::info!("cloud_subscriber: access token expired or near expiry, attempting refresh");
    match crate::muxbus::pkce::refresh_token(&creds, http).await {
        Ok(refreshed) => {
            if let Err(e) = wstore.muxbus_save(&refreshed) {
                tracing::warn!(error = %e, "cloud_subscriber: failed to save refreshed credentials");
            }
            Some(refreshed.access_token)
        }
        Err(e) => {
            tracing::warn!(error = %e, "cloud_subscriber: token refresh failed; user must re-login via muxbus.login");
            None
        }
    }
}

