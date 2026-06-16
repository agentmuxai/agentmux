// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

pub(crate) mod cli_handlers;
mod files;
mod app_api;
mod agent_handlers;
mod identity_handlers;
pub mod install_handlers;
mod messagebus;
mod reactive;
pub(crate) mod service;
mod tool_handlers;
pub(crate) mod wave_obj_bridge;
mod websocket;
mod drone_handlers;
mod muxbus_handlers;

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
use crate::backend::lan_discovery::LanDiscoveryController;
use crate::backend::lsp::LspSupervisor;
use crate::backend::messagebus::MessageBus;
use crate::backend::reactive::{Poller, ReactiveHandler};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
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
    pub wstore: Arc<Store>,
    pub filestore: Arc<FileStore>,
    /// GLOBAL, channel-independent transcript store backing the
    /// `agent:<defId>:current` zone. `None` when the shared root can't be
    /// resolved (global transcripts disabled — falls back to per-channel
    /// `filestore`). Lets an agent's conversation load when opened from any
    /// build/channel, finishing the cross-channel arc started by #1387–#1396.
    /// See `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`.
    pub global_transcript_store: Option<Arc<FileStore>>,
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
    /// Live controller for mDNS-based LAN/host peer discovery. The controller
    /// owns a swappable daemon slot so the `network:lan_discovery` setting can
    /// be toggled at runtime without restarting the process.
    /// See `specs/lan-discovery-toggle.md`.
    pub lan_discovery: Arc<LanDiscoveryController>,
    /// Language Server Protocol supervisor — owns the lifecycle of LSP
    /// server child processes (one per workspace/language) and proxies
    /// LSP messages between the editor pane and the server.
    /// Spec: `specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md`.
    pub lsp_supervisor: Arc<LspSupervisor>,
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

    /// In-flight `install.start` sessions. Frontend subscribes to
    /// `install_chunk` WPS events scoped by session id; the registry
    /// holds per-session cancel handles so `install.cancel` can abort
    /// an install mid-flight.
    /// See `SPEC_AGENT_INSTALL_STAGE_2026_05_17.md` §9.
    pub install_sessions: std::sync::Arc<crate::server::install_handlers::InstallSessionRegistry>,
    /// Docker container manager for container-type agent panes (Phase 2).
    /// `None` when Docker is not available on this host — container agents
    /// will refuse to start rather than crashing the server.
    pub container_manager: Option<std::sync::Arc<crate::backend::container::ContainerManager>>,
    /// Phase 3 — per-shell stop handles so `ShellStop` (MCP tool) and the UI
    /// stop button can tree-kill a running persistent shell node. See
    /// `docs/specs/SPEC_PERSISTENT_SHELL_PHASE3_STOP_2026_06_14.md`.
    pub shell_sessions: std::sync::Arc<crate::backend::shell_node::ShellSessionRegistry>,
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
        .route("/agentmux/discovery", get(handle_discovery))
        .route("/agentmux/diag/sagas", get(handle_diag_sagas))
        // Streaming-bash wrapper publish endpoint
        // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §4.3). agentmux-bashwrap
        // POSTs `{event, scopes, data}` here while a PreToolUse-rewritten
        // Bash command is running; we forward to the in-process WPS broker.
        // Auth-gated like the other reactive routes (PR #801 pattern).
        .route("/agentmux/wps/publish", post(handle_wps_publish))
        // Persistent shell launch endpoint
        // (SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md §5.3). agentmux-mcp's
        // Shell tool POSTs here; we publish shell_node_create + spawn a
        // ShellNodeRunner that streams shell_chunk events to the frontend.
        .route("/api/v1/shell/create", post(handle_shell_create))
        // Stop a persistent shell (Phase 3). agentmux-mcp's `ShellStop` tool
        // POSTs here; tree-kills the shell's process group.
        .route("/api/v1/shell/stop", post(handle_shell_stop))
        // Open a pane (editor/term/browser/…) from an agent tool call.
        // agentmux-mcp's OpenEditor tool POSTs `{view:"editor", file, …}` here;
        // shares the exact pane.open logic with the WebSocket RPC handler
        // (app_api::open_pane). See ANALYSIS_AGENT_APP_API_OPEN_IN_EDITOR_2026_05_30.
        .route("/api/v1/pane/open", post(handle_pane_open))
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
    Json(json!(state.lan_discovery.get_instances()))
}

/// `GET /agentmux/discovery` — a unified, agent-facing view of what exists and
/// what is reachable across the muxbus delivery tiers, so an agent can resolve a
/// target before sending. Aggregates:
///   - `host.addressable`: the authoritative Tier-1/2 reachable set
///     (`reactive_handler.list_agents()` — every entry has a live block_id).
///   - `host.agents`: this host's agent directory (SQLite `instance_list`),
///     each flagged `addressable` iff its name is in the reachable set.
///   - `lan`: Tier-3 mDNS peers (each `LanInstance` carries its own `agents`).
///   - `wan.subscribed_agents`: Tier-4 cloud subscriptions (empty when no token).
/// Addressing is case-insensitive (registration lowercases the key). Authed like
/// the other reactive routes; agents reach it via AGENTMUX_LOCAL_URL + X-AuthKey.
/// See SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16.
async fn handle_discovery(State(state): State<AppState>) -> Json<serde_json::Value> {
    use std::collections::HashSet;

    // Tier-1/2 reachable set — the authoritative "addressable" answer.
    let reachable = state.reactive_handler.list_agents();
    let addressable: HashSet<String> =
        reachable.iter().map(|a| a.agent_id.to_lowercase()).collect();

    // Host directory (live SQLite instances), each flagged against the
    // reachable set. Hidden ("forgotten") rows are excluded.
    let instances = state.wstore.instance_list(None, None).unwrap_or_default();
    let agents: Vec<serde_json::Value> = instances
        .into_iter()
        .filter(|i| !i.display_hidden)
        .map(|i| {
            let is_addressable = !i.instance_name.is_empty()
                && addressable.contains(&i.instance_name.to_lowercase());
            json!({
                "name": i.instance_name,
                "id": i.id,
                "definition_id": i.definition_id,
                "block_id": i.block_id,
                "status": i.status,
                "working_directory": i.working_directory,
                "addressable": is_addressable,
            })
        })
        .collect();

    let wan_agents = crate::muxbus::cloud_subscriber::get_global_subscriber()
        .map(|s| s.subscribed_agents())
        .unwrap_or_default();

    let lan = state.lan_discovery.get_instances();
    let version = state.version.clone();
    let local_url = state.local_web_url.clone();

    Json(json!({
        "host": {
            "version": version,
            "local_url": local_url,
            "addressable": reachable,
            "agents": agents,
        },
        "lan": lan,
        "wan": { "subscribed_agents": wan_agents },
    }))
}

async fn stub_501() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "not implemented"})),
    )
}

/// Wire shape for `POST /agentmux/wps/publish`. Mirrors `WaveEvent`
/// but keeps the field set narrow for what `agentmux-bashwrap`
/// actually needs (no `sender`).
#[derive(serde::Deserialize)]
struct WpsPublishRequest {
    /// WPS event name. We use a fixed `tool_chunk` for every
    /// streaming chunk (the tool_use_id lives in the payload), but
    /// the handler is general-purpose.
    event: String,
    /// Optional scope filters (e.g. `["block:<id>"]`) so only
    /// subscribers watching that block receive the event.
    #[serde(default)]
    scopes: Vec<String>,
    /// Per-scope event ring size. Lets late subscribers replay
    /// events that landed before they subscribed. agentmux-bashwrap
    /// sets this to 1024 for `tool_chunk` so the frontend's
    /// subscription (installed on pane mount) picks up chunks that
    /// flew before Claude's stream-json caught up enough to surface
    /// the tool_use_id. Zero (or omitted) disables persistence —
    /// pure fan-out. See SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §6.
    #[serde(default)]
    persist: usize,
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
        persist: req.persist,
        data: Some(req.data),
    };
    state.broker.publish(event);
    (StatusCode::OK, Json(json!({"ok": true})))
}

// ---- Shell create ----

#[derive(serde::Deserialize)]
struct ShellCreateRequest {
    /// Block UUID of the agent pane that launched the shell.
    /// Events will be scoped to `block:<agent_block_id>` so only
    /// that pane's subscription receives them.
    agent_block_id: String,
    cmd: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    env: Option<std::collections::HashMap<String, String>>,
}

#[derive(serde::Serialize)]
struct ShellCreateResponse {
    shell_id: String,
}

/// `POST /api/v1/shell/create` — start a persistent background shell.
///
/// Called by `agentmux-mcp`'s `Shell` tool. Returns immediately with a
/// `shell_id`; the `ShellNodeRunner` streams stdout/stderr to the frontend
/// as `shell_chunk` WPS events without blocking the agent.
async fn handle_shell_create(
    State(state): State<AppState>,
    Json(req): Json<ShellCreateRequest>,
) -> impl IntoResponse {
    let shell_id = uuid::Uuid::new_v4().to_string();
    let title = req.title.as_deref().unwrap_or(&req.cmd).to_string();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Read the agent block once for both the cwd and env fallbacks below.
    let agent_block = state.wstore
        .get::<crate::backend::obj::Block>(&req.agent_block_id)
        .ok()
        .flatten();

    // If cwd wasn't supplied by the caller, fall back to the agent block's
    // cmd:cwd — the working directory the agent pane was launched with.
    // Without this, ShellNodeRunner would inherit agentmux-srv's cwd
    // (typically the portable runtime/ dir) instead of the project dir.
    let effective_cwd = req.cwd.or_else(|| {
        agent_block.as_ref().and_then(|block| {
            let cwd = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
            if cwd.is_empty() { None } else { Some(cwd.to_string()) }
        })
    });

    // Normalize the cwd before it reaches the spawner. Agents on Windows run
    // inside a bash shell and emit MSYS paths like `/c/Users/asafe/project`;
    // passing those straight to `Command::current_dir` fails with os error 267
    // (ERROR_DIRECTORY). This converts them to native form and expands `~`.
    let effective_cwd =
        effective_cwd.and_then(|c| crate::backend::base::normalize_working_dir(&c));

    // Env parity with the agent CLI: start from the agent block's stored
    // cmd:env (the per-agent env the agent process is launched with — same
    // shape app_api.rs / websocket.rs read), then let the caller-supplied
    // req.env override on top (explicit Shell env wins). This forwards the
    // concrete per-agent env so `Shell(...)` runs with the same env the agent
    // itself sees, mirroring the cmd:cwd fallback above.
    //
    // NOT forwarded here: the dynamic identity bindings (resolver.rs) and the
    // bundled tools/bin PATH prefix that blockcontroller/shell.rs injects at
    // agent-CLI spawn time — those are resolved live per spawn, not stored in
    // cmd:env. The MCP server (agentmux-mcp) is itself launched by the agent
    // CLI through the bundled tools/bin, so tools it spawns inherit that PATH;
    // shells created here run from agentmux-srv's env plus cmd:env + req.env.
    let mut effective_env: std::collections::HashMap<String, String> = agent_block
        .as_ref()
        .and_then(|block| match block.meta.get("cmd:env") {
            Some(serde_json::Value::Object(obj)) => Some(
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    if let Some(req_env) = req.env {
        effective_env.extend(req_env);
    }

    tracing::info!(
        block_id = %req.agent_block_id,
        shell_id = %shell_id,
        cmd = %req.cmd,
        cwd = ?effective_cwd,
        "shell.create"
    );

    // Publish shell_node_create so the frontend inserts the row
    // before the first chunk arrives (avoids a flash of orphaned chunks).
    // persist: 64 — retain up to 64 shell_node_create events per block scope
    // so multiple shells in a pane all replay on WS reconnect / pane remount.
    // (persist: 1 meant only the last shell's create event was kept; earlier
    // shells lost their create event while their shell_chunk events at
    // persist: 1024 still replayed, causing the reducer to silently drop
    // orphaned chunks.)
    state.broker.publish(crate::backend::wps::WaveEvent {
        event: crate::backend::wps::EVENT_SHELL_NODE_CREATE.to_string(),
        scopes: vec![format!("block:{}", req.agent_block_id)],
        sender: String::new(),
        persist: 64,
        data: Some(json!({
            "shell_id": shell_id,
            "cmd": req.cmd,
            "cwd": effective_cwd,
            "title": title,
            "timestamp": now_ms,
        })),
    });

    // Spawn the runner — fire-and-forget; events flow independently.
    let runner = crate::backend::shell_node::ShellNodeRunner {
        shell_id: shell_id.clone(),
        block_id: req.agent_block_id,
        cmd: req.cmd,
        cwd: effective_cwd,
        extra_env: effective_env,
        broker: Arc::clone(&state.broker),
        registry: Arc::clone(&state.shell_sessions),
    };
    tokio::spawn(runner.run());

    (StatusCode::OK, Json(ShellCreateResponse { shell_id }))
}

// ---- Shell stop ----

#[derive(serde::Deserialize)]
struct ShellStopRequest {
    shell_id: String,
}

/// `POST /api/v1/shell/stop` — stop a running persistent shell.
///
/// Called by `agentmux-mcp`'s `ShellStop` tool. Tree-kills the shell's
/// process group (so `task dev` → `task.exe`/`node` grandchildren die too),
/// which makes the runner publish a `stopped` exit event. Returns `{ stopped }`
/// — false if the id is unknown (never started or already exited).
async fn handle_shell_stop(
    State(state): State<AppState>,
    Json(req): Json<ShellStopRequest>,
) -> impl IntoResponse {
    let stopped = state.shell_sessions.stop(&req.shell_id);
    tracing::info!(shell_id = %req.shell_id, stopped, "shell.stop");
    (StatusCode::OK, Json(json!({ "stopped": stopped })))
}

/// `POST /api/v1/pane/open` — open a pane (editor/term/browser/…).
///
/// Called by `agentmux-mcp`'s `OpenEditor` tool. Thin HTTP wrapper over
/// `app_api::open_pane` (the same logic the WebSocket `pane.open` RPC uses):
/// creates the block, enqueues the layout action, and broadcasts the updates
/// so the frontend renders the pane. Body is `CommandPaneOpenData`.
async fn handle_pane_open(
    State(state): State<AppState>,
    Json(req): Json<crate::backend::rpc_types::CommandPaneOpenData>,
) -> impl IntoResponse {
    match app_api::open_pane(&state, req).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            // Argument/validation errors from build_pane_meta are the caller's
            // fault (400); everything else is a server-side failure (500).
            let status = if e.starts_with("MISSING_ARG") || e.starts_with("INVALID_VIEW") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(json!({ "error": e }))).into_response()
        }
    }
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
