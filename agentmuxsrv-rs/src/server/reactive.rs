use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::reactive::InjectionRequest;

use super::AppState;

pub(super) async fn handle_reactive_inject(
    State(state): State<AppState>,
    Json(req): Json<InjectionRequest>,
) -> Json<serde_json::Value> {
    let resp = state.reactive_handler.inject_message(req);
    Json(serde_json::to_value(&resp).unwrap_or_default())
}

pub(super) async fn handle_reactive_agents(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let agents = state.reactive_handler.list_agents();
    Json(serde_json::to_value(&agents).unwrap_or(json!([])))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentQuery {
    id: Option<String>,
}

pub(super) async fn handle_reactive_agent(
    State(state): State<AppState>,
    Query(params): Query<AgentQuery>,
) -> Response {
    let id = match &params.id {
        Some(id) if !id.is_empty() => id.as_str(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing id param"})),
            )
                .into_response()
        }
    };
    match state.reactive_handler.get_agent(id) {
        Some(agent) => Json(serde_json::to_value(&agent).unwrap_or_default()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "agent not found"})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
}
fn default_audit_limit() -> usize {
    100
}

pub(super) async fn handle_reactive_audit(
    State(state): State<AppState>,
    Query(params): Query<AuditQuery>,
) -> Json<serde_json::Value> {
    let log = state.reactive_handler.get_audit_log(params.limit);
    Json(serde_json::to_value(&log).unwrap_or(json!([])))
}

#[derive(serde::Deserialize)]
pub(super) struct RegisterRequest {
    agent_id: String,
    block_id: String,
    tab_id: Option<String>,
}

pub(super) async fn handle_reactive_register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Response {
    match state
        .reactive_handler
        .register_agent(&req.agent_id, &req.block_id, req.tab_id.as_deref())
    {
        Ok(()) => Json(json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct UnregisterRequest {
    agent_id: String,
}

pub(super) async fn handle_reactive_unregister(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> Json<serde_json::Value> {
    state.reactive_handler.unregister_agent(&req.agent_id);
    Json(json!({"success": true}))
}

pub(super) async fn handle_reactive_poller_stats(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let stats = state.poller.stats();
    Json(serde_json::to_value(&stats).unwrap_or(json!({})))
}

#[derive(serde::Deserialize)]
pub(super) struct PollerConfigRequest {
    url: Option<String>,
    token: Option<String>,
}

pub(super) async fn handle_reactive_poller_config(
    State(state): State<AppState>,
    Json(req): Json<PollerConfigRequest>,
) -> Json<serde_json::Value> {
    state.poller.reconfigure(req.url, req.token);
    Json(json!({"success": true}))
}

pub(super) async fn handle_reactive_poller_status(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let status = state.poller.status();
    Json(serde_json::to_value(&status).unwrap_or(json!({})))
}
