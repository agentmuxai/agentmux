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
    CommandBlockInputData, CommandControllerResyncData, CommandEventReadHistoryData,
    CommandGetMetaData, CommandSetMetaData, CommandToolDecisionData,
    RpcMessage, COMMAND_CONTROLLER_INPUT,
    COMMAND_CONTROLLER_RESYNC, COMMAND_EVENT_READ_HISTORY, COMMAND_EVENT_SUB, COMMAND_EVENT_UNSUB,
    COMMAND_EVENT_UNSUB_ALL, COMMAND_GET_FULL_CONFIG, COMMAND_GET_META,
    COMMAND_GET_AI_RATE_LIMIT, COMMAND_ROUTE_ANNOUNCE, COMMAND_ROUTE_UNANNOUNCE,
    COMMAND_SET_META, COMMAND_SET_CONFIG, COMMAND_APP_INFO,
    COMMAND_TOOL_DECISION, COMMAND_AGENT_ANSWER,
    CommandAgentAnswerData,
};
use crate::backend::obj::{Block, TermSize, WaveObjUpdate, wave_obj_to_value};
use super::service::update_object_meta;

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
        if let Ok(config_val) = serde_json::to_value(config.as_ref()) {
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

    loop {
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
            Some(event) = priority_rx.recv() => {
                if forward_event(&mut socket, event).await {
                    break;
                }
            }

            // Background event lane → WebSocket. Droppable perf telemetry
            // (sysinfo/blockstats) only; serviced when the priority lanes above
            // are momentarily idle, so it can never delay a keystroke echo.
            //
            // Tradeoff of `biased;`: a SUSTAINED priority-lane flood (e.g.
            // `! yes` pumping terminal output continuously) keeps the priority
            // branches ready, so this lane is starved and its unbounded receiver
            // accumulates sysinfo/blockstats until the flood pauses, then
            // flushes as a stale burst. Acceptable for now because telemetry
            // ingress is low-rate (sysinfo ~1/s) and a flood also saturates the
            // socket writes — during which the priority lanes do drain, so this
            // lane gets serviced — making true never-empty starvation the rare
            // worst case. Phase 2 bounds it properly by coalescing this lane to
            // the latest reading per (event, scope), so it cannot grow and the
            // post-flood flush is one current frame, not a backlog. See
            // SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md §4.2.
            Some(event) = background_rx.recv() => {
                if forward_event(&mut socket, event).await {
                    break;
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
                    let reactive_req = crate::backend::reactive::InjectionRequest {
                        target_agent: target.clone(),
                        message: message.clone(),
                        source_agent: Some(from.to_string()),
                        request_id: None,
                        priority: incoming.priority.clone(),
                        wait_for_idle: false,
                    };
                    let resp = state.reactive_handler.inject_message(reactive_req);
                    if resp.success {
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
                    Ok(v) => Ok(Some(v)),
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
                tracing::info!(oref = %oref_str, keys = ?meta_keys, "SetMeta");
                update_object_meta(&wstore, &oref_str, &cmd.meta)?;
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
    engine.register_handler(
        COMMAND_CONTROLLER_RESYNC,
        Box::new(move |data, _ctx| {
            let wstore = wstore_resync.clone();
            let broker = broker_resync.clone();
            let event_bus = event_bus_resync.clone();
            let filestore = filestore_resync.clone();
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
                blockcontroller::resync_controller(
                    &block,
                    &cmd.tabid,
                    cmd.rtopts,
                    cmd.forcerestart,
                    Some(broker),
                    Some(event_bus),
                    Some(wstore),
                    Some(filestore),
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
                //    See specs/lan-discovery-toggle.md.
                lan.apply(lan_enabled);

                // 4. Broadcast updated config now — no waiting for fs watcher
                let config = cw.get_full_config();
                if let Ok(config_val) = serde_json::to_value(config.as_ref()) {
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

    // App API handlers (agent.open, agent.send, agent.stop, agent.status, agent.list, agent.output)
    super::app_api::register_app_api_handlers(engine, &state);

    // MuxBus cloud connectivity (muxbus.login / muxbus.status / muxbus.disconnect)
    super::muxbus_handlers::register_muxbus_handlers(engine, &state);
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
