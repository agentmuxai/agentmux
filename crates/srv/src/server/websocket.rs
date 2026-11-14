// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, State, WebSocketUpgrade,
    },
    response::Response,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{CommandBlockInputData, RpcMessage, COMMAND_CONTROLLER_INPUT};
use crate::backend::obj::TermSize;

use super::AppState;
pub(super) use super::ws_register::register_handlers;

/// Incoming WebSocket message envelope.
/// Supports both ping/pong messages and wscommand-based RPC.
#[derive(Deserialize)]
struct WSIncoming {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    #[allow(dead_code)]
    stime: Option<i64>,
    wscommand: Option<String>,
    message: Option<RpcMessage>,
    // Fields for setblocktermsize / blockinput
    blockid: Option<String>,
    inputdata64: Option<String>,
    termsize: Option<serde_json::Value>,
    // Fields for bus:* commands
    agent_id: Option<String>, token: Option<String>, // the token proves a bus:register
    from: Option<String>,
    to: Option<String>,
    target: Option<String>,
    payload: Option<String>,
    #[serde(rename = "bus_message")]
    bus_message_text: Option<String>,
    priority: Option<String>,
}

/// The `srvinfo` event sent once on every WebSocket connect, right after
/// `config`: srv's version, which the UI compares with its own, and the
/// machine srv runs on (`crate::srv_info`).
fn srv_info_event(info: crate::srv_info::SrvInfo) -> serde_json::Value {
    crate::backend::eventbus::EventBus::typed_frame::<crate::backend::mps_events::SrvInfo>(&info)
}

pub(super) async fn handle_ws(
    State(state): State<AppState>,
    session: Option<Extension<super::local_session::SessionId>>,
    ws: WebSocketUpgrade,
) -> Response {
    let session = session.map(|Extension(s)| s);
    // A signed-in browser offers its session token as a second protocol (it can't set a header on a
    // WebSocket); srv selects only the fixed name, never the token.
    ws.protocols([super::local_session_routes::WS_PROTOCOL]).on_upgrade(move |socket| handle_ws_connection(socket, state, session))
}

async fn handle_ws_connection(mut socket: WebSocket, state: AppState, session: Option<super::local_session::SessionId>) {
    let ws_start = std::time::Instant::now();
    let conn_id = uuid::Uuid::new_v4().to_string();
    let tab_id = String::new();

    tracing::info!(conn_id = %conn_id, "WebSocket client connected");

    // Two egress lanes per connection: interactive (terminal echo, RPC-routed
    // wave events, obj updates) and background (droppable perf telemetry —
    // sysinfo/blockstats). The select! below drains priority before background
    // so typing never waits behind a perf tick. See
    // docs/specs/SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md.
    let crate::backend::eventbus::WsReceivers {
        priority: mut priority_rx,
        background: mut background_rx,
        drops,
    } = state.event_bus.register_ws(&conn_id, &tab_id);
    tracing::info!("[ws-perf] register_ws: {:.2}ms", ws_start.elapsed().as_secs_f64() * 1000.0);

    // Optional messagebus receiver — activated when pane sends bus:register
    let mut bus_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::backend::messagebus::BusMessage>> = None;
    let mut bus_agent_id: Option<String> = None;

    // Send initial "config" wave event via the RPC eventrecv path so the frontend
    // populates fullConfigAtom (and shows the widget bar).
    // Frontend only processes events via: {"eventtype":"rpc","data":{"command":"eventrecv","data":{"event":"config","data":{...}}}}
    {
        let t = std::time::Instant::now();
        let config = state.config_watcher.get_full_config();
        if let Ok(mut config_val) = serde_json::to_value(config.as_ref()) {
            crate::backend::wconfig::redact_full_config_for_renderer(&mut config_val);
            let config_event = crate::backend::eventbus::EventBus::typed_frame::<crate::backend::mps_events::Config>(&crate::backend::mps_events::ConfigEvent { fullconfig: config_val });
            if let Ok(msg) = serde_json::to_string(&config_event) {
                let _ = socket.send(Message::Text(msg.into())).await;
            }
        }
        tracing::info!("[ws-perf] send_initial_config: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0);
    }

    // Tell the client which srv it reached: the version, so a UI from another
    // version (an update replaced one side) can ask the user to reload, and
    // the machine, whose names and paths the UI shows and builds.
    let info = crate::srv_info::srv_info(&state.version, &state.hostname);
    if let Ok(msg) = serde_json::to_string(&srv_info_event(info)) {
        let _ = socket.send(Message::Text(msg.into())).await;
    }

    // Create RPC engine for this connection
    let t = std::time::Instant::now();
    let (engine, mut rpc_output_rx) = WshRpcEngine::new();

    // Register handlers
    register_handlers(&engine, state.clone(), conn_id.clone());
    tracing::info!("[ws-perf] create_engine+register_handlers: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0);
    tracing::info!("[ws-perf] TOTAL ws_setup: {:.2}ms", ws_start.elapsed().as_secs_f64() * 1000.0);

    // Periodic ping interval (10 seconds)
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(10));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Fairly-ordered priority-lane events awaiting their turn, sent one per `'ws` loop iteration
    // (priority-lane branch below, then the flush branch). Forwarding a whole drained batch inside
    // one `select!` branch body would starve `socket.recv()` (this connection's OWN keystrokes)
    // for the batch, since `select!` re-polls siblings only when a body returns; one event per
    // iteration keeps each send interleaved with a fresh `socket.recv()` poll, while the batch
    // keeps its fair pane ordering.
    let mut pending_priority: std::collections::VecDeque<serde_json::Value> =
        std::collections::VecDeque::new();

    let session_ended = { super::operator_proof::vouch_for_signed_in_browser(&engine, session.as_ref()); super::local_session_routes::session_ended(session) };
    tokio::pin!(session_ended);
    'ws: loop {
        tokio::select! {
            // Biased: poll branches top-to-bottom so interactive terminal I/O
            // always wins over droppable perf telemetry. Order = incoming
            // keystrokes → RPC replies → agent bus → priority events (terminal
            // echo) → background events (sysinfo/blockstats) → keepalive ping.
            // See docs/specs/SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md.
            biased;

            _ = &mut session_ended => break, // the browser's sign-in ended (logout, idle expiry)

            // Incoming WebSocket messages → parse & dispatch (keystrokes first)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Text(text))) => {
                        match handle_incoming_text(&text, &engine, &state, &mut socket).await {
                            Err(true) => break,
                            Ok(Some((new_rx, agent_id))) => {
                                // bus:register returned a new receiver
                                bus_rx = Some(new_rx);
                                bus_agent_id = Some(agent_id);
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(_)) => {
                        // Binary or other message types — ignore
                    }
                    Some(Err(_)) => break,
                }
            }

            // Forward RPC engine output → WebSocket (wrapped as eventtype:rpc)
            Some(rpc_msg) = rpc_output_rx.recv() => {
                let wrapped = json!({
                    "eventtype": "rpc",
                    "data": rpc_msg,
                });
                let msg = serde_json::to_string(&wrapped).unwrap_or_default();
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }

            // Forward MessageBus messages → WebSocket (if registered as agent)
            Some(bus_msg) = async {
                match bus_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let wrapped = json!({
                    "type": "bus:message",
                    "data": bus_msg,
                });
                let msg = serde_json::to_string(&wrapped).unwrap_or_default();
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }

            // Priority event lane → WebSocket. Two sources feed it:
            //   1. MPS Broker (via EventBusBridge) — already wrapped as
            //      { eventtype: "rpc", data: { command: "eventrecv", data: MuxEvent } }
            //   2. Direct broadcasts (e.g., SetMeta's obj:update) — raw
            //      { eventtype: "waveobj:update", oref: "block:xxx", data: ... }
            // This carries terminal echo output and all interactive events.
            //
            // Fairness: this lane is shared by EVERY pane on the connection, not
            // just one. A single FIFO here means a noisy pane (heavy PTY output)
            // can queue an unbounded number of frames ahead of a different pane's
            // own keystroke echo, which arrives in the same channel. Draining all
            // immediately-available events and round-robining them by pane (see
            // `fair_drain_priority`) bounds that delay to one "round" per distinct
            // active pane instead of the length of the noisy pane's backlog. See
            // docs/analysis/ANALYSIS_CROSS_PANE_INPUT_DELAY_UNDER_OUTPUT_LOAD_2026_09_04.md.
            //
            // Only drains a fresh batch when `pending_priority` is empty — while
            // it's still being flushed (one per iteration, below), this branch
            // stays disabled so we don't pull more out of `priority_rx` than
            // we've finished sending.
            Some(event) = priority_rx.recv(), if pending_priority.is_empty() => {
                let mut fair = fair_drain_priority(event, &mut priority_rx);
                // `fair` always has >= 1 element (it contains at least `event`).
                let first = fair.remove(0);
                pending_priority.extend(fair);
                if forward_event(&mut socket, first).await {
                    break 'ws;
                }
                if super::ws_resync::report_when_drained(&mut socket, &drops, &priority_rx, &pending_priority).await {
                    break 'ws;
                }
            }

            // Flush one fairly-ordered priority event per iteration. A no-op
            // branch guarded on `pending_priority` being non-empty — see the
            // comment on `pending_priority`'s declaration above for why this
            // is its own branch instead of a loop in the arm above.
            _ = std::future::ready(()), if !pending_priority.is_empty() => {
                let ev = pending_priority.pop_front().expect("guarded non-empty");
                if forward_event(&mut socket, ev).await
                    || super::ws_resync::report_when_drained(&mut socket, &drops, &priority_rx, &pending_priority).await
                {
                    break 'ws;
                }
            }

            // Background event lane → WebSocket. Droppable perf telemetry
            // (sysinfo/blockstats) only; serviced when the priority lanes above
            // are momentarily idle, so it can never delay a keystroke echo.
            //
            // Tradeoff of `biased;`: a SUSTAINED priority-lane flood (`! yes` pumping terminal
            // output) keeps the priority branches ready, so telemetry queues here until the flood
            // pauses. The lane is bounded (`BACKGROUND_LANE_CAPACITY`, drop-on-full: see
            // eventbus.rs), so a starvation long enough to fill it drops new readings instead of
            // growing commit; fine for ~1/s telemetry. Phase 2: coalesce everything immediately
            // available to the latest reading per (event-name:scope) key before forwarding, so
            // the flush is at most 1 sysinfo + 1 blockstats per open block, not a burst of stale
            // frames the browser processes back-to-back ahead of queued keypresses.
            // See SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md §4.2.
            Some(first_bg) = background_rx.recv() => {
                let coalesced = coalesce_background(first_bg, &mut background_rx);
                for ev in coalesced {
                    if forward_event(&mut socket, ev).await {
                        break 'ws;
                    }
                }
            }

            // Periodic ping
            _ = ping_interval.tick() => {
                if super::ws_resync::report_when_drained(&mut socket, &drops, &priority_rx, &pending_priority).await {
                    break;
                }
                let now = agentmux_common::time::now_ms();
                let ping = json!({ "type": "ping", "stime": now });
                let msg = serde_json::to_string(&ping).unwrap_or_default();
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
        }
    }

    tracing::info!(conn_id = %conn_id, "WebSocket client disconnected");
    state.event_bus.unregister_ws(&conn_id);
    state.broker.unsubscribe_all(&conn_id);
    if let Some(r) = crate::backend::notify::router::get(&state.broker) {
        r.disconnect(&conn_id);
    }

    // Unregister from messagebus if this connection was an agent
    if let Some(ref agent_id) = bus_agent_id {
        state.messagebus.unregister(agent_id);
    }
}

/// Forward a queued event-bus value to the WebSocket. Returns `true` if the
/// send failed (the caller should break the loop).
///
/// Two shapes arrive: already-RPC-wrapped values (from the MPS broker via
/// EventBusBridge) are forwarded as-is; raw event-bus values (e.g. SetMeta's
/// `waveobj:update`) are wrapped as an RPC `eventrecv` so the frontend
/// WshRouter routes them to handleMuxEvent → updateMuxObject. Shared by both
/// the priority and background egress lanes.
async fn forward_event(socket: &mut WebSocket, event: serde_json::Value) -> bool {
    let msg = if event["eventtype"] == "rpc" {
        // Already an RPC message (from MPS broker via EventBusBridge)
        serde_json::to_string(&event).unwrap_or_default()
    } else {
        // Raw event bus event — wrap as RPC eventrecv
        // Re-wraps a raw event-bus event into the frame the renderer reads. It is typed where it
        // was built (`EventBus::typed_ws_event`); check-untyped-rpc-handlers counts any raw event
        // built by hand.
        let mux_event = json!({
            "event": event["eventtype"],
            "scopes": [event["oref"]],
            "data": event["data"],
        });
        let wrapped = json!({
            "eventtype": "rpc",
            "data": {
                // untyped-relay: forwards an event built elsewhere, as it was built.
                "command": "eventrecv",
                "data": mux_event,
            },
        });
        serde_json::to_string(&wrapped).unwrap_or_default()
    };
    socket.send(Message::Text(msg.into())).await.is_err()
}

/// Extract a coalescing key from a background-lane event.
/// Background events are RPC-wrapped MPS events:
///   { "eventtype": "rpc", "data": { "command": "eventrecv", "data": { "event": "sysinfo", "scopes": ["local"] } } }
/// Key = "<event-name>:<first-scope>", e.g. "sysinfo:local" or "blockstats:block:abc123".
/// Falls back to "_:_" for events that don't match the expected shape (safe: they coalesce
/// to one slot, which is acceptable because non-sysinfo/blockstats never reach this lane).
fn background_event_key(event: &serde_json::Value) -> String {
    let inner = event.get("data").and_then(|d| d.get("data"));
    let event_name = inner
        .and_then(|d| d.get("event"))
        .and_then(|e| e.as_str())
        .unwrap_or("_");
    let scope = inner
        .and_then(|d| d.get("scopes"))
        .and_then(|s| s.get(0))
        .and_then(|s| s.as_str())
        .unwrap_or("_");
    format!("{event_name}:{scope}")
}

/// Drain all immediately-available events from `rx` (including `first`) and
/// coalesce them to the latest reading per (event-name:scope) key. Iteration
/// order in the returned Vec is first-seen (stable across coalescing rounds).
fn coalesce_background(
    first: serde_json::Value,
    rx: &mut tokio::sync::mpsc::Receiver<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    let key = background_event_key(&first);
    order.push(key.clone());
    map.insert(key, first);
    while let Ok(next) = rx.try_recv() {
        let k = background_event_key(&next);
        if !map.contains_key(&k) {
            order.push(k.clone());
        }
        map.insert(k, next);
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

/// Sentinel key for every priority-lane event that is NOT a `blockfile`
/// (terminal/PTY output) event. Not a valid `block:<id>` scope string, so it
/// can never collide with a real pane key.
const NON_PANE_EVENT_KEY: &str = "\u{0}non-pane-event";

/// Extract a fairness key so a flood of PTY output from one pane can't queue
/// an unbounded number of frames ahead of a different pane's own keystroke
/// echo sitting in the same lane.
///
/// Reordering is scoped to `blockfile` events ONLY — that's the one event
/// type a single pane can flood the shared lane with, and PTY bytes from
/// different panes are causally independent (only same-pane ordering
/// matters, which each key's own FIFO group preserves). Every OTHER event
/// type on this lane (`waveobj:update`, `waveobj:batchedupdates`, the initial
/// `config` push, RPC replies) returns the same `NON_PANE_EVENT_KEY` and so
/// stays in ONE FIFO group, never reordered relative to each other.
///
/// This restriction is load-bearing, not a simplification: a `waveobj:update`
/// for object X and a LATER `waveobj:batchedupdates` that also touches X must
/// never be reordered relative to each other, but an unscoped batch has no
/// single object id to key on — an earlier version of this function keyed
/// updates by their individual `oref`/scope and left unscoped batches in
/// their own group, which let round-robin interleave `[update, delete]` into
/// `[update, delete, update]`-style reorderings; the frontend doesn't
/// version-guard deletes, so a resurrected-then-redeleted object could render
/// as resurrected until reload. See the review discussion on the PR that
/// introduced this function.
fn priority_pane_key(event: &serde_json::Value) -> Option<String> {
    if event["eventtype"] != "rpc" {
        // Raw (non-rpc) events on this lane are always object-model
        // broadcasts (e.g. SetMeta's `waveobj:update`), never blockfile —
        // see `forward_event`'s shape comment.
        return Some(NON_PANE_EVENT_KEY.to_string());
    }
    let inner = event.get("data").and_then(|d| d.get("data"));
    let is_blockfile = inner.and_then(|d| d.get("event")).and_then(|e| e.as_str())
        == Some(crate::backend::mps::EVENT_BLOCK_FILE);
    if !is_blockfile {
        return Some(NON_PANE_EVENT_KEY.to_string());
    }
    inner
        .and_then(|d| d.get("scopes"))
        .and_then(|s| s.get(0))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some(NON_PANE_EVENT_KEY.to_string()))
}

fn enqueue_priority_event(
    order: &mut Vec<String>,
    groups: &mut std::collections::HashMap<String, std::collections::VecDeque<serde_json::Value>>,
    event: serde_json::Value,
) {
    let key = priority_pane_key(&event).unwrap_or_default();
    if !groups.contains_key(&key) {
        order.push(key.clone());
    }
    groups.entry(key).or_default().push_back(event);
}

/// Drain all immediately-available events from `rx` (including `first`) and
/// interleave them round-robin by pane (`priority_pane_key`), instead of
/// forwarding in raw arrival order. Without this, a pane producing output
/// faster than the socket can flush it fills this same lane with its own
/// frames, and a different pane's keystroke-echo frame — queued in the same
/// FIFO — waits behind the entire backlog. Round-robining bounds that wait to
/// one slot per OTHER distinct pane with events currently queued, regardless
/// of how deep any single pane's backlog is.
///
/// Ordering guarantee: events for the SAME pane are never reordered relative
/// to each other (each pane's VecDeque is FIFO); only the INTERLEAVING across
/// different panes changes.
fn fair_drain_priority(
    first: serde_json::Value,
    rx: &mut tokio::sync::mpsc::Receiver<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, std::collections::VecDeque<serde_json::Value>> =
        std::collections::HashMap::new();

    enqueue_priority_event(&mut order, &mut groups, first);
    while let Ok(next) = rx.try_recv() {
        enqueue_priority_event(&mut order, &mut groups, next);
    }

    // Common case — only one pane (or only one event) queued this round.
    // Skip the round-robin bookkeeping and preserve arrival order directly.
    if order.len() <= 1 {
        return order
            .into_iter()
            .filter_map(|k| groups.remove(&k))
            .flat_map(|q| q.into_iter())
            .collect();
    }

    let total: usize = groups.values().map(|q| q.len()).sum();
    let mut out = Vec::with_capacity(total);
    loop {
        let mut progressed = false;
        for key in &order {
            if let Some(q) = groups.get_mut(key) {
                if let Some(ev) = q.pop_front() {
                    out.push(ev);
                    progressed = true;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

/// Handle an incoming text message.
/// Returns Err(true) if the socket send failed.
/// Returns Ok(Some((rx, agent_id))) if a bus:register was processed.
async fn handle_incoming_text(
    text: &str,
    engine: &Arc<WshRpcEngine>,
    state: &AppState,
    socket: &mut WebSocket,
) -> Result<Option<(tokio::sync::mpsc::UnboundedReceiver<crate::backend::messagebus::BusMessage>, String)>, bool> {
    let incoming: WSIncoming = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("ws: invalid JSON: {}", e);
            return Ok(None);
        }
    };

    // Handle ping/pong by type field
    if let Some(ref msg_type) = incoming.msg_type {
        if msg_type.starts_with("bus:") {
            if let Err(error) = super::bus_frames::frame_allowed(&state.mstore, msg_type, &engine.rpc_context().agent_id, (incoming.agent_id.as_deref(), incoming.token.as_deref(), incoming.from.as_deref())) {
                let msg = serde_json::to_string(&json!({ "type": "bus:error", "error": error })).unwrap_or_default();
                return if socket.send(Message::Text(msg.into())).await.is_err() { Err(true) } else { Ok(None) };
            }
        }
        match msg_type.as_str() {
            "ping" => {
                let now = agentmux_common::time::now_ms();
                let pong = json!({ "type": "pong", "stime": now });
                let msg = serde_json::to_string(&pong).unwrap_or_default();
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    return Err(true);
                }
                return Ok(None);
            }
            "pong" => {
                return Ok(None);
            }
            "bus:register" => {
                if let Some(ref claimed) = incoming.agent_id {
                    let Ok(agent_id) = super::bus_frames::proven_slug(&state.mstore, incoming.token.as_deref(), claimed) else { return Ok(None) }; // proven above; registered by slug
                    let rx = state.messagebus.register(&agent_id, "websocket");
                    // Stamp agent_id on the RPC context so MuxAPI S1 checks work.
                    engine.set_rpc_context(crate::backend::rpc_types::RpcContext {
                        agent_id: agent_id.clone(),
                        ..Default::default()
                    });
                    let msg = serde_json::to_string(&json!({ "type": "bus:registered", "agent_id": agent_id })).unwrap_or_default();
                    if socket.send(Message::Text(msg.into())).await.is_err() {
                        return Err(true);
                    }
                    return Ok(Some((rx, agent_id)));
                }
                return Ok(None);
            }
            "bus:send" => {
                if let (Some(ref from), Some(ref to), Some(ref payload)) =
                    (&incoming.from, &incoming.to, &incoming.payload)
                {
                    let priority = super::bus_frames::priority(incoming.priority.as_deref());
                    let bus_msg = crate::backend::messagebus::BusMessage::new(
                        from, to, crate::backend::messagebus::MessageType::Send, payload, priority,
                    );
                    let msg_id = bus_msg.id.clone();
                    let _ = state.messagebus.send(bus_msg);
                    let ack = json!({ "type": "bus:sent", "message_id": msg_id });
                    let msg = serde_json::to_string(&ack).unwrap_or_default();
                    if socket.send(Message::Text(msg.into())).await.is_err() {
                        return Err(true);
                    }
                }
                return Ok(None);
            }
            "bus:inject" => {
                let from = incoming.from.as_deref().unwrap_or("unknown");
                if let (Some(ref target), Some(ref message)) =
                    (&incoming.target, &incoming.bus_message_text)
                {
                    // Try direct PTY injection via ReactiveHandler first, with
                    // the same signature check and transcript-request fields
                    // as /api/bus/inject (`jekt_entry`).
                    let reactive_req = super::jekt_entry::bus_inject_request(
                        state, from, target, message, incoming.priority.clone(), String::new(),
                    );
                    let resp = state.reactive_handler.inject_message(reactive_req.clone());
                    if resp.success {
                        // Sender-side echo (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2),
                        // carrying the verdicts the delivery was decided on.
                        super::reactive::echo_local_delivery(state, &reactive_req, &resp);
                        let ack = json!({ "type": "bus:injected", "via": "pty", "block_id": resp.block_id });
                        let msg = serde_json::to_string(&ack).unwrap_or_default();
                        if socket.send(Message::Text(msg.into())).await.is_err() {
                            return Err(true);
                        }
                        return Ok(None);
                    }

                    // Not a fallback case (`jekt_entry::bus_fallback_allowed`) — report it
                    if !super::jekt_entry::bus_fallback_allowed(&reactive_req, &resp) {
                        let err = json!({ "type": "bus:error", "error": resp.error });
                        let msg = serde_json::to_string(&err).unwrap_or_default();
                        if socket.send(Message::Text(msg.into())).await.is_err() {
                            return Err(true);
                        }
                        return Ok(None);
                    }

                    // Fall back to MessageBus WebSocket push
                    let priority = super::bus_frames::priority(incoming.priority.as_deref());
                    match state.messagebus.inject(from, "", target, message, priority) {
                        Ok(msg_id) => {
                            // Sender-side echo for the messagebus fallback path.
                            // Tier is unknown here (no ReactiveHandler wrap) — None
                            // renders as the default "coord".
                            super::reactive::echo_jekt_to_sender(
                                state,
                                Some(from),
                                target,
                                message,
                                &msg_id,
                                None,
                                None,
                                super::reactive::EchoTrust {
                                    delivery_tier: "host",
                                    sig_verified: None,
                                    reagent_verified: None,
                                    lan_verified: None,
                                    channel_verified: None,
                                },
                                incoming.priority.as_deref().unwrap_or("normal"),
                            );
                            let ack = json!({ "type": "bus:injected", "via": "messagebus", "message_id": msg_id });
                            let msg = serde_json::to_string(&ack).unwrap_or_default();
                            if socket.send(Message::Text(msg.into())).await.is_err() {
                                return Err(true);
                            }
                        }
                        Err(e) => {
                            let err = json!({ "type": "bus:error", "error": e });
                            let msg = serde_json::to_string(&err).unwrap_or_default();
                            if socket.send(Message::Text(msg.into())).await.is_err() {
                                return Err(true);
                            }
                        }
                    }
                }
                return Ok(None);
            }
            "bus:broadcast" => {
                let from = incoming.from.as_deref().unwrap_or("unknown");
                if let Some(ref payload) = incoming.payload {
                    let priority = super::bus_frames::priority(incoming.priority.as_deref());
                    let _ = state.messagebus.broadcast(from, "", payload, priority);
                }
                return Ok(None);
            }
            _ => {}
        }
    }

    // Handle wscommand-based messages
    if let Some(ref wscommand) = incoming.wscommand {
        match wscommand.as_str() {
            "rpc" => {
                if let Some(rpc_msg) = incoming.message {
                    engine.handle_message(rpc_msg);
                } else {
                    tracing::warn!("ws: rpc wscommand missing message field");
                }
            }
            "blockinput" => {
                if let Some(ref block_id) = incoming.blockid {
                    if let Some(ref data64) = incoming.inputdata64 {
                        if !data64.is_empty() {
                            match base64::engine::general_purpose::STANDARD.decode(data64) {
                                Ok(data) => {
                                    if let Err(e) = dispatch_blockinput(block_id, data) {
                                        tracing::debug!("ws: blockinput error: {}", e);
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("ws: blockinput base64 decode error: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            "setblocktermsize" => {
                if let Some(ref block_id) = incoming.blockid {
                    if let Some(ref ts_val) = incoming.termsize {
                        match serde_json::from_value::<TermSize>(ts_val.clone()) {
                            Ok(ts) => {
                                let input = blockcontroller::BlockInputUnion::resize(ts);
                                if let Err(e) = blockcontroller::send_input(block_id, input, None) {
                                    tracing::debug!("ws: setblocktermsize error: {}", e);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("ws: setblocktermsize parse error: {}", e);
                            }
                        }
                    }
                }
            }
            other => {
                tracing::warn!("ws: unknown wscommand: {}", other);
            }
        }
    }

    Ok(None)
}

/// Route a decoded `blockinput` payload — the actual human-keystroke path
/// (`termViewModel.ts`'s `blockinput` WS command, used by both the
/// standalone Terminal widget and an agent pane's composer-drawer shell) —
/// to its block's controller.
///
/// Mirrors `controllerinput`'s own lock check above: drops the input
/// outright while `block_id` is under an active PtyShell agent lease,
/// rather than forwarding it. This was `agent_lock`'s documented "Known
/// gap" — `blockinput`, not `controllerinput`, is where a human's real
/// keystrokes actually travel, and until now nothing server-side stopped
/// one from interleaving with an agent's own `PtyShellInput` write into the
/// same PTY. The frontend's `agentLocked()` gate is still the primary UX
/// layer (stops it before it's even sent), but it is eventually consistent;
/// this is the synchronous backstop against this handler's own state.
fn dispatch_blockinput(block_id: &str, data: Vec<u8>) -> Result<(), String> {
    if blockcontroller::agent_lock::is_locked(block_id) {
        tracing::debug!(block_id = %block_id, "blockinput: dropped — agent lock active");
        return Ok(());
    }
    let input = blockcontroller::BlockInputUnion::data(data);
    blockcontroller::send_input(block_id, input, None)
}

/// Run `f` on `block_id`'s persistent controller for `agent.answer` /
/// `agent.cancel`. The error strings are matched by the frontend's
/// SAFE_TO_RETRY_VIA_FOLLOWUP allowlist (useAgentQuestions.ts).
pub(super) fn with_question_controller<T>(
    command: &str,
    action: &str,
    block_id: &str,
    tool_use_id: &str,
    f: impl FnOnce(&blockcontroller::persistent::PersistentSubprocessController) -> Result<T, String>,
) -> Result<T, String> {
    if tool_use_id.is_empty() {
        return Err(format!("{command}: MISSING_ARG: tool_use_id"));
    }
    let ctrl = blockcontroller::get_controller(block_id)
        .ok_or_else(|| format!("{command}: no controller for block {block_id}"))?;
    match ctrl
        .as_any()
        .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
    {
        Some(persistent_ctrl) => f(persistent_ctrl),
        None => Err(format!(
            "{command}: UNSUPPORTED_CONTROLLER: {action} AskUserQuestion \
             requires a persistent (host) agent; container/one-shot agents are \
             not yet supported (Phase 2)"
        )),
    }
}

/// Parse a CommandBlockInputData into a BlockInputUnion.
pub(super) fn parse_block_input(
    cmd: &CommandBlockInputData,
) -> Result<blockcontroller::BlockInputUnion, String> {
    if !cmd.inputdata64.is_empty() {
        let data = base64::engine::general_purpose::STANDARD
            .decode(&cmd.inputdata64)
            .map_err(|e| format!("controllerinput: base64 decode: {e}"))?;
        return Ok(blockcontroller::BlockInputUnion::data(data));
    }
    if !cmd.signame.is_empty() {
        return Ok(blockcontroller::BlockInputUnion::signal(&cmd.signame));
    }
    if let Some(ref ts_val) = cmd.termsize {
        let ts: TermSize =
            serde_json::from_value(ts_val.clone()).map_err(|e| format!("controllerinput: {e}"))?;
        return Ok(blockcontroller::BlockInputUnion::resize(ts));
    }
    Err("controllerinput: no input data, signal, or termsize".to_string())
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_controller_errors_keep_the_strings_the_frontend_matches() {
        let missing = with_question_controller("agent.cancel", "canceling", "blk", "", |_| Ok(()));
        assert_eq!(missing.unwrap_err(), "agent.cancel: MISSING_ARG: tool_use_id");
        let absent = with_question_controller("agent.answer", "answering", "no-such-block-qc", "tu-1", |_| Ok(()));
        assert_eq!(absent.unwrap_err(), "agent.answer: no controller for block no-such-block-qc");
    }

    /// Every no-argument stub on this connection sends `null`, not `{}`.
    ///
    /// `waveinfo`, `getfullconfig`, `eventunsuball`, `routeannounce` and
    /// `routeunannounce` are all `client.rpcCall("...", null, opts)`. That is
    /// the opposite of the assumption the earlier slices were built on -- those
    /// stubs send `{}` -- and it is why these register `Option<NoArgsReq>`
    /// rather than the bare struct: serde deserializes a struct only from an
    /// object, so a bare `NoArgsReq` would reject every one of these calls
    /// while compiling and passing every gate.
    #[test]
    fn the_no_argument_commands_accept_the_null_their_stubs_send() {
        for payload in [serde_json::Value::Null, serde_json::json!({})] {
            serde_json::from_value::<Option<crate::backend::rpc_types::NoArgsReq>>(payload.clone())
                .unwrap_or_else(|e| panic!("{payload} should deserialize, got {e}"));
        }
        // ...and the bare struct really does reject null, so the Option is
        // load-bearing rather than decoration.
        assert!(
            serde_json::from_value::<crate::backend::rpc_types::NoArgsReq>(serde_json::Value::Null)
                .is_err(),
            "if this ever passes, the Option wrappers here are redundant",
        );
    }

    /// `eventunsub` takes a BARE STRING payload -- the only command in the app
    /// shaped that way. Its stub is `data: string`.
    ///
    /// Registered as `serde_json::Value`, NOT `Option<String>`. reagent caught
    /// that difference on #3348 and was right: the old handler read
    /// `data.as_str().unwrap_or("")`, so a number/object/array was a silent
    /// no-op, while `Option<String>` hard-errors on exactly those. Claiming it
    /// "keeps that tolerance" was false. `Value` keeps it for real.
    ///
    /// This test now covers "anything else" rather than only null, which is
    /// what its name always promised.
    #[test]
    fn eventunsub_takes_a_bare_string_and_tolerates_anything_else() {
        // Mirrors the handler exactly: deserialize as Value, then as_str().
        let name = |v: serde_json::Value| {
            serde_json::from_value::<serde_json::Value>(v)
                .expect("Value deserializes from anything")
                .as_str()
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(name(serde_json::json!("blockfile:mux")), "blockfile:mux");
        // Each of these was a silent no-op before the migration and must stay one.
        for junk in [
            serde_json::Value::Null,
            serde_json::json!(42),
            serde_json::json!({ "event": "x" }),
            serde_json::json!(["x"]),
            serde_json::json!(true),
        ] {
            assert_eq!(name(junk.clone()), "", "{junk} must unsubscribe nothing, not error");
        }
    }

    /// Codex P1 on PR #3194: the frontend's own lock gate
    /// (`AgentShellSubblock.tsx` dropping `sendDataHandler` while
    /// `term:agentlockuntil` is in the future) is necessarily eventually-
    /// consistent — it reacts to a WS-pushed meta value, not something
    /// synchronous with the backend's own state — so a human keystroke
    /// already in flight could otherwise still reach `send_input` and
    /// interleave with an agent's own write. `controllerinput` must reject
    /// it itself, checked against the backend's own current state at the
    /// moment it actually runs.
    ///
    /// Distinguishes "silently dropped by the lock gate" from "attempted
    /// and failed for lack of a live controller" by the response's `error`
    /// field: a target block here has no real controller registered at
    /// all, so an UNLOCKED call reaches `blockcontroller::send_input` and
    /// gets a real `"no controller for block"` error back — proving the
    /// call path is live. A LOCKED call must short-circuit before that
    /// point, returning cleanly with no error.
    #[tokio::test]
    async fn controllerinput_is_dropped_while_an_agent_lock_is_active() {
        use crate::backend::rpc::engine::WshRpcEngine;
        use crate::backend::rpc_types::RpcMessage;
        use crate::server::tests::test_state;

        let state = test_state();
        let (engine, mut output_rx) = WshRpcEngine::new();
        register_handlers(&engine, state.clone(), "test-conn".to_string());

        async fn send_input(
            engine: &std::sync::Arc<WshRpcEngine>,
            output_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
            block_id: &str,
        ) -> RpcMessage {
            engine.handle_message(RpcMessage {
                command: COMMAND_CONTROLLER_INPUT.to_string(),
                reqid: uuid::Uuid::new_v4().to_string(),
                data: Some(serde_json::json!({ "blockid": block_id, "inputdata64": "eQ==" })), // "y"
                ..Default::default()
            });
            tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
                .await
                .expect("handler should respond within 2s")
                .expect("response channel should not close")
        }

        // Locked target: no live controller registered, but the check must
        // never reach that far. The lease is seeded through
        // `blockcontroller::agent_lock` — the in-memory registry this
        // handler actually enforces against — NOT through the block's
        // `term:agentlockuntil` meta, which is now the frontend-badge copy
        // only (see the handler's own comment and `agent_lock`'s module doc:
        // the meta read this replaced put a synchronous SQLite query on the
        // per-keystroke path).
        blockcontroller::agent_lock::lock_until(
            "ci-locked-shell",
            agentmux_common::time::now_ms() + 60_000,
        );
        let resp = send_input(&engine, &mut output_rx, "ci-locked-shell").await;
        assert!(
            resp.error.is_empty(),
            "a locked target must be silently dropped, not reach send_input at all: {}",
            resp.error
        );

        // Unlocked target (no lease at all) — proves the call path above is
        // genuinely live: it must reach `send_input` and get a real "no
        // controller" error, not silently succeed.
        let resp = send_input(&engine, &mut output_rx, "ci-unlocked-shell").await;
        assert!(
            resp.error.contains("no controller"),
            "an unlocked target must reach send_input for real: {}",
            resp.error
        );

        // A stale `term:agentlockuntil` in meta must NOT lock anything by
        // itself. This is the fail-open half of moving enforcement into
        // memory (`agent_lock`'s module doc): meta can outlive the process
        // that wrote it, memory cannot, and a lease that survives a restart
        // is a human who can't type `exit` in their own terminal.
        let mut stale_meta_block = crate::backend::obj::Block {
            oid: "ci-stale-meta-shell".to_string(),
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert(
                    "term:agentlockuntil".to_string(),
                    serde_json::json!(agentmux_common::time::now_ms() + 60_000),
                );
                m
            },
            ..Default::default()
        };
        state.mstore.insert(&mut stale_meta_block).unwrap();
        let resp = send_input(&engine, &mut output_rx, "ci-stale-meta-shell").await;
        assert!(
            resp.error.contains("no controller"),
            "a lease that exists only in stale meta must not lock the keyboard: {}",
            resp.error
        );

        blockcontroller::agent_lock::release("ci-locked-shell");
    }

    /// Regression test for the gap `agent_lock`'s own module doc calls out
    /// by name ("Known gap this does NOT close"): unlike `controllerinput`
    /// above (Stop/SIGINT/resize), `blockinput` is the ACTUAL human-keystroke
    /// path (`termViewModel.ts`, both the standalone Terminal widget and an
    /// agent pane's composer-drawer shell) and had no lease check at all —
    /// the frontend's own `agentLocked()` gate was the only thing standing
    /// between a human keystroke and an agent's own `PtyShellInput` write
    /// landing in the same PTY, and that gate is eventually consistent (it
    /// reacts to a WS-pushed meta value), not synchronous with this
    /// handler's own state. A keystroke slipping through that window
    /// interleaves at the byte level with whatever the agent is mid-write
    /// to — vim's cursor-addressed alternate-screen redraw does not degrade
    /// gracefully under that; it renders as a visibly garbled pane, not a
    /// hang. This is exactly what corrupted a live pane during a vim
    /// investigation on 2026-09-16.
    ///
    /// Same "no controller registered" trick as the `controllerinput` test
    /// above: a LOCKED call must short-circuit before ever reaching
    /// `blockcontroller::send_input`, returning `Ok(())` with no error even
    /// though no controller exists for this block id. An UNLOCKED call DOES
    /// reach `send_input` and gets back a real `"no controller for block"`
    /// error — proving the call path is live, not merely never exercised.
    #[test]
    fn blockinput_is_dropped_while_an_agent_lock_is_active() {
        let locked_id = "ws-blockinput-lock-test-locked";
        blockcontroller::agent_lock::lock_until(locked_id, agentmux_common::time::now_ms() + 60_000);
        let result = dispatch_blockinput(locked_id, b"y".to_vec());
        assert!(
            result.is_ok(),
            "a locked blockinput call must be dropped cleanly, not forwarded to send_input: {result:?}"
        );
        blockcontroller::agent_lock::release(locked_id);

        let unlocked_id = "ws-blockinput-lock-test-unlocked";
        let result = dispatch_blockinput(unlocked_id, b"y".to_vec());
        assert_eq!(
            result,
            Err(format!("no controller for block {unlocked_id}")),
            "an unlocked blockinput call must still reach send_input for real"
        );
    }

    /// The lease must stop dropping input the moment it lapses, with nobody
    /// calling `PtyShellStop` — an agent that crashed mid-write, or simply
    /// stopped writing, must not leave the human unable to type into their
    /// own shell (up to and including typing `exit` to close the pane,
    /// SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §10).
    #[tokio::test]
    async fn controllerinput_flows_again_once_the_agent_lease_lapses() {
        use crate::backend::rpc::engine::WshRpcEngine;
        use crate::backend::rpc_types::RpcMessage;
        use crate::server::tests::test_state;

        let state = test_state();
        let (engine, mut output_rx) = WshRpcEngine::new();
        register_handlers(&engine, state, "test-conn-lapse".to_string());

        let block_id = "ci-lapsing-shell";
        blockcontroller::agent_lock::lock_until(block_id, agentmux_common::time::now_ms() + 40);

        let send = |engine: std::sync::Arc<WshRpcEngine>| {
            engine.handle_message(RpcMessage {
                command: COMMAND_CONTROLLER_INPUT.to_string(),
                reqid: uuid::Uuid::new_v4().to_string(),
                data: Some(serde_json::json!({ "blockid": block_id, "inputdata64": "eQ==" })),
                ..Default::default()
            });
        };

        send(engine.clone());
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .expect("handler should respond within 2s")
            .expect("response channel should not close");
        assert!(resp.error.is_empty(), "still leased: input must be dropped");

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        send(engine.clone());
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .expect("handler should respond within 2s")
            .expect("response channel should not close");
        assert!(
            resp.error.contains("no controller"),
            "a lapsed lease must let input through without any explicit release: {}",
            resp.error
        );
    }

    fn rpc_event(block_id: &str) -> serde_json::Value {
        json!({
            "eventtype": "rpc",
            "data": {
                "command": "eventrecv",
                "data": {
                    "event": "blockfile",
                    "scopes": [format!("block:{block_id}")],
                    "data": {"data64": "aGk="},
                },
            },
        })
    }

    fn raw_waveobj_update(oref: &str) -> serde_json::Value {
        json!({
            "eventtype": "waveobj:update",
            "oref": oref,
            "data": {},
        })
    }

    fn batched_updates_event() -> serde_json::Value {
        json!({
            "eventtype": "waveobj:batchedupdates",
            "data": [],
        })
    }

    #[test]
    fn test_priority_pane_key_from_rpc_blockfile_scopes() {
        let event = rpc_event("abc123");
        assert_eq!(priority_pane_key(&event), Some("block:abc123".to_string()));
    }

    /// Raw (non-rpc) object-model broadcasts are NEVER split by pane —
    /// splitting them would let round-robin reorder them relative to other
    /// object updates for the same object (see `priority_pane_key`'s doc
    /// comment for the concrete resurrected-object failure this prevents).
    #[test]
    fn test_priority_pane_key_raw_waveobj_update_is_non_pane() {
        let event = raw_waveobj_update("block:xyz789");
        assert_eq!(priority_pane_key(&event), Some(NON_PANE_EVENT_KEY.to_string()));
    }

    #[test]
    fn test_priority_pane_key_non_pane_for_batched_updates() {
        let event = batched_updates_event();
        assert_eq!(priority_pane_key(&event), Some(NON_PANE_EVENT_KEY.to_string()));
    }

    /// The core regression this exists to prevent: a noisy pane (A) producing
    /// many queued frames must not delay a quiet pane's (B) single frame by
    /// the full length of A's backlog. Round-robin interleaving means B's
    /// frame comes out within one "round" (bounded by the number of DISTINCT
    /// panes with queued output), not after every one of A's frames.
    #[tokio::test]
    async fn test_fair_drain_priority_interleaves_noisy_and_quiet_panes() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);

        // Pane A floods 10 frames; pane B contributes exactly 1, queued
        // in the middle of A's backlog (mirrors real arrival: B's keystroke
        // echo lands while A's flood is still being drained into the lane).
        for i in 0..5 {
            tx.try_send(rpc_event("A")).unwrap();
            let _ = i;
        }
        tx.try_send(rpc_event("B")).unwrap();
        for _ in 0..5 {
            tx.try_send(rpc_event("A")).unwrap();
        }

        let first = rx.recv().await.unwrap();
        let drained = fair_drain_priority(first, &mut rx);

        assert_eq!(drained.len(), 11, "all 11 queued events must be returned, none dropped");

        // Find B's position in the fairly-drained output — it must appear
        // near the front (within the first "round" across 2 distinct panes),
        // not at position 6 where naive FIFO would place it.
        let b_pos = drained
            .iter()
            .position(|e| priority_pane_key(e).as_deref() == Some("block:B"))
            .expect("pane B's event must be present");
        assert!(
            b_pos <= 1,
            "pane B's frame should be at most 1 slot behind pane A in round-robin order, was at position {b_pos}"
        );
    }

    #[tokio::test]
    async fn test_fair_drain_priority_single_pane_preserves_order() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        for _ in 0..4 {
            tx.try_send(rpc_event("A")).unwrap();
        }
        let first = rx.recv().await.unwrap();
        let drained = fair_drain_priority(first, &mut rx);
        assert_eq!(drained.len(), 4);
    }

    #[tokio::test]
    async fn test_fair_drain_priority_no_events_lost_across_many_panes() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let pane_ids = ["A", "B", "C", "D"];
        for (i, pane) in pane_ids.iter().cycle().take(40).enumerate() {
            let _ = i;
            tx.try_send(rpc_event(pane)).unwrap();
        }
        let first = rx.recv().await.unwrap();
        let drained = fair_drain_priority(first, &mut rx);
        assert_eq!(drained.len(), 40);
    }

    /// Regression test for the object-ordering bug caught in review: a
    /// `waveobj:update` for object X followed by a LATER
    /// `waveobj:batchedupdates` that also touches X must come out in the
    /// same relative order they arrived in. An earlier version of
    /// `priority_pane_key` keyed the individual update by its oref and left
    /// the unscoped batch in its own group, so round-robin turned
    /// `[update1, update2, batch]` into `[update1, batch, update2]` —
    /// resurrecting a deleted object on the frontend until reload (the
    /// frontend doesn't version-guard deletes). Object-model events must
    /// never be split by pane key, only `blockfile` (terminal output) events
    /// may be.
    #[tokio::test]
    async fn test_fair_drain_priority_preserves_object_update_order() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);

        let update1 = raw_waveobj_update("tab:X");
        let update2 = raw_waveobj_update("tab:X");
        let batch_delete = batched_updates_event();

        tx.try_send(update1.clone()).unwrap();
        tx.try_send(update2.clone()).unwrap();
        tx.try_send(batch_delete.clone()).unwrap();

        let first = rx.recv().await.unwrap();
        let drained = fair_drain_priority(first, &mut rx);

        assert_eq!(drained, vec![update1, update2, batch_delete], "object-model events must stay in pure arrival order, never reordered by round-robin");
    }

    #[test]
    fn srv_info_event_carries_the_info_on_the_eventrecv_path() {
        let ev = super::srv_info_event(crate::srv_info::srv_info("1.2.3", "box"));
        assert_eq!(ev["eventtype"], "rpc");
        assert_eq!(ev["data"]["command"], "eventrecv");
        assert_eq!(ev["data"]["data"]["event"], "srvinfo");
        assert_eq!(ev["data"]["data"]["data"]["version"], "1.2.3");
    }
}
