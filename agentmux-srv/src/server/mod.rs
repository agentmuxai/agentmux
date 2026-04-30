pub(crate) mod cli_handlers;
mod files;
mod app_api;
mod forge_handlers;
mod messagebus;
mod reactive;
pub(crate) mod service;
mod tool_handlers;
mod websocket;

#[cfg(test)]
pub(crate) mod tests;

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

use crate::backend::eventbus::EventBus;
use crate::backend::lan_discovery::LanDiscovery;
use crate::backend::messagebus::MessageBus;
use crate::backend::reactive::{Poller, ReactiveHandler};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::wstore::WaveStore;
use crate::backend::history::HistoryService;
use crate::backend::subagent_watcher::SubagentWatcher;
use crate::backend::wconfig;
use crate::backend::wps::Broker;

// ---- AppState ----

#[derive(Clone)]
pub struct AppState {
    pub auth_key: String,
    pub version: String,
    pub app_path: String,
    pub wstore: Arc<WaveStore>,
    pub filestore: Arc<FileStore>,
    pub event_bus: Arc<EventBus>,
    pub broker: Arc<Broker>,
    pub reactive_handler: &'static ReactiveHandler,
    pub poller: Arc<Poller>,
    pub config_watcher: Arc<wconfig::ConfigWatcher>,
    pub messagebus: Arc<MessageBus>,
    pub subagent_watcher: Arc<SubagentWatcher>,
    pub history_service: Arc<HistoryService>,
    /// Tracks every OS-level process each agent CLI has spawned, via
    /// platform-specific mechanisms (Windows Job Objects, Linux cgroups,
    /// macOS process groups). Surfaces the tree to the swarm pane and
    /// provides kill-tree on pane close / host exit.
    /// See `backend::process_tracker` + `agentmux-ai/AGENT_SPAWNED_PROCESSES_SPEC.md`.
    pub process_tracker: Arc<crate::backend::process_tracker::registry::AgentProcessRegistry>,
    pub lan_discovery: Option<Arc<LanDiscovery>>,
    /// Local HTTP URL of this instance (e.g. "http://127.0.0.1:PORT").
    /// Used for cross-instance inject forwarding and file registry entries.
    pub local_web_url: String,
    /// Shared HTTP client for cross-instance inject forwarding.
    pub http_client: reqwest::Client,
    /// Phase E.2c.2 — srv reducer's canonical state. Workspace HTTP/WS
    /// RPC handlers route through the reducer (dispatch
    /// `Command::Create/Delete/...Workspace` and read out of
    /// `state.workspaces`); the persist subscriber mirrors emitted
    /// events back to SQLite. Tab/Block RPC migrations land in
    /// E.2c.3 / E.2c.4.
    pub srv_state: std::sync::Arc<tokio::sync::Mutex<crate::state::State>>,
    /// Phase E.2c.2 — broadcast bus for srv reducer events. RPC
    /// handlers publish reducer-emitted events here so the persist
    /// subscriber writes them back to SQLite. Pipe IPC server (when
    /// bound) shares the same bus.
    pub srv_events_tx: tokio::sync::broadcast::Sender<agentmux_common::ipc::Event>,
    /// Phase E.5.5 — monotonic saga-id allocator. Each saga
    /// (TearOffTab, TearOffBlock, RestoreTornOffTab, etc.) calls
    /// `fetch_add` to claim a unique id; the id is stamped onto
    /// `Event::SagaStarted/Completed/Failed` so subscribers can
    /// correlate. Per-instance scope (no cross-process sharing — see
    /// `docs/retro/saga-coordinator-location-analysis-2026-04-30.md`).
    pub saga_id_alloc: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// Build the Axum router with all routes, auth middleware, and CORS.
pub fn build_router(state: AppState) -> Router {
    // CORS: allow all origins, methods, headers (matching Go pkg/web/web.go:536-573)
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(vec![
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::ACCEPT,
            "X-Session-Id".parse().unwrap(),
            "X-AuthKey".parse().unwrap(),
            "X-Requested-With".parse().unwrap(),
            "x-vercel-ai-ui-message-stream".parse().unwrap(),
        ]);

    // No-auth routes (matching Go SkipAuth: true — localhost-only reactive endpoints)
    let reactive_routes = Router::new()
        .route("/agentmux/reactive/inject", post(reactive::handle_reactive_inject))
        .route("/agentmux/reactive/agents", get(reactive::handle_reactive_agents))
        .route("/agentmux/reactive/agent", get(reactive::handle_reactive_agent))
        .route("/agentmux/reactive/audit", get(reactive::handle_reactive_audit))
        .route("/agentmux/reactive/register", post(reactive::handle_reactive_register))
        .route(
            "/agentmux/reactive/unregister",
            post(reactive::handle_reactive_unregister),
        )
        .route(
            "/agentmux/reactive/poller/stats",
            get(reactive::handle_reactive_poller_stats),
        )
        .route(
            "/agentmux/reactive/poller/config",
            post(reactive::handle_reactive_poller_config),
        )
        .route(
            "/agentmux/reactive/poller/status",
            get(reactive::handle_reactive_poller_status),
        );

    // MessageBus routes (authed, localhost-only)
    let bus_routes = Router::new()
        .route("/api/bus/register", post(messagebus::handle_register))
        .route("/api/bus/send", post(messagebus::handle_send))
        .route("/api/bus/inject", post(messagebus::handle_inject))
        .route("/api/bus/broadcast", post(messagebus::handle_broadcast))
        .route("/api/bus/messages", get(messagebus::handle_read_messages))
        .route("/api/bus/messages/delete", post(messagebus::handle_delete_messages))
        .route("/api/bus/agents", get(messagebus::handle_list_agents));

    let authed_routes = Router::new()
        .route("/ws", get(websocket::handle_ws))
        .route("/agentmux/service", post(service::handle_service))
        .route("/agentmux/file", get(files::handle_wave_file))
        .route("/agentmux/stream-file", get(stub_501))
        .route("/agentmux/stream-file/*path", get(stub_501))
        .route("/agentmux/stream-local-file", get(stub_501))
        .route("/api/post-chat-message", get(stub_501).post(stub_501))
        .route("/docsite/*path", get(files::handle_docsite))
        .route("/schema/*path", get(files::handle_schema))
        .route("/api/lan-instances", get(handle_lan_instances))
        .merge(bus_routes)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Health endpoint (no auth)
    let health = Router::new().route("/", get(health_handler));

    Router::new()
        .merge(health)
        .merge(reactive_routes)
        .merge(authed_routes)
        .layer(cors)
        .with_state(state)
}

// ---- Health ----

async fn health_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": state.version,
    }))
}

async fn handle_lan_instances(State(state): State<AppState>) -> Json<serde_json::Value> {
    let instances = state
        .lan_discovery
        .as_ref()
        .map(|d| d.get_instances())
        .unwrap_or_default();
    Json(json!(instances))
}

async fn stub_501() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "not implemented"})),
    )
}

// ---- Auth Middleware ----

/// Auth middleware matching Go pkg/authkey/authkey.go:18-42.
async fn auth_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }

    let auth_key = req
        .headers()
        .get("X-AuthKey")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let auth_key = auth_key.or_else(|| {
        req.uri().query().and_then(|q| {
            q.split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(k, _)| *k == "authkey")
                .map(|(_, v)| v.to_string())
        })
    });

    match auth_key {
        Some(key) if key == state.auth_key => next.run(req).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}
