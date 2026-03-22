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

use crate::backend::ai::chatstore::get_default_chat_store;
use crate::backend::blockcontroller;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandBlockInputData, CommandControllerResyncData, CommandEventReadHistoryData,
    CommandGetMetaData, CommandSetMetaData, RpcMessage, COMMAND_CONTROLLER_INPUT,
    COMMAND_CONTROLLER_RESYNC, COMMAND_EVENT_READ_HISTORY, COMMAND_EVENT_SUB, COMMAND_EVENT_UNSUB,
    COMMAND_EVENT_UNSUB_ALL, COMMAND_GET_FULL_CONFIG, COMMAND_GET_META, COMMAND_GET_AI_CHAT,
    COMMAND_GET_AI_RATE_LIMIT, COMMAND_ROUTE_ANNOUNCE, COMMAND_ROUTE_UNANNOUNCE,
    COMMAND_SET_META, COMMAND_SET_CONFIG, COMMAND_APP_INFO,
    COMMAND_LIST_FORGE_AGENTS, COMMAND_CREATE_FORGE_AGENT, COMMAND_UPDATE_FORGE_AGENT,
    COMMAND_DELETE_FORGE_AGENT, COMMAND_GET_FORGE_CONTENT, COMMAND_SET_FORGE_CONTENT,
    COMMAND_GET_ALL_FORGE_CONTENT,
    COMMAND_LIST_FORGE_SKILLS, COMMAND_CREATE_FORGE_SKILL, COMMAND_UPDATE_FORGE_SKILL,
    COMMAND_DELETE_FORGE_SKILL,
    COMMAND_APPEND_FORGE_HISTORY, COMMAND_LIST_FORGE_HISTORY, COMMAND_SEARCH_FORGE_HISTORY,
    COMMAND_IMPORT_FORGE_FROM_CLAW,
    COMMAND_RESEED_FORGE_AGENTS,
    CommandCreateForgeAgentData, CommandUpdateForgeAgentData, CommandDeleteForgeAgentData,
    CommandGetForgeContentData, CommandSetForgeContentData, CommandGetAllForgeContentData,
    CommandListForgeSkillsData, CommandCreateForgeSkillData, CommandUpdateForgeSkillData,
    CommandDeleteForgeSkillData,
    CommandAppendForgeHistoryData, CommandListForgeHistoryData, CommandSearchForgeHistoryData,
    CommandImportForgeFromClawData,
    COMMAND_SUBPROCESS_SPAWN, COMMAND_AGENT_INPUT, COMMAND_AGENT_STOP, COMMAND_WRITE_AGENT_CONFIG,
    COMMAND_RESOLVE_CLI, COMMAND_CHECK_CLI_AUTH,
    CommandSubprocessSpawnData, CommandAgentInputData, CommandAgentStopData, CommandWriteAgentConfigData,
    CommandResolveCliData, ResolveCliResult, CommandCheckCliAuthData, CheckCliAuthResult,
    CommandRunCliLoginData, RunCliLoginResult,
};
use crate::backend::storage::{ForgeAgent, ForgeContent, ForgeSkill};
use crate::backend::waveobj::{Block, TermSize, WaveObjUpdate, wave_obj_to_value};
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
            //   2. Direct broadcasts (e.g., SetMeta's waveobj:update) — raw
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

    // getwaveaichat → return UIChat for the given chatid (null if not found)
    engine.register_handler(
        COMMAND_GET_AI_CHAT,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let obj: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_value(data).map_err(|e| format!("getwaveaichat: {e}"))?;
                let chat_id = obj
                    .get("chatid")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let result = get_default_chat_store().get_as_ui_chat(&chat_id);
                Ok(result) // None → JSON null, Some(v) → JSON object
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
    engine.register_handler(
        COMMAND_CONTROLLER_RESYNC,
        Box::new(move |data, _ctx| {
            let wstore = wstore_resync.clone();
            let broker = broker_resync.clone();
            let event_bus = event_bus_resync.clone();
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

    // subprocessspawn → spawn agent CLI as subprocess for a single turn
    let wstore_spawn = state.wstore.clone();
    let broker_spawn = state.broker.clone();
    let event_bus_spawn = state.event_bus.clone();
    engine.register_handler(
        COMMAND_SUBPROCESS_SPAWN,
        Box::new(move |data, _ctx| {
            let wstore = wstore_spawn.clone();
            let broker = broker_spawn.clone();
            let event_bus = event_bus_spawn.clone();
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
                        );
                        let ctrl = std::sync::Arc::new(ctrl);
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
                };
                subprocess_ctrl.spawn_turn(config)?;
                Ok(None)
            })
        }),
    );

    // agentinput → send follow-up message to agent (re-spawns with --resume)
    let wstore_ai = state.wstore.clone();
    engine.register_handler(
        COMMAND_AGENT_INPUT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ai.clone();
            Box::pin(async move {
                let cmd: CommandAgentInputData = serde_json::from_value(data)
                    .map_err(|e| format!("agentinput: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, "AgentInput");

                let ctrl = blockcontroller::get_controller(&cmd.blockid)
                    .ok_or_else(|| format!("no controller for block {}", cmd.blockid))?;

                let subprocess_ctrl = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                    .ok_or_else(|| "controller is not a SubprocessController".to_string())?;

                // Re-read the original spawn config from block metadata
                let block: Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("agentinput: load block: {e}"))?
                    .ok_or_else(|| format!("block {} not found", cmd.blockid))?;

                let cli_command = crate::backend::waveobj::meta_get_string(
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
                let working_dir = crate::backend::waveobj::meta_get_string(
                    &block.meta, "cmd:cwd", "",
                );
                let env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
                    Some(serde_json::Value::Object(obj)) => obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect(),
                    _ => std::collections::HashMap::new(),
                };

                let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                    cli_command,
                    cli_args,
                    working_dir,
                    env_vars,
                    message: cmd.message,
                };
                subprocess_ctrl.spawn_turn(config)?;
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
                    "WriteAgentConfig"
                );

                let base_path = std::path::Path::new(&cmd.working_dir);
                if !base_path.exists() {
                    std::fs::create_dir_all(base_path)
                        .map_err(|e| format!("failed to create working dir: {e}"))?;
                }

                for file in &cmd.files {
                    let file_path = base_path.join(&file.path);
                    // Prevent path traversal: resolved path must stay within base_path
                    let canonical_base = base_path.canonicalize()
                        .map_err(|e| format!("failed to canonicalize base path: {e}"))?;
                    let canonical_file = file_path.canonicalize().unwrap_or_else(|_| {
                        // File doesn't exist yet — normalize by resolving parent + filename
                        if let (Some(parent), Some(name)) = (file_path.parent(), file_path.file_name()) {
                            parent.canonicalize().map(|p| p.join(name)).unwrap_or_else(|_| file_path.clone())
                        } else {
                            file_path.clone()
                        }
                    });
                    if !canonical_file.starts_with(&canonical_base) {
                        return Err(format!("path traversal denied: {} escapes working dir", file.path));
                    }
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

                Ok(None)
            })
        }),
    );

    // resolvecli → detect or install a CLI tool for an agent provider
    // Each AgentMux version gets its own isolated CLI install at:
    //   ~/.agentmux/<AGENTMUX_VERSION>/cli/<provider>/
    // Never falls back to system PATH.
    let event_bus_rc = state.event_bus.clone();
    engine.register_handler(
        COMMAND_RESOLVE_CLI,
        Box::new(move |data, _ctx| {
            let event_bus = event_bus_rc.clone();
            Box::pin(async move {
                const AGENTMUX_VERSION: &str = env!("CARGO_PKG_VERSION");

                let cmd: CommandResolveCliData = serde_json::from_value(data)
                    .map_err(|e| format!("resolvecli: {e}"))?;
                tracing::info!(
                    provider = %cmd.provider_id,
                    cli = %cmd.cli_command,
                    agentmux_version = AGENTMUX_VERSION,
                    "ResolveCli"
                );

                // Resolve home directory
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .map_err(|_| "cannot determine home directory".to_string())?;

                // Versioned install directory: ~/.agentmux/<version>/cli/<provider>/
                let provider_dir = format!(
                    "{}/.agentmux/{}/cli/{}",
                    home, AGENTMUX_VERSION, cmd.provider_id
                );
                let bin_dir = format!("{}/bin", provider_dir);

                // Expected binary path
                let cli_bin = if cfg!(windows) {
                    format!("{}/{}.exe", bin_dir, cmd.cli_command)
                } else {
                    format!("{}/{}", bin_dir, cmd.cli_command)
                };

                // Also check npm-style path (for npm-based providers like codex/gemini)
                let npm_bin = if cfg!(windows) {
                    format!("{}/node_modules/.bin/{}.cmd", provider_dir, cmd.cli_command)
                } else {
                    format!("{}/node_modules/.bin/{}", provider_dir, cmd.cli_command)
                };

                // Step 1: Check if already installed in versioned directory
                for candidate in [&cli_bin, &npm_bin] {
                    if std::path::Path::new(candidate).exists() {
                        let version = get_cli_version(candidate).await;
                        tracing::info!(
                            path = %candidate, version = %version,
                            "CLI found in versioned install"
                        );
                        return Ok(Some(serde_json::to_value(&ResolveCliResult {
                            cli_path: candidate.clone(),
                            version,
                            source: "local_install".to_string(),
                        }).unwrap()));
                    }
                }

                // Step 2: Not in versioned dir yet. Try to copy from a known location.
                let exe_name = if cfg!(windows) {
                    format!("{}.exe", cmd.cli_command)
                } else {
                    cmd.cli_command.clone()
                };

                // Known locations where CLIs get installed on the system
                let known_paths: Vec<String> = vec![
                    format!("{}/.local/bin/{}", home, exe_name),
                    format!("{}/.claude/local/bin/{}", home, exe_name),
                    format!("{}/AppData/Local/Programs/{}/{}", home, cmd.cli_command, exe_name),
                ];

                // Also check PATH via where/which (to find binary, NOT to use directly)
                let mut system_bin: Option<String> = None;
                for path in &known_paths {
                    if std::path::Path::new(path).exists() {
                        system_bin = Some(path.clone());
                        break;
                    }
                }
                if system_bin.is_none() {
                    let which_cmd = if cfg!(windows) { "where" } else { "which" };
                    if let Ok(output) = tokio::process::Command::new(which_cmd)
                        .arg(&cmd.cli_command)
                        .output()
                        .await
                    {
                        if output.status.success() {
                            let path = String::from_utf8_lossy(&output.stdout)
                                .lines().next().unwrap_or("").trim().to_string();
                            if !path.is_empty() && std::path::Path::new(&path).exists() {
                                system_bin = Some(path);
                            }
                        }
                    }
                }

                // Create versioned directory
                std::fs::create_dir_all(&bin_dir).map_err(|e| {
                    format!("failed to create {}: {e}", bin_dir)
                })?;

                // Fast path: copy existing binary to versioned dir (no network needed)
                if let Some(ref source) = system_bin {
                    tracing::info!(
                        source = %source, target = %cli_bin,
                        "copying existing CLI binary to versioned directory"
                    );
                    std::fs::copy(source, &cli_bin).map_err(|e| {
                        format!("failed to copy {} → {}: {e}", source, cli_bin)
                    })?;
                    let version = get_cli_version(&cli_bin).await;
                    tracing::info!(path = %cli_bin, version = %version, "CLI copied to versioned dir");
                    return Ok(Some(serde_json::to_value(&ResolveCliResult {
                        cli_path: cli_bin,
                        version,
                        source: "local_install".to_string(),
                    }).unwrap()));
                }

                // Slow path: binary not found anywhere — need to install from network
                let install_cmd = if cfg!(windows) {
                    &cmd.windows_install_command
                } else {
                    &cmd.unix_install_command
                };

                if install_cmd.is_empty() {
                    return Err(format!(
                        "{} not found and no install command configured for this provider",
                        cmd.cli_command
                    ));
                }

                tracing::info!(
                    provider = %cmd.provider_id,
                    install_cmd = %install_cmd,
                    target_dir = %provider_dir,
                    "CLI not found locally, installing from network"
                );

                // Determine if this is an npm-based provider or official installer
                let is_npm_install = install_cmd.contains("npm install");

                if is_npm_install {
                    // Step A: Ensure the install directory exists
                    if let Err(e) = std::fs::create_dir_all(&provider_dir) {
                        return Err(format!(
                            "failed to create install directory {}: {}", provider_dir, e
                        ));
                    }

                    // Step B: Find the npm executable path.
                    // On Windows npm is a .cmd batch file; we need the full path for direct
                    // invocation (avoids cmd.exe /C shell escaping issues with paths).
                    let npm_path = {
                        let (find_cmd, arg) = if cfg!(windows) {
                            ("where", "npm.cmd")
                        } else {
                            ("which", "npm")
                        };
                        let out = tokio::process::Command::new(find_cmd)
                            .arg(arg)
                            .output()
                            .await
                            .ok();
                        let found = out
                            .filter(|o| o.status.success())
                            .and_then(|o| String::from_utf8(o.stdout).ok())
                            .map(|s| s.lines().next().unwrap_or("").trim().to_string())
                            .filter(|s| !s.is_empty());
                        match found {
                            Some(p) => p,
                            None => {
                                return Err(format!(
                                    "{} requires Node.js/npm to install. \
                                    Install Node.js from https://nodejs.org then restart AgentMux.",
                                    cmd.cli_command
                                ));
                            }
                        }
                    };

                    // Step C: Run npm install --prefix <dir> <package>@<version>.
                    // Using --prefix avoids cd + shell path issues on Windows.
                    let native_dir = if cfg!(windows) {
                        provider_dir.replace('/', "\\")
                    } else {
                        provider_dir.clone()
                    };
                    let package_spec = format!("{}@{}", cmd.npm_package, cmd.pinned_version);
                    tracing::info!(
                        npm = %npm_path, dir = %native_dir, package = %package_spec,
                        "Running npm install"
                    );

                    use tokio::io::{AsyncBufReadExt, BufReader};
                    use std::sync::{Arc as StdArc, Mutex as StdMutex};
                    let mut child = tokio::process::Command::new(&npm_path)
                        .args(["install", "--prefix", &native_dir, &package_spec])
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|e| format!("failed to run npm install: {e}"))?;

                    // Read stdout and stderr concurrently so npm progress lines appear as they arrive
                    let combined_output = StdArc::new(StdMutex::new(String::new()));

                    let stdout_task = {
                        let emit = {
                            let eb = event_bus.clone();
                            let bid = cmd.block_id.clone();
                            move |line: String| {
                                if let Some(ref b) = bid {
                                    eb.broadcast_event(&crate::backend::eventbus::WSEventType {
                                        eventtype: "cli:install:log".to_string(),
                                        oref: format!("block:{}", b),
                                        data: Some(serde_json::json!({ "line": line })),
                                    });
                                }
                            }
                        };
                        let buf = combined_output.clone();
                        let pipe = child.stdout.take();
                        tokio::spawn(async move {
                            if let Some(p) = pipe {
                                let mut lines = BufReader::new(p).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    tracing::debug!(line = %line, "npm stdout");
                                    emit(line.clone());
                                    let mut g = buf.lock().unwrap();
                                    g.push_str(&line);
                                    g.push('\n');
                                }
                            }
                        })
                    };

                    let stderr_task = {
                        let emit = {
                            let eb = event_bus.clone();
                            let bid = cmd.block_id.clone();
                            move |line: String| {
                                if let Some(ref b) = bid {
                                    eb.broadcast_event(&crate::backend::eventbus::WSEventType {
                                        eventtype: "cli:install:log".to_string(),
                                        oref: format!("block:{}", b),
                                        data: Some(serde_json::json!({ "line": line })),
                                    });
                                }
                            }
                        };
                        let buf = combined_output.clone();
                        let pipe = child.stderr.take();
                        tokio::spawn(async move {
                            if let Some(p) = pipe {
                                let mut lines = BufReader::new(p).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    tracing::debug!(line = %line, "npm stderr");
                                    emit(line.clone());
                                    let mut g = buf.lock().unwrap();
                                    g.push_str(&line);
                                    g.push('\n');
                                }
                            }
                        })
                    };

                    let _ = tokio::join!(stdout_task, stderr_task);
                    let status = child.wait().await
                        .map_err(|e| format!("npm install wait error: {e}"))?;
                    tracing::info!(
                        exit_code = status.code().unwrap_or(-1),
                        "npm install completed"
                    );

                    if !status.success() {
                        let out = combined_output.lock().unwrap().clone();
                        return Err(check_network_error(
                            &out, &cmd.cli_command, install_cmd,
                        ));
                    }

                    // Step D: Verify binary exists at expected path
                    if std::path::Path::new(&npm_bin).exists() {
                        let version = get_cli_version(&npm_bin).await;
                        tracing::info!(path = %npm_bin, version = %version, "CLI installed (npm)");
                        return Ok(Some(serde_json::to_value(&ResolveCliResult {
                            cli_path: npm_bin,
                            version,
                            source: "installed".to_string(),
                        }).unwrap()));
                    }

                    return Err(format!(
                        "npm install completed but binary not found at {}",
                        npm_bin
                    ));
                }

                // Official installer (Claude): run installer with 120s timeout
                let install_future = if cfg!(windows) {
                    tokio::process::Command::new("powershell")
                        .args(["-NoProfile", "-Command", install_cmd])
                        .output()
                } else {
                    tokio::process::Command::new("bash")
                        .args(["-c", install_cmd])
                        .output()
                };

                let install_output = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    install_future,
                ).await
                    .map_err(|_| format!("install timed out after 120s — try manually:\n  {}", install_cmd))?
                    .map_err(|e| format!("failed to run install command: {e}"))?;

                let stdout_str = String::from_utf8_lossy(&install_output.stdout);
                let stderr_str = String::from_utf8_lossy(&install_output.stderr);
                tracing::info!(
                    exit_code = install_output.status.code().unwrap_or(-1),
                    stdout_len = stdout_str.len(),
                    stderr_len = stderr_str.len(),
                    "official installer completed"
                );

                if !install_output.status.success() {
                    let combined = format!("{}{}", stdout_str, stderr_str);
                    return Err(check_network_error(
                        &combined, &cmd.cli_command, install_cmd,
                    ));
                }

                // Find where the official installer placed the binary
                let search_paths = known_paths;

                let mut found_source: Option<String> = None;
                for search in &search_paths {
                    if std::path::Path::new(search).exists() {
                        found_source = Some(search.clone());
                        break;
                    }
                }

                // Also try `where`/`which` as last resort to find installed binary
                if found_source.is_none() {
                    let which_cmd = if cfg!(windows) { "where" } else { "which" };
                    if let Ok(output) = tokio::process::Command::new(which_cmd)
                        .arg(&cmd.cli_command)
                        .output()
                        .await
                    {
                        if output.status.success() {
                            let path = String::from_utf8_lossy(&output.stdout)
                                .lines().next().unwrap_or("").trim().to_string();
                            if !path.is_empty() {
                                found_source = Some(path);
                            }
                        }
                    }
                }

                let source_path = found_source.ok_or_else(|| format!(
                    "installer ran successfully but cannot find {} binary. \
                     Searched: {:?}",
                    cmd.cli_command, search_paths
                ))?;

                // Copy binary to versioned directory
                tracing::info!(
                    source = %source_path,
                    target = %cli_bin,
                    "copying CLI binary to versioned directory"
                );
                std::fs::copy(&source_path, &cli_bin).map_err(|e| {
                    format!("failed to copy {} → {}: {e}", source_path, cli_bin)
                })?;

                let version = get_cli_version(&cli_bin).await;
                tracing::info!(path = %cli_bin, version = %version, "CLI installed successfully");
                Ok(Some(serde_json::to_value(&ResolveCliResult {
                    cli_path: cli_bin,
                    version,
                    source: "installed".to_string(),
                }).unwrap()))
            })
        }),
    );

    // checkcliauth → check if a CLI tool is authenticated
    // For Claude: reads ~/.claude/.credentials.json directly (instant, no subprocess).
    // For other providers: falls back to running the CLI auth check command.
    engine.register_handler(
        COMMAND_CHECK_CLI_AUTH,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandCheckCliAuthData = serde_json::from_value(data)
                    .map_err(|e| format!("checkcliauth: {e}"))?;
                tracing::info!(cli = %cmd.cli_path, "CheckCliAuth");

                // Fast path: read credentials file directly (Claude)
                if cmd.cli_path.contains("claude") {
                    // Use CLAUDE_CONFIG_DIR from auth_env if provided (isolated auth dir).
                    // Fall back to ~/.claude/ for legacy/non-isolated invocations.
                    let creds_path = if let Some(config_dir) = cmd.auth_env.get("CLAUDE_CONFIG_DIR") {
                        format!("{}/.credentials.json", config_dir)
                    } else {
                        let home = std::env::var("HOME")
                            .or_else(|_| std::env::var("USERPROFILE"))
                            .unwrap_or_default();
                        format!("{}/.claude/.credentials.json", home)
                    };

                    if let Ok(content) = std::fs::read_to_string(&creds_path) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                            // Check claudeAiOauth credentials
                            let oauth = json.get("claudeAiOauth");
                            let has_token = oauth
                                .and_then(|o| o.get("accessToken"))
                                .and_then(|v| v.as_str())
                                .map(|s| !s.is_empty())
                                .unwrap_or(false);

                            let has_refresh = oauth
                                .and_then(|o| o.get("refreshToken"))
                                .and_then(|v| v.as_str())
                                .map(|s| !s.is_empty())
                                .unwrap_or(false);

                            // Authenticated if we have an access token OR a refresh token
                            // (CLI auto-refreshes expired tokens transparently)
                            let authenticated = has_token || has_refresh;

                            let subscription = oauth
                                .and_then(|o| o.get("subscriptionType"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            let auth_method = Some("claude.ai oauth".to_string());

                            tracing::info!(
                                authenticated = authenticated,
                                subscription = ?subscription,
                                has_refresh = has_refresh,
                                "claude auth check (credentials file)"
                            );

                            let result = CheckCliAuthResult {
                                authenticated,
                                email: subscription.clone(), // no email in creds, show subscription
                                auth_method,
                                raw_output: format!("subscription: {}", subscription.unwrap_or_default()),
                            };
                            return Ok(Some(serde_json::to_value(&result).unwrap()));
                        }
                    }
                    // Credentials file not found or unparseable — not authenticated
                    let result = CheckCliAuthResult {
                        authenticated: false,
                        email: None,
                        auth_method: None,
                        raw_output: "no credentials file found".to_string(),
                    };
                    return Ok(Some(serde_json::to_value(&result).unwrap()));
                }

                // Slow path: run CLI auth check command (other providers)
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    {
                        let mut check_cmd = tokio::process::Command::new(&cmd.cli_path);
                        check_cmd.args(&cmd.auth_check_args);
                        for (k, v) in &cmd.auth_env {
                            check_cmd.env(k, v);
                        }
                        check_cmd.output()
                    },
                ).await
                    .map_err(|_| "auth check timed out (25s)".to_string())?
                    .map_err(|e| format!("failed to run auth check: {e}"))?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                let mut authenticated = false;
                let mut email = None;
                let mut auth_method = None;

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    authenticated = json.get("loggedIn")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    email = json.get("email")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    auth_method = json.get("authMethod")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                } else {
                    authenticated = output.status.success();
                }

                let raw_output = if !stdout.is_empty() { stdout } else { stderr };

                let result = CheckCliAuthResult {
                    authenticated,
                    email,
                    auth_method,
                    raw_output,
                };
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );

    // runclilogin → spawn CLI login flow, extract OAuth URL from output, return immediately
    engine.register_handler(
        "runclilogin",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandRunCliLoginData = serde_json::from_value(data)
                    .map_err(|e| format!("runclilogin: {e}"))?;
                tracing::info!(cli = %cmd.cli_path, args = ?cmd.login_args, "RunCliLogin");

                // Spawn the login process. On most platforms it opens the browser
                // automatically and writes the URL to stderr. On Windows, stderr is
                // block-buffered when piped so we can't reliably read it in real-time.
                // Strategy: inherit stdout/stderr so the CLI can open the browser normally,
                // then return immediately — the frontend polls auth status until done.
                let mut child = tokio::process::Command::new(&cmd.cli_path)
                    .args(&cmd.login_args)
                    .envs(&cmd.auth_env)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| format!("failed to spawn login: {e}"))?;

                // Keep child alive in background — it waits for the user to complete OAuth
                tokio::spawn(async move { let _ = child.wait().await; });

                let result = RunCliLoginResult { auth_url: None, raw_output: String::new() };
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );

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

    // listforgeagents → return all forge agents
    let wstore_lfa = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_FORGE_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_lfa.clone();
            Box::pin(async move {
                let agents = wstore.forge_list().map_err(|e| format!("listforgeagents: {e}"))?;
                Ok(Some(serde_json::to_value(&agents).unwrap_or_default()))
            })
        }),
    );

    // createforgeagent → insert new agent, broadcast forgeagents:changed
    let wstore_cfa = state.wstore.clone();
    let broker_cfa = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_FORGE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfa.clone();
            let broker = broker_cfa.clone();
            Box::pin(async move {
                let cmd: CommandCreateForgeAgentData = serde_json::from_value(data)
                    .map_err(|e| format!("createforgeagent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let agent = ForgeAgent {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: cmd.name,
                    icon: cmd.icon,
                    provider: cmd.provider,
                    description: cmd.description,
                    working_directory: cmd.working_directory,
                    shell: cmd.shell,
                    provider_flags: cmd.provider_flags,
                    auto_start: cmd.auto_start,
                    restart_on_crash: cmd.restart_on_crash,
                    idle_timeout_minutes: cmd.idle_timeout_minutes,
                    created_at: now,
                    agent_type: cmd.agent_type,
                    environment: cmd.environment,
                    agent_bus_id: cmd.agent_bus_id,
                    is_seeded: 0,
                };
                wstore.forge_insert(&agent).map_err(|e| format!("createforgeagent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // updateforgeagent → update existing agent, broadcast forgeagents:changed
    let wstore_ufa = state.wstore.clone();
    let broker_ufa = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_FORGE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufa.clone();
            let broker = broker_ufa.clone();
            Box::pin(async move {
                let cmd: CommandUpdateForgeAgentData = serde_json::from_value(data)
                    .map_err(|e| format!("updateforgeagent: {e}"))?;
                // Fetch existing to preserve created_at
                let existing = wstore.forge_list().map_err(|e| format!("updateforgeagent: {e}"))?;
                let old = existing.iter().find(|a| a.id == cmd.id)
                    .ok_or_else(|| format!("updateforgeagent: agent {} not found", cmd.id))?;
                let agent = ForgeAgent {
                    id: cmd.id,
                    name: cmd.name,
                    icon: cmd.icon,
                    provider: cmd.provider,
                    description: cmd.description,
                    working_directory: cmd.working_directory,
                    shell: cmd.shell,
                    provider_flags: cmd.provider_flags,
                    auto_start: cmd.auto_start,
                    restart_on_crash: cmd.restart_on_crash,
                    idle_timeout_minutes: cmd.idle_timeout_minutes,
                    created_at: old.created_at,
                    agent_type: cmd.agent_type,
                    environment: cmd.environment,
                    agent_bus_id: cmd.agent_bus_id,
                    is_seeded: old.is_seeded,
                };
                let found = wstore.forge_update(&agent).map_err(|e| format!("updateforgeagent: {e}"))?;
                if !found {
                    return Err(format!("updateforgeagent: agent {} not found", agent.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // deleteforgeagent → delete agent by id, broadcast forgeagents:changed
    let wstore_dfa = state.wstore.clone();
    let broker_dfa = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_FORGE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfa.clone();
            let broker = broker_dfa.clone();
            Box::pin(async move {
                let cmd: CommandDeleteForgeAgentData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteforgeagent: {e}"))?;
                wstore.forge_delete(&cmd.id).map_err(|e| format!("deleteforgeagent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    // getforgecontent → return a single content blob for an agent
    let wstore_gfc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_FORGE_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gfc.clone();
            Box::pin(async move {
                let cmd: CommandGetForgeContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getforgecontent: {e}"))?;
                let content = wstore.forge_get_content(&cmd.agent_id, &cmd.content_type)
                    .map_err(|e| format!("getforgecontent: {e}"))?;
                Ok(content.map(|c| serde_json::to_value(&c).unwrap_or_default()))
            })
        }),
    );

    // setforgecontent → upsert a content blob, broadcast forgecontent:changed
    let wstore_sfc = state.wstore.clone();
    let broker_sfc = state.broker.clone();
    engine.register_handler(
        COMMAND_SET_FORGE_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfc.clone();
            let broker = broker_sfc.clone();
            Box::pin(async move {
                let cmd: CommandSetForgeContentData = serde_json::from_value(data)
                    .map_err(|e| format!("setforgecontent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let content = ForgeContent {
                    agent_id: cmd.agent_id,
                    content_type: cmd.content_type,
                    content: cmd.content,
                    updated_at: now,
                };
                wstore.forge_set_content(&content).map_err(|e| format!("setforgecontent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgecontent:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&content).unwrap_or_default()))
            })
        }),
    );

    // getallforgecontent → return all content blobs for an agent
    let wstore_gafc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_ALL_FORGE_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gafc.clone();
            Box::pin(async move {
                let cmd: CommandGetAllForgeContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getallforgecontent: {e}"))?;
                let contents = wstore.forge_get_all_content(&cmd.agent_id)
                    .map_err(|e| format!("getallforgecontent: {e}"))?;
                Ok(Some(serde_json::to_value(&contents).unwrap_or_default()))
            })
        }),
    );

    // ── Forge Skills handlers ──────────────────────────────────────────────

    // listforgeskills → return all skills for an agent
    let wstore_lfs = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_FORGE_SKILLS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfs.clone();
            Box::pin(async move {
                let cmd: CommandListForgeSkillsData = serde_json::from_value(data)
                    .map_err(|e| format!("listforgeskills: {e}"))?;
                let skills = wstore.forge_list_skills(&cmd.agent_id)
                    .map_err(|e| format!("listforgeskills: {e}"))?;
                Ok(Some(serde_json::to_value(&skills).unwrap_or_default()))
            })
        }),
    );

    // createforgeskill → insert new skill, broadcast forgeskills:changed
    let wstore_cfs = state.wstore.clone();
    let broker_cfs = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_FORGE_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfs.clone();
            let broker = broker_cfs.clone();
            Box::pin(async move {
                let cmd: CommandCreateForgeSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("createforgeskill: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let skill = ForgeSkill {
                    id: uuid::Uuid::new_v4().to_string(),
                    agent_id: cmd.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: now,
                };
                wstore.forge_insert_skill(&skill).map_err(|e| format!("createforgeskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // updateforgeskill → update existing skill, broadcast forgeskills:changed
    let wstore_ufs = state.wstore.clone();
    let broker_ufs = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_FORGE_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufs.clone();
            let broker = broker_ufs.clone();
            Box::pin(async move {
                let cmd: CommandUpdateForgeSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("updateforgeskill: {e}"))?;
                let existing = wstore.forge_get_skill(&cmd.id)
                    .map_err(|e| format!("updateforgeskill: {e}"))?
                    .ok_or_else(|| format!("updateforgeskill: skill {} not found", cmd.id))?;
                let skill = ForgeSkill {
                    id: cmd.id,
                    agent_id: existing.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: existing.created_at,
                };
                let found = wstore.forge_update_skill(&skill).map_err(|e| format!("updateforgeskill: {e}"))?;
                if !found {
                    return Err(format!("updateforgeskill: skill {} not found", skill.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // deleteforgeskill → delete skill by id, broadcast forgeskills:changed
    let wstore_dfs = state.wstore.clone();
    let broker_dfs = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_FORGE_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfs.clone();
            let broker = broker_dfs.clone();
            Box::pin(async move {
                let cmd: CommandDeleteForgeSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteforgeskill: {e}"))?;
                wstore.forge_delete_skill(&cmd.id).map_err(|e| format!("deleteforgeskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    // ── Forge History handlers ─────────────────────────────────────────────

    // appendforgehistory → append a history entry, broadcast forgehistory:changed
    let wstore_afh = state.wstore.clone();
    let broker_afh = state.broker.clone();
    engine.register_handler(
        COMMAND_APPEND_FORGE_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_afh.clone();
            let broker = broker_afh.clone();
            Box::pin(async move {
                let cmd: CommandAppendForgeHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("appendforgehistory: {e}"))?;
                let entry = wstore.forge_append_history(&cmd.agent_id, &cmd.entry)
                    .map_err(|e| format!("appendforgehistory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgehistory:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&entry).unwrap_or_default()))
            })
        }),
    );

    // listforgehistory → return history entries with pagination
    let wstore_lfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_FORGE_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfh.clone();
            Box::pin(async move {
                let cmd: CommandListForgeHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("listforgehistory: {e}"))?;
                let entries = wstore.forge_list_history(
                    &cmd.agent_id,
                    cmd.session_date.as_deref(),
                    cmd.limit,
                    cmd.offset,
                ).map_err(|e| format!("listforgehistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // searchforgehistory → search history entries by query
    let wstore_sfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_SEARCH_FORGE_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfh.clone();
            Box::pin(async move {
                let cmd: CommandSearchForgeHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("searchforgehistory: {e}"))?;
                let entries = wstore.forge_search_history(&cmd.agent_id, &cmd.query, cmd.limit)
                    .map_err(|e| format!("searchforgehistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // ── Forge Import handler ───────────────────────────────────────────────

    // importforgefromclaw → read claw workspace, create agent + content
    let wstore_ifc = state.wstore.clone();
    let broker_ifc = state.broker.clone();
    engine.register_handler(
        COMMAND_IMPORT_FORGE_FROM_CLAW,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ifc.clone();
            let broker = broker_ifc.clone();
            Box::pin(async move {
                let cmd: CommandImportForgeFromClawData = serde_json::from_value(data)
                    .map_err(|e| format!("importforgefromclaw: {e}"))?;

                let workspace_path = std::path::Path::new(&cmd.workspace_path);
                if !workspace_path.exists() {
                    return Err(format!("importforgefromclaw: path does not exist: {}", cmd.workspace_path));
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                // Detect provider from .claude/settings.json if present
                let mut provider = "claude".to_string();
                let settings_path = workspace_path.join(".claude").join("settings.json");
                if settings_path.exists() {
                    if let Ok(settings_str) = std::fs::read_to_string(&settings_path) {
                        if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&settings_str) {
                            if let Some(p) = settings.get("provider").and_then(|v| v.as_str()) {
                                provider = p.to_string();
                            }
                        }
                    }
                }

                // Create the agent
                let agent = ForgeAgent {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: cmd.agent_name.clone(),
                    icon: "\u{2726}".to_string(),
                    provider,
                    description: format!("Imported from {}", cmd.workspace_path),
                    working_directory: cmd.workspace_path.clone(),
                    shell: String::new(),
                    provider_flags: String::new(),
                    auto_start: 0,
                    restart_on_crash: 0,
                    idle_timeout_minutes: 0,
                    created_at: now,
                    agent_type: "standalone".to_string(),
                    environment: String::new(),
                    agent_bus_id: String::new(),
                    is_seeded: 0,
                };
                wstore.forge_insert(&agent).map_err(|e| format!("importforgefromclaw: {e}"))?;

                // Read CLAUDE.md → agentmd content
                let claude_md_path = workspace_path.join("CLAUDE.md");
                if claude_md_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&claude_md_path) {
                        let fc = ForgeContent {
                            agent_id: agent.id.clone(),
                            content_type: "agentmd".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.forge_set_content(&fc);
                    }
                }

                // Read .mcp.json → mcp content
                let mcp_path = workspace_path.join(".mcp.json");
                if mcp_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&mcp_path) {
                        let fc = ForgeContent {
                            agent_id: agent.id.clone(),
                            content_type: "mcp".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.forge_set_content(&fc);
                    }
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // reseedforgeagents → delete all seeded agents and re-run seed from manifest
    let wstore_rsfa = state.wstore.clone();
    let broker_rsfa = state.broker.clone();
    engine.register_handler(
        COMMAND_RESEED_FORGE_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_rsfa.clone();
            let broker = broker_rsfa.clone();
            Box::pin(async move {
                // Delete all previously seeded agents (cascade deletes content, skills, history)
                let deleted = wstore.forge_delete_seeded()
                    .map_err(|e| format!("reseedforgeagents: delete seeded: {e}"))?;

                // Re-run seed
                let report = crate::backend::forge_seed::seed_forge_agents(&wstore)
                    .map_err(|e| format!("reseedforgeagents: seed: {e}"))?;

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(json!({
                    "deleted": deleted,
                    "created": report.created,
                    "skipped": report.skipped,
                })))
            })
        }),
    );
}

fn check_network_error(combined_output: &str, cli_command: &str, install_cmd: &str) -> String {
    let lower = combined_output.to_lowercase();
    if lower.contains("could not resolve host")
        || lower.contains("network")
        || lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("no internet")
        || lower.contains("getaddrinfo")
        || lower.contains("enotfound")
    {
        format!(
            "no internet connection — cannot install {}. \
             Connect to the internet and try again, or install manually:\n  {}",
            cli_command, install_cmd
        )
    } else {
        format!(
            "install failed: {}",
            combined_output.chars().take(500).collect::<String>()
        )
    }
}

async fn get_cli_version(cli_path: &str) -> String {
    match tokio::process::Command::new(cli_path)
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_string()
        }
        _ => "unknown".to_string(),
    }
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
