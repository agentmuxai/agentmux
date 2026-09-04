// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandBlockInputData, CommandControllerResyncData, CommandCreateSubBlockData,
    CommandDeleteSubBlockData, CommandEventReadHistoryData,
    CommandGetMetaData, CommandSetMetaData, CommandToolDecisionData,
    RpcMessage, COMMAND_CONTROLLER_INPUT,
    COMMAND_CONTROLLER_RESYNC, COMMAND_CREATE_SUB_BLOCK, COMMAND_DELETE_SUB_BLOCK,
    COMMAND_EVENT_READ_HISTORY, COMMAND_EVENT_SUB, COMMAND_EVENT_UNSUB,
    COMMAND_EVENT_UNSUB_ALL, COMMAND_GET_FULL_CONFIG, COMMAND_GET_META,
    COMMAND_GET_AI_RATE_LIMIT, COMMAND_ROUTE_ANNOUNCE, COMMAND_ROUTE_UNANNOUNCE,
    COMMAND_SET_META, COMMAND_SET_CONFIG, COMMAND_APP_INFO,
    COMMAND_TOOL_DECISION, COMMAND_AGENT_ANSWER, COMMAND_AGENT_CANCEL,
    CommandAgentAnswerData, CommandAgentCancelData,
    COMMAND_DOCK_NODE_STATUS, CommandDockNodeStatusData,
    COMMAND_BACKGROUND_TASK_COMPLETION, CommandBackgroundTaskCompletionData,
    COMMAND_BACKGROUND_TASK_PID, CommandBackgroundTaskPidData,
    COMMAND_LIST_BACKGROUND_TASKS, CommandListBackgroundTasksData,
};
use crate::backend::base::normalize_working_dir;
use crate::backend::obj::{Block, TermSize, WaveObjUpdate, wave_obj_to_value};
use super::service::{update_object_meta, schedule_agent_zoom_mirror};

use super::AppState;

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
    agent_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    target: Option<String>,
    payload: Option<String>,
    #[serde(rename = "bus_message")]
    bus_message_text: Option<String>,
    priority: Option<String>,
}

pub(super) async fn handle_ws(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_ws_connection(socket, state))
}

async fn handle_ws_connection(mut socket: WebSocket, state: AppState) {
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
            let config_event = json!({
                "eventtype": "rpc",
                "data": {
                    "command": "eventrecv",
                    "data": {
                        "event": "config",
                        "data": { "fullconfig": config_val }
                    }
                }
            });
            if let Ok(msg) = serde_json::to_string(&config_event) {
                let _ = socket.send(Message::Text(msg.into())).await;
            }
        }
        tracing::info!("[ws-perf] send_initial_config: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0);
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

    'ws: loop {
        tokio::select! {
            // Biased: poll branches top-to-bottom so interactive terminal I/O
            // always wins over droppable perf telemetry. Order = incoming
            // keystrokes → RPC replies → agent bus → priority events (terminal
            // echo) → background events (sysinfo/blockstats) → keepalive ping.
            // See docs/specs/SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md.
            biased;

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
            //   1. WPS Broker (via EventBusBridge) — already wrapped as
            //      { eventtype: "rpc", data: { command: "eventrecv", data: WaveEvent } }
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
            Some(event) = priority_rx.recv() => {
                let fair = fair_drain_priority(event, &mut priority_rx);
                for ev in fair {
                    if forward_event(&mut socket, ev).await {
                        break 'ws;
                    }
                }
            }

            // Background event lane → WebSocket. Droppable perf telemetry
            // (sysinfo/blockstats) only; serviced when the priority lanes above
            // are momentarily idle, so it can never delay a keystroke echo.
            //
            // Tradeoff of `biased;`: a SUSTAINED priority-lane flood (e.g.
            // `! yes` pumping terminal output continuously) keeps the priority
            // branches ready, so this lane is starved and events queue here
            // until the flood pauses, then flush as a stale burst. As of B.2
            // (SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02) this is a
            // bounded `mpsc::channel(BACKGROUND_LANE_CAPACITY)` with
            // `try_send` drop-on-full semantics (see eventbus.rs), not an
            // unbounded receiver — a starvation period long enough to fill
            // all 256 slots drops each new event as it arrives (the queued
            // events already in the channel are unaffected) rather than
            // growing commit without limit. Acceptable for now because telemetry
            // ingress is low-rate (sysinfo ~1/s) and a flood also saturates the
            // socket writes — during which the priority lanes do drain, so this
            // lane gets serviced — making true never-empty starvation the rare
            // Phase 2: coalesce all immediately-available background events to the
            // latest reading per (event-name:scope) key before forwarding. During
            // sustained priority-lane activity, sysinfo/blockstats queue here; without
            // coalescing they flush as a burst of stale frames that the browser
            // processes back-to-back, delaying queued keypress events. Coalescing
            // bounds the flush to O(distinct event×scope pairs) — at most 1 sysinfo +
            // 1 blockstats per open block — regardless of how long the lane was starved.
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
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
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

    // Unregister from messagebus if this connection was an agent
    if let Some(ref agent_id) = bus_agent_id {
        state.messagebus.unregister(agent_id);
    }
}

/// Forward a queued event-bus value to the WebSocket. Returns `true` if the
/// send failed (the caller should break the loop).
///
/// Two shapes arrive: already-RPC-wrapped values (from the WPS broker via
/// EventBusBridge) are forwarded as-is; raw event-bus values (e.g. SetMeta's
/// `waveobj:update`) are wrapped as an RPC `eventrecv` so the frontend
/// WshRouter routes them to handleWaveEvent → updateWaveObject. Shared by both
/// the priority and background egress lanes.
async fn forward_event(socket: &mut WebSocket, event: serde_json::Value) -> bool {
    let msg = if event["eventtype"] == "rpc" {
        // Already an RPC message (from WPS broker via EventBusBridge)
        serde_json::to_string(&event).unwrap_or_default()
    } else {
        // Raw event bus event — wrap as RPC eventrecv
        let wave_event = json!({
            "event": event["eventtype"],
            "scopes": [event["oref"]],
            "data": event["data"],
        });
        let wrapped = json!({
            "eventtype": "rpc",
            "data": {
                "command": "eventrecv",
                "data": wave_event,
            },
        });
        serde_json::to_string(&wrapped).unwrap_or_default()
    };
    socket.send(Message::Text(msg.into())).await.is_err()
}

/// Extract a coalescing key from a background-lane event.
/// Background events are RPC-wrapped WPS events:
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

/// Extract a fairness key (the pane/block a priority-lane event belongs to)
/// so a flood of output from one pane can't queue an unbounded number of
/// frames ahead of a different pane's own keystroke echo sitting in the same
/// lane. Mirrors the two shapes `forward_event` already handles: RPC-wrapped
/// WPS events (from the broker) carry `scopes: ["block:<id>", ...]` inside
/// the nested WaveEvent; raw direct broadcasts (e.g. SetMeta's
/// `waveobj:update`) carry `oref: "block:<id>"` at the top level. Events with
/// no identifiable scope (e.g. the initial `config` push, batched multi-object
/// updates) return `None` — `fair_drain_priority` groups those under one
/// shared key, so they still round-robin fairly as one more participant
/// rather than jumping the queue or starving.
fn priority_pane_key(event: &serde_json::Value) -> Option<String> {
    if event["eventtype"] == "rpc" {
        event
            .get("data")
            .and_then(|d| d.get("data"))
            .and_then(|d| d.get("scopes"))
            .and_then(|s| s.get(0))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
    } else {
        event
            .get("oref")
            .and_then(|o| o.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }
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
        match msg_type.as_str() {
            "ping" => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
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
                if let Some(ref agent_id) = incoming.agent_id {
                    let rx = state.messagebus.register(agent_id, "websocket");
                    // Stamp agent_id on the RPC context so App API S1 checks work.
                    engine.set_rpc_context(crate::backend::rpc_types::RpcContext {
                        agent_id: agent_id.clone(),
                        ..Default::default()
                    });
                    let ack = json!({ "type": "bus:registered", "agent_id": agent_id });
                    let msg = serde_json::to_string(&ack).unwrap_or_default();
                    if socket.send(Message::Text(msg.into())).await.is_err() {
                        return Err(true);
                    }
                    return Ok(Some((rx, agent_id.clone())));
                }
                return Ok(None);
            }
            "bus:send" => {
                if let (Some(ref from), Some(ref to), Some(ref payload)) =
                    (&incoming.from, &incoming.to, &incoming.payload)
                {
                    let priority = match incoming.priority.as_deref() {
                        Some("high") => crate::backend::messagebus::Priority::High,
                        Some("urgent") => crate::backend::messagebus::Priority::Urgent,
                        _ => crate::backend::messagebus::Priority::Normal,
                    };
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
                    // Try direct PTY injection via ReactiveHandler first
                    let mut reactive_req = crate::backend::reactive::InjectionRequest {
                        target_agent: target.clone(),
                        message: message.clone(),
                        source_agent: Some(from.to_string()),
                        request_id: None,
                        priority: incoming.priority.clone(),
                        wait_for_idle: false,
                        jekt_tier: None,   // auto-detected from keywords
                        delivery_tier: Some("host".to_string()),
                        forward_hops: 0,
                        ..Default::default()
                    };
                    // reagentx P0 on PR #2565: same bypass as
                    // messagebus.rs::handle_inject — this WS message type
                    // built InjectionRequest from client-controlled
                    // `incoming.from` and called inject_message directly,
                    // never going through host-tier signature verification.
                    super::reactive::verify_jekt_signature(state, &mut reactive_req);
                    let resp = state.reactive_handler.inject_message(reactive_req);
                    if resp.success {
                        // Sender-side echo (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2)
                        super::reactive::echo_jekt_to_sender(
                            state,
                            Some(from),
                            target,
                            message,
                            &resp.request_id,
                            resp.effective_tier.as_deref(),
                            resp.requires_stop,
                            "host",
                            None,
                            // Always host-tier here (this is the pane's own outbound
                            // send) — reagent/lan verification never applies.
                            None,
                            None,
                            incoming.priority.as_deref().unwrap_or("normal"),
                        );
                        let ack = json!({ "type": "bus:injected", "via": "pty", "block_id": resp.block_id });
                        let msg = serde_json::to_string(&ack).unwrap_or_default();
                        if socket.send(Message::Text(msg.into())).await.is_err() {
                            return Err(true);
                        }
                        return Ok(None);
                    }

                    // Non-"agent not found" error — report it
                    let is_not_found = resp.error.as_deref().map(|e| e.contains("not found")).unwrap_or(false);
                    if !is_not_found {
                        let err = json!({ "type": "bus:error", "error": resp.error });
                        let msg = serde_json::to_string(&err).unwrap_or_default();
                        if socket.send(Message::Text(msg.into())).await.is_err() {
                            return Err(true);
                        }
                        return Ok(None);
                    }

                    // Fall back to MessageBus WebSocket push
                    let priority = match incoming.priority.as_deref() {
                        Some("high") => crate::backend::messagebus::Priority::High,
                        Some("urgent") => crate::backend::messagebus::Priority::Urgent,
                        _ => crate::backend::messagebus::Priority::Normal,
                    };
                    match state.messagebus.inject(from, target, message, priority) {
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
                                "host",
                                None,
                                None,
                                None,
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
                    let priority = match incoming.priority.as_deref() {
                        Some("high") => crate::backend::messagebus::Priority::High,
                        Some("urgent") => crate::backend::messagebus::Priority::Urgent,
                        _ => crate::backend::messagebus::Priority::Normal,
                    };
                    let _ = state.messagebus.broadcast(from, payload, priority);
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
                                    let input = blockcontroller::BlockInputUnion::data(data);
                                    if let Err(e) = blockcontroller::send_input(block_id, input, None) {
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

/// Notify subscribers that `block_id`'s `db_background_tasks` state
/// changed (observed, pid recorded, or completed), so the frontend can
/// re-query `COMMAND_LIST_BACKGROUND_TASKS` instead of polling. Live-only
/// (`persist: 0`, mirroring `process_tracker::registry`'s `emit()` for
/// `agent:process-added`/`-exited`) — a late subscriber gets the current
/// state via the mount-time list query, not event replay. Deliberately
/// carries no task data itself (just an invalidation signal): the list
/// query is the single source of truth for the actual rows, so there's
/// nothing to keep in sync between two payload shapes. See
/// docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.2.
fn publish_background_task_updated(broker: &crate::backend::wps::Broker, block_id: &str) {
    broker.publish(crate::backend::wps::WaveEvent {
        event: "background-task-updated".to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({ "block_id": block_id })),
    });
}

fn register_handlers(engine: &Arc<WshRpcEngine>, state: AppState, conn_id: String) {
    // getfullconfig → return full config as JSON
    let config_watcher = state.config_watcher.clone();
    engine.register_handler(
        COMMAND_GET_FULL_CONFIG,
        Box::new(move |_data, _ctx| {
            let cw = config_watcher.clone();
            Box::pin(async move {
                let config = cw.get_full_config();
                match serde_json::to_value(config.as_ref()) {
                    Ok(mut v) => {
                        crate::backend::wconfig::redact_full_config_for_renderer(&mut v);
                        Ok(Some(v))
                    }
                    Err(e) => Err(format!("failed to serialize config: {}", e)),
                }
            })
        }),
    );

    // routeannounce → log + no-op (fire-and-forget, may have no reqid)
    engine.register_handler(
        COMMAND_ROUTE_ANNOUNCE,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                tracing::debug!("routeannounce: {:?}", data);
                Ok(None)
            })
        }),
    );

    // routeunannounce → no-op
    engine.register_handler(
        COMMAND_ROUTE_UNANNOUNCE,
        Box::new(|_data, _ctx| Box::pin(async move { Ok(None) })),
    );

    // eventsub → register subscription with the WPS broker
    let broker_sub = state.broker.clone();
    let conn_id_sub = conn_id.clone();
    engine.register_handler(
        COMMAND_EVENT_SUB,
        Box::new(move |data, _ctx| {
            let broker = broker_sub.clone();
            let conn_id = conn_id_sub.clone();
            Box::pin(async move {
                let sub: crate::backend::wps::SubscriptionRequest =
                    serde_json::from_value(data).map_err(|e| format!("eventsub: {e}"))?;
                tracing::debug!("eventsub: event={} scopes={:?} allscopes={}", sub.event, sub.scopes, sub.allscopes);
                broker.subscribe(&conn_id, sub);
                Ok(None)
            })
        }),
    );

    // eventunsub → unsubscribe from the WPS broker
    let broker_unsub = state.broker.clone();
    let conn_id_unsub = conn_id.clone();
    engine.register_handler(
        COMMAND_EVENT_UNSUB,
        Box::new(move |data, _ctx| {
            let broker = broker_unsub.clone();
            let conn_id = conn_id_unsub.clone();
            Box::pin(async move {
                let event_name = data.as_str().unwrap_or("").to_string();
                if !event_name.is_empty() {
                    broker.unsubscribe(&conn_id, &event_name);
                }
                Ok(None)
            })
        }),
    );

    // eventunsuball → unsubscribe all from the WPS broker
    let broker_unsub_all = state.broker.clone();
    let conn_id_unsub_all = conn_id.clone();
    engine.register_handler(
        COMMAND_EVENT_UNSUB_ALL,
        Box::new(move |_data, _ctx| {
            let broker = broker_unsub_all.clone();
            let conn_id = conn_id_unsub_all.clone();
            Box::pin(async move {
                broker.unsubscribe_all(&conn_id);
                Ok(None)
            })
        }),
    );

    // setmeta → update object metadata in the DB, broadcast update event
    let wstore_sm = state.wstore.clone();
    let event_bus_sm = state.event_bus.clone();
    engine.register_handler(
        COMMAND_SET_META,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sm.clone();
            let event_bus = event_bus_sm.clone();
            Box::pin(async move {
                let cmd: CommandSetMetaData =
                    serde_json::from_value(data).map_err(|e| format!("setmeta: {e}"))?;
                let oref_str = cmd.oref.to_string();
                let meta_keys: Vec<&String> = cmd.meta.keys().collect();
                // debug, not info: fires on every metadata write (zoom, title,
                // view-state edits); at info this became a meaningful slice of
                // an unrotated launcher-log mirror (SPEC_WIN10_PAGEFILE_OOM_
                // CRASH_2026_06_29 P1). Default production filter is info, so
                // this is now suppressed unless RUST_LOG=debug is set.
                tracing::debug!(oref = %oref_str, keys = ?meta_keys, "SetMeta");
                update_object_meta(&wstore, &oref_str, &cmd.meta)?;
                // Per-agent zoom persistence (SPEC_AGENT_ZOOM_PERSISTENCE): the
                // frontend writes term:zoom via this WebSocket path, not the HTTP
                // UpdateObjectMeta handler where the mirror was originally placed.
                // Mirror term:zoom → ui:zoom here so the zoom survives pane close.
                let oref_parsed = crate::backend::ORef::parse(&oref_str)
                    .map_err(|e| e.to_string())?;
                if oref_parsed.otype == "block" && cmd.meta.contains_key("term:zoom") {
                    if let Ok(block) = wstore.must_get::<Block>(&oref_parsed.oid) {
                        let agent_id = block.meta.get("agentId")
                            .and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if !agent_id.is_empty() {
                            let zoom = cmd.meta.get("term:zoom").and_then(|v| v.as_f64());
                            schedule_agent_zoom_mirror(wstore.clone(), agent_id, zoom);
                        }
                    }
                }
                // Read the updated object and broadcast a proper WaveObjUpdate
                // so all WS clients refresh their atoms with the new data.
                let oref = crate::backend::ORef::parse(&oref_str)
                    .map_err(|e| e.to_string())?;
                let update_data = if oref.otype == "block" {
                    if let Ok(block) = wstore.must_get::<Block>(&oref.oid) {
                        Some(serde_json::to_value(&WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: oref.otype.clone(),
                            oid: oref.oid.clone(),
                            obj: Some(wave_obj_to_value(&block)),
                        }).unwrap_or_default())
                    } else { None }
                } else { None };
                event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref: oref_str,
                    data: update_data,
                });
                Ok(None)
            })
        }),
    );

    // getmeta → return metadata for a wave object
    let wstore_gm = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_META,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gm.clone();
            Box::pin(async move {
                let cmd: CommandGetMetaData =
                    serde_json::from_value(data).map_err(|e| format!("getmeta: {e}"))?;
                let obj: Option<serde_json::Value> = wstore
                    .get_raw(&cmd.oref.otype, &cmd.oref.oid)
                    .map_err(|e| format!("getmeta: {e}"))?;
                match obj {
                    Some(val) => {
                        // Return the "meta" field if present, otherwise the full object
                        let meta = val.get("meta").cloned().unwrap_or(val);
                        Ok(Some(meta))
                    }
                    None => Err(format!("getmeta: object {} not found", cmd.oref)),
                }
            })
        }),
    );

    // waveinfo → return version and build info
    let version_info = state.version.clone();
    engine.register_handler(
        COMMAND_APP_INFO,
        Box::new(move |_data, _ctx| {
            let version = version_info.clone();
            Box::pin(async move {
                Ok(Some(serde_json::json!({
                    "version": version,
                })))
            })
        }),
    );

    // getwaveairatelimit → AgentMux has no rate limits; return unlimited/unknown
    engine.register_handler(
        COMMAND_GET_AI_RATE_LIMIT,
        Box::new(|_data, _ctx| {
            Box::pin(async move {
                Ok(Some(serde_json::json!({
                    "req": 9999,
                    "reqlimit": 9999,
                    "preq": 9999,
                    "preqlimit": 9999,
                    "resetepoch": 0,
                    "unknown": true
                })))
            })
        }),
    );

    // controllerresync → load block from DB, create/restart controller with PTY
    let wstore_resync = state.wstore.clone();
    let broker_resync = state.broker.clone();
    let event_bus_resync = state.event_bus.clone();
    let filestore_resync = state.filestore.clone();
    let boot_id_resync = state.boot_id.clone();
    engine.register_handler(
        COMMAND_CONTROLLER_RESYNC,
        Box::new(move |data, _ctx| {
            let wstore = wstore_resync.clone();
            let broker = broker_resync.clone();
            let event_bus = event_bus_resync.clone();
            let filestore = filestore_resync.clone();
            let boot_id = boot_id_resync.clone();
            Box::pin(async move {
                let cmd: CommandControllerResyncData = serde_json::from_value(data)
                    .map_err(|e| format!("controllerresync: {e}"))?;
                tracing::info!(
                    block_id = %cmd.blockid,
                    tab_id = %cmd.tabid,
                    forcerestart = cmd.forcerestart,
                    "ControllerResync"
                );
                let block: Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("controllerresync: load block: {e}"))?
                    .ok_or_else(|| format!("controllerresync: block {} not found", cmd.blockid))?;
                let registry = wstore.shared_agent_registry();
                blockcontroller::resync_controller(
                    &block,
                    &cmd.tabid,
                    cmd.rtopts,
                    cmd.forcerestart,
                    Some(broker),
                    Some(event_bus),
                    Some(wstore),
                    Some(filestore),
                    registry,
                    boot_id,
                )?;
                Ok(None)
            })
        }),
    );

    // controllerinput → route keyboard input / signals / resize to block controller
    engine.register_handler(
        COMMAND_CONTROLLER_INPUT,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandBlockInputData = serde_json::from_value(data)
                    .map_err(|e| format!("controllerinput: {e}"))?;
                let input = parse_block_input(&cmd)?;
                blockcontroller::send_input(&cmd.blockid, input, cmd.seq)?;
                Ok(None)
            })
        }),
    );

    // createsubblock → create a headless sub-block (no tab/layout entry)
    // parented to an existing block, e.g. a `term`-view PTY embedded in an
    // agent pane's details drawer.
    let wstore_csb = state.wstore.clone();
    engine.register_handler(
        COMMAND_CREATE_SUB_BLOCK,
        Box::new(move |data, _ctx| {
            let wstore = wstore_csb.clone();
            Box::pin(async move {
                let cmd: CommandCreateSubBlockData = serde_json::from_value(data)
                    .map_err(|e| format!("createsubblock: {e}"))?;

                let mut meta = cmd.blockdef.meta.clone();
                let raw_cwd = meta
                    .get(blockcontroller::META_KEY_CMD_CWD)
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                if let Some(raw_cwd) = raw_cwd {
                    // The PTY spawn path (blockcontroller/shell/lifecycle.rs)
                    // reads cmd:cwd raw with no MSYS→Windows conversion, unlike
                    // shellexec — normalize here so a Git-Bash-style path
                    // doesn't hit os error 267 on Windows. Mirrors
                    // server/mod.rs's shellexec cwd-fallback precedent: an
                    // invalid/non-absolute value degrades to no cwd rather
                    // than failing the whole call — this is a best-effort
                    // convenience derived from the parent's meta, not a
                    // caller-supplied value worth hard-rejecting.
                    match normalize_working_dir(&raw_cwd).filter(|p| std::path::Path::new(p).is_absolute()) {
                        Some(norm) => {
                            meta.insert(
                                blockcontroller::META_KEY_CMD_CWD.to_string(),
                                serde_json::Value::String(norm),
                            );
                        }
                        None => {
                            tracing::warn!(
                                raw_cwd = %raw_cwd,
                                "createsubblock: invalid or non-absolute cmd:cwd, dropping (no cwd)"
                            );
                            meta.remove(blockcontroller::META_KEY_CMD_CWD);
                        }
                    }
                }

                let child_id = uuid::Uuid::new_v4().to_string();
                let mut block = Block {
                    oid: child_id.clone(),
                    parentoref: format!("block:{}", cmd.parentblockid),
                    meta,
                    ..Default::default()
                };
                wstore
                    .insert(&mut block)
                    .map_err(|e| format!("createsubblock: insert: {e}"))?;

                // Best-effort link into the parent's subblockids. Not a CAS
                // (Store::update is a blind version-bump) — an acceptable
                // race for a single lazily-created shell per agent pane.
                if let Ok(mut parent) = wstore.must_get::<Block>(&cmd.parentblockid) {
                    parent
                        .subblockids
                        .get_or_insert_with(Vec::new)
                        .push(child_id.clone());
                    let _ = wstore.update(&mut parent);
                }

                tracing::info!(
                    child_id = %child_id,
                    parent_id = %cmd.parentblockid,
                    "CreateSubBlock"
                );
                // TS `CreateSubBlockCommand` resolves `Promise<ORef>`, and
                // `ORef` (gotypes.d.ts:1258) is a plain string — return the
                // bare "block:<id>" string, not a wrapper object.
                Ok(Some(serde_json::Value::String(format!("block:{child_id}"))))
            })
        }),
    );

    // deletesubblock → tear down a sub-block created via createsubblock.
    // Kill-first ordering matches sagas/delete_block.rs; unlike that saga,
    // this does NOT touch tab bookkeeping — sub-blocks are never
    // tab-referenced, so delete_block.rs's precondition would reject them.
    let wstore_dsb = state.wstore.clone();
    engine.register_handler(
        COMMAND_DELETE_SUB_BLOCK,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dsb.clone();
            Box::pin(async move {
                let cmd: CommandDeleteSubBlockData = serde_json::from_value(data)
                    .map_err(|e| format!("deletesubblock: {e}"))?;

                // Kill process FIRST — a lingering PTY tree is worse than a
                // delayed row delete.
                blockcontroller::delete_controller(&cmd.blockid);

                let parent_id = wstore
                    .get::<Block>(&cmd.blockid)
                    .ok()
                    .flatten()
                    .and_then(|b| b.parentoref.strip_prefix("block:").map(str::to_string));

                wstore
                    .delete::<Block>(&cmd.blockid)
                    .map_err(|e| format!("deletesubblock: delete: {e}"))?;

                // Best-effort unlink from the parent's subblockids — the
                // controller is already dead and the row is already gone
                // either way, so don't fail the call over this step.
                if let Some(parent_id) = parent_id {
                    if let Ok(mut parent) = wstore.must_get::<Block>(&parent_id) {
                        if let Some(ids) = parent.subblockids.as_mut() {
                            ids.retain(|id| id != &cmd.blockid);
                            let _ = wstore.update(&mut parent);
                        }
                    }
                }

                tracing::info!(block_id = %cmd.blockid, "DeleteSubBlock");
                Ok(None)
            })
        }),
    );

    // tooldecision → reply to a per-tool-call permission gate.
    //
    // The original PR-3a draft tried to write `y\n` / `n\n` to the
    // subprocess's stdin via `blockcontroller::send_input`. Codex P1
    // on PR #557 caught that this would fail: `SubprocessController::
    // send_input` (and `PersistentSubprocessController::send_input`)
    // both reject raw `input_data`, returning `Err("...use
    // AgentInputCommand")`. The deeper truth is that AgentMux runs
    // the agent CLI in non-interactive `--print` mode — the CLI
    // never reads stdin and a y/n write would be a no-op even if
    // the controller accepted it. See SPEC_DECISION_PROMPT
    // _2026_04_24.md §9.1.
    //
    // For now this handler accepts the decision, validates the
    // payload, logs it (audit trail via `~/.agentmux/logs/`), and
    // returns Ok. The actual delivery mechanism (rule-persistence
    // for next-turn application, or interactive-mode subprocess
    // launch with stdin write) is decided in PR-3b / PR-4 once we
    // pick a CLI integration strategy.
    engine.register_handler(
        COMMAND_TOOL_DECISION,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandToolDecisionData = serde_json::from_value(data)
                    .map_err(|e| format!("tooldecision: {e}"))?;
                match cmd.outcome.as_str() {
                    "allow" | "deny" => {}
                    other => {
                        return Err(format!(
                            "tooldecision: invalid outcome '{}' (expected 'allow' or 'deny')",
                            other
                        ));
                    }
                }
                // Validate scope so PR-3b's rules-persistence layer
                // can trust the value without re-checking. Reagent P1
                // round-3 on PR #557.
                match cmd.scope.as_str() {
                    "once" | "session" | "project" | "global" => {}
                    other => {
                        return Err(format!(
                            "tooldecision: invalid scope '{}' (expected 'once'/'session'/'project'/'global')",
                            other
                        ));
                    }
                }
                tracing::info!(
                    block_id = %cmd.blockid,
                    request_id = %cmd.request_id,
                    outcome = %cmd.outcome,
                    scope = %cmd.scope,
                    has_feedback = cmd.feedback.is_some(),
                    "[tooldecision] received (delivery mechanism deferred to PR-3b/PR-4)"
                );
                Ok(None)
            })
        }),
    );

    // docknodestatus → fire-and-forget push of a ToolNode's latest status,
    // cached in-memory per block for `muxspect dock` to read. See
    // docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.1.
    //
    // Also mirrors a declared-background task's liveness into the durable
    // `db_background_tasks` registry (see
    // docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md) —
    // `DockSnapshotCache` above is intentionally ephemeral (never persisted,
    // 1-hour eviction on read); the registry is the durable source of truth
    // consumers that need to survive a cache eviction or a session
    // reconnect (Swarm, #2492's teardown-survival) read from instead.
    //
    // `run_in_background == Some(true)` is ONLY ever sent once
    // `isAcceptedBackgroundLaunch()` holds client-side (`tool-adapter.ts`),
    // which by its own definition requires `status == "success"` — checking
    // `status == "running"` here (an earlier version of this handler did)
    // made the observe call unreachable, since the flag and that status
    // value are never sent together (reagentx P0 / codex P1 on PR #2590).
    // Best-effort: a registry write failure never blocks the dock push it
    // rides alongside — `dock_snapshots.push_delta` below already gives the
    // user the live UI signal regardless. Completion is NOT detected here —
    // see `COMMAND_BACKGROUND_TASK_COMPLETION` below for why this handler
    // structurally can't see it (the originating ToolNode's status never
    // changes again after acceptance).
    let dock_snapshots_dns = state.dock_snapshots.clone();
    let wstore_dns = state.wstore.clone();
    let pending_pids_dns = state.pending_background_pids.clone();
    let broker_dns = state.broker.clone();
    engine.register_handler(
        COMMAND_DOCK_NODE_STATUS,
        Box::new(move |data, _ctx| {
            let dock_snapshots = dock_snapshots_dns.clone();
            let wstore = wstore_dns.clone();
            let pending_pids = pending_pids_dns.clone();
            let broker = broker_dns.clone();
            Box::pin(async move {
                let cmd: CommandDockNodeStatusData = serde_json::from_value(data)
                    .map_err(|e| format!("docknodestatus: {e}"))?;
                let observed_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                if cmd.run_in_background == Some(true) {
                    match wstore.background_task_observe(
                        &cmd.node_id,
                        &cmd.blockid,
                        &cmd.tool_name,
                        observed_at,
                        observed_at,
                    ) {
                        Ok(()) => publish_background_task_updated(&broker, &cmd.blockid),
                        Err(e) => tracing::warn!(
                            target: "background_tasks",
                            node_id = %cmd.node_id,
                            error = %e,
                            "failed to observe declared-background task in the durable registry",
                        ),
                    }
                    // bashwrap's own pid publish (COMMAND_BACKGROUND_TASK_PID
                    // below) routinely races ahead of the observe call above —
                    // it fires at bashwrap's process start, before the frontend
                    // even sees an accepted-background tool result. Check for
                    // (and apply) a pid stashed ahead of this row's existence.
                    // Runs unconditionally on observe failure too — a stashed
                    // pid may still apply if the row already existed from an
                    // earlier retry. Atomic with the pid handler's own
                    // set_or_stash call for the same id — see
                    // pending_background_pids.rs's module doc for why that
                    // matters (Codex/reagentx findings on PR #2681).
                    let node_id = cmd.node_id.clone();
                    let wstore_apply = wstore.clone();
                    let result: Result<(), crate::backend::storage::StoreError> = pending_pids
                        .observe_and_apply(&cmd.node_id, observed_at, |pid| {
                            wstore_apply.background_task_set_pid(&node_id, pid)
                        });
                    if let Err(e) = result {
                        tracing::warn!(
                            target: "background_tasks",
                            node_id = %cmd.node_id,
                            error = %e,
                            "failed to apply a pid stashed ahead of this task's registry row",
                        );
                    }
                }

                dock_snapshots.push_delta(
                    &cmd.blockid,
                    crate::backend::dock_snapshot::DockNodeSnapshot {
                        node_id: cmd.node_id,
                        tool_name: cmd.tool_name,
                        status: cmd.status,
                        timestamp: cmd.timestamp,
                        observed_at,
                        run_in_background: cmd.run_in_background,
                    },
                );
                Ok(None)
            })
        }),
    );

    // backgroundtaskcompletion → fire-and-forget push of a declared-
    // background task's REAL terminal outcome, parsed client-side from its
    // `<task-notification>` message (`tool-adapter.ts`'s
    // `parseTaskNotification`). This is a separate command from
    // `docknodestatus` above on purpose: the originating ToolNode's own
    // `status` field goes "success" at acceptance and never changes again
    // (that's the raw tool_result's outcome, not the background task's) —
    // there is no later `docknodestatus` push this handler could key off of
    // to learn a background task actually finished (reagentx P0 / codex P1
    // on PR #2590, corrected here). Routing this through `docknodestatus`
    // instead of a dedicated command would also corrupt `DockSnapshotCache`:
    // `push_delta` fully overwrites a node's snapshot, and this event has no
    // `tool_name`/original `run_in_background` value to carry forward — the
    // exact bug class #2520 already fixed once for a different call site.
    let wstore_btc = state.wstore.clone();
    let broker_btc = state.broker.clone();
    engine.register_handler(
        COMMAND_BACKGROUND_TASK_COMPLETION,
        Box::new(move |data, _ctx| {
            let wstore = wstore_btc.clone();
            let broker = broker_btc.clone();
            Box::pin(async move {
                let cmd: CommandBackgroundTaskCompletionData = serde_json::from_value(data)
                    .map_err(|e| format!("backgroundtaskcompletion: {e}"))?;
                let ended_at = cmd.timestamp.unwrap_or_else(|| {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64
                });
                let status = crate::backend::storage::background_tasks::BackgroundTaskStatus::from_str(&cmd.status);
                match wstore.background_task_complete(&cmd.node_id, status, ended_at) {
                    Ok(_) => publish_background_task_updated(&broker, &cmd.blockid),
                    Err(e) => tracing::warn!(
                        target: "background_tasks",
                        node_id = %cmd.node_id,
                        error = %e,
                        "failed to mark background task terminal in the durable registry",
                    ),
                }
                Ok(None)
            })
        }),
    );

    // backgroundtaskpid → fire-and-forget push of a declared-background
    // task's real OS pid, relayed from `agentmux-bashwrap`'s own WPS "pid"
    // chunk. Closes the gap where `db_background_tasks.pid` existed but
    // nothing in production ever wrote it (Phase A of
    // docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md). Best-effort,
    // same as the two handlers above: a write failure here doesn't block
    // anything else and is only ever a diagnostic/teardown-survival input,
    // never something the model-visible tool_result depends on.
    //
    // bashwrap publishes this essentially at process start — routinely
    // BEFORE `COMMAND_DOCK_NODE_STATUS` above has created this task's
    // `db_background_tasks` row (that requires a full round-trip through
    // the frontend recognizing an accepted background launch first).
    // `background_task_set_pid` silently no-ops on a missing row
    // (`Ok(false)`), which would lose the pid permanently with no way to
    // retry a write that will never succeed — stash it instead, and
    // `COMMAND_DOCK_NODE_STATUS`'s handler applies it the moment the row
    // exists. See Codex/reagentx findings on PR #2681.
    let wstore_btp = state.wstore.clone();
    let pending_pids_btp = state.pending_background_pids.clone();
    let broker_btp = state.broker.clone();
    engine.register_handler(
        COMMAND_BACKGROUND_TASK_PID,
        Box::new(move |data, _ctx| {
            let wstore = wstore_btp.clone();
            let pending_pids = pending_pids_btp.clone();
            let broker = broker_btp.clone();
            Box::pin(async move {
                let cmd: CommandBackgroundTaskPidData = serde_json::from_value(data)
                    .map_err(|e| format!("backgroundtaskpid: {e}"))?;
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let node_id = cmd.node_id.clone();
                let wstore_set = wstore.clone();
                let result = pending_pids.set_or_stash(&cmd.node_id, cmd.pid as i64, now_ms, |pid| {
                    wstore_set.background_task_set_pid(&node_id, pid)
                });
                match result {
                    Ok(()) => publish_background_task_updated(&broker, &cmd.blockid),
                    Err(e) => tracing::warn!(
                        target: "background_tasks",
                        node_id = %cmd.node_id,
                        error = %e,
                        "failed to record background task pid in the durable registry",
                    ),
                }
                Ok(None)
            })
        }),
    );

    // listbackgroundtasks → request/response: this block's current
    // db_background_tasks rows, so the frontend can seed its attachedTask
    // axis from the durable registry on mount/reconnect instead of only
    // ever re-deriving it from this tab's own live transcript replay
    // (which has no way to know about a task that survived a session
    // restart under a controller generation with no transcript history of
    // ever launching it — Phase B of
    // docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md).
    // See docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.1.
    let wstore_lbt = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_BACKGROUND_TASKS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lbt.clone();
            Box::pin(async move {
                let cmd: CommandListBackgroundTasksData = serde_json::from_value(data)
                    .map_err(|e| format!("listbackgroundtasks: {e}"))?;
                let tasks = wstore
                    .background_task_list_for_block(&cmd.blockid)
                    .map_err(|e| format!("listbackgroundtasks: {e}"))?;
                let views: Vec<super::muxspect_handlers::BackgroundTaskView> =
                    tasks.into_iter().map(Into::into).collect();
                Ok(Some(serde_json::to_value(views).map_err(|e| format!("listbackgroundtasks: {e}"))?))
            })
        }),
    );

    // agent.answer → deliver an AskUserQuestion answer to the running agent CLI
    // via the Agent SDK **control protocol**: the persistent controller replies
    // to the CLI's parked `can_use_tool` control_request with a control_response
    // carrying `updatedInput.answers`. (Delivering a `tool_result` on stdin does
    // NOT work — the CLI auto-rejects AskUserQuestion within the turn; see spec
    // §2/§3.) Only the persistent controller speaks the control protocol;
    // container/one-shot subprocess agents return UNSUPPORTED_CONTROLLER (Phase 2).
    // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    engine.register_handler(
        COMMAND_AGENT_ANSWER,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandAgentAnswerData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.answer: {e}"))?;
                if cmd.tool_use_id.is_empty() {
                    return Err("agent.answer: MISSING_ARG: tool_use_id".to_string());
                }
                let ctrl = blockcontroller::get_controller(&cmd.blockid)
                    .ok_or_else(|| format!("agent.answer: no controller for block {}", cmd.blockid))?;
                if let Some(persistent_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
                {
                    persistent_ctrl.answer_question(cmd.tool_use_id.clone(), cmd.answers)?;
                    tracing::info!(
                        block_id = %cmd.blockid,
                        tool_use_id = %cmd.tool_use_id,
                        "[agent.answer] control_response delivered to persistent stdin"
                    );
                    Ok(None)
                } else {
                    Err("agent.answer: UNSUPPORTED_CONTROLLER: answering AskUserQuestion \
                         requires a persistent (host) agent; container/one-shot agents are \
                         not yet supported (Phase 2)".to_string())
                }
            })
        }),
    );

    // agent.cancel → a REAL protocol-level decline of a pending AskUserQuestion
    // (Cancel button / Escape in AgentQuestionPanel.tsx), not a UI-only dismiss.
    // Sends `behavior: "deny"` over the same control protocol agent.answer uses
    // for `behavior: "allow"` — see `deny_question`'s doc comment for why this
    // is a general, documented Agent SDK mechanism rather than something
    // special-cased for AskUserQuestion. Error strings below deliberately match
    // agent.answer's wording verbatim (`no controller for block`,
    // `UNSUPPORTED_CONTROLLER`): the frontend's SAFE_TO_RETRY_VIA_FOLLOWUP
    // allowlist (useAgentQuestions.ts) matches on these substrings for both
    // commands. Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    engine.register_handler(
        COMMAND_AGENT_CANCEL,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandAgentCancelData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.cancel: {e}"))?;
                if cmd.tool_use_id.is_empty() {
                    return Err("agent.cancel: MISSING_ARG: tool_use_id".to_string());
                }
                let ctrl = blockcontroller::get_controller(&cmd.blockid)
                    .ok_or_else(|| format!("agent.cancel: no controller for block {}", cmd.blockid))?;
                if let Some(persistent_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
                {
                    persistent_ctrl.deny_question(
                        cmd.tool_use_id.clone(),
                        blockcontroller::persistent::ASK_USER_QUESTION_DENY_MESSAGE.to_string(),
                    )?;
                    tracing::info!(
                        block_id = %cmd.blockid,
                        tool_use_id = %cmd.tool_use_id,
                        "[agent.cancel] deny control_response delivered to persistent stdin"
                    );
                    Ok(None)
                } else {
                    Err("agent.cancel: UNSUPPORTED_CONTROLLER: canceling AskUserQuestion \
                         requires a persistent (host) agent; container/one-shot agents are \
                         not yet supported (Phase 2)".to_string())
                }
            })
        }),
    );

    // Agent input/stop + subprocess spawn handlers
    super::agent_handlers::register_agent_input_handlers(engine, &state);

    // Shell exec/stop handlers
    super::shell_handlers::register_shell_handlers(engine, &state);

    // Editor/file-ops + write_agent_config handlers
    super::editor_handlers::register_editor_handlers(engine, &state);

    // LSP handlers (lspstart, lspsend, lspstop)
    super::lsp_handlers::register_lsp_handlers(engine, &state);

    // CLI handlers (resolvecli, checkcliauth, runclilogin)
    super::cli_handlers::register_cli_handlers(engine, &state);

    // Tool store handlers (gettoolstatus, installtool)
    super::tool_handlers::register_tool_handlers(engine, &state);

    // Provider model catalog (providers.models → authoritative /v1/models list)
    super::providers_handlers::register_providers_handlers(engine, &state);

    // reactive.registrations → Stash "Registration" tab live status (#2696)
    super::reactive::register_reactive_ws_handlers(engine, &state);

    // eventreadhistory → read persisted event history from the WPS broker
    let broker_history = state.broker.clone();
    engine.register_handler(
        COMMAND_EVENT_READ_HISTORY,
        Box::new(move |data, _ctx| {
            let broker = broker_history.clone();
            Box::pin(async move {
                let cmd: CommandEventReadHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("eventreadhistory: {e}"))?;
                let max_items = if cmd.maxitems == 0 { 1024 } else { cmd.maxitems };
                let events = broker.read_event_history(&cmd.event, &cmd.scope, max_items);
                Ok(Some(serde_json::to_value(&events).unwrap_or_default()))
            })
        }),
    );

    // setconfig → merge settings keys into settings.json AND update in-memory config immediately.
    // Writing to disk + broadcasting directly gives instant UI response without waiting for
    // the fs watcher (which has a ~300-800ms debounce + polling delay on Windows).
    // The fs watcher's subsequent reload is a no-op (settings already up to date).
    let config_watcher_setconfig = state.config_watcher.clone();
    let event_bus_setconfig = state.event_bus.clone();
    let lan_discovery_setconfig = state.lan_discovery.clone();
    engine.register_handler(
        COMMAND_SET_CONFIG,
        Box::new(move |data, _ctx| {
            let cw = config_watcher_setconfig.clone();
            let eb = event_bus_setconfig.clone();
            let lan = lan_discovery_setconfig.clone();
            Box::pin(async move {
                let new_keys: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_value(data).map_err(|e| format!("setconfig: {e}"))?;

                // 1. Write to disk (fs watcher will re-broadcast, harmlessly)
                crate::backend::config_watcher_fs::merge_settings_to_disk(new_keys.clone())
                    .map_err(|e| format!("setconfig write: {e}"))?;

                // 2. Update in-memory config immediately
                let merged_settings = crate::backend::config_watcher_fs::merge_settings_into_current(&cw, new_keys);
                let lan_enabled = merged_settings.network_lan_discovery;
                cw.update_settings(merged_settings);

                // 3. Live-toggle LAN discovery if the key changed. `apply` is
                //    idempotent so it's safe to call unconditionally — when the
                //    daemon is already in the requested state, this is a no-op.
                //    See docs/specs/lan-discovery-toggle.md.
                lan.apply(lan_enabled);

                // 4. Broadcast updated config now — no waiting for fs watcher
                let config = cw.get_full_config();
                if let Ok(mut config_val) = serde_json::to_value(config.as_ref()) {
                    crate::backend::wconfig::redact_full_config_for_renderer(&mut config_val);
                    let event = crate::backend::eventbus::WSEventType {
                        eventtype: crate::backend::eventbus::WS_EVENT_RPC.to_string(),
                        oref: String::new(),
                        data: Some(serde_json::json!({
                            "command": "eventrecv",
                            "data": {
                                "event": "config",
                                "data": { "fullconfig": config_val }
                            }
                        })),
                    };
                    eb.broadcast_event(&event);
                }
                Ok(None)
            })
        }),
    );

    // Agent handlers (definitions, content, skills, history, import, reseed)
    super::agent_handlers::register_agent_handlers(engine, &state);

    // Drone handlers (issue #753 — Drone pane DAG executor)
    super::drone_handlers::register_drone_handlers(engine, &state);

    // Pre-launch OAuth handlers (auth.start / poll / submitcallback /
    // cancel / submitapikey — see docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md)
    super::identity_handlers::register_identity_handlers(engine, &state);

    // Install handlers (install.start / install.cancel — see
    // docs/specs/SPEC_AGENT_INSTALL_STAGE_2026_05_17.md)
    super::install_handlers::register_install_handlers(engine, &state);

    // System-toolchain install handlers (toolchain.resolve_install_command /
    // toolchain.install_system_tool — see
    // docs/specs/SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md). Shares
    // state.install_sessions with the handlers just above (same
    // InstallSessionRegistry — install.cancel already works unchanged for
    // these sessions too, no new cancel command needed).
    super::system_install_handlers::register_system_install_handlers(engine, &state);

    // App API handlers (agent.open, agent.send, agent.stop, agent.status, agent.list, agent.output)
    super::app_api::register_app_api_handlers(engine, &state);

    // MuxBus cloud connectivity (muxbus.login / muxbus.login.cancel / muxbus.status / muxbus.disconnect)
    super::muxbus_handlers::register_muxbus_handlers(engine, &state);

    // Native memory file browser (agent:memory:list / read_file / write_file)
    super::native_memory_handlers::register_native_memory_handlers(engine, &state);
}


/// Parse a CommandBlockInputData into a BlockInputUnion.
fn parse_block_input(
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

    fn raw_oref_event(block_id: &str) -> serde_json::Value {
        json!({
            "eventtype": "waveobj:update",
            "oref": format!("block:{block_id}"),
            "data": {},
        })
    }

    fn global_event() -> serde_json::Value {
        json!({
            "eventtype": "waveobj:batchedupdates",
            "data": [],
        })
    }

    #[test]
    fn test_priority_pane_key_from_rpc_scopes() {
        let event = rpc_event("abc123");
        assert_eq!(priority_pane_key(&event), Some("block:abc123".to_string()));
    }

    #[test]
    fn test_priority_pane_key_from_raw_oref() {
        let event = raw_oref_event("xyz789");
        assert_eq!(priority_pane_key(&event), Some("block:xyz789".to_string()));
    }

    #[test]
    fn test_priority_pane_key_none_for_global_event() {
        let event = global_event();
        assert_eq!(priority_pane_key(&event), None);
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
}
