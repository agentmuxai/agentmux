mod ai;
mod files;
mod messagebus;
mod reactive;
pub(crate) mod service;
mod websocket;

#[cfg(test)]
mod tests;

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
    /// Local HTTP URL of this instance (e.g. "http://127.0.0.1:PORT").
    /// Used for cross-instance inject forwarding and file registry entries.
    pub local_web_url: String,
    /// Shared HTTP client for cross-instance inject forwarding.
    pub http_client: reqwest::Client,
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
        .route("/wave/reactive/inject", post(reactive::handle_reactive_inject))
        .route("/wave/reactive/agents", get(reactive::handle_reactive_agents))
        .route("/wave/reactive/agent", get(reactive::handle_reactive_agent))
        .route("/wave/reactive/audit", get(reactive::handle_reactive_audit))
        .route("/wave/reactive/register", post(reactive::handle_reactive_register))
        .route(
            "/wave/reactive/unregister",
            post(reactive::handle_reactive_unregister),
        )
        .route(
            "/wave/reactive/poller/stats",
            get(reactive::handle_reactive_poller_stats),
        )
        .route(
            "/wave/reactive/poller/config",
            post(reactive::handle_reactive_poller_config),
        )
        .route(
            "/wave/reactive/poller/status",
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
        .route("/wave/service", post(service::handle_service))
        .route("/wave/file", get(files::handle_wave_file))
        .route("/wave/stream-file", get(stub_501))
        .route("/wave/stream-file/*path", get(stub_501))
        .route("/wave/stream-local-file", get(stub_501))
        .route("/wave/aichat", post(ai::handle_ai_chat))
        .route("/api/post-chat-message", get(stub_501).post(stub_501))
        .route("/docsite/*path", get(files::handle_docsite))
        .route("/schema/*path", get(files::handle_schema))
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
