use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::reactive::InjectionRequest;
use crate::backend::reactive::registry as agent_registry;
use crate::backend::wavebase;

use super::AppState;

pub(super) async fn handle_reactive_inject(
    State(state): State<AppState>,
    Json(req): Json<InjectionRequest>,
) -> Json<serde_json::Value> {
    tracing::info!(
        target_agent = %req.target_agent,
        source_agent = ?req.source_agent,
        msg_len = req.message.len(),
        "reactive inject request received"
    );

    // 1. Try local ReactiveHandler first (fast path — same instance).
    let resp = state.reactive_handler.inject_message(req.clone());
    if resp.success {
        return Json(serde_json::to_value(&resp).unwrap_or_default());
    }

    // 2. On "agent not found", check cross-instance file registry and forward.
    let is_not_found = resp
        .error
        .as_deref()
        .map(|e| e.starts_with("agent not found"))
        .unwrap_or(false);

    if is_not_found {
        let data_dir = wavebase::get_wave_data_dir();
        if let Some(entry) = agent_registry::lookup(&data_dir, &req.target_agent) {
            // Guard against self-forwarding loops.
            if entry.local_url != state.local_web_url {
                let forward_url = format!("{}/wave/reactive/inject", entry.local_url);
                tracing::debug!(
                    target = %req.target_agent,
                    url = %forward_url,
                    "cross-instance inject forward"
                );
                match state.http_client.post(&forward_url).json(&req).send().await {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            return Json(body);
                        }
                    }
                    Ok(r) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            status = %r.status(),
                            url = %forward_url,
                            "cross-instance forward: non-success status"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            error = %e,
                            url = %forward_url,
                            "cross-instance forward failed — removing stale registry entry"
                        );
                        // Remove stale entry so next call doesn't retry a dead instance.
                        agent_registry::remove(&data_dir, &req.target_agent);
                    }
                }
            }
        }
    }

    // 3. Return original error (agentbus-client will fall back to cloud).
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
    tracing::info!(
        agent_id = %req.agent_id,
        block_id = %req.block_id,
        "reactive register request"
    );
    match state
        .reactive_handler
        .register_agent(&req.agent_id, &req.block_id, req.tab_id.as_deref())
    {
        Ok(()) => {
            // Also write to cross-instance file registry so other AgentMux
            // instances can forward inject requests to this one.
            let data_dir = wavebase::get_wave_data_dir();
            agent_registry::write(&data_dir, &req.agent_id, &state.local_web_url, &req.block_id);
            Json(json!({"success": true})).into_response()
        }
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
    // Also remove from cross-instance file registry.
    let data_dir = wavebase::get_wave_data_dir();
    agent_registry::remove(&data_dir, &req.agent_id);
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
