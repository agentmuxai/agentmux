pub(crate) mod cli_handlers;
mod files;
mod app_api;
mod forge_handlers;
mod identity_handlers;
mod messagebus;
mod reactive;
pub(crate) mod service;
mod tool_handlers;
mod websocket;
mod workflow_handlers;

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
    /// Saga durability — durable on-disk log of saga lifecycle.
    /// Written by `SagaCtx::dispatch` / `compensate` (per-step) and
    /// `emit_terminal` (per-saga) so a srv crash mid-saga leaves a
    /// recoverable trail. PR 1 ships the log + instrumentation; PR 2
    /// adds resume-on-startup + `--diag sagas`.
    /// See `docs/specs/SPEC_SAGA_DURABILITY_2026-05-01.md`.
    pub saga_log: std::sync::Arc<crate::sagas::log::SagaLog>,
    /// Pre-launch OAuth session state — one entry per in-flight
    /// "Connect with OAuth" attempt from the launch modal. See
    /// `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md`.
    pub auth_session_manager: std::sync::Arc<crate::identity::auth_session::AuthSessionManager>,
}

/// Build the Axum router with all routes, auth middleware, and CORS.
pub fn build_router(state: AppState) -> Router {
    // CORS: reflect only loopback origins.
    //
    // Before the 2026-05-11 security audit (C3) this allowed any origin
    // (matching the historical Go pkg/web/web.go). That made every web
    // page the user happened to have open a potential CSRF source —
    // localhost is not a trust boundary on a developer machine.
    //
    // The legitimate cross-origin callers are:
    //   - The CEF frontend served from `http://127.0.0.1:<host-port>`
    //   - Vite dev server at `http://localhost:5173` (and similar)
    //
    // Both are loopback. The predicate accepts http://127.0.0.1:* and
    // http://localhost:* (any port, http only; https is irrelevant for
    // loopback). External origins are denied, which means a malicious
    // page in the user's browser can't drive the sidecar even if it
    // discovers the port.
    use tower_http::cors::AllowOrigin;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _req| {
            let Ok(s) = origin.to_str() else { return false };
            s.starts_with("http://127.0.0.1:")
                || s.starts_with("http://localhost:")
                || s == "http://127.0.0.1"
                || s == "http://localhost"
        }))
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

    // Reactive routes. Previously registered without auth on the
    // assumption that localhost is a trust boundary; the 2026-05-11
    // security audit (C1 + C2) showed that any local process — or a
    // web page driving 127.0.0.1 via the permissive CORS layer — could
    // drive `/agentmux/reactive/inject` and reconfigure the cloud
    // agentbus poller. These routes are now merged into `authed_routes`
    // below and gated by `auth_middleware`.
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
        .route("/agentmux/diag/sagas", get(handle_diag_sagas))
        // Streaming-bash wrapper publish endpoint
        // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §4.3). agentmux-bashwrap
        // POSTs `{event, scopes, data}` here while a PreToolUse-rewritten
        // Bash command is running; we forward to the in-process WPS broker.
        // Auth-gated like the other reactive routes (PR #801 pattern).
        .route("/agentmux/wps/publish", post(handle_wps_publish))
        .merge(bus_routes)
        .merge(reactive_routes)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Health endpoint (no auth)
    let health = Router::new().route("/", get(health_handler));

    Router::new()
        .merge(health)
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

/// Saga durability PR 2 — operator visibility into the durable saga
/// log. Returns the most-recent 50 saga lifecycle rows + an in-flight
/// count derived from `unresolved_sagas`.
///
/// **Why a JSON HTTP endpoint and not a launcher `--diag sagas`
/// pipe-IPC client.** The `--diag srv` pipe transport (see
/// `agentmux-launcher/src/diag.rs`) routes through `Tool` registration
/// + a 2 s observation window with `GetSrvSnapshot` + `GetEvents`.
/// Adding `GetSagaLogSnapshot` to the IPC `Command` enum + an
/// `Event::SagaLogSnapshot` variant with a Vec of `SagaSnapshot`
/// triples the touched-files surface for one operator command.
/// JSON HTTP is the precedent for raw operator queries (cf
/// `/api/lan-instances`) and matches the spec §9 PR 2 phrasing
/// "tightened scope". Promoting to first-class `--diag sagas` via
/// pipe IPC is a follow-up if anyone asks.
///
/// Operator workflow today:
/// ```text
/// curl -s -H "X-AuthKey: $KEY" http://127.0.0.1:$PORT/agentmux/diag/sagas | jq .
/// ```
/// Response shape:
/// ```json
/// {
///   "recent": [ { "saga_id": ..., "name": ..., "state": ..., ... }, ... ],
///   "in_flight_count": 1,
///   "recently_failed_count": 0,
///   "total_returned": 50
/// }
/// ```
async fn handle_diag_sagas(State(state): State<AppState>) -> Json<serde_json::Value> {
    const LIMIT: u32 = 50;
    let recent = match state.saga_log.snapshot_recent(LIMIT) {
        Ok(rows) => rows,
        Err(e) => {
            return Json(json!({
                "error": format!("snapshot_recent failed: {}", e),
            }));
        }
    };
    let in_flight = match state.saga_log.unresolved_sagas() {
        Ok(rows) => rows.len(),
        Err(e) => {
            tracing::warn!("[diag/sagas] unresolved_sagas failed: {}", e);
            0
        }
    };
    let recently_failed = recent
        .iter()
        .filter(|s| s.state == "failed" || s.state == "failed_compensation")
        .count();
    Json(json!({
        "recent": recent,
        "in_flight_count": in_flight,
        "recently_failed_count": recently_failed,
        "total_returned": recent.len(),
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

/// Wire shape for `POST /agentmux/wps/publish`. Mirrors `WaveEvent`
/// but keeps the field set narrow for what `agentmux-bashwrap`
/// actually needs (no `sender`, no `persist`).
#[derive(serde::Deserialize)]
struct WpsPublishRequest {
    /// WPS event name. We use `tool_chunk:<tool_use_id>` for
    /// streaming chunks, but the handler is general-purpose.
    event: String,
    /// Optional scope filters (e.g. `["block:<id>"]`) so only
    /// subscribers watching that block receive the event.
    #[serde(default)]
    scopes: Vec<String>,
    /// Free-form payload. For tool_chunk events this is the
    /// `{op, kind, content, timestamp}` shape from
    /// `SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §4.3.
    data: serde_json::Value,
}

/// Auth-gated WPS publish endpoint
/// (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §3.2). `agentmux-bashwrap`
/// POSTs here while running a Bash command; we forward to the
/// in-process WPS broker so subscribed frontends receive the event.
async fn handle_wps_publish(
    State(state): State<AppState>,
    Json(req): Json<WpsPublishRequest>,
) -> impl IntoResponse {
    let event = crate::backend::wps::WaveEvent {
        event: req.event,
        scopes: req.scopes,
        sender: String::new(),
        persist: 0,
        data: Some(req.data),
    };
    state.broker.publish(event);
    (StatusCode::OK, Json(json!({"ok": true})))
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

    // 2026-05-11 audit (C3): the query-string `?authkey=` fallback
    // bypasses CORS preflight and is preserved in browser history,
    // navigation `Referer` headers, server access logs, etc. — a CSRF
    // amplifier whenever the key leaks. It is allowed **only** on the
    // WebSocket upgrade route (`/ws`), where the browser WS API doesn't
    // permit custom headers and there is no other practical channel
    // for the key. Every other route requires the header.
    let auth_key = auth_key.or_else(|| {
        if req.uri().path() != "/ws" {
            return None;
        }
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
