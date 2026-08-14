// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cloud push subscription — replaces per-agent polling with a single
//! sidecar-level WebSocket to muxbus-ws.agentmux.ai.
//!
//! When MUXBUS_TOKEN is present, the subscriber:
//!   1. Opens wss://muxbus-ws.agentmux.ai with the bearer token.
//!   2. Listens for { type: "inject_available" } broadcast wake signals (zero metadata).
//!   3. On each signal, polls REST GET /reactive/pending/:id for every locally-registered agent.
//!   4. Claims all pending injections via REST POST /reactive/ack (atomic
//!      pending->delivered transition server-side) *before* delivering — only
//!      ids this call actually won the claim on get delivered via
//!      ReactiveHandler. A claimed injection that fails local delivery is
//!      released back to pending via REST POST /reactive/release so it's
//!      retried, rather than silently dropped.
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
use crate::broker::RefreshErrorKind;

// Dedicated custom domain on the API Gateway WebSocket API (apigatewayv2
// DomainName + ApiMapping), not a path under muxbus.agentmux.ai's CloudFront
// distribution. A CloudFront path behavior would have forwarded this
// client's literal /ws request path prefixed by originPath, landing at
// /{stage}/ws on the origin — but the WS handshake endpoint only exists at
// exactly /{stage}. No path suffix here: the domain root maps directly to
// the API's default stage. Full design/history in the agentmux-cloud repo's
// muxbus/ directory (search for the WebSocket relay redesign writeup).
const MUXBUS_WS_URL: &str = "wss://muxbus-ws.agentmux.ai";
pub(crate) const MUXBUS_REST_URL: &str = "https://muxbus.agentmux.ai";
const RECONNECT_DELAY_SECS: u64 = 5;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;
// AWS API Gateway WebSocket APIs enforce a 10-minute idle timeout with no
// server-initiated keepalive of their own — the connection is dropped on
// silence in both directions. Ping well under that so a quiet connection
// (no inject_available traffic) survives indefinitely. The ping is an
// app-level ClientMsg::Ping frame, not a WS-protocol-level Message::Ping —
// API Gateway does not reliably relay raw protocol ping/pong control frames,
// so a normal data frame is what actually keeps the connection alive in
// production (see ClientMsg::Ping's doc comment).
const CLIENT_PING_INTERVAL_SECS: u64 = 240;
// How often the broker's background sweep re-checks the stored MuxBus
// credential and proactively refreshes it if it's nearing expiry — now the
// SOLE proactive-refresh trigger (see `run_loop`'s registration call and the
// broker module doc); replaces what used to be two independent, uncoordinated
// checks (the outer reconnect loop's `load_valid_token`, and an inline check
// on every WS ping tick). Cheap (one DB read) when not actually stale, so a
// short interval is fine.
const BROKER_SWEEP_INTERVAL_SECS: u64 = 60;

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
    // App-level keepalive — see CLIENT_PING_INTERVAL_SECS. AWS API Gateway
    // WebSocket APIs (the production transport) do not reliably relay raw
    // WS-protocol Ping/Pong control frames, so the keepalive must be a normal
    // data frame the server's $default route can see and reply to.
    Ping,
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

    // Register this credential with the global broker scheduler once, up
    // front. The broker's own background sweep (spawned inside
    // `crate::broker::init_global`) now owns proactive refresh entirely —
    // it keeps the stored token fresh even while this WS session is
    // disconnected/backing off, which the old ping-tick-only check could not.
    let scheduler = crate::broker::init_global(Duration::from_secs(BROKER_SWEEP_INTERVAL_SECS));
    {
        let wstore_fresh = wstore.clone();
        let wstore_refresh = wstore.clone();
        let http_refresh = http.clone();
        scheduler
            .register(
                crate::muxbus::CREDENTIAL_ID,
                // muxbus_is_fresh, not muxbus_load — reagent P2 on #2260:
                // register()'s own contract requires is_fresh to be
                // side-effect-free (called under the per-id lock on every
                // ensure_fresh/sweep tick), but muxbus_load's lazy-migration
                // branch can perform a keychain write + SQL update for a
                // legacy row. muxbus_is_fresh shares the same read logic
                // with that branch disabled.
                //
                // spawn_blocking, not a direct call — reagent P1 on #2260:
                // muxbus_is_fresh does a synchronous OS-keychain read, which
                // can hang on a slow/unresponsive Secret Service D-Bus
                // daemon (a scenario headless Linux specifically has to
                // handle). Without this, that hang stalls the tokio worker
                // thread `run_sweep_loop` runs on, and via the scheduler's
                // held per-id lock, can block OTHER credentials' refreshes
                // too — the exact convention this codebase already applies
                // to keychain reads elsewhere (see
                // app_api/mod.rs's account_validate_impl).
                move || {
                    let wstore = wstore_fresh.clone();
                    Box::pin(async move {
                        tokio::task::spawn_blocking(move || wstore.muxbus_is_fresh())
                            .await
                            .unwrap_or(false)
                    })
                },
                move || {
                    let wstore = wstore_refresh.clone();
                    let http = http_refresh.clone();
                    Box::pin(async move {
                        let load_store = wstore.clone();
                        let current = tokio::task::spawn_blocking(move || load_store.muxbus_load())
                            .await
                            .map_err(|e| RefreshErrorKind::Transient(format!("muxbus_load task: {e}")))?
                            .map_err(|e| RefreshErrorKind::Transient(e.to_string()))?
                            // Nothing stored at all — not a transient storage
                            // blip, there's genuinely no credential to refresh;
                            // only a fresh login produces one.
                            .ok_or_else(|| {
                                RefreshErrorKind::PermanentAuthFailure(
                                    "no muxbus credentials stored".to_string(),
                                )
                            })?;
                        if current.refresh_token.is_empty() {
                            // Same reasoning: no refresh_token will ever
                            // appear on its own — only re-login fixes it.
                            return Err(RefreshErrorKind::PermanentAuthFailure(
                                "no refresh_token stored".to_string(),
                            ));
                        }
                        // Preserve-on-failure: `refresh_token` returning Err
                        // here means `muxbus_save` is simply never called —
                        // the last-known-good credential in the store is
                        // left untouched rather than overwritten with a
                        // failed/partial result.
                        let refreshed = crate::muxbus::pkce::refresh_token(&current, &http)
                            .await
                            .map_err(classify_refresh_token_error)?;
                        let save_store = wstore.clone();
                        tokio::task::spawn_blocking(move || save_store.muxbus_save(&refreshed))
                            .await
                            .map_err(|e| RefreshErrorKind::Transient(format!("muxbus_save task: {e}")))?
                            .map_err(|e| RefreshErrorKind::Transient(e.to_string()))
                    })
                },
            )
            .await;
    }

    loop {
        // Load token — refresh if expired.
        // Two distinct None cases:
        //   a) No credentials in DB at all → wait for muxbus.login ReloadToken signal
        //   b) Credentials exist but refresh failed transiently → back off and retry
        // has_stored_creds = true only when a retry is meaningful:
        //   - access_token present AND still valid (use it now), OR
        //   - access_token expired but refresh_token present (can refresh).
        // An expired token with no refresh_token is a permanent failure → park like no-creds.
        //
        // A real load ERROR (keychain locked/unavailable, distinct from
        // muxbus_load's Ok(None) "genuinely nothing stored" — see that
        // function's own reagent-fixed error handling) must NOT collapse to
        // has_stored_creds=false: that would park indefinitely waiting for a
        // muxbus.login the user was never actually missing, instead of
        // backing off and retrying once the transient failure clears.
        // spawn_blocking — reagent P1 on #2260: muxbus_load does a
        // synchronous OS-keychain read; a hang there (slow/unresponsive
        // Secret Service D-Bus daemon) must not stall this loop's tokio
        // worker thread.
        let has_stored_creds_load = {
            let wstore = wstore.clone();
            tokio::task::spawn_blocking(move || wstore.muxbus_load()).await
        };
        let has_stored_creds = match has_stored_creds_load {
            Ok(Ok(Some(c))) => !c.access_token.is_empty() && (c.is_valid() || !c.refresh_token.is_empty()),
            Ok(Ok(None)) => false,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "cloud_subscriber: muxbus_load failed — assuming credentials exist and retrying with backoff");
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "cloud_subscriber: muxbus_load task panicked — assuming credentials exist and retrying with backoff");
                true
            }
        };
        let token = match load_valid_token(&wstore, &scheduler).await {
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
    wstore: &Arc<Store>,
    http: &reqwest::Client,
) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::ClientRequestBuilder;

    // Issue #2091: a hand-built `http::Request` (as this used to be, via
    // `Request::builder()...body(())`) is NOT auto-completed by tungstenite.
    // `IntoClientRequest for Request` is a bare passthrough (`Ok(self)`) —
    // the Host/Connection/Upgrade/Sec-WebSocket-Version/Sec-WebSocket-Key
    // headers this comment used to claim were "added automatically" are only
    // generated when tungstenite builds the request itself from a bare
    // `Uri`/`&str`/`String` (`impl IntoClientRequest for Uri`, which calls
    // `generate_key()`). A hand-built Request with only `Authorization` set
    // therefore had no `Sec-WebSocket-Key` at all, and tungstenite's own
    // `generate_request()` requires it to already be present — every
    // connection attempt failed handshake validation on its OWN outgoing
    // request before a single byte reached the network, logged as "Missing,
    // duplicated or incorrect header sec-websocket-key". This has been
    // broken since the file's first commit; reproduced in isolation
    // (`examples/ws_probe.rs`, removed after confirming the fix) and
    // confirmed fixed by switching to `ClientRequestBuilder` — tungstenite's
    // own purpose-built API for "URI + extra headers": it builds the request
    // from the `Uri` (generating the required headers, including a fresh
    // `Sec-WebSocket-Key`) and layers `with_header` calls on top, so nothing
    // required is ever missing.
    let uri: tokio_tungstenite::tungstenite::http::Uri =
        MUXBUS_WS_URL.parse().map_err(|e| format!("parse url: {e}"))?;
    let request = ClientRequestBuilder::new(uri)
        .with_header("Authorization", format!("Bearer {}", token));

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

    let mut ping_interval = tokio::time::interval(Duration::from_secs(CLIENT_PING_INTERVAL_SECS));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping_interval.tick().await; // first tick fires immediately — consume it, we just connected

    loop {
        tokio::select! {
            // Keepalive — see CLIENT_PING_INTERVAL_SECS. Proactive credential
            // freshness is no longer piggybacked on this tick: the broker's
            // own background sweep (registered once in `run_loop`, see
            // `crate::broker`) now keeps `db_muxbus_credentials` fresh on its
            // own schedule regardless of whether a WS session happens to be
            // open, so a long-lived session no longer needs to special-case
            // its own credential check here.
            _ = ping_interval.tick() => {
                let ping_msg = serde_json::to_string(&ClientMsg::Ping)
                    .map_err(|e| format!("serialize ping: {e}"))?;
                if let Err(e) = write.send(Message::Text(ping_msg.into())).await {
                    return Err(format!("send ping: {e}"));
                }
            }

            // Incoming WebSocket message from cloud
            msg = read.next() => {
                match msg {
                    None => return Ok(()), // server closed
                    Some(Err(e)) => return Err(format!("ws recv: {e}")),
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(server_msg) = serde_json::from_str::<ServerMsg>(&text) {
                            match handle_server_msg(server_msg, token, http, &agents, wstore).await {
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
    wstore: &Arc<Store>,
) -> Result<(), String> {
    match msg {
        ServerMsg::InjectAvailable => {
            // Collect currently-registered agents without holding the lock across awaits.
            let registered: Vec<String> = agents.lock().unwrap().iter().cloned().collect();
            let handler = get_global_handler();

            // Concurrent, not sequential: each agent's sync can incur up to
            // two CREDENTIAL_HTTP_TIMEOUT-bounded HTTP calls (credential
            // fetch + pending fetch, worst case an unrevoked-then-retried
            // claim too) — sequentially awaiting N agents compounds that
            // into an N-times-longer block of this whole select! loop
            // (pings, incoming messages, ctrl signals all wait on it).
            // join_all bounds the wall-clock cost to the slowest single
            // agent's chain regardless of N. reagentx P1 (round 4) on
            // PR #2342.
            let outcomes = futures_util::future::join_all(
                registered
                    .iter()
                    .map(|agent_id| sync_agent_reactive(agent_id, token, http, wstore, handler)),
            )
            .await;

            if outcomes
                .iter()
                .any(|o| matches!(o, AgentSyncOutcome::ReconnectSharedTokenExpired))
            {
                // Shared account-level token expired during session — reconnect to refresh.
                return Err("reconnect:token_expired".to_string());
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

enum AgentSyncOutcome {
    /// This agent's sync finished (successfully or with an already-logged,
    /// non-fatal error) — nothing further to signal to the caller.
    Ok,
    /// The SHARED account-level token (not a per-agent credential) was
    /// rejected — the whole WS session's auth is stale and needs a
    /// reconnect. Only ever produced when no per-agent credential was in
    /// play for the rejected call.
    ReconnectSharedTokenExpired,
}

/// One registered agent's full pending-fetch → claim → deliver cycle for an
/// `InjectAvailable` broadcast. Extracted from the loop body it used to be
/// so `handle_server_msg` can run every agent's sync concurrently via
/// `join_all` instead of sequentially — see the call site's own comment.
///
/// Retries once with the shared `token` if a per-agent credential is
/// rejected (401) — without this, invalidating the credential alone left
/// this agent's pending injection undelivered until an unrelated
/// InjectAvailable broadcast happened to fire again (no periodic resync
/// exists). reagentx P1 (round 4) on PR #2342.
async fn sync_agent_reactive(
    agent_id: &str,
    token: &str,
    http: &reqwest::Client,
    wstore: &Arc<Store>,
    handler: &'static crate::backend::reactive::handler::ReactiveHandler,
) -> AgentSyncOutcome {
    #[derive(Deserialize)]
    struct PendingResp { injections: Vec<PendingInj> }
    #[derive(Deserialize)]
    struct PendingInj {
        id: String,
        source_agent: Option<String>,
        message: String,
        priority: Option<String>,
        // Present only for messages signed by an AgentMux-operated WAN
        // sender (currently just the GitHub review-notification consumer,
        // "reagent") — see agentmux_common::jekt_sign::verify_reagent_jekt.
        // All four travel together; verification is attempted only when
        // every one of them deserializes present (see below).
        #[serde(default)]
        reagent_sig: Option<String>,
        #[serde(default)]
        reagent_key_id: Option<String>,
        #[serde(default)]
        reagent_msg_id: Option<String>,
        #[serde(default)]
        reagent_ts_secs: Option<i64>,
    }
    #[derive(Deserialize)]
    struct AckResp {
        acknowledged: Vec<String>,
        delivered_at: String,
    }

    // Prefer a credential bound to exactly this agent_id over the shared
    // account-level token — see agent_credentials.rs. Falls back to `token`
    // (today's self-declared behavior) whenever this agent isn't
    // provisioned yet or provisioning fails, so rollout never blocks
    // delivery. `using_per_agent` tracks which one is currently in play so
    // a 401 can be attributed correctly and, for a per-agent credential,
    // retried once with the shared token instead of just giving up.
    let per_agent_token = crate::muxbus::agent_credentials::ensure_agent_credential(agent_id, wstore, http).await;
    let mut agent_token = per_agent_token.clone().unwrap_or_else(|| token.to_string());
    let mut using_per_agent = per_agent_token.is_some();

    let url = format!("{}/reactive/pending/{}", MUXBUS_REST_URL, agent_id);
    let body: PendingResp = loop {
        let resp = match http
            .get(&url)
            .header("Authorization", format!("Bearer {}", agent_token))
            .header("X-Agent-ID", agent_id)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = %e, "cloud_subscriber: fetch pending failed");
                return AgentSyncOutcome::Ok;
            }
        };

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            if using_per_agent {
                // The per-agent credential's cached token is stale/revoked
                // server-side even though it looked locally valid —
                // invalidate it (so the next agent_credentials call
                // re-fetches instead of retrying the same rejected token)
                // and retry THIS fetch immediately with the shared token,
                // rather than tearing down the whole shared session (which
                // would starve delivery for every OTHER agent too).
                crate::muxbus::agent_credentials::invalidate_cached_token(agent_id, wstore);
                tracing::warn!(
                    agent_id = %agent_id,
                    "cloud_subscriber: per-agent credential rejected (401) — invalidated, retrying with shared token",
                );
                agent_token = token.to_string();
                using_per_agent = false;
                continue;
            }
            return AgentSyncOutcome::ReconnectSharedTokenExpired;
        }
        if !resp.status().is_success() {
            tracing::warn!(
                status = %resp.status(),
                agent_id = %agent_id,
                "cloud_subscriber: fetch pending non-2xx"
            );
            return AgentSyncOutcome::Ok;
        }

        break match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "cloud_subscriber: parse pending failed");
                return AgentSyncOutcome::Ok;
            }
        };
    };

    if body.injections.is_empty() {
        return AgentSyncOutcome::Ok;
    }

    // Claim BEFORE delivering, not after: this is what prevents
    // double-delivery when the same agent_id is concurrently registered by
    // another AgentMux channel on this host (an intentionally-supported
    // "two seats" workflow — see
    // docs/specs/SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md).
    // /reactive/ack now performs an atomic pending->delivered transition
    // server-side; only injection ids that come back in `acknowledged` were
    // actually won by this call and get delivered locally below. A
    // previous version of this code delivered first and only acked
    // successes afterward, which let two concurrent pollers both deliver
    // the same injection.
    let all_ids: Vec<String> = body.injections.iter().map(|inj| inj.id.clone()).collect();
    let ack_url = format!("{}/reactive/ack", MUXBUS_REST_URL);
    let claimed: AckResp = loop {
        let claim_resp = match http
            .post(&ack_url)
            .header("Authorization", format!("Bearer {}", agent_token))
            .header("X-Agent-ID", agent_id)
            .json(&serde_json::json!({ "injection_ids": all_ids }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = %e, "cloud_subscriber: claim request failed");
                return AgentSyncOutcome::Ok; // nothing claimed — retried on the next wake/poll
            }
        };

        if claim_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            if using_per_agent {
                // Same reasoning as the /reactive/pending 401 branch above:
                // invalidate and retry THIS claim immediately with the
                // shared token, rather than reconnecting the whole session
                // or leaving this agent's already-fetched pending
                // injections unclaimed until an unrelated broadcast fires.
                crate::muxbus::agent_credentials::invalidate_cached_token(agent_id, wstore);
                tracing::warn!(
                    agent_id = %agent_id,
                    "cloud_subscriber: per-agent credential rejected (401) on claim — invalidated, retrying with shared token",
                );
                agent_token = token.to_string();
                using_per_agent = false;
                continue;
            }
            return AgentSyncOutcome::ReconnectSharedTokenExpired;
        }
        if !claim_resp.status().is_success() {
            tracing::warn!(
                status = %claim_resp.status(),
                agent_id = %agent_id,
                "cloud_subscriber: claim request non-2xx"
            );
            return AgentSyncOutcome::Ok; // nothing claimed — retried on the next wake/poll
        }

        break match claim_resp.json().await {
            Ok(b) => b,
            Err(e) => {
                // The server processed the claim (status was 2xx) before this
                // body failed to parse — some subset of `all_ids` may now be
                // flipped to "delivered" server-side with no local delivery
                // and no way for us to release them, since we don't know
                // which ids succeeded or their delivered_at stamp (required
                // by /reactive/release). We deliberately do NOT guess (e.g.
                // via /reactive/status) and release whatever looks delivered:
                // some of `all_ids` may have been legitimately claimed and
                // delivered by a *different* concurrent poller (another
                // channel/seat racing for the same agent_id), and blindly
                // releasing those would reintroduce the exact duplicate-
                // delivery bug this change exists to fix. Logging every
                // affected id loudly is the safe tradeoff: rare silent
                // message loss here, never a resurrected duplicate.
                tracing::error!(
                    agent_id = %agent_id,
                    injection_ids = ?all_ids,
                    error = %e,
                    "cloud_subscriber: parse claim response failed — these injections may be claimed server-side with no local delivery and cannot be safely auto-recovered"
                );
                return AgentSyncOutcome::Ok;
            }
        };
    };

    let delivered_at = claimed.delivered_at;
    let claimed_ids: std::collections::HashSet<String> = claimed.acknowledged.into_iter().collect();

    for inj in &body.injections {
        if !claimed_ids.contains(&inj.id) {
            // Another concurrent poller (this account's other channel/seat)
            // already won this claim — expected under the "two seats"
            // workflow, not an error.
            continue;
        }

        // Verify a reagent-signed WAN message (see PendingInj's doc comment)
        // before delivery. Only attempted when every one of the four
        // signing fields is present — a partial set (e.g. a sig but no
        // key_id) is treated the same as "not signed," not "signed but
        // broken," since a legitimate sender always sends all four
        // together. Never affects escalation (WAN stays unconditionally
        // TRUST=network-claimed / sensitive-eligible) — only which SIG=
        // marker field renders. See InjectionRequest::reagent_verified.
        let reagent_verified = match (&inj.reagent_sig, &inj.reagent_key_id, &inj.reagent_msg_id, inj.reagent_ts_secs) {
            (Some(sig), Some(key_id), Some(msg_id), Some(ts_secs)) => Some(
                agentmux_common::jekt_sign::verify_reagent_jekt(
                    key_id,
                    msg_id,
                    inj.source_agent.as_deref().unwrap_or(""),
                    agent_id,
                    ts_secs,
                    &inj.message,
                    sig,
                ),
            ),
            _ => None,
        };

        let req = InjectionRequest {
            target_agent: agent_id.to_string(),
            message: inj.message.clone(),
            source_agent: inj.source_agent.clone(),
            request_id: Some(inj.id.clone()),
            priority: inj.priority.clone(),
            wait_for_idle: false,
            jekt_tier: None,       // auto-detected from keywords
            delivery_tier: Some("wan".to_string()),
            forward_hops: 0,
            reagent_verified,
            ..Default::default()
        };
        let delivery = handler.inject_message(req);
        tracing::debug!(
            injection_id = %inj.id,
            agent_id = %agent_id,
            success = delivery.success,
            "cloud_subscriber: delivered injection"
        );

        if !delivery.success {
            // We hold the claim but local delivery failed (e.g. agent not
            // ready) — release it back to pending so it's retried, instead
            // of silently dropping it now that claiming already marked it
            // "delivered".
            let release_url = format!("{}/reactive/release", MUXBUS_REST_URL);
            match http
                .post(&release_url)
                .header("Authorization", format!("Bearer {}", agent_token))
                .header("X-Agent-ID", agent_id)
                .json(&serde_json::json!({
                    "injection_id": inj.id,
                    "delivered_at": delivered_at,
                }))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    // Claim is stranded as "delivered" with no local
                    // delivery and no retry until this is visible — log
                    // loudly rather than discarding silently.
                    tracing::warn!(
                        injection_id = %inj.id,
                        agent_id = %agent_id,
                        status = %r.status(),
                        "cloud_subscriber: release request non-2xx — injection stranded as delivered, message lost"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        injection_id = %inj.id,
                        agent_id = %agent_id,
                        error = %e,
                        "cloud_subscriber: release request failed — injection stranded as delivered, message lost"
                    );
                }
            }
        }
    }

    AgentSyncOutcome::Ok
}

/// Load a valid (non-expired) access token via the broker, refreshing first
/// if the stored credential is missing/stale. Returns None if no credentials
/// are stored, the token is expired and refresh fails, or the refresh_token
/// is absent — `ensure_fresh`'s own single-flight guard means this can run
/// concurrently with the broker's background sweep for the same credential
/// without either one duplicating the other's refresh attempt.
///
/// pub(crate): also used by agent_credentials.rs to authenticate the
/// POST /agents/provision call, which requires the human's own user-level
/// (PKCE) token, not an agent-bound M2M one.
pub(crate) async fn load_valid_token(
    wstore: &Arc<Store>,
    scheduler: &crate::broker::RefreshScheduler,
) -> Option<String> {
    if let Err(e) = scheduler.ensure_fresh(crate::muxbus::CREDENTIAL_ID).await {
        tracing::warn!(error = %e, "cloud_subscriber: token refresh failed");
    }
    // Preserve-on-failure means a failed refresh leaves the last-known-good
    // credential in place — re-read regardless of ensure_fresh's outcome and
    // fall back to it if it's still within its validity window, matching the
    // pre-broker behavior: a transient refresh failure should not prevent
    // connecting with a token that's still technically valid.
    //
    // spawn_blocking — reagent P1 on #2260: same synchronous-keychain-read
    // concern as every other muxbus_load call site in this file.
    let load_store = wstore.clone();
    let creds = tokio::task::spawn_blocking(move || load_store.muxbus_load())
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten();
    match creds {
        Some(c) if c.is_valid() => Some(c.access_token),
        _ => None,
    }
}

/// Classify a `pkce::refresh_token` failure for the broker's
/// `RefreshErrorKind` — `NoRefreshToken`/`Rejected` in the 4xx range mean
/// the credential itself is the problem (only a fresh `muxbus.login` fixes
/// it) *except* 408/429, which mean the request itself was throttled or
/// timed out, not that the refresh_token was rejected — treating those as
/// permanent would strand the credential in `NeedsReauth` past a temporary
/// rate limit (reagent P2 on #2275). Everything else (network blips, 5xx,
/// response parse failures) is worth retrying on the next sweep tick.
fn classify_refresh_token_error(e: crate::muxbus::pkce::RefreshTokenError) -> RefreshErrorKind {
    use crate::muxbus::pkce::RefreshTokenError;
    match &e {
        RefreshTokenError::NoRefreshToken => RefreshErrorKind::PermanentAuthFailure(e.to_string()),
        RefreshTokenError::Rejected { status, .. }
            if (400..500).contains(status) && !matches!(status, 408 | 429) =>
        {
            RefreshErrorKind::PermanentAuthFailure(e.to_string())
        }
        RefreshTokenError::Rejected { .. }
        | RefreshTokenError::Network(_)
        | RefreshTokenError::ParseFailed(_) => RefreshErrorKind::Transient(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::classify_refresh_token_error;
    use crate::broker::RefreshErrorKind;
    use crate::muxbus::pkce::RefreshTokenError;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::ClientRequestBuilder;

    #[test]
    fn no_refresh_token_is_permanent() {
        assert!(matches!(
            classify_refresh_token_error(RefreshTokenError::NoRefreshToken),
            RefreshErrorKind::PermanentAuthFailure(_)
        ));
    }

    #[test]
    fn a_4xx_rejection_is_permanent() {
        assert!(matches!(
            classify_refresh_token_error(RefreshTokenError::Rejected {
                status: 400,
                body: "invalid_grant".into(),
            }),
            RefreshErrorKind::PermanentAuthFailure(_)
        ));
    }

    #[test]
    fn a_5xx_rejection_is_transient() {
        assert!(matches!(
            classify_refresh_token_error(RefreshTokenError::Rejected {
                status: 503,
                body: "unavailable".into(),
            }),
            RefreshErrorKind::Transient(_)
        ));
    }

    #[test]
    fn rate_limit_and_timeout_rejections_are_transient_not_permanent() {
        // A 429/408 means the request was throttled/timed out, not that the
        // refresh_token itself was rejected — must not strand the
        // credential in NeedsReauth past a temporary rate limit.
        assert!(matches!(
            classify_refresh_token_error(RefreshTokenError::Rejected {
                status: 429,
                body: "rate limited".into(),
            }),
            RefreshErrorKind::Transient(_)
        ));
        assert!(matches!(
            classify_refresh_token_error(RefreshTokenError::Rejected {
                status: 408,
                body: "request timeout".into(),
            }),
            RefreshErrorKind::Transient(_)
        ));
    }

    #[test]
    fn network_and_parse_errors_are_transient() {
        assert!(matches!(
            classify_refresh_token_error(RefreshTokenError::Network("timeout".into())),
            RefreshErrorKind::Transient(_)
        ));
        assert!(matches!(
            classify_refresh_token_error(RefreshTokenError::ParseFailed("bad json".into())),
            RefreshErrorKind::Transient(_)
        ));
    }

    /// Regression for issue #2091: every connection attempt failed the
    /// WebSocket handshake with "Missing, duplicated or incorrect header
    /// sec-websocket-key" from the moment this file was first written,
    /// because the old code hand-built an `http::Request` with only
    /// `Authorization` set — tungstenite's `IntoClientRequest for Request`
    /// is a bare passthrough, so none of Host/Connection/Upgrade/
    /// Sec-WebSocket-Version/Sec-WebSocket-Key were ever added, and
    /// tungstenite's handshake code requires all five to already be present
    /// on the outgoing request. This test builds the request the exact way
    /// `connect_and_run` now does and asserts every required header exists
    /// — it would have failed against the old `Request::builder()` code and
    /// passes against `ClientRequestBuilder`. No network access needed: this
    /// only inspects the request tungstenite would send, before any I/O.
    #[test]
    fn ws_request_carries_every_header_the_handshake_requires() {
        let uri: tokio_tungstenite::tungstenite::http::Uri =
            "wss://muxbus-ws.agentmux.ai".parse().unwrap();
        let request = ClientRequestBuilder::new(uri)
            .with_header("Authorization", "Bearer test-token")
            .into_client_request()
            .expect("request should build");

        let headers = request.headers();
        for required in ["Host", "Connection", "Upgrade", "Sec-WebSocket-Version", "Sec-WebSocket-Key"] {
            assert!(
                headers.contains_key(required),
                "missing required WebSocket handshake header: {required}"
            );
        }
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer test-token");
    }
}

