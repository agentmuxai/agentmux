// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

pub(crate) mod cli_handlers;
mod files;
pub(crate) mod app_api;
mod agent_handlers;
mod editor_handlers;
mod identity_auth_dirs;
mod identity_auth_persist;
mod identity_auth_spawn;
mod identity_handlers;
pub mod install_handlers;
mod lsp_handlers;
mod messagebus;
mod reactive;
pub(crate) mod service;
mod shell_handlers;
mod tool_handlers;
mod providers_handlers;
mod voice;
pub(crate) mod wave_obj_bridge;
mod websocket;
mod drone_handlers;
mod cron;
mod messaging_handlers;
mod muxbus_handlers;
mod native_memory_handlers;

#[cfg(test)]
pub(crate) mod tests;

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Query, Request, State},
    http::{header, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, patch, post},
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
use agentmux_common::api_types::{
    PaneTitleRequest, ShellCreateRequest, ShellCreateResponse, ShellInputFailure,
    ShellInputRequest, ShellInputResponse, ShellStatusRequest, ShellStatusResponse, ShellStopRequest,
    TabActivateRequest, TabNameRequest, TabNewRequest, WindowFocusRequest, WindowNameRequest,
    WorkspaceNameRequest, WpsPublishRequest,
};

// ---- AppState ----

#[derive(Clone)]
pub struct AppState {
    pub auth_key: String,
    /// Random identifier generated once per process boot — NOT the
    /// `--instance` channel name (`config.instance_id`), which is
    /// stable across restarts and shared by every process on the same
    /// channel. Used as the owner id for cross-process session leases
    /// (`registry::LeaseStore`) so two live processes can be told
    /// apart even when they're the same channel/version.
    /// See `docs/retros/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`.
    pub boot_id: Arc<str>,
    pub version: String,
    pub app_path: String,
    pub wstore: Arc<Store>,
    /// GLOBAL shared store (`~/.agentmux/shared/store.db`). Holds durable
    /// user content that must survive version upgrades: identity accounts,
    /// memory bundles, drone definitions, and MuxBus credentials.
    /// `None` when the shared root can't be resolved (CI / unusual envs).
    /// See `docs/specs/SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md`.
    pub shared_store: Option<Arc<Store>>,
    /// Effective identity/memory/drone/muxbus store — `shared_store` when
    /// available, otherwise `wstore`. Handlers capture this instead of
    /// `wstore` for any operation that must survive across version upgrades.
    pub id_store: Arc<Store>,
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
    /// Process Broker (Phase A) — unified `ProcessStatus` per block, read
    /// through instead of composing `blockcontroller`/`process_tracker`
    /// directly at each call site. See `crate::broker::process` and
    /// `docs/specs/REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`.
    pub process_broker: Arc<crate::broker::ProcessBroker>,
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
    /// Docker container runtime handle for container-type agent panes
    /// (Phase 2). Self-healing — `.get()`/`.is_available()` retry the
    /// Docker connection on demand rather than being fixed at process
    /// boot, so a daemon that starts after AgentMux launched is picked up
    /// without an app restart. See `ContainerRuntimeHandle` and
    /// docs/retros/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    pub container_manager: std::sync::Arc<crate::backend::container::ContainerRuntimeHandle>,
    /// Phase 3 — per-shell stop handles so `ShellStop` (MCP tool) and the UI
    /// stop button can tree-kill a running persistent shell node. See
    /// `docs/specs/SPEC_PERSISTENT_SHELL_PHASE3_STOP_2026_06_14.md`.
    pub shell_sessions: std::sync::Arc<crate::backend::shell_node::ShellSessionRegistry>,
    /// Persistent cron scheduler — loaded from `db_cron_jobs` at startup.
    /// Creates/cancels tokio tasks that fire on the specified UTC schedule and
    /// POST to `/agentmux/reactive/inject`. See
    /// `docs/specs/SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md §3.2`.
    pub cron_scheduler: std::sync::Arc<crate::backend::cron::CronScheduler>,
    /// Filesystem watcher for files open in editor/preview panes — publishes
    /// `EVENT_EDITOR_FILE_CHANGED` (scoped per-block) when a watched path
    /// changes on disk, so panes can refresh instead of silently going
    /// stale. `None` when the underlying `notify` watcher couldn't be
    /// created (live-reload is a nice-to-have, not a boot requirement).
    /// See docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md.
    pub editor_file_watcher: Option<std::sync::Arc<crate::backend::editor_file_watcher::EditorFileWatcher>>,
    /// Filesystem watcher for directories a Media pane is pointed at —
    /// publishes `EVENT_MEDIA_FILE_CHANGED` (scoped per-block) when a
    /// matching-extension file is created/modified. `None` when the
    /// underlying `notify` watcher couldn't be created.
    /// See docs/specs/SPEC_MEDIA_PANE_2026_07_26.md.
    pub media_file_watcher: Option<std::sync::Arc<crate::backend::media_file_watcher::MediaFileWatcher>>,
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
        ])
        // Custom response headers are invisible to cross-origin `fetch()`
        // callers (Response.headers.get(...) silently returns null) unless
        // explicitly exposed here — a browser CORS default, not an
        // allow_headers concern (that list governs REQUEST headers only).
        // `X-ZoneFileInfo` (files.rs's handle_wave_file) is read by
        // `fetchWaveFile` on every blockfile GET; without this, any 200
        // response is indistinguishable from a malformed one to the
        // frontend ("missing zone file info for ..." — the exact failure
        // mode `TermWrap.loadInitialTerminalData()` hit once terminal
        // scrollback write-through started actually producing real 200s
        // instead of always-404, see
        // SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1's
        // follow-up). Not dev-only — the CEF frontend's own origin is
        // cross-origin from this server in production too (see the
        // allow_origin comment above).
        .expose_headers(vec!["X-ZoneFileInfo".parse().unwrap()]);

    // Reactive routes. Previously registered without auth on the
    // assumption that localhost is a trust boundary; the 2026-05-11
    // security audit (C1 + C2) showed that any local process — or a
    // web page driving 127.0.0.1 via the permissive CORS layer — could
    // drive `/agentmux/reactive/inject` and reconfigure the cloud
    // muxbus poller. These routes are now merged into `authed_routes`
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
        .route("/agentmux/stream-local-file", get(files::handle_stream_local_file))
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
        // Phase 3b — write to a running shell's stdin (for interactive prompts).
        .route("/api/v1/shell/input", post(handle_shell_input))
        // Phase 3b — query running state, exit code, and line count.
        .route("/api/v1/shell/status", post(handle_shell_status))
        // Open a pane (editor/term/browser/…) from an agent tool call.
        // agentmux-mcp's OpenEditor tool POSTs `{view:"editor", file, …}` here;
        // shares the exact pane.open logic with the WebSocket RPC handler
        // (app_api::open_pane). See ANALYSIS_AGENT_APP_API_OPEN_IN_EDITOR_2026_05_30.
        .route("/api/v1/pane/open", post(handle_pane_open))
        // Voice speech-to-text: the renderer POSTs mic audio (one
        // silence-bounded utterance per request); we forward to a Whisper
        // backend and return the transcript. Key stays server-side.
        // See SPEC_VOICE_STT_ENGINE_2026_06_20.md and #1591.
        .route("/api/v1/voice/transcribe", post(voice::handle_voice_transcribe))
        // First-class agent API (SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md).
        // `GET /api/v1/self?block_id=` resolves the caller's place in the tree;
        // `POST /api/v1/window/name` sets the window display name (taskbar title).
        // agentmux-mcp's `WhoAmI` / `SetWindowName` tools call these.
        .route("/api/v1/self", get(handle_self))
        .route("/api/v1/window/name", post(handle_window_name))
        // Naming verbs (SPEC §4.3): rename the caller's own tab / pane / workspace
        // (or an explicit target). agentmux-mcp's SetTabName / SetPaneTitle /
        // SetWorkspaceName tools POST here.
        .route("/api/v1/tab/name", post(handle_tab_name))
        .route("/api/v1/pane/title", post(handle_pane_title))
        .route("/api/v1/workspace/name", post(handle_workspace_name))
        // Introspection verbs (SPEC §4.6): read-only views of the UI tree so an
        // agent can see what's around it. agentmux-mcp's GetLayout / ListWindows
        // / ListWorkspaces / ListTabs tools GET these.
        .route("/api/v1/layout", get(handle_layout))
        .route("/api/v1/windows", get(handle_list_windows))
        .route("/api/v1/workspaces", get(handle_list_workspaces))
        .route("/api/v1/tabs", get(handle_list_tabs))
        // Layout / navigation verbs (SPEC §4.5): switch the active tab, open a
        // new tab, focus a window. agentmux-mcp's SetActiveTab / NewTab /
        // FocusWindow tools POST here.
        .route("/api/v1/tab/activate", post(handle_tab_activate))
        .route("/api/v1/tab/new", post(handle_tab_new))
        .route("/api/v1/window/focus", post(handle_window_focus))
        // Agent App API — identity / preset / memory namespaces, the MCP-facing
        // slice of the app-API RPC surface (SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28).
        // The agent identity (`agent_id`) is supplied by agentmux-mcp from its
        // trusted AGENTMUX_AGENT_ID env; an agent's PTY has no auth key to reach
        // these directly, so the slug cannot be forged. Each handler calls the
        // same `app_api::*_impl` the WebSocket RPC handlers use.
        .route("/api/v1/agent/memory/list", get(handle_agent_memory_list))
        .route("/api/v1/agent/memory/read", get(handle_agent_memory_read))
        .route("/api/v1/agent/memory/write", post(handle_agent_memory_write))
        .route("/api/v1/agent/preset/list", get(handle_agent_preset_list))
        .route("/api/v1/agent/preset/get", get(handle_agent_preset_get))
        .route("/api/v1/agent/identity/accounts", get(handle_agent_identity_accounts))
        .route("/api/v1/agent/identity/validate", post(handle_agent_identity_validate))
        .route("/api/messaging/status", get(messaging_handlers::handle_status))
        .route("/api/messaging/discord/send", post(messaging_handlers::handle_discord_send))
        .route("/api/messaging/telegram/send", post(messaging_handlers::handle_telegram_send))
        .route("/api/messaging/slack/send", post(messaging_handlers::handle_slack_send))
        .route("/api/messaging/whatsapp/send", post(messaging_handlers::handle_whatsapp_send))
        // Persistent cron scheduler (SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md §3.2.4).
        // Auth-gated like reactive routes.
        .route("/agentmux/cron", post(cron::handle_cron_create))
        .route("/agentmux/cron", get(cron::handle_cron_list))
        .route("/agentmux/cron/:id", delete(cron::handle_cron_delete))
        .route("/agentmux/cron/:id", patch(cron::handle_cron_patch))
        .merge(bus_routes)
        .merge(reactive_routes)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Health endpoint (no auth)
    let health = Router::new().route("/", get(health_handler));

    // WhatsApp Cloud API webhook receiver (no auth). Meta's servers call
    // these directly and cannot supply the X-AuthKey header auth_middleware
    // requires, so — like `health` — these must be merged at the top level,
    // outside `authed_routes`'s `route_layer(auth_middleware)`, not added to
    // that router. The GET handshake and POST delivery are authenticated by
    // a different mechanism suited to a third party AgentMux doesn't
    // control the request format of: hub.verify_token comparison on GET,
    // and HMAC-SHA256(app_secret, raw_body) signature validation on every
    // POST (see messaging/whatsapp/webhook.rs). See
    // docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md §8.2.
    let whatsapp_webhooks = Router::new().route(
        "/webhook/whatsapp",
        get(crate::messaging::whatsapp::handle_verify)
            .post(crate::messaging::whatsapp::handle_inbound),
    );

    Router::new()
        .merge(health)
        .merge(whatsapp_webhooks)
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
    // Tier-1/2 reachable set — the authoritative "addressable" answer. Keep the
    // live block_id per name (lowercased) so an addressable row can surface its
    // real delivery block; the SQLite directory below does not carry one.
    let reachable = state.reactive_handler.list_agents();
    let reachable_block: std::collections::HashMap<String, String> = reachable
        .iter()
        .map(|a| (a.agent_id.to_lowercase(), a.block_id.clone()))
        .collect();

    // Host directory (live SQLite instances). `instance_list` already excludes
    // hidden (user_hidden = 0) and template rows in SQL, and the consolidated
    // path leaves block_id/status empty (agents.rs) — so addressability AND the
    // live block_id come from the reachable set above. `block_id` is null for a
    // known-but-unreachable agent; the always-empty `status` is omitted.
    let instances = state.wstore.instance_list(None, None).unwrap_or_default();
    let agents: Vec<serde_json::Value> = instances
        .into_iter()
        .map(|i| {
            let live_block = if i.instance_name.is_empty() {
                None
            } else {
                reachable_block.get(&i.instance_name.to_lowercase()).cloned()
            };
            json!({
                "name": i.instance_name,
                "id": i.id,
                "definition_id": i.definition_id,
                "working_directory": i.working_directory,
                "addressable": live_block.is_some(),
                "block_id": live_block,
            })
        })
        .collect();

    let wan_agents = crate::muxbus::cloud_subscriber::get_global_subscriber()
        .map(|s| s.subscribed_agents())
        .unwrap_or_default();

    let lan = state.lan_discovery.get_instances();
    let version = state.version.clone();
    let local_url = state.local_web_url.clone();

    // Tier 2b — other channels' agents on this same host (issue #1916), via
    // the host-global shared registry. Excludes this instance's own channel
    // (already covered by `agents`/`addressable` above) and this instance's
    // own URL (a stale self-entry from a prior crash, if any).
    let own_channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
    let cross_channel: Vec<serde_json::Value> = crate::registry::resolve_shared_reactive_dir()
        .map(|shared_dir| {
            crate::backend::reactive::registry::list_all_shared(&shared_dir)
                .into_iter()
                .filter(|e| e.channel != own_channel && e.local_url != local_url)
                .map(|e| {
                    json!({
                        "name": e.agent_id,
                        "channel": e.channel,
                        "local_url": e.local_url,
                        "block_id": e.block_id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Json(json!({
        "host": {
            "version": version,
            "local_url": local_url,
            "addressable": reachable,
            "agents": agents,
            "cross_channel": cross_channel,
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
        title,
        cwd: effective_cwd,
        extra_env: effective_env,
        broker: Arc::clone(&state.broker),
        registry: Arc::clone(&state.shell_sessions),
        capture_stdin: req.capture_stdin.unwrap_or(false),
    };
    tokio::spawn(runner.run());

    (StatusCode::OK, Json(ShellCreateResponse { shell_id }))
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

/// `POST /api/v1/shell/input` — write text to a running shell's stdin (Phase 3b).
///
/// Appends a newline so single answers like "y" work without the caller
/// needing to know the line discipline. Returns `{ written: false }` if the
/// shell is not running or the write fails (e.g. the process closed its stdin).
async fn handle_shell_input(
    State(state): State<AppState>,
    Json(req): Json<ShellInputRequest>,
) -> impl IntoResponse {
    // Non-blocking send to the stdin relay task — no mutex, no risk of
    // blocking if the child's pipe buffer is full (the relay owns that concern).
    // resolve_stdin distinguishes "not running" from "running but no captured
    // stdin" so the caller gets an actionable reason instead of a bare false.
    let (written, reason) = match state.shell_sessions.resolve_stdin(&req.shell_id) {
        Ok(tx) => {
            let mut text = req.text.clone();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            if tx.send(text).is_ok() {
                tracing::debug!(shell_id = %req.shell_id, "shell.input: written");
                (true, None)
            } else {
                // Relay task gone / channel closed — process closed its stdin.
                tracing::debug!(shell_id = %req.shell_id, "shell.input: write failed");
                (false, Some(ShellInputFailure::WriteFailed))
            }
        }
        Err(failure) => {
            tracing::debug!(shell_id = %req.shell_id, ?failure, "shell.input: no target");
            (false, Some(failure))
        }
    };
    (StatusCode::OK, Json(ShellInputResponse { written, reason }))
}

/// `POST /api/v1/shell/status` — query a shell's running state (Phase 3b).
///
/// Returns `{ running, exit_code, line_count }`. If the shell_id is unknown
/// (never started or never seen by this sidecar), `running` is false and
/// `exit_code` is absent.
async fn handle_shell_status(
    State(state): State<AppState>,
    Json(req): Json<ShellStatusRequest>,
) -> impl IntoResponse {
    let s = state.shell_sessions.get_status(&req.shell_id);
    (StatusCode::OK, Json(ShellStatusResponse {
        running: s.running,
        exit_code: s.exit_code,
        line_count: s.line_count,
    }))
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

// ---------------------------------------------------------------------------
// Agent App API REST handlers (identity / preset / memory).
//
// `agent_id` is the agent slug, supplied by agentmux-mcp from its trusted
// AGENTMUX_AGENT_ID env. Each handler maps a 4xx for caller/validation errors
// (FORBIDDEN, "not found", "provide …", "not a regular file") and 5xx otherwise,
// then delegates to the shared `app_api::*_impl`.
// ---------------------------------------------------------------------------

/// Classify an app-API impl error string into an HTTP status. FORBIDDEN and
/// argument/not-found errors are the caller's fault (4xx); the rest are 5xx.
fn app_api_error_status(e: &str) -> StatusCode {
    if e.starts_with("FORBIDDEN") {
        StatusCode::FORBIDDEN
    } else if e.contains("not found")
        || e.contains("provide ")
        || e.contains("not a regular file")
        || e.contains("too large")
        || e.contains("invalid")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

fn app_api_response(result: Result<serde_json::Value, String>) -> axum::response::Response {
    match result {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (app_api_error_status(&e), Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct AgentMemoryListQuery {
    agent_id: String,
}

/// `GET /api/v1/agent/memory/list?agent_id=<slug>` — list the agent's own
/// native-memory markdown files. Backs the `MemoryList` MCP tool.
async fn handle_agent_memory_list(
    State(state): State<AppState>,
    Query(q): Query<AgentMemoryListQuery>,
) -> impl IntoResponse {
    app_api_response(app_api::memory_list_impl(&state, &q.agent_id))
}

#[derive(serde::Deserialize)]
struct AgentMemoryReadQuery {
    agent_id: String,
    filename: String,
}

/// `GET /api/v1/agent/memory/read?agent_id=<slug>&filename=<f>` — read one of
/// the agent's own memory files. Backs the `MemoryRead` MCP tool.
async fn handle_agent_memory_read(
    State(state): State<AppState>,
    Query(q): Query<AgentMemoryReadQuery>,
) -> impl IntoResponse {
    app_api_response(app_api::memory_read_impl(&state, &q.agent_id, &q.filename))
}

#[derive(serde::Deserialize)]
struct AgentMemoryWriteRequest {
    agent_id: String,
    filename: String,
    content: String,
}

/// `POST /api/v1/agent/memory/write` — create/overwrite one of the agent's own
/// memory files (atomic tmp→rename). Backs the `MemoryWrite` MCP tool.
async fn handle_agent_memory_write(
    State(state): State<AppState>,
    Json(req): Json<AgentMemoryWriteRequest>,
) -> impl IntoResponse {
    match app_api::memory_write_impl(&state, &req.agent_id, &req.filename, &req.content) {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => (app_api_error_status(&e), Json(json!({ "error": e }))).into_response(),
    }
}

/// `GET /api/v1/agent/preset/list` — list all presets (shared catalog, summary
/// fields only). Backs the `PresetList` MCP tool.
async fn handle_agent_preset_list(State(state): State<AppState>) -> impl IntoResponse {
    app_api_response(app_api::bundle_list_impl(&state).await)
}

#[derive(serde::Deserialize)]
struct AgentPresetGetQuery {
    #[serde(default)]
    agent_id: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

/// `GET /api/v1/agent/preset/get?agent_id=<slug>&id=&name=` — fetch a preset by
/// id or name; with neither, return the agent's own bound preset (self).
/// Backs the `PresetGet` MCP tool.
async fn handle_agent_preset_get(
    State(state): State<AppState>,
    Query(q): Query<AgentPresetGetQuery>,
) -> impl IntoResponse {
    if q.id.is_empty() && q.name.is_empty() {
        if q.agent_id.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "preset.get: provide id, name, or agent_id (for self)" })),
            )
                .into_response();
        }
        return app_api_response(app_api::bundle_self_get_impl(&state, &q.agent_id).await);
    }
    app_api_response(app_api::bundle_get_impl(&state, &q.id, &q.name).await)
}

#[derive(serde::Deserialize)]
struct AgentIdentityAccountsQuery {
    agent_id: String,
}

/// `GET /api/v1/agent/identity/accounts?agent_id=<slug>` — list the agent's own
/// linked credential accounts (masked tails only, never secrets). Backs the
/// `IdentityAccounts` MCP tool.
async fn handle_agent_identity_accounts(
    State(state): State<AppState>,
    Query(q): Query<AgentIdentityAccountsQuery>,
) -> impl IntoResponse {
    app_api_response(app_api::identity_self_accounts_impl(&state, &q.agent_id).await)
}

#[derive(serde::Deserialize)]
struct AgentIdentityValidateRequest {
    agent_id: String,
    account_id: String,
}

/// `POST /api/v1/agent/identity/validate` — live-probe one of the agent's own
/// linked accounts using its stored keychain secret (the agent never supplies a
/// secret). Backs the `IdentityValidate` MCP tool.
async fn handle_agent_identity_validate(
    State(state): State<AppState>,
    Json(req): Json<AgentIdentityValidateRequest>,
) -> impl IntoResponse {
    app_api_response(
        app_api::identity_account_validate_stored_impl(&state, &req.agent_id, &req.account_id).await,
    )
}

#[derive(serde::Deserialize)]
struct SelfQuery {
    /// Block UUID of the calling agent pane (its `AGENTMUX_BLOCKID`).
    block_id: Option<String>,
}

/// `GET /api/v1/self?block_id=<id>` — resolve the calling agent's place in the
/// object tree (block → tab → window → workspace, with their names). The
/// sidecar serves many agents, so the caller identifies itself by its block id
/// (the MCP `WhoAmI` tool passes `AGENTMUX_BLOCKID`). Naming verbs reuse the
/// same resolver to default their target to the agent's own context.
async fn handle_self(
    State(state): State<AppState>,
    Query(q): Query<SelfQuery>,
) -> impl IntoResponse {
    let block_id = q.block_id.unwrap_or_default();
    if block_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing block_id" }))).into_response();
    }
    match service::resolve_agent_context(&state.wstore, &block_id) {
        Ok(ctx) => Json(serde_json::to_value(&ctx).unwrap_or_default()).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
    }
}

/// `POST /api/v1/window/name` — set a window's display name
/// (`window:displayname`), which the frontend turns into the OS/taskbar title.
/// Defaults to the caller's own window (resolved from `block_id`). Routes
/// through the same `object.UpdateObjectMeta` service path the InstancePanel
/// rename uses, so persistence + live-title update are identical.
/// agentmux-mcp's `SetWindowName` tool POSTs here.
async fn handle_window_name(
    State(state): State<AppState>,
    Json(req): Json<WindowNameRequest>,
) -> impl IntoResponse {
    // window:displayname is documented as ≤64 chars (window-title.ts).
    let name: String = req.name.trim().chars().take(64).collect();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name must not be empty" }))).into_response();
    }

    let window_id = match req.window_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => {
            let block_id = req.block_id.unwrap_or_default();
            if block_id.is_empty() {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": "provide window_id or block_id" }))).into_response();
            }
            match service::resolve_agent_context(&state.wstore, &block_id) {
                Ok(ctx) => match ctx.window_id {
                    Some(w) => w,
                    None => {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(json!({ "error": "no live window for this agent (tab not attached to a window)" })),
                        )
                            .into_response()
                    }
                },
                Err(e) => return (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
            }
        }
    };

    let call = crate::backend::service::WebCallType {
        service: "object".to_string(),
        method: "UpdateObjectMeta".to_string(),
        uicontext: None,
        args: vec![
            json!(format!("window:{window_id}")),
            json!({ "window:displayname": name }),
        ],
    };
    let result = service::run_service_call(&state, &call).await;
    if let Some(err) = result.error {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": err }))).into_response();
    }
    Json(json!({ "success": true, "window_id": window_id, "name": name })).into_response()
}

/// Trim a user-supplied name and clamp to `max` chars; `None` if empty.
fn clean_name(raw: &str, max: usize) -> Option<String> {
    let n: String = raw.trim().chars().take(max).collect();
    if n.is_empty() {
        None
    } else {
        Some(n)
    }
}

/// `POST /api/v1/tab/name` — rename a tab. Defaults to the caller's own tab
/// (resolved from `block_id`). Routes through `object.UpdateTabName`.
async fn handle_tab_name(
    State(state): State<AppState>,
    Json(req): Json<TabNameRequest>,
) -> impl IntoResponse {
    let name = match clean_name(&req.name, 128) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name must not be empty" }))).into_response(),
    };
    let tab_id = match req.tab_id.filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => match resolve_own(&state, req.block_id, |c| Some(c.tab_id.clone())) {
            Ok(t) => t,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "object".to_string(),
        method: "UpdateTabName".to_string(),
        uicontext: None,
        args: vec![json!(tab_id), json!(name)],
    };
    finish_name_call(&state, call, json!({ "success": true, "tab_id": tab_id, "name": name })).await
}

/// `POST /api/v1/pane/title` — set a pane's display title (`frame:title`).
/// Targets the caller's own pane (its `block_id`). Routes through
/// `object.UpdateObjectMeta`.
async fn handle_pane_title(
    State(state): State<AppState>,
    Json(req): Json<PaneTitleRequest>,
) -> impl IntoResponse {
    let title = match clean_name(&req.title, 128) {
        Some(t) => t,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "title must not be empty" }))).into_response(),
    };
    let block_id = match req.block_id.filter(|b| !b.is_empty()) {
        Some(b) => b,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing block_id" }))).into_response(),
    };
    let call = crate::backend::service::WebCallType {
        service: "object".to_string(),
        method: "UpdateObjectMeta".to_string(),
        uicontext: None,
        args: vec![
            json!(format!("block:{block_id}")),
            json!({ "frame:title": title }),
        ],
    };
    finish_name_call(&state, call, json!({ "success": true, "block_id": block_id, "title": title })).await
}

/// `POST /api/v1/workspace/name` — rename a workspace. Defaults to the
/// caller's own workspace (resolved from `block_id`). Routes through
/// `workspace.UpdateWorkspace`.
async fn handle_workspace_name(
    State(state): State<AppState>,
    Json(req): Json<WorkspaceNameRequest>,
) -> impl IntoResponse {
    let name = match clean_name(&req.name, 128) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name must not be empty" }))).into_response(),
    };
    let workspace_id = match req.workspace_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => match resolve_own(&state, req.block_id, |c| c.workspace_id.clone()) {
            Ok(w) => w,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "workspace".to_string(),
        method: "UpdateWorkspace".to_string(),
        uicontext: None,
        args: vec![json!(workspace_id), json!(name)],
    };
    finish_name_call(&state, call, json!({ "success": true, "workspace_id": workspace_id, "name": name })).await
}

/// Resolve a target id from the caller's own context (via `block_id`), using
/// `pick` to select the field. Returns the route's error response on failure
/// (missing block_id, unresolvable block, or the field is `None`).
fn resolve_own(
    state: &AppState,
    block_id: Option<String>,
    pick: impl Fn(&service::AgentContext) -> Option<String>,
) -> Result<String, Response> {
    let block_id = block_id.unwrap_or_default();
    if block_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "provide an explicit target id or block_id" }))).into_response());
    }
    let ctx = service::resolve_agent_context(&state.wstore, &block_id)
        .map_err(|e| (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response())?;
    pick(&ctx).filter(|s| !s.is_empty()).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such target resolved for this agent" })),
        )
            .into_response()
    })
}

/// Run a naming service call and map the result to a JSON HTTP response.
async fn finish_name_call(
    state: &AppState,
    call: crate::backend::service::WebCallType,
    ok_body: serde_json::Value,
) -> Response {
    let result = service::run_service_call(state, &call).await;
    if let Some(err) = result.error {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": err }))).into_response();
    }
    Json(ok_body).into_response()
}

/// `GET /api/v1/layout` — read-only window → workspace → tab → pane tree.
async fn handle_layout(State(state): State<AppState>) -> impl IntoResponse {
    Json(service::agent_layout(&state.wstore))
}

/// `GET /api/v1/windows` — flat list of windows.
async fn handle_list_windows(State(state): State<AppState>) -> impl IntoResponse {
    Json(service::agent_windows(&state.wstore))
}

/// `GET /api/v1/workspaces` — flat list of workspaces.
async fn handle_list_workspaces(State(state): State<AppState>) -> impl IntoResponse {
    Json(service::agent_workspaces(&state.wstore))
}

#[derive(serde::Deserialize)]
struct ListTabsQuery {
    /// Limit to this workspace's tabs. Omit (or pass `block_id`) for all tabs.
    #[serde(default)]
    workspace_id: Option<String>,
    /// Calling agent's block id — scopes to the caller's own workspace when
    /// `workspace_id` is omitted.
    #[serde(default)]
    block_id: Option<String>,
}

/// `GET /api/v1/tabs` — flat list of tabs, optionally scoped to a workspace
/// (explicit `workspace_id`, or the caller's own via `block_id`).
async fn handle_list_tabs(
    State(state): State<AppState>,
    Query(q): Query<ListTabsQuery>,
) -> impl IntoResponse {
    let ws_id = q.workspace_id.filter(|w| !w.is_empty()).or_else(|| {
        q.block_id
            .filter(|b| !b.is_empty())
            .and_then(|b| service::resolve_agent_context(&state.wstore, &b).ok())
            .and_then(|ctx| ctx.workspace_id)
    });
    Json(service::agent_tabs(&state.wstore, ws_id.as_deref()))
}

/// `POST /api/v1/tab/activate` — make `tab_id` the active tab in its
/// workspace. Routes through `workspace.SetActiveTab`.
async fn handle_tab_activate(
    State(state): State<AppState>,
    Json(req): Json<TabActivateRequest>,
) -> impl IntoResponse {
    let tab_id = req.tab_id.trim().to_string();
    if tab_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing tab_id" }))).into_response();
    }
    let ws_id = match service::workspace_id_for_tab(&state.wstore, &tab_id) {
        Some(w) => w,
        None => return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("no workspace owns tab {tab_id}") }))).into_response(),
    };
    let call = crate::backend::service::WebCallType {
        service: "workspace".to_string(),
        method: "SetActiveTab".to_string(),
        uicontext: None,
        args: vec![json!(ws_id), json!(tab_id)],
    };
    finish_name_call(&state, call, json!({ "success": true, "tab_id": tab_id })).await
}

/// `POST /api/v1/tab/new` — create (and activate) a new tab in the caller's
/// workspace (or an explicit `workspace_id`). Routes through
/// `workspace.CreateTab`.
async fn handle_tab_new(
    State(state): State<AppState>,
    Json(req): Json<TabNewRequest>,
) -> impl IntoResponse {
    let name = req.name.map(|n| n.trim().chars().take(128).collect::<String>()).unwrap_or_default();
    let workspace_id = match req.workspace_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => match resolve_own(&state, req.block_id, |c| c.workspace_id.clone()) {
            Ok(w) => w,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "workspace".to_string(),
        method: "CreateTab".to_string(),
        uicontext: None,
        // [ws_id, name, activate]; empty name → backend auto-names tab{N}.
        args: vec![json!(workspace_id), json!(name), json!(true)],
    };
    finish_name_call(&state, call, json!({ "success": true, "workspace_id": workspace_id })).await
}

/// `POST /api/v1/window/focus` — bring a window to the foreground. Defaults to
/// the caller's own window. Routes through `client.FocusWindow`.
async fn handle_window_focus(
    State(state): State<AppState>,
    Json(req): Json<WindowFocusRequest>,
) -> impl IntoResponse {
    let window_id = match req.window_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => match resolve_own(&state, req.block_id, |c| c.window_id.clone()) {
            Ok(w) => w,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "client".to_string(),
        method: "FocusWindow".to_string(),
        uicontext: None,
        args: vec![json!(window_id)],
    };
    finish_name_call(&state, call, json!({ "success": true, "window_id": window_id })).await
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
