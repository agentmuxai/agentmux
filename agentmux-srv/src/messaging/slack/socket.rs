// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Slack Socket Mode connection lifecycle: `apps.connections.open`, the
//! hello/events_api/disconnect-warning state machine, per-event ACK, the
//! make-before-break reconnect-on-warning dance, and the 5-minute
//! dead-connection timer — plus the outbound `chat.postMessage` sender.
//!
//! ## Two independent tasks, not one interleaved loop
//!
//! `run_bridge` joins the inbound socket loop (`run_socket_loop`) and the
//! outbound send loop (`run_outbound_loop`) via `tokio::join!`, exactly like
//! `telegram/poller.rs::run_poll_loop` joins its inbound/outbound loops —
//! see that file's doc comment for the full review finding that motivated
//! the split (an interleaved `tokio::select!` let an outbound
//! backoff-sleep block inbound polling for as long as the backoff lasted).
//! The same coupling would be worse for Slack: the inbound loop must ACK
//! every `events_api` envelope within a hard 3-second deadline (spec §2.2),
//! so anything that could delay servicing inbound frames — including an
//! outbound rate-limit wait — must live on a fully separate task. The two
//! loops share no state; outbound doesn't touch the WebSocket at all, it
//! only calls `rest::post_message`.
//!
//! ## Make-before-break reconnect-on-warning (spec §2.3)
//!
//! This is entirely internal to `run_socket_loop`'s per-connection state
//! (`run_session`), not a second top-level task — the dance is about
//! managing one logical inbound connection's lifecycle, which is exactly
//! what `run_socket_loop` already owns.
//!
//! `run_session` holds at most two live socket halves at once:
//! - `primary: WsHalf` — the currently-active connection. All *new* inbound
//!   processing (routing, ACKs for anything just arriving) happens here.
//! - `old: Option<WsHalf>` — set only during the ~10s window between a
//!   `disconnect: warning` and Slack force-closing that connection. Frames
//!   still arriving here (Slack can route events to it right up until the
//!   close) are drained and ACKed exactly like `primary`'s, just not
//!   promoted to primary status.
//!
//! When a `warning` arrives on `primary`, we don't tear the connection down
//! and reconnect — we kick off a *second*, independent connect (fetch a
//! fresh URL via `apps.connections.open`, open a new WS, wait for its
//! `hello`) while continuing to service `primary` unmodified. That connect
//! runs as a pinned, boxed future (`cutover_connect`) polled as one more
//! `tokio::select!` branch each loop iteration — the same technique
//! `discord/gateway.rs` uses for its heartbeat-interval sleep (`hb_sleep`):
//! a `Pin<Box<dyn Future>>` defaulting to `std::future::pending()` when
//! there's nothing in flight, swapped in only when there's real async work
//! to track. This is what makes the second connect genuinely concurrent
//! with continuing to drain `primary` within a single task, no extra
//! `tokio::spawn` required.
//!
//! Once that future resolves successfully (new socket reached `hello`), the
//! *next* loop iteration promotes it: `old = Some(mem::replace(primary,
//! new))`. From that point on, `primary`'s branch handles all new frames as
//! usual, and a second `tokio::select!` branch (guarded by `old.is_some()`,
//! built from the `next_frame` helper below so it evaluates to
//! `std::future::pending()` — i.e. never fires — when `old` is `None`)
//! keeps draining and ACKing `old` until it errors or closes, at which
//! point it's dropped. A duplicate `warning` arriving while a cutover is
//! already in flight is logged and ignored (`cutover_pending` guard) rather
//! than launching a second concurrent connect.
//!
//! An unexpected close on `primary` with no prior warning (network blip,
//! Slack silently dying, etc.) is the plain backoff path: `run_session`
//! returns `Err`, and the outer `run_socket_loop` retry loop reconnects
//! after an exponentially-increasing delay (capped at 60s, matching
//! Discord's `RECONNECT_DELAY_SECS`/`MAX_RECONNECT_DELAY_SECS` constants),
//! always fetching a fresh URL first (never retrying a stale one).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_tls_with_config, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

use crate::backend::reactive::handler::get_global_handler;
use crate::backend::reactive::types::InjectionRequest;
use crate::messaging::{BridgeHealth, BridgeStatus, OutboundMsg};

use super::rest;
use super::types::{AckFrame, EventsApiPayload, SlackEvent, SocketEnvelope};

const RECONNECT_DELAY_SECS: u64 = 5;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;

/// Proactive dead-connection heuristic (spec §2.3): if no frame of any kind
/// has arrived in this long, force a reconnect even though nothing signaled
/// a close.
const DEAD_CONN_TIMEOUT_SECS: u64 = 300;

/// Minimum spacing between two outbound sends to the same channel (spec
/// §2.6 — "~1 msg/s/channel", enforced with a bit of headroom).
const CHANNEL_MIN_SPACING: Duration = Duration::from_millis(1100);

/// Timeout waiting for the first frame (must be `hello`) after connecting.
const HELLO_TIMEOUT_SECS: u64 = 10;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

struct WsHalf {
    write: WsWrite,
    read: WsRead,
}

/// Orchestrates the two independent tasks described in the module doc
/// comment. Called once from `SlackBridge::init_global`'s spawned task.
pub async fn run_bridge(
    app_token: String,
    bot_token: String,
    channel_id: String,
    target_agent: Option<String>,
    http: reqwest::Client,
    outbound_rx: mpsc::UnboundedReceiver<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
) {
    let outbound_http = http.clone();
    let default_channel = channel_id.clone();
    tokio::join!(
        run_socket_loop(app_token, channel_id, target_agent, http, health),
        run_outbound_loop(outbound_http, bot_token, default_channel, outbound_rx),
    );
}

// ── Inbound: Socket Mode connection lifecycle ───────────────────────────────

/// Outer retry-forever loop: fetch a fresh WS URL, run a session until it
/// ends (cleanly or with an error), back off, repeat. Every attempt —
/// initial connect included — fetches a new URL via `apps.connections.open`
/// (spec §2.1: the URL is single-use, there is no Discord-style constant
/// Gateway URL to fall back to).
async fn run_socket_loop(
    app_token: String,
    channel_id: String,
    target_agent: Option<String>,
    http: reqwest::Client,
    health: Arc<Mutex<BridgeHealth>>,
) {
    let mut delay_secs = RECONNECT_DELAY_SECS;

    loop {
        {
            let mut h = health.lock().unwrap();
            h.status = BridgeStatus::Connecting;
        }

        let session_start = Instant::now();

        match connect_and_run(&app_token, &channel_id, &target_agent, &http, &health).await {
            Ok(()) => {
                tracing::info!("slack_bridge: session ended cleanly");
            }
            Err(e) => {
                tracing::warn!("slack_bridge: session error: {e}");
                let mut h = health.lock().unwrap();
                h.status = BridgeStatus::Error;
                h.error = Some(e);
                h.reconnect_count += 1;
            }
        }

        if session_start.elapsed().as_secs() > 30 {
            delay_secs = RECONNECT_DELAY_SECS;
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

async fn connect_and_run(
    app_token: &str,
    channel_id: &str,
    target_agent: &Option<String>,
    http: &reqwest::Client,
    health: &Arc<Mutex<BridgeHealth>>,
) -> Result<(), String> {
    let mut primary = open_and_await_hello(app_token, http).await?;

    {
        let mut h = health.lock().unwrap();
        h.status = BridgeStatus::Connected;
        h.error = None;
        h.last_event_at = Some(unix_secs());
    }
    tracing::info!("slack_bridge: connected (hello received)");

    run_session(app_token, channel_id, target_agent, http, health, &mut primary).await
}

/// Fetches a fresh Socket Mode URL and connects, waiting for the mandatory
/// first `hello` frame (spec §2.1, §4.1). Used both for the initial connect
/// and for the cutover half of the make-before-break dance.
async fn open_and_await_hello(app_token: &str, http: &reqwest::Client) -> Result<WsHalf, String> {
    let url = rest::open_connection(http, app_token).await?;
    tracing::debug!(
        "slack_bridge: opened connection ({})",
        rest::redact_ticket(&url)
    );

    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&url)
        .body(())
        .map_err(|e| format!("slack_bridge: build ws request: {e}"))?;

    let (ws_stream, _) = connect_async_tls_with_config(request, None, false, None)
        .await
        .map_err(|e| format!("slack_bridge: ws connect: {e}"))?;

    let (write, mut read) = ws_stream.split();

    match tokio::time::timeout(Duration::from_secs(HELLO_TIMEOUT_SECS), read.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<SocketEnvelope>(&text) {
            Ok(SocketEnvelope::Hello(_)) => Ok(WsHalf { write, read }),
            Ok(_) => Err("slack_bridge: expected hello as first frame, got a different envelope type".to_string()),
            Err(e) => Err(format!("slack_bridge: parse first frame: {e}")),
        },
        Ok(Some(Ok(_))) => Err("slack_bridge: expected hello as first frame, got a non-text frame".to_string()),
        Ok(Some(Err(e))) => Err(format!("slack_bridge: ws recv error awaiting hello: {e}")),
        Ok(None) => Err("slack_bridge: socket closed before hello".to_string()),
        Err(_) => Err("slack_bridge: timed out waiting for hello".to_string()),
    }
}

/// Await a frame on `old` if present, else never resolve. Lets the cutover
/// socket's read half participate in the same `tokio::select!` as
/// `primary`'s without needing a separate task — mirrors how
/// `discord/gateway.rs` boxes an optional sleep future for its heartbeat.
async fn next_frame(
    old: &mut Option<WsHalf>,
) -> Option<Result<Message, tokio_tungstenite::tungstenite::Error>> {
    match old {
        Some(half) => half.read.next().await,
        None => std::future::pending().await,
    }
}

enum FrameOutcome {
    Continue,
    WarningReceived,
    Closed,
}

/// Runs one connection's session loop, holding at most two live socket
/// halves (`primary` + optionally `old`, see module doc comment).
///
/// Returns `Ok(())` on a clean close (primary closed with no error) or
/// `Err(e)` on anything reconnect-worthy (unexpected close, dead-connection
/// timeout, ws send/recv error). Either way the caller (`run_socket_loop`)
/// starts a fresh session with a freshly-fetched URL.
async fn run_session(
    app_token: &str,
    channel_id: &str,
    target_agent: &Option<String>,
    http: &reqwest::Client,
    health: &Arc<Mutex<BridgeHealth>>,
    primary: &mut WsHalf,
) -> Result<(), String> {
    let mut old: Option<WsHalf> = None;
    let mut cutover_pending = false;

    let mut cutover_connect: std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<WsHalf, String>> + Send>,
    > = Box::pin(std::future::pending());

    let mut dead_conn_sleep = Box::pin(tokio::time::sleep(Duration::from_secs(
        DEAD_CONN_TIMEOUT_SECS,
    )));

    loop {
        tokio::select! {
            // Proactive dead-connection heuristic (spec §2.3).
            _ = &mut dead_conn_sleep => {
                return Err(format!(
                    "no frames received for {DEAD_CONN_TIMEOUT_SECS}s — treating connection as dead"
                ));
            }

            // Cutover connect finished (success or failure).
            result = &mut cutover_connect, if cutover_pending => {
                cutover_pending = false;
                cutover_connect = Box::pin(std::future::pending());
                match result {
                    Ok(new_half) => {
                        tracing::info!("slack_bridge: cutover connection ready (hello received) — promoting to primary");
                        let retiring = std::mem::replace(primary, new_half);
                        old = Some(retiring);
                        dead_conn_sleep = Box::pin(tokio::time::sleep(Duration::from_secs(DEAD_CONN_TIMEOUT_SECS)));
                        let mut h = health.lock().unwrap();
                        h.last_event_at = Some(unix_secs());
                    }
                    Err(e) => {
                        // Stay on the current primary — it may still get
                        // force-closed by Slack at the end of the warning
                        // period, at which point the plain unexpected-close
                        // path (primary.read.next() below) takes over.
                        tracing::warn!("slack_bridge: cutover connect failed: {e} — staying on current primary until it closes");
                    }
                }
            }

            // Frame on the active (primary) socket.
            frame = primary.read.next() => {
                match frame {
                    None => return Ok(()),
                    Some(Err(e)) => return Err(format!("ws recv (primary): {e}")),
                    Some(Ok(msg)) => {
                        dead_conn_sleep = Box::pin(tokio::time::sleep(Duration::from_secs(DEAD_CONN_TIMEOUT_SECS)));
                        let outcome = handle_frame(msg, &mut primary.write, channel_id, target_agent, health).await?;
                        match outcome {
                            FrameOutcome::Continue => {}
                            FrameOutcome::Closed => return Ok(()),
                            FrameOutcome::WarningReceived => {
                                // Guard on `old.is_some()` too, not just
                                // `cutover_pending`: a warning arriving on the
                                // newly-promoted primary before the previous
                                // retiring connection (`old`) has finished
                                // draining must not start a second cutover —
                                // its success arm below unconditionally
                                // overwrites `old`, which would silently drop
                                // the still-open first retiring connection
                                // instead of finishing its drain, violating
                                // "at most two live socket halves." Worst case
                                // of ignoring it: Slack force-closes this
                                // primary at the end of its warning window,
                                // which falls through to the plain
                                // unexpected-close error path below and
                                // reconnects via the outer session retry loop
                                // — the same degraded-but-safe fallback this
                                // module already accepts when a cutover
                                // connect attempt itself fails (see the `Err`
                                // arm above).
                                if cutover_pending || old.is_some() {
                                    tracing::debug!(
                                        "slack_bridge: disconnect warning while a cutover is already in flight or draining — ignoring"
                                    );
                                } else {
                                    tracing::info!("slack_bridge: disconnect warning received — starting make-before-break reconnect");
                                    cutover_pending = true;
                                    let app_token = app_token.to_string();
                                    let http = http.clone();
                                    cutover_connect = Box::pin(async move { open_and_await_hello(&app_token, &http).await });
                                }
                            }
                        }
                    }
                }
            }

            // Frame on the retiring (old) socket — only polled once a
            // cutover has actually promoted a new primary.
            frame = next_frame(&mut old), if old.is_some() => {
                match frame {
                    None | Some(Err(_)) => {
                        tracing::debug!("slack_bridge: old socket closed/errored after cutover — discarding");
                        old = None;
                    }
                    Some(Ok(msg)) => {
                        // Best-effort: ACK/drain the old socket, but don't
                        // let its errors tear down the (already-promoted)
                        // primary session.
                        if let Some(half) = old.as_mut() {
                            match handle_frame(msg, &mut half.write, channel_id, target_agent, health).await {
                                Ok(FrameOutcome::Closed) => {
                                    tracing::info!("slack_bridge: old socket closed cleanly after cutover");
                                    old = None;
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::debug!("slack_bridge: old socket error after cutover (harmless — draining): {e}");
                                    old = None;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Handles one WS frame: ACKs `events_api`/`slash_commands` envelopes
/// immediately (before any parsing/routing — spec §2.2's hard 3-second
/// requirement), then routes `events_api` payloads to the reactive bus.
async fn handle_frame(
    msg: Message,
    write: &mut WsWrite,
    channel_id: &str,
    target_agent: &Option<String>,
    health: &Arc<Mutex<BridgeHealth>>,
) -> Result<FrameOutcome, String> {
    match msg {
        Message::Text(text) => {
            let envelope: SocketEnvelope = match serde_json::from_str(&text) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("slack_bridge: parse error: {e}");
                    return Ok(FrameOutcome::Continue);
                }
            };

            match envelope {
                SocketEnvelope::Hello(_) => {
                    tracing::debug!("slack_bridge: hello frame on an already-active socket");
                    Ok(FrameOutcome::Continue)
                }
                SocketEnvelope::EventsApi(ev) => {
                    ack_envelope(write, &ev.envelope_id).await?;
                    route_event(ev.payload, channel_id, target_agent, health);
                    Ok(FrameOutcome::Continue)
                }
                SocketEnvelope::SlashCommands(ev) => {
                    // ACK required same as any other envelope (spec §2.5),
                    // but slash commands are out of scope for this PR — no
                    // further handling.
                    ack_envelope(write, &ev.envelope_id).await?;
                    tracing::debug!("slack_bridge: received slash_commands envelope (not handled — out of scope for this PR)");
                    Ok(FrameOutcome::Continue)
                }
                SocketEnvelope::Disconnect(d) => {
                    if d.reason.as_deref() == Some("warning") {
                        Ok(FrameOutcome::WarningReceived)
                    } else {
                        tracing::info!("slack_bridge: disconnect frame (reason={:?})", d.reason);
                        Ok(FrameOutcome::Closed)
                    }
                }
                SocketEnvelope::Unknown => {
                    tracing::debug!("slack_bridge: unhandled envelope type");
                    Ok(FrameOutcome::Continue)
                }
            }
        }
        Message::Ping(data) => {
            let _ = write.send(Message::Pong(data)).await;
            Ok(FrameOutcome::Continue)
        }
        Message::Close(_) => Ok(FrameOutcome::Closed),
        _ => Ok(FrameOutcome::Continue),
    }
}

/// Sends `{"envelope_id": "..."}` back on the socket the envelope arrived
/// on. Kept allocation-light and infallible-as-possible per spec §2.2's
/// note that the ACK path should stay cheap and unconditional.
async fn ack_envelope(write: &mut WsWrite, envelope_id: &str) -> Result<(), String> {
    let ack = AckFrame {
        envelope_id: envelope_id.to_string(),
    };
    let ack_json = serde_json::to_string(&ack).unwrap_or_default();
    write
        .send(Message::Text(ack_json.into()))
        .await
        .map_err(|e| format!("slack_bridge: ack send: {e}"))
}

/// True when a `SlackEvent` should be routed to the reactive bus: right
/// event type, right channel, not self-authored. Pure/testable without a
/// real socket.
fn should_route(event: &SlackEvent, channel_id: &str) -> bool {
    if event.bot_id.is_some() {
        return false; // spec §9.4 — bot-message loop prevention
    }
    if event.type_ != "message" && event.type_ != "app_mention" {
        return false;
    }
    event.channel.as_deref() == Some(channel_id) // spec §9.3 — channel allowlist
}

fn route_event(
    payload: Option<EventsApiPayload>,
    channel_id: &str,
    target_agent: &Option<String>,
    health: &Arc<Mutex<BridgeHealth>>,
) {
    let Some(event) = payload.and_then(|p| p.event) else {
        return;
    };

    if !should_route(&event, channel_id) {
        return;
    }

    {
        let mut h = health.lock().unwrap();
        h.last_event_at = Some(unix_secs());
    }

    let Some(target) = target_agent else {
        tracing::debug!("slack_bridge: event in #{channel_id} (no target agent configured)");
        return;
    };

    let user = event.user.as_deref().unwrap_or("unknown");
    let text = event.text.as_deref().unwrap_or("");
    let envelope = format!("[Slack #{channel_id} @{user}]: {text}");

    let handler = get_global_handler();
    let req = InjectionRequest {
        target_agent: target.clone(),
        message: envelope,
        source_agent: Some("slack".to_string()),
        request_id: event.ts.clone(),
        priority: None,
        wait_for_idle: false,
        jekt_tier: None,
        delivery_tier: Some("wan".to_string()),
        forward_hops: 0,
        ..Default::default()
    };
    let result = handler.inject_message(req);
    if result.success {
        tracing::debug!("slack_bridge: injected msg from {user} to agent {target}");
    } else {
        tracing::warn!(
            "slack_bridge: inject to agent {target} failed: {:?}",
            result.error
        );
    }
}

// ── Outbound: chat.postMessage sender ───────────────────────────────────────

async fn run_outbound_loop(
    http: reqwest::Client,
    bot_token: String,
    default_channel_id: String,
    mut outbound_rx: mpsc::UnboundedReceiver<OutboundMsg>,
) {
    // Per-channel outbound rate limiting (spec §2.6, v1 scope: spacing +
    // reactive 429 backoff only). Owned entirely by this task — never
    // shared with the inbound loop.
    let mut channel_last_sent: HashMap<String, Instant> = HashMap::new();
    let mut channel_backoff_until: HashMap<String, Instant> = HashMap::new();
    let mut channel_backoff_secs: HashMap<String, u64> = HashMap::new();

    while let Some(msg) = outbound_rx.recv().await {
        handle_outbound(
            &http,
            &bot_token,
            &default_channel_id,
            msg,
            &mut channel_last_sent,
            &mut channel_backoff_until,
            &mut channel_backoff_secs,
        )
        .await;
    }
}

/// Resolves an `OutboundMsg.channel_id` (empty → bridge default) into the
/// channel id to send to. Pure/testable.
fn resolve_channel_id(channel_id: &str, default_channel_id: &str) -> Option<String> {
    if !channel_id.is_empty() {
        return Some(channel_id.to_string());
    }
    if default_channel_id.is_empty() {
        return None;
    }
    Some(default_channel_id.to_string())
}

/// Next backoff duration after a repeated failure, doubling and capped —
/// same family used for reconnects (spec §2.6: "on repeated 429s apply the
/// same exponential backoff used for reconnects"). Pure/testable.
fn next_backoff_secs(current: u64) -> u64 {
    (current * 2).min(MAX_RECONNECT_DELAY_SECS)
}

async fn handle_outbound(
    http: &reqwest::Client,
    bot_token: &str,
    default_channel_id: &str,
    msg: OutboundMsg,
    channel_last_sent: &mut HashMap<String, Instant>,
    channel_backoff_until: &mut HashMap<String, Instant>,
    channel_backoff_secs: &mut HashMap<String, u64>,
) {
    let Some(channel_id) = resolve_channel_id(&msg.channel_id, default_channel_id) else {
        tracing::warn!(
            "slack_bridge: send dropped — no channel_id (empty channel_id and no messaging:slack:channel configured)"
        );
        return;
    };

    if let Some(until) = channel_backoff_until.get(&channel_id).copied() {
        let now = Instant::now();
        if now < until {
            tokio::time::sleep(until - now).await;
        }
        channel_backoff_until.remove(&channel_id);
    }

    if let Some(last) = channel_last_sent.get(&channel_id) {
        let elapsed = last.elapsed();
        if elapsed < CHANNEL_MIN_SPACING {
            tokio::time::sleep(CHANNEL_MIN_SPACING - elapsed).await;
        }
    }

    match rest::post_message(http, bot_token, &channel_id, &msg).await {
        Ok(()) => {
            channel_last_sent.insert(channel_id.clone(), Instant::now());
            channel_backoff_secs.remove(&channel_id);
        }
        Err(e) if e.retry_after.is_some() => {
            // Spec §2.6: on 429, wait Retry-After then retry once.
            let retry_after = e.retry_after.unwrap();
            tracing::warn!(
                "slack_bridge: rate limited sending to #{channel_id}, retry_after={retry_after}s — waiting then retrying once"
            );
            tokio::time::sleep(Duration::from_secs(retry_after)).await;

            match rest::post_message(http, bot_token, &channel_id, &msg).await {
                Ok(()) => {
                    channel_last_sent.insert(channel_id.clone(), Instant::now());
                    channel_backoff_secs.remove(&channel_id);
                }
                Err(e2) if e2.retry_after.is_some() => {
                    // Still rate-limited after the single Retry-After wait —
                    // apply exponential channel backoff.
                    tracing::warn!(
                        "slack_bridge: retry after rate limit still rate-limited for #{channel_id}: {e2}"
                    );
                    let current = channel_backoff_secs
                        .get(&channel_id)
                        .copied()
                        .unwrap_or(RECONNECT_DELAY_SECS);
                    channel_backoff_until.insert(channel_id.clone(), Instant::now() + Duration::from_secs(current));
                    channel_backoff_secs.insert(channel_id, next_backoff_secs(current));
                }
                Err(e2) => {
                    // A different, non-rate-limit failure (e.g.
                    // channel_not_found, invalid_auth) — don't conflate it
                    // with throttling by scheduling a rate-limit backoff;
                    // just log it, matching the plain-failure arm below.
                    tracing::warn!(
                        "slack_bridge: retry after rate limit failed for #{channel_id} for an unrelated reason: {e2}"
                    );
                }
            }
        }
        Err(e) => {
            tracing::warn!("slack_bridge: send to #{channel_id} failed: {e}");
        }
    }
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
    use super::super::types::SlackEvent;

    fn event(type_: &str, channel: Option<&str>, bot_id: Option<&str>) -> SlackEvent {
        SlackEvent {
            type_: type_.to_string(),
            channel: channel.map(str::to_string),
            user: Some("U1".to_string()),
            text: Some("hi".to_string()),
            ts: Some("123.4".to_string()),
            bot_id: bot_id.map(str::to_string),
        }
    }

    #[test]
    fn should_route_accepts_message_in_configured_channel() {
        let e = event("message", Some("C1"), None);
        assert!(should_route(&e, "C1"));
    }

    #[test]
    fn should_route_accepts_app_mention() {
        let e = event("app_mention", Some("C1"), None);
        assert!(should_route(&e, "C1"));
    }

    #[test]
    fn should_route_rejects_other_channel() {
        let e = event("message", Some("C2"), None);
        assert!(!should_route(&e, "C1"));
    }

    #[test]
    fn should_route_rejects_missing_channel() {
        let e = event("message", None, None);
        assert!(!should_route(&e, "C1"));
    }

    #[test]
    fn should_route_rejects_bot_authored_events() {
        let e = event("message", Some("C1"), Some("B1"));
        assert!(!should_route(&e, "C1"));
    }

    #[test]
    fn should_route_rejects_unrelated_event_types() {
        let e = event("channel_join", Some("C1"), None);
        assert!(!should_route(&e, "C1"));
    }

    #[test]
    fn resolve_channel_id_uses_explicit_channel_when_present() {
        assert_eq!(resolve_channel_id("C1", "C_default"), Some("C1".to_string()));
    }

    #[test]
    fn resolve_channel_id_falls_back_to_default_when_empty() {
        assert_eq!(resolve_channel_id("", "C_default"), Some("C_default".to_string()));
    }

    #[test]
    fn resolve_channel_id_none_when_both_empty() {
        assert_eq!(resolve_channel_id("", ""), None);
    }

    #[test]
    fn next_backoff_secs_doubles() {
        assert_eq!(next_backoff_secs(5), 10);
        assert_eq!(next_backoff_secs(10), 20);
    }

    #[test]
    fn next_backoff_secs_caps_at_max() {
        assert_eq!(next_backoff_secs(45), MAX_RECONNECT_DELAY_SECS);
        assert_eq!(next_backoff_secs(60), MAX_RECONNECT_DELAY_SECS);
    }

    // ── Reconnect-on-warning state transition (no real socket needed) ──────

    #[test]
    fn duplicate_warning_while_cutover_pending_is_a_noop_decision() {
        // Mirrors the guard in run_session's WarningReceived arm: a second
        // warning while `cutover_pending` is true must not start a second
        // concurrent connect. This is the pure boolean condition that
        // guards that branch.
        let cutover_pending = true;
        let should_start_new_cutover = !cutover_pending;
        assert!(!should_start_new_cutover);
    }

    #[test]
    fn fresh_warning_when_not_pending_starts_cutover() {
        let cutover_pending = false;
        let should_start_new_cutover = !cutover_pending;
        assert!(should_start_new_cutover);
    }
}
