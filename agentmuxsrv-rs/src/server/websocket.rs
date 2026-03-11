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
    COMMAND_DELETE_FORGE_AGENT, CommandCreateForgeAgentData, CommandUpdateForgeAgentData,
    CommandDeleteForgeAgentData,
};
use crate::backend::storage::ForgeAgent;
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
                    let priority = match incoming.priority.as_deref() {
                        Some("high") => crate::backend::messagebus::Priority::High,
                        Some("urgent") => crate::backend::messagebus::Priority::Urgent,
                        _ => crate::backend::messagebus::Priority::Normal,
                    };
                    match state.messagebus.inject(from, target, message, priority) {
                        Ok(msg_id) => {
                            let ack = json!({ "type": "bus:injected", "message_id": msg_id });
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
                    created_at: now,
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
                    created_at: old.created_at,
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
