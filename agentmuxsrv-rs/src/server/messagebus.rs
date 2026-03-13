//! HTTP endpoint handlers for the local MessageBus.
//!
//! Routes:
//!   POST /api/bus/register      - Register an agent
//!   POST /api/bus/send          - Send a message to an agent
//!   POST /api/bus/inject        - Inject into an agent's terminal (jekt)
//!   POST /api/bus/broadcast     - Broadcast to all agents
//!   GET  /api/bus/messages      - Read queued messages (polling fallback)
//!   GET  /api/bus/agents        - List connected agents
//!   POST /api/bus/messages/delete - Delete messages by ID

use axum::{
    extract::{Query, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::messagebus::{BusMessage, MessageType, Priority};
use crate::backend::reactive::InjectionRequest;
use super::AppState;

// ---- Request types ----

#[derive(Deserialize)]
pub(super) struct RegisterRequest {
    agent_id: String,
}

#[derive(Deserialize)]
pub(super) struct SendRequest {
    from: String,
    to: String,
    payload: String,
    #[serde(default)]
    priority: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct InjectRequest {
    from: String,
    target: String,
    message: String,
    #[serde(default)]
    priority: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct BroadcastRequest {
    from: String,
    payload: String,
    #[serde(default)]
    priority: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ReadMessagesQuery {
    agent_id: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

#[derive(Deserialize)]
pub(super) struct DeleteMessagesRequest {
    agent_id: String,
    message_ids: Vec<String>,
}

// ---- Helpers ----

fn parse_priority(s: &Option<String>) -> Priority {
    match s.as_deref() {
        Some("high") => Priority::High,
        Some("urgent") => Priority::Urgent,
        _ => Priority::Normal,
    }
}

// ---- Handlers ----

/// POST /api/bus/register
pub(super) async fn handle_register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Json<Value> {
    // HTTP-registered agents use polling via /api/bus/messages.
    // WebSocket agents get their push channel wired in the WS handler.
    state.messagebus.register_http(&req.agent_id);
    Json(json!({
        "status": "registered",
        "agent_id": req.agent_id,
    }))
}

/// POST /api/bus/send
pub(super) async fn handle_send(
    State(state): State<AppState>,
    Json(req): Json<SendRequest>,
) -> Json<Value> {
    let priority = parse_priority(&req.priority);
    let msg = BusMessage::new(&req.from, &req.to, MessageType::Send, &req.payload, priority);
    let msg_id = msg.id.clone();

    match state.messagebus.send(msg) {
        Ok(()) => Json(json!({
            "status": "sent",
            "message_id": msg_id,
            "to": req.to,
        })),
        Err(e) => Json(json!({
            "status": "error",
            "error": e,
        })),
    }
}

/// POST /api/bus/inject
///
/// Tries ReactiveHandler first (direct PTY write via blockcontroller).
/// Falls back to MessageBus WebSocket push if agent has no block_id registered.
pub(super) async fn handle_inject(
    State(state): State<AppState>,
    Json(req): Json<InjectRequest>,
) -> Json<Value> {
    // Try direct PTY injection via ReactiveHandler (agent has registered block_id)
    let reactive_req = InjectionRequest {
        target_agent: req.target.clone(),
        message: req.message.clone(),
        source_agent: Some(req.from.clone()),
        request_id: None,
        priority: req.priority.clone(),
        wait_for_idle: false,
    };
    let resp = state.reactive_handler.inject_message(reactive_req);
    if resp.success {
        return Json(json!({
            "status": "injected",
            "via": "pty",
            "block_id": resp.block_id,
            "target": req.target,
        }));
    }

    // Agent not registered with a block_id — fall back to MessageBus WS push
    // (only fall back on "agent not found", propagate other errors)
    let is_not_found = resp.error.as_deref().map(|e| e.contains("not found")).unwrap_or(false);
    if !is_not_found {
        return Json(json!({
            "status": "error",
            "error": resp.error,
        }));
    }

    let priority = parse_priority(&req.priority);
    match state.messagebus.inject(&req.from, &req.target, &req.message, priority) {
        Ok(msg_id) => Json(json!({
            "status": "injected",
            "via": "messagebus",
            "message_id": msg_id,
            "target": req.target,
        })),
        Err(e) => Json(json!({
            "status": "error",
            "error": e,
        })),
    }
}

/// POST /api/bus/broadcast
pub(super) async fn handle_broadcast(
    State(state): State<AppState>,
    Json(req): Json<BroadcastRequest>,
) -> Json<Value> {
    let priority = parse_priority(&req.priority);

    match state.messagebus.broadcast(&req.from, &req.payload, priority) {
        Ok(delivered) => Json(json!({
            "status": "broadcast",
            "delivered": delivered,
        })),
        Err(e) => Json(json!({
            "status": "error",
            "error": e,
        })),
    }
}

/// GET /api/bus/messages?agent_id=...&limit=...
pub(super) async fn handle_read_messages(
    State(state): State<AppState>,
    Query(query): Query<ReadMessagesQuery>,
) -> Json<Value> {
    let messages = state.messagebus.read_messages(&query.agent_id, query.limit);
    Json(json!({
        "agent_id": query.agent_id,
        "messages": messages,
        "count": messages.len(),
    }))
}

/// GET /api/bus/agents
pub(super) async fn handle_list_agents(
    State(state): State<AppState>,
) -> Json<Value> {
    let agents = state.messagebus.list_agents();
    Json(json!({
        "agents": agents,
        "total_count": agents.len(),
    }))
}

/// POST /api/bus/messages/delete
pub(super) async fn handle_delete_messages(
    State(state): State<AppState>,
    Json(req): Json<DeleteMessagesRequest>,
) -> Json<Value> {
    let deleted = state.messagebus.delete_messages(&req.agent_id, &req.message_ids);
    Json(json!({
        "status": "deleted",
        "deleted": deleted,
    }))
}
