// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The viewer routes a paired device calls, served only on the viewer
//! listener (TLS, `backend::lan_listeners`), never on the loopback or LAN
//! routers (agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07
//! §13.2, §13.3):
//!
//! - `POST /agentmux/viewer/pair`: a pairing code for a viewer token. The
//!   only route without a token.
//! - `GET /agentmux/viewer/hello`, `GET /agentmux/viewer/agents` and
//!   `GET /agentmux/viewer/agents/:name/feed`: `Authorization: Bearer
//!   amxv_…` only. Neither the `lan_key` nor the instance `auth_key` opens
//!   them, and the token opens nothing else: no other router knows it.
//!
//! Read-only throughout. An agent hidden from paired devices is left out of
//! the list and its feed is 404; agents of other channels on this machine
//! are 404 too for now (they are not in this channel's registry).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path};
use futures_util::Stream;
use serde::Deserialize;

use super::*;
use crate::backend::rpc_types::CommandBlockfileReadRangeData;
use crate::backend::viewer::feed::{self, AppendStep, Cursor, HubEvent};
use crate::backend::viewer::{pairing, FeedLease, FeedRefused};

/// Heartbeat comment on a quiet feed.
pub(crate) const FEED_HEARTBEAT: Duration = Duration::from_secs(15);
/// How often a feed looks at its agent's state. One controller lookup and one
/// block read, so a second is cheap and keeps the device's chip current.
pub(crate) const STATUS_POLL: Duration = Duration::from_secs(1);
/// The snapshot: the last 50 turns or 256 KB, whichever is smaller.
const SNAPSHOT_TAIL_TURNS: u32 = 50;
const SNAPSHOT_TAIL_BYTES: u64 = 256 * 1024;
/// The most lines one bounded read returns (`blockfile:read_range`'s cap).
const READ_LIMIT: u64 = 10_000;
const RETRY_PREAMBLE: &[u8] = b"retry: 3000\n\n";
const HEARTBEAT: &[u8] = b": hb\n\n";

/// The paired device a request authenticated as.
#[derive(Debug, Clone)]
pub(crate) struct ViewerCaller {
    pub device_id: String,
}

/// The viewer router: pairing, plus the token-gated read-only routes.
pub fn build_viewer_router(state: AppState) -> Router {
    let authed = Router::new()
        .route("/agentmux/viewer/hello", get(handle_viewer_hello))
        .route("/agentmux/viewer/agents", get(handle_viewer_agents))
        .route("/agentmux/viewer/agents/:name/feed", get(handle_viewer_feed))
        .route_layer(middleware::from_fn_with_state(state.clone(), viewer_auth_middleware));
    Router::new()
        .route("/agentmux/viewer/pair", post(handle_viewer_pair))
        .merge(authed)
        .with_state(state)
}

/// Admit a request carrying a paired device's viewer token: its SHA-256 is
/// compared, in constant time, with every paired device's. Stamps the
/// device's `last_seen_ms` at most once a minute.
async fn viewer_auth_middleware(State(state): State<AppState>, mut req: Request<Body>, next: Next) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| t.starts_with(pairing::TOKEN_PREFIX));
    let Some(token) = token else {
        return unauthorized();
    };
    let hash = pairing::token_hash(token);
    let devices = match state.mstore.viewer_device_token_hashes() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "viewer: could not read paired devices");
            return unauthorized();
        }
    };
    let mut device_id = None;
    for (id, stored) in devices {
        if secret_eq(stored.as_bytes(), hash.as_bytes()) {
            device_id = Some(id);
        }
    }
    let Some(device_id) = device_id else {
        return unauthorized();
    };
    let now = agentmux_common::time::now_ms();
    if state.viewer.last_seen_due(&device_id, now) {
        if let Err(e) = state.mstore.viewer_device_touch(&device_id, now) {
            tracing::debug!(error = %e, "viewer: could not stamp last_seen");
        }
    }
    req.extensions_mut().insert(ViewerCaller { device_id });
    next.run(req).await
}

#[derive(Deserialize)]
pub(crate) struct PairBody {
    code: String,
    device_name: String,
    #[serde(default)]
    device_key: Option<String>,
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
}

/// `POST /agentmux/viewer/pair`: `{code, device_name, device_key?}` for
/// `{token, device_id, hostname, channel, version, install_id?}`; 401 for an
/// unknown or expired code, 429 after too many wrong ones.
async fn handle_viewer_pair(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    body: Result<Json<PairBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return bad_request("expected {code, device_name, device_key?}");
    };
    let device_name = body.device_name.trim().to_string();
    let name_len = device_name.chars().count();
    if !(1..=64).contains(&name_len) || device_name.chars().any(char::is_control) {
        return bad_request("device_name must be 1 to 64 characters");
    }
    let device_key = match body.device_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        None => String::new(),
        Some(key) => {
            use base64::Engine as _;
            match base64::engine::general_purpose::STANDARD.decode(key) {
                Ok(bytes) if bytes.len() == 32 => key.to_string(),
                _ => return bad_request("device_key must be standard base64 of a 32-byte public key"),
            }
        }
    };
    let from = peer.map(|Extension(ConnectInfo(addr))| addr.ip());
    match state.viewer.pairing.redeem(&body.code, from) {
        pairing::Redeem::Accepted => {}
        pairing::Redeem::Refused => return unauthorized(),
        pairing::Redeem::RateLimited => {
            return (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too many wrong codes; wait a minute" })))
                .into_response()
        }
    }
    let token = pairing::new_token();
    let device_id = uuid::Uuid::new_v4().to_string();
    let now = agentmux_common::time::now_ms();
    if let Err(e) =
        state.mstore.viewer_device_insert(&device_id, &pairing::token_hash(&token), &device_name, &device_key, now)
    {
        tracing::warn!(error = %e, "viewer: could not record a paired device");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "could not record the device" })))
            .into_response();
    }
    tracing::info!(%device_id, %device_name, "viewer: device paired");
    // The desktop shows a toast and closes the QR (frontend `viewer:paired`).
    state.event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
        eventtype: "viewer:paired".to_string(),
        oref: String::new(),
        data: Some(json!({ "device_id": device_id, "device_name": device_name })),
    });
    let mut out = json!({
        "token": token,
        "device_id": device_id,
        "hostname": state.hostname,
        "channel": crate::backend::reactive::registry::local_channel_id(),
        "version": state.version,
    });
    if let Some(id) = crate::backend::fleet_source::install_id() {
        out["install_id"] = json!(id);
    }
    Json(out).into_response()
}

/// `GET /agentmux/viewer/hello`: who this is, and which device is asking.
async fn handle_viewer_hello(State(state): State<AppState>, Extension(caller): Extension<ViewerCaller>) -> Response {
    let mut out = json!({
        "hostname": state.hostname,
        "channel": crate::backend::reactive::registry::local_channel_id(),
        "version": state.version,
        "device_id": caller.device_id,
    });
    if let Some(id) = crate::backend::fleet_source::install_id() {
        out["install_id"] = json!(id);
    }
    Json(out).into_response()
}

/// Whether the agent in `block_id` is hidden from paired devices: its
/// definition (the block's `agentId`) has `hide_from_devices` set.
pub(crate) fn agent_hidden(state: &AppState, block_id: &str) -> bool {
    let Ok(Some(block)) = state.mstore.get::<crate::backend::obj::Block>(block_id) else {
        return false;
    };
    let def_id = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
    !def_id.is_empty() && state.mstore.agent_hide_from_devices_get(&def_id).unwrap_or(false)
}

/// The agent's provider id (`claude`, `codex`, …), '' when unknown.
fn provider_of(state: &AppState, block_id: &str) -> String {
    let Ok(Some(block)) = state.mstore.get::<crate::backend::obj::Block>(block_id) else {
        return String::new();
    };
    let raw = crate::backend::obj::meta_get_string(&block.meta, "agentProvider", "");
    match crate::backend::providers::resolve_provider_alias(&raw) {
        "" => raw,
        id => id.to_string(),
    }
}

/// `GET /agentmux/viewer/agents`: this channel's agents, hidden ones left
/// out, sorted case-insensitively.
async fn handle_viewer_agents(State(state): State<AppState>) -> Response {
    let mut agents: Vec<(String, String)> = state
        .reactive_handler
        .list_agents()
        .into_iter()
        .filter(|a| !agent_hidden(&state, &a.block_id))
        .map(|a| (a.agent_id, a.block_id))
        .collect();
    agents.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then(a.0.cmp(&b.0)));
    agents.dedup_by(|later, earlier| later.0.eq_ignore_ascii_case(&earlier.0));
    let list: Vec<serde_json::Value> = agents
        .into_iter()
        .map(|(name, block_id)| {
            let mut agent = json!({ "name": name });
            if let Some(kind) = crate::backend::operator_config_seed::agent_kind_of_block(&state.mstore, &block_id) {
                agent["kind"] = json!(kind);
            }
            // The state the LAN feed reports, from the same tracker, so both
            // give the same `since_ms`. An agent with no known state has neither.
            if let Some(status) = crate::backend::agent_state::agent_status_of_block(&state.mstore, &block_id) {
                agent["state"] = json!(status.state);
                agent["since_ms"] = json!(status.since_ms);
            }
            agent
        })
        .collect();
    // Read after the states, so no `since_ms` is later than it.
    Json(json!({ "now_ms": agentmux_common::time::now_ms_u64(), "agents": list })).into_response()
}

/// `GET /agentmux/viewer/agents/:name/feed`: the agent's transcript as
/// Server-Sent Events (see [`feed_stream`]). 404 for an agent that isn't
/// this channel's or is hidden; 503 past the feed caps.
async fn handle_viewer_feed(
    State(state): State<AppState>,
    Extension(caller): Extension<ViewerCaller>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(agent) = state.reactive_handler.get_agent(&name) else {
        return not_found();
    };
    if agent_hidden(&state, &agent.block_id) {
        return not_found();
    }
    let lease = match state.viewer.open_feed(&caller.device_id) {
        Ok(lease) => lease,
        Err(refused) => {
            let error = match refused {
                FeedRefused::PerDevice => "too many open feeds for this device",
                FeedRefused::Total => "too many open feeds",
            };
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": error }))).into_response();
        }
    };
    let last_event_id = headers.get("last-event-id").and_then(|v| v.to_str().ok()).map(str::to_string);
    let provider = provider_of(&state, &agent.block_id);
    let stream = feed_stream(
        FeedSource::of(&state, &agent.block_id),
        provider,
        last_event_id,
        lease,
        FeedTiming::default(),
        AgentProbe::of(&state, &agent.block_id),
    );
    (
        [(header::CONTENT_TYPE, "text/event-stream"), (header::CACHE_CONTROL, "no-cache")],
        Body::from_stream(stream),
    )
        .into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "no such agent" }))).into_response()
}

/// A feed's clocks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FeedTiming {
    pub heartbeat: Duration,
    pub status_poll: Duration,
}

impl Default for FeedTiming {
    fn default() -> Self {
        Self { heartbeat: FEED_HEARTBEAT, status_poll: STATUS_POLL }
    }
}

/// What a feed asks about its agent while it runs: whether it has been hidden
/// from paired devices, and its state.
pub(crate) struct AgentProbe {
    pub hidden: Box<dyn Fn() -> bool + Send>,
    pub status: Box<dyn Fn() -> Option<crate::backend::agent_state::AgentStatus> + Send>,
}

impl AgentProbe {
    /// The agent in `block_id`: hidden per its definition, and its state from
    /// the tracker the LAN feed uses (`agent_state::agent_status_of_block`).
    pub(crate) fn of(state: &AppState, block_id: &str) -> Self {
        let (hidden_state, hidden_block) = (state.clone(), block_id.to_string());
        let (mstore, status_block) = (state.mstore.clone(), block_id.to_string());
        Self {
            hidden: Box::new(move || agent_hidden(&hidden_state, &hidden_block)),
            status: Box::new(move || crate::backend::agent_state::agent_status_of_block(&mstore, &status_block)),
        }
    }
}

/// Where a feed reads its agent's transcript from.
pub(crate) struct FeedSource {
    broker: Arc<Broker>,
    filestore: Arc<FileStore>,
    global_store: Option<Arc<FileStore>>,
    mstore: Arc<Store>,
    hub: Arc<crate::backend::viewer::feed::FeedHub>,
    block_id: String,
}

/// A snapshot as read: where the feed stands after it, and its lines from
/// `from_line`.
struct Snapshot {
    cursor: Cursor,
    from_line: u64,
    lines: Vec<String>,
}

impl FeedSource {
    pub(crate) fn of(state: &AppState, block_id: &str) -> Self {
        Self {
            broker: state.broker.clone(),
            filestore: state.filestore.clone(),
            global_store: state.global_transcript_store.clone(),
            mstore: state.mstore.clone(),
            hub: state.viewer.hub.clone(),
            block_id: block_id.to_string(),
        }
    }

    async fn read(&self, offset: u64, expect_gen: &str, tail: bool) -> Option<crate::backend::rpc_types::BlockfileReadRangeResult> {
        let cmd = CommandBlockfileReadRangeData {
            block_id: self.block_id.clone(),
            filename: crate::backend::agent_session::OUTPUT_FILE.to_string(),
            offset,
            limit: READ_LIMIT,
            expect_gen: Some(expect_gen.to_string()),
            tail_turns: tail.then_some(SNAPSHOT_TAIL_TURNS),
            tail_bytes: tail.then_some(SNAPSHOT_TAIL_BYTES),
        };
        let result = super::app_api::blockfile::read_range(
            self.broker.clone(),
            self.filestore.clone(),
            self.global_store.clone(),
            self.mstore.clone(),
            cmd,
        )
        .await
        .ok()?;
        (result.gen_mismatch != Some(true)).then_some(result)
    }

    async fn position(&self) -> Option<(String, String, u64)> {
        super::app_api::blockfile::transcript_position(&self.filestore, &self.global_store, &self.mstore, &self.block_id)
            .await
    }

    /// The tail, through the bounded range read. Retried when the transcript
    /// is replaced between the count and the read; `None` if it keeps moving
    /// or can't be read.
    async fn snapshot(&self) -> Option<Snapshot> {
        for _ in 0..3 {
            let Some((stream, gen, total)) = self.position().await else {
                // No transcript yet: an empty snapshot; its first append
                // starts the stream at line 0.
                let cursor = Cursor { stream: format!("b:{}", self.block_id), gen: String::new(), next_line: 0 };
                return Some(Snapshot { cursor, from_line: 0, lines: Vec::new() });
            };
            let offset = total.saturating_sub(READ_LIMIT);
            let Some(result) = self.read(offset, &gen, true).await else { continue };
            let from_line = result.offset.unwrap_or(offset);
            let next_line = from_line + result.lines.len() as u64;
            let cursor = Cursor { stream: result.stream.unwrap_or(stream), gen, next_line };
            return Some(Snapshot { cursor, from_line, lines: result.lines });
        }
        None
    }

    /// The lines from `line` on, for a reconnect that sent `gen:line`: `None`
    /// when the transcript has another generation now, or more has been
    /// written since than a stream may fall behind by (a snapshot then).
    async fn resume(&self, gen: &str, line: u64) -> Option<(Cursor, Vec<String>)> {
        let (stream, current, total) = self.position().await?;
        if current != gen || line > total || total - line > READ_LIMIT {
            return None;
        }
        let result = self.read(line, gen, false).await?;
        let bytes: usize = result.lines.iter().map(|l| feed::wire_size(l.as_bytes())).sum();
        if bytes > feed::MAX_BEHIND_BYTES {
            return None;
        }
        let cursor = Cursor { stream: result.stream.unwrap_or(stream), gen: gen.to_string(), next_line: line + result.lines.len() as u64 };
        Some((cursor, result.lines))
    }
}

fn snapshot_event(snapshot: Snapshot, provider: &str) -> Bytes {
    let id = snapshot.cursor.event_id();
    feed::sse_event(
        "snapshot",
        Some(&id),
        &json!({
            "provider": provider,
            "gen": snapshot.cursor.gen,
            "from_line": snapshot.from_line,
            "next_line": snapshot.cursor.next_line,
            "lines": feed::device_frames(snapshot.lines),
        }),
    )
}

fn append_event(cursor: &Cursor, line: u64, lines: Vec<String>) -> Bytes {
    feed::sse_event(
        "append",
        Some(&cursor.event_id()),
        &json!({ "gen": cursor.gen, "line": line, "lines": feed::device_frames(lines) }),
    )
}

fn reset_event(reason: &str) -> Bytes {
    feed::sse_event("reset", None, &json!({ "reason": reason }))
}

/// `status` `{state, since_ms, now_ms}`; `state` and `since_ms` are null when
/// a state the device was shown has gone (it drops its chip).
fn status_event(status: Option<crate::backend::agent_state::AgentStatus>) -> Bytes {
    feed::sse_event(
        "status",
        None,
        &json!({
            "state": status.map(|s| s.state),
            "since_ms": status.map(|s| s.since_ms),
            "now_ms": agentmux_common::time::now_ms_u64(),
        }),
    )
}

/// One feed: `retry`, then a `snapshot` (or, for a reconnect whose
/// `Last-Event-ID` still holds, the lines it missed as one `append`), then an
/// `append` per published transcript append, `reset` + `snapshot` when the
/// transcript is rewritten or the feed loses its place, and a heartbeat on a
/// quiet stream. A `status` follows each `snapshot` and every change of the
/// agent's state ([`feed::status_step`]). Ends with `reset` when the device falls more than 1 MB
/// behind or the agent is hidden, and silently when the device is revoked
/// (its lease's token) or goes away. The lease is held until the stream is
/// dropped.
pub(crate) fn feed_stream(
    source: FeedSource,
    provider: String,
    last_event_id: Option<String>,
    lease: FeedLease,
    timing: FeedTiming,
    probe: AgentProbe,
) -> impl Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    async_stream::stream! {
        let lease = lease;
        // The state last sent to the device; `None` until one exists.
        let mut shown = None;
        // Subscribed before anything is read, so nothing published between
        // the read and the first event is missed.
        let mut sub = source.hub.subscribe(&source.block_id);
        yield Ok(Bytes::from_static(RETRY_PREAMBLE));

        let resumed = match last_event_id.as_deref().and_then(feed::parse_event_id) {
            Some((gen, line)) => source.resume(&gen, line).await,
            None => None,
        };
        let mut cursor = match resumed {
            Some((cursor, lines)) => {
                if !lines.is_empty() {
                    let line = cursor.next_line - lines.len() as u64;
                    yield Ok(append_event(&cursor, line, lines));
                }
                Some(cursor)
            }
            None => match source.snapshot().await {
                Some(snapshot) => {
                    let cursor = snapshot.cursor.clone();
                    yield Ok(snapshot_event(snapshot, &provider));
                    if let Some(status) = feed::status_step(&mut shown, (probe.status)(), true) {
                        yield Ok(status_event(status));
                    }
                    Some(cursor)
                }
                None => None,
            },
        };
        if cursor.is_none() {
            yield Ok(reset_event("unavailable"));
        }

        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + timing.heartbeat, timing.heartbeat);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut status_tick = tokio::time::interval_at(tokio::time::Instant::now() + timing.status_poll, timing.status_poll);
        status_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while let Some(current) = cursor.as_mut() {
            // What to do after this event: keep going, resync from a fresh
            // snapshot after a `reset`, or stop.
            let mut resync: Option<String> = None;
            tokio::select! {
                biased;
                _ = lease.token.cancelled() => break,
                event = sub.recv() => match event {
                    None => break,
                    Some(HubEvent::Behind) => {
                        yield Ok(reset_event("behind"));
                        break;
                    }
                    Some(HubEvent::Rewritten { fileop }) => resync = Some(fileop),
                    Some(HubEvent::Append { pos, records }) => match feed::apply_append(current, &pos, &records) {
                        AppendStep::Send { line, lines } => yield Ok(append_event(current, line, lines)),
                        AppendStep::Skip => {}
                        AppendStep::Resync(reason) => resync = Some(reason.to_string()),
                    },
                },
                _ = status_tick.tick() => {
                    if let Some(status) = feed::status_step(&mut shown, (probe.status)(), false) {
                        yield Ok(status_event(status));
                    }
                }
                _ = tick.tick() => {
                    if (probe.hidden)() {
                        yield Ok(reset_event("hidden"));
                        break;
                    }
                    yield Ok(Bytes::from_static(HEARTBEAT));
                }
            }
            if let Some(reason) = resync {
                yield Ok(reset_event(&reason));
                match source.snapshot().await {
                    Some(snapshot) => {
                        cursor = Some(snapshot.cursor.clone());
                        yield Ok(snapshot_event(snapshot, &provider));
                        if let Some(status) = feed::status_step(&mut shown, (probe.status)(), true) {
                            yield Ok(status_event(status));
                        }
                    }
                    None => {
                        yield Ok(reset_event("unavailable"));
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/viewer/viewer_tests.rs"]
mod tests;
