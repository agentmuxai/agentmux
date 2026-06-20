// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cloud push subscription — replaces per-agent polling with a single
//! sidecar-level WebSocket to muxbus.agentmux.ai.
//!
//! When MUXBUS_TOKEN is present, the subscriber:
//!   1. Opens wss://muxbus.agentmux.ai/ws with the bearer token.
//!   2. Listens for { type: "inject_available" } broadcast wake signals (zero metadata).
//!   3. On each signal, polls REST GET /reactive/pending/:id for every locally-registered agent.
//!   4. Delivers via ReactiveHandler; ACKs successful deliveries via REST POST /reactive/ack.
//!   5. Reconnects with exponential back-off on any disconnect.
//!
//! The server broadcasts to ALL connected sidecars on every injection, so a subscriber
//! cannot correlate the signal to any particular agent, account, or timing.
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
const MUXBUS_REST_URL: &str = "https://muxbus.agentmux.ai";
const RECONNECT_DELAY_SECS: u64 = 5;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;

// ── Wire protocol ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Subscribe { agents: Vec<String> },
    #[serde(rename = "subscribe:add")]
    SubscribeAdd { agents: Vec<String> },
    #[serde(rename = "subscribe:remove")]
    SubscribeRemove { agents: Vec<String> },
    // Ack removed — ACK is sent via REST POST /reactive/ack, not WS
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    // Zero-metadata wake signal. The sidecar polls all its registered agents on receipt.
    // No agent_id, no injection_id — subscribers gain no information from this frame.
    InjectAvailable,
    Subscribed {
        #[allow(dead_code)]
        agents: Vec<String>,
    },
    Pong,
    Error {
        message: String,
    },
    Evicted {
        #[allow(dead_code)]
        agents: Vec<String>,
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
    /// Normalizes to lowercase and skips the WS send if already subscribed,
    /// so calling this on every COMMAND_AGENT_INPUT turn is safe.
    pub fn add_agent(&self, agent_id: &str) {
        let key = agent_id.to_lowercase();
        let mut agents = self.agents.lock().unwrap();
        if agents.contains(&key) {
            return;
        }
        agents.insert(key.clone());
        drop(agents);
        let _ = self.ctrl_tx.send(CtrlMsg::AddAgent(key));
    }

    /// Notify the WS loop that an agent has been unregistered.
    pub fn remove_agent(&self, agent_id: &str) {
        let key = agent_id.to_lowercase();
        self.agents.lock().unwrap().remove(&key);
        let _ = self.ctrl_tx.send(CtrlMsg::RemoveAgent(key));
    }

    /// Called after muxbus.login completes — trigger a fresh WS connection
    /// with the newly stored token.
    pub fn reload_token(&self) {
        let _ = self.ctrl_tx.send(CtrlMsg::ReloadToken);
    }

    /// Snapshot of the agents this sidecar has subscribed to the cloud relay
    /// (Tier-4), sorted for stable output. Empty when no MUXBUS_TOKEN is set
    /// (cloud disabled). Read-only; used by the discovery endpoint.
    pub fn subscribed_agents(&self) -> Vec<String> {
        let mut v: Vec<String> = self.agents.lock().unwrap().iter().cloned().collect();
        v.sort();
        v
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
        // has_stored_creds = true only when a retry is meaningful:
        //   - access_token present AND still valid (use it now), OR
        //   - access_token expired but refresh_token present (can refresh).
        // An expired token with no refresh_token is a permanent failure → park like no-creds.
        let has_stored_creds = wstore
            .muxbus_load()
            .ok()
            .flatten()
            .map(|c| !c.access_token.is_empty() && (c.is_valid() || !c.refresh_token.is_empty()))
            .unwrap_or(false);
        let token = match load_valid_token(&wstore, &http).await {
            Some(t) => t,
            None if !has_stored_creds => {
                // No credentials at all — wait for muxbus.login to signal us
                while let Some(msg) = ctrl_rx.recv().await {
                    if matches!(msg, CtrlMsg::ReloadToken) { break; }
                    // AddAgent / RemoveAgent already updated `agents` mutex
                }
                delay_secs = RECONNECT_DELAY_SECS; // reset back-off after fresh login
                continue;
            }
            None => {
                // Credentials present but refresh failed transiently — back off and retry.
                // Loop inside the select! so AddAgent/RemoveAgent messages drain without
                // cancelling the sleep; only ReloadToken (or sleep expiry) breaks out.
                tracing::warn!(
                    "cloud_subscriber: token refresh failed, retrying in {}s",
                    delay_secs
                );
                let sleep = tokio::time::sleep(Duration::from_secs(delay_secs));
                tokio::pin!(sleep);
                'backoff: loop {
                    tokio::select! {
                        _ = &mut sleep => break 'backoff,
                        Some(msg) = ctrl_rx.recv() => match msg {
                            CtrlMsg::ReloadToken => {
                                delay_secs = RECONNECT_DELAY_SECS;
                                break 'backoff;
                            }
                            CtrlMsg::AddAgent(_) | CtrlMsg::RemoveAgent(_) => {
                                // agents mutex already updated; continue sleeping
                            }
                        }
                    }
                }
                delay_secs = (delay_secs * 2).min(MAX_RECONNECT_DELAY_SECS);
                continue;
            }
        };

        tracing::info!("cloud_subscriber: connecting to {}", MUXBUS_WS_URL);
        let session_start = std::time::Instant::now();

        match connect_and_run(&token, agents.clone(), &mut ctrl_rx, &wstore, &http).await {
            Ok(()) => {
                // Apply the same >30s healthy-session guard as the error branch: a server
                // that immediately closes after handshake must not suppress back-off.
                if session_start.elapsed().as_secs() > 30 {
                    delay_secs = RECONNECT_DELAY_SECS;
                }
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

        // Post-disconnect back-off. Loop inside the select! so AddAgent/RemoveAgent
        // messages drain without cancelling the sleep; only ReloadToken breaks out early.
        let sleep = tokio::time::sleep(Duration::from_secs(delay_secs));
        tokio::pin!(sleep);
        'reconnect: loop {
            tokio::select! {
                _ = &mut sleep => break 'reconnect,
                Some(msg) = ctrl_rx.recv() => match msg {
                    CtrlMsg::ReloadToken => {
                        delay_secs = RECONNECT_DELAY_SECS;
                        break 'reconnect;
                    }
                    CtrlMsg::AddAgent(_) | CtrlMsg::RemoveAgent(_) => {
                        // agents mutex already updated; continue sleeping
                    }
                }
            }
        }
        delay_secs = (delay_secs * 2).min(MAX_RECONNECT_DELAY_SECS);
    }
}

async fn connect_and_run(
    token: &str,
    agents: Arc<Mutex<HashSet<String>>>,
    ctrl_rx: &mut mpsc::UnboundedReceiver<CtrlMsg>,
    _wstore: &Arc<Store>,
    http: &reqwest::Client,
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

    loop {
        tokio::select! {
            // Incoming WebSocket message from cloud
            msg = read.next() => {
                match msg {
                    None => return Ok(()), // server closed
                    Some(Err(e)) => return Err(format!("ws recv: {e}")),
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(server_msg) = serde_json::from_str::<ServerMsg>(&text) {
                            match handle_server_msg(server_msg, token, http, &agents).await {
                                Ok(()) => {}
                                // Eviction or expired token — close stream to trigger reconnect.
                                Err(ref e) if e.starts_with("reconnect:") => {
                                    tracing::info!("cloud_subscriber: {e}, reconnecting");
                                    return Ok(());
                                }
                                Err(e) => {
                                    tracing::warn!("cloud_subscriber: handle msg error: {e}");
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) => return Ok(()),
                    Some(Ok(_)) => {} // binary frames etc — ignore
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

/// Handle a message from the cloud server.
/// The WS wake signal carries zero metadata — the sidecar polls all its registered
/// agents via REST to find pending injections, so a compromised subscriber gains nothing.
async fn handle_server_msg(
    msg: ServerMsg,
    token: &str,
    http: &reqwest::Client,
    agents: &Arc<Mutex<HashSet<String>>>,
) -> Result<(), String> {
    match msg {
        ServerMsg::InjectAvailable => {
            // Collect currently-registered agents without holding the lock across awaits.
            let registered: Vec<String> = agents.lock().unwrap().iter().cloned().collect();

            #[derive(Deserialize)]
            struct PendingResp { injections: Vec<PendingInj> }
            #[derive(Deserialize)]
            struct PendingInj {
                id: String,
                source_agent: Option<String>,
                message: String,
                priority: Option<String>,
            }

            let handler = get_global_handler();
            for agent_id in &registered {
                let url = format!("{}/reactive/pending/{}", MUXBUS_REST_URL, agent_id);
                let resp = match http
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .header("X-Agent-ID", agent_id)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(agent_id = %agent_id, error = %e, "cloud_subscriber: fetch pending failed");
                        continue;
                    }
                };

                if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                    // Token expired during session — reconnect to refresh.
                    return Err("reconnect:token_expired".to_string());
                }
                if !resp.status().is_success() {
                    tracing::warn!(
                        status = %resp.status(),
                        agent_id = %agent_id,
                        "cloud_subscriber: fetch pending non-2xx"
                    );
                    continue;
                }

                let body: PendingResp = match resp.json().await {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(error = %e, "cloud_subscriber: parse pending failed");
                        continue;
                    }
                };

                let mut ack_ids: Vec<String> = Vec::new();
                for inj in &body.injections {
                    let req = InjectionRequest {
                        target_agent: agent_id.clone(),
                        message: inj.message.clone(),
                        source_agent: inj.source_agent.clone(),
                        request_id: Some(inj.id.clone()),
                        priority: inj.priority.clone(),
                        wait_for_idle: false,
                    };
                    let delivery = handler.inject_message(req);
                    tracing::debug!(
                        injection_id = %inj.id,
                        agent_id = %agent_id,
                        success = delivery.success,
                        "cloud_subscriber: delivered injection"
                    );
                    // Only ACK successfully delivered injections — failed deliveries stay
                    // in the queue so the cloud can retry (e.g. rate-limited or agent not ready).
                    if delivery.success {
                        ack_ids.push(inj.id.clone());
                    }
                }

                if !ack_ids.is_empty() {
                    let ack_url = format!("{}/reactive/ack", MUXBUS_REST_URL);
                    let _ = http
                        .post(&ack_url)
                        .header("Authorization", format!("Bearer {}", token))
                        .header("X-Agent-ID", agent_id)
                        .json(&serde_json::json!({ "injection_ids": ack_ids }))
                        .send()
                        .await;
                }
            }
        }
        ServerMsg::Error { message } => {
            tracing::warn!("cloud_subscriber: server error: {message}");
        }
        ServerMsg::Evicted { .. } => {
            // Another sidecar evicted us. Reconnect to re-subscribe (reclaim).
            return Err("reconnect:evicted".to_string());
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
    if creds.is_valid() && !creds.nearly_expired() {
        return Some(creds.access_token);
    }
    // Token expired or expiring within 300s — proactively refresh
    tracing::info!("cloud_subscriber: access token expired, attempting refresh");
    match crate::muxbus::pkce::refresh_token(&creds, http).await {
        Ok(refreshed) => {
            if let Err(e) = wstore.muxbus_save(&refreshed) {
                tracing::warn!(error = %e, "cloud_subscriber: failed to save refreshed credentials");
            }
            Some(refreshed.access_token)
        }
        Err(e) => {
            tracing::warn!(error = %e, "cloud_subscriber: proactive refresh failed");
            // Fall back to the existing token if it's still within its validity window.
            // A transient refresh failure should not prevent connecting with a working token.
            if creds.is_valid() {
                Some(creds.access_token)
            } else {
                None
            }
        }
    }
}

