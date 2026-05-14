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

use crate::backend::base::expand_home_dir_safe;
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
    COMMAND_SUBPROCESS_SPAWN, COMMAND_AGENT_INPUT, COMMAND_AGENT_STOP, COMMAND_TOOL_DECISION,
    COMMAND_WRITE_AGENT_CONFIG,
    CommandSubprocessSpawnData, CommandAgentInputData, CommandAgentStopData, CommandWriteAgentConfigData,
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

    let mut event_rx = state.event_bus.register_ws(&conn_id, &tab_id);
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
    register_handlers(&engine, state.clone());
    tracing::info!("[ws-perf] create_engine+register_handlers: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0);
    tracing::info!("[ws-perf] TOTAL ws_setup: {:.2}ms", ws_start.elapsed().as_secs_f64() * 1000.0);

    // Periodic ping interval (10 seconds)
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(10));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // Forward event bus events → WebSocket.
            // Two sources feed the event bus:
            //   1. WPS Broker (via EventBusBridge) — already wrapped as
            //      { eventtype: "rpc", data: { command: "eventrecv", data: WaveEvent } }
            //   2. Direct broadcasts (e.g., SetMeta's obj:update) — raw
            //      { eventtype: "waveobj:update", oref: "block:xxx", data: ... }
            // Type 1: forward as-is (already RPC-wrapped).
            // Type 2: wrap as RPC "eventrecv" so the frontend WshRouter routes
            //         it to handleWaveEvent → updateWaveObject → Jotai re-render.
            Some(event) = event_rx.recv() => {
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
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
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

            // Incoming WebSocket messages → parse & dispatch
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

    // Unregister from messagebus if this connection was an agent
    if let Some(ref agent_id) = bus_agent_id {
        state.messagebus.unregister(agent_id);
    }
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
                                    if let Err(e) = blockcontroller::send_input(block_id, input) {
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
                                if let Err(e) = blockcontroller::send_input(block_id, input) {
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

fn register_handlers(engine: &Arc<WshRpcEngine>, state: AppState) {
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
    engine.register_handler(
        COMMAND_EVENT_SUB,
        Box::new(move |data, _ctx| {
            let broker = broker_sub.clone();
            Box::pin(async move {
                let sub: crate::backend::wps::SubscriptionRequest =
                    serde_json::from_value(data).map_err(|e| format!("eventsub: {e}"))?;
                tracing::debug!("eventsub: event={} scopes={:?} allscopes={}", sub.event, sub.scopes, sub.allscopes);
                broker.subscribe("ws-main", sub);
                Ok(None)
            })
        }),
    );

    // eventunsub → unsubscribe from the WPS broker
    let broker_unsub = state.broker.clone();
    engine.register_handler(
        COMMAND_EVENT_UNSUB,
        Box::new(move |data, _ctx| {
            let broker = broker_unsub.clone();
            Box::pin(async move {
                let event_name = data.as_str().unwrap_or("").to_string();
                if !event_name.is_empty() {
                    broker.unsubscribe("ws-main", &event_name);
                }
                Ok(None)
            })
        }),
    );

    // eventunsuball → unsubscribe all from the WPS broker
    let broker_unsub_all = state.broker.clone();
    engine.register_handler(
        COMMAND_EVENT_UNSUB_ALL,
        Box::new(move |_data, _ctx| {
            let broker = broker_unsub_all.clone();
            Box::pin(async move {
                broker.unsubscribe_all("ws-main");
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
                blockcontroller::send_input(&cmd.blockid, input)?;
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

    // subprocessspawn → spawn agent CLI as subprocess for a single turn
    let wstore_spawn = state.wstore.clone();
    let broker_spawn = state.broker.clone();
    let event_bus_spawn = state.event_bus.clone();
    let filestore_spawn = state.filestore.clone();
    engine.register_handler(
        COMMAND_SUBPROCESS_SPAWN,
        Box::new(move |data, _ctx| {
            let wstore = wstore_spawn.clone();
            let broker = broker_spawn.clone();
            let event_bus = event_bus_spawn.clone();
            let filestore = filestore_spawn.clone();
            Box::pin(async move {
                let cmd: CommandSubprocessSpawnData = serde_json::from_value(data)
                    .map_err(|e| format!("subprocessspawn: {e}"))?;
                tracing::info!(
                    block_id = %cmd.blockid,
                    cli = %cmd.cli_command,
                    "SubprocessSpawn"
                );

                // Get or create a SubprocessController for this block
                let ctrl = match blockcontroller::get_controller(&cmd.blockid) {
                    Some(c) if c.controller_type() == blockcontroller::BLOCK_CONTROLLER_SUBPROCESS => c,
                    _ => {
                        // Create and register a new SubprocessController
                        let ctrl = blockcontroller::subprocess::SubprocessController::new(
                            cmd.tabid.clone(),
                            cmd.blockid.clone(),
                            Some(broker),
                            Some(event_bus),
                            Some(wstore),
                            Some(filestore),
                        );
                        let ctrl = std::sync::Arc::new(ctrl);
                        ctrl.set_self_ref();
                        blockcontroller::register_controller(&cmd.blockid, ctrl.clone());
                        ctrl as std::sync::Arc<dyn blockcontroller::Controller>
                    }
                };

                // Downcast to SubprocessController to call spawn_turn
                let subprocess_ctrl = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                    .ok_or_else(|| "controller is not a SubprocessController".to_string())?;

                let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                    cli_command: cmd.cli_command,
                    cli_args: cmd.cli_args,
                    working_dir: cmd.working_dir,
                    env_vars: cmd.env_vars,
                    message: cmd.message,
                    resume_flag: "--resume".to_string(),
                    session_id_field: "session_id".to_string(),
                    message_id: None,
                };
                subprocess_ctrl.spawn_turn(config)?;
                Ok(None)
            })
        }),
    );

    // agentinput → send message to agent (persistent or per-turn subprocess)
    let wstore_ai = state.wstore.clone();
    // Streaming-bash wrapper auth — clone the per-launch auth_key into the
    // handler's closure so each spawn can inject it into Claude's env.
    // See SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7.
    let auth_key_ai = state.auth_key.clone();
    engine.register_handler(
        COMMAND_AGENT_INPUT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ai.clone();
            let auth_key = auth_key_ai.clone();
            Box::pin(async move {
                let cmd: CommandAgentInputData = serde_json::from_value(data)
                    .map_err(|e| format!("agentinput: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, "AgentInput");

                let ctrl = blockcontroller::get_controller(&cmd.blockid)
                    .ok_or_else(|| format!("no controller for block {}", cmd.blockid))?;

                // Re-read the spawn config from block metadata
                let block: Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("agentinput: load block: {e}"))?
                    .ok_or_else(|| format!("block {} not found", cmd.blockid))?;

                let cli_command = crate::backend::obj::meta_get_string(
                    &block.meta, "cmd", "claude",
                );
                let cli_args: Vec<String> = match block.meta.get("cmd:args") {
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    _ => vec![
                        "-p".to_string(),
                        "--input-format".to_string(),
                        "stream-json".to_string(),
                        "--output-format".to_string(),
                        "stream-json".to_string(),
                    ],
                };
                let working_dir = crate::backend::obj::meta_get_string(
                    &block.meta, "cmd:cwd", "",
                );
                let mut env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
                    Some(serde_json::Value::Object(obj)) => obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect(),
                    _ => std::collections::HashMap::new(),
                };
                // Identity injection: look up the active AgentInstance for
                // this block, resolve its identity_id's bindings, and merge
                // each per-provider env var into the spawn map. Failures
                // are logged and skipped — the agent CLI launches with
                // whatever resolved cleanly plus the static cmd:env block.
                // See agentmux-srv/src/identity/resolver.rs.
                crate::identity::inject_identity_env(
                    wstore.clone(),
                    &cmd.blockid,
                    &mut env_vars,
                );
                // Streaming-bash wrapper auth + discovery
                // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7).
                //
                // 1. AGENTMUX_AUTH_KEY — config.rs:42 removed it from
                //    the process env at startup (security PR #801).
                //    Re-inject for this spawn so the wrapper (running
                //    inside Claude's bash subprocess tree) can
                //    authenticate against the auth_middleware-gated
                //    /agentmux/wps/publish endpoint via X-AuthKey.
                // 2. PATH — prepend the bundled tools/bin dir so
                //    `agentmux-bashwrap.exe` resolves when the
                //    PreToolUse hook (auto-injected by agent_config.rs)
                //    rewrites the command to invoke it. AGENTMUX_LOCAL_URL
                //    is already in the inherited process env (main.rs:498).
                env_vars.insert("AGENTMUX_AUTH_KEY".to_string(), auth_key.clone());
                // Block id so the wrapper can scope its WPS publishes
                // to `block:<id>`. Without this, chunks publish without
                // a scope and the frontend's per-block subscription
                // doesn't receive them.
                env_vars.insert("AGENTMUX_BLOCKID".to_string(), cmd.blockid.clone());
                // PATH includes BOTH bundled tools dir (portable
                // builds, runtime/tools/bin/) AND user tools dir
                // (~/.agentmux/tools/bin/). bundled is None in dev
                // mode (target/debug exclusion in tool_store), so
                // without user_tools_dir the wrapper wouldn't be on
                // the agent's PATH during `task dev`.
                {
                    let existing = env_vars
                        .get("PATH")
                        .cloned()
                        .or_else(|| std::env::var("PATH").ok())
                        .unwrap_or_default();
                    let sep = if cfg!(windows) { ";" } else { ":" };
                    let mut extras: Vec<String> = Vec::new();
                    if let Some(d) = crate::backend::tool_store::bundled_tools_dir() {
                        if d.exists() {
                            extras.push(d.to_string_lossy().into_owned());
                        }
                    }
                    if let Some(d) = crate::backend::tool_store::user_tools_dir() {
                        if d.exists() {
                            extras.push(d.to_string_lossy().into_owned());
                        }
                    }
                    if !extras.is_empty() {
                        let new_path = format!("{}{}{}", extras.join(sep), sep, existing);
                        env_vars.insert("PATH".to_string(), new_path);
                    }
                }

                let session_id_field = crate::backend::obj::meta_get_string(
                    &block.meta, "agent:session_id_field", "session_id",
                );

                // Try persistent controller first, fall back to subprocess
                if let Some(persistent_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
                {
                    let config = blockcontroller::persistent::PersistentSpawnConfig {
                        cli_command,
                        cli_args,
                        working_dir,
                        env_vars,
                        session_id_field,
                    };
                    persistent_ctrl.send_message(cmd.message, config)?;
                } else if let Some(subprocess_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                {
                    let resume_flag = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:resume_flag", "--resume",
                    );
                    let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                        cli_command,
                        cli_args,
                        working_dir,
                        env_vars,
                        message: cmd.message,
                        resume_flag,
                        session_id_field,
                        message_id: cmd.message_id,
                    };
                    subprocess_ctrl.spawn_turn(config)?;
                } else {
                    return Err("controller is not a SubprocessController or PersistentSubprocessController".to_string());
                }

                Ok(None)
            })
        }),
    );

    // agentstop → stop the running agent subprocess
    engine.register_handler(
        COMMAND_AGENT_STOP,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandAgentStopData = serde_json::from_value(data)
                    .map_err(|e| format!("agentstop: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, force = cmd.force, "AgentStop");
                match blockcontroller::get_controller(&cmd.blockid) {
                    Some(ctrl) => {
                        ctrl.stop(!cmd.force, blockcontroller::STATUS_DONE)?;
                        Ok(None)
                    }
                    None => Ok(None),
                }
            })
        }),
    );

    // writeagentconfig → write config files atomically to agent working directory
    engine.register_handler(
        COMMAND_WRITE_AGENT_CONFIG,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandWriteAgentConfigData = serde_json::from_value(data)
                    .map_err(|e| format!("writeagentconfig: {e}"))?;
                tracing::info!(
                    working_dir = %cmd.working_dir,
                    file_count = cmd.files.len(),
                    auto_allocate = cmd.auto_allocate,
                    "WriteAgentConfig"
                );

                // Resolve to a final on-disk path. For auto-generated
                // instance paths (`auto_allocate: true`), use the
                // atomic `<base>-N` allocator so concurrent same-hour
                // launches don't share a workdir. For user-specified
                // paths, mkdir-p as before — never rewrite.
                let expanded_working_dir = expand_home_dir_safe(&cmd.working_dir);
                let final_working_dir = if cmd.auto_allocate {
                    let desired = expanded_working_dir.to_string_lossy().to_string();
                    crate::server::app_api::allocate_agent_workdir(&desired)?
                } else {
                    let p = expanded_working_dir.as_path();
                    if !p.exists() {
                        std::fs::create_dir_all(p)
                            .map_err(|e| format!("failed to create working dir: {e}"))?;
                    }
                    expanded_working_dir.to_string_lossy().to_string()
                };
                let base_path = std::path::Path::new(&final_working_dir);
                // Canonicalize base ONCE (it exists — allocate_agent_workdir
                // or the explicit-path mkdir-p created it just above). Used
                // by the per-file symlink-escape verifier so we catch a
                // symlinked ancestor like `<base>/.claude -> /tmp/outside`
                // before fs::write follows it.
                let canonical_base = base_path.canonicalize().map_err(|e| {
                    format!("failed to canonicalize working dir {}: {e}", base_path.display())
                })?;

                for file in &cmd.files {
                    // Lexical join + traversal check — works on Windows where
                    // canonicalize() adds the `\\?\` UNC prefix and breaks
                    // starts_with against not-yet-created files. Catches `..`,
                    // absolute paths, drive-letter prefixes (root and inner).
                    let file_path = crate::backend::base::safe_join_within_base(
                        base_path,
                        &file.path,
                    )
                    .map_err(|e| format!("path traversal denied: {} ({e})", file.path))?;
                    // Symlink-escape guard: if any EXISTING ancestor is a
                    // symlink that resolves outside the workdir, reject.
                    // No-op for fully-fresh agent dirs (the common case
                    // where every component is new).
                    crate::backend::base::verify_no_symlink_escape(&file_path, &canonical_base)
                        .map_err(|e| format!("path traversal denied: {} ({e})", file.path))?;
                    // Create parent directories if needed
                    if let Some(parent) = file_path.parent() {
                        if !parent.exists() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| format!("failed to create dir for {}: {e}", file.path))?;
                        }
                    }
                    std::fs::write(&file_path, &file.content)
                        .map_err(|e| format!("failed to write {}: {e}", file.path))?;
                    tracing::debug!(path = %file_path.display(), "wrote config file");
                }

                // Return the final path so the caller can patch
                // `cmd:cwd` if collision resolution changed it.
                Ok(Some(serde_json::json!({
                    "working_dir": final_working_dir,
                })))
            })
        }),
    );

    // readeditorfile → read file from disk for the editor pane
    engine.register_handler(
        "readeditorfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { path: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("readeditorfile: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.path);
                let path = expanded.as_path();

                // Size guard: reject files > 10MB
                let metadata = std::fs::metadata(path)
                    .map_err(|e| format!("readeditorfile: {e}"))?;
                if metadata.len() > 10_000_000 {
                    return Err("File too large (>10MB)".to_string());
                }

                let content = std::fs::read_to_string(path)
                    .map_err(|e| format!("readeditorfile: {e}"))?;
                let read_only = metadata.permissions().readonly();

                Ok(Some(serde_json::json!({
                    "content": content,
                    "read_only": read_only,
                })))
            })
        }),
    );

    // writeeditorfile → write file to disk from the editor pane
    engine.register_handler(
        "writeeditorfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { path: String, content: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("writeeditorfile: {e}"))?;

                // Size guard: match readeditorfile's 10MB limit
                if cmd.content.len() > 10_000_000 {
                    return Err("Content too large (>10MB)".to_string());
                }

                let expanded = expand_home_dir_safe(&cmd.path);
                let path = expanded.as_path();

                // Path safety: restrict writes to under the user's home directory.
                // Allowlist approach — safer than an incomplete denylist.
                let home = dirs::home_dir()
                    .ok_or("writeeditorfile: cannot determine home directory")?;
                let canonical_home = home.canonicalize()
                    .map_err(|e| format!("writeeditorfile: home dir: {e}"))?;

                // Resolve the target path (canonicalize existing, or parent + filename for new files)
                let canonical = path.canonicalize().or_else(|_| {
                    path.parent()
                        .and_then(|p| p.canonicalize().ok())
                        .map(|p| p.join(path.file_name().unwrap_or_default()))
                        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "invalid path"))
                }).map_err(|e| format!("writeeditorfile: {e}"))?;

                if !canonical.starts_with(&canonical_home) {
                    return Err(format!(
                        "writeeditorfile: path {} is outside home directory",
                        canonical.display()
                    ));
                }

                std::fs::write(&canonical, &cmd.content)
                    .map_err(|e| format!("writeeditorfile: {e}"))?;
                tracing::info!(path = %canonical.display(), bytes = cmd.content.len(), "editor file saved");

                Ok(None)
            })
        }),
    );

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
    engine.register_handler(
        COMMAND_SET_CONFIG,
        Box::new(move |data, _ctx| {
            let cw = config_watcher_setconfig.clone();
            let eb = event_bus_setconfig.clone();
            Box::pin(async move {
                let new_keys: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_value(data).map_err(|e| format!("setconfig: {e}"))?;

                // 1. Write to disk (fs watcher will re-broadcast, harmlessly)
                crate::backend::config_watcher_fs::merge_settings_to_disk(new_keys.clone())
                    .map_err(|e| format!("setconfig write: {e}"))?;

                // 2. Update in-memory config immediately
                let merged_settings = crate::backend::config_watcher_fs::merge_settings_into_current(&cw, new_keys);
                cw.update_settings(merged_settings);

                // 3. Broadcast updated config now — no waiting for fs watcher
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

    // Forge handlers (agents, content, skills, history, import, reseed)
    super::forge_handlers::register_forge_handlers(engine, &state);

    // Workflow handlers (issue #753 — Workflows pane DAG executor)
    super::workflow_handlers::register_workflow_handlers(engine, &state);

    // Pre-launch OAuth handlers (auth.start / poll / submitcallback /
    // cancel / submitapikey — see docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md)
    super::identity_handlers::register_identity_handlers(engine, &state);

    // App API handlers (agent.open, agent.send, agent.stop, agent.status, agent.list, agent.output)
    super::app_api::register_app_api_handlers(engine, &state);
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
