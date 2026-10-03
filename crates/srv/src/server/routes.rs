// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The HTTP route table and frontend fallback.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// The two routers srv serves. `full` is every route and is bound on
/// loopback only. `lan` is what `backend::lan_listeners` binds on LAN
/// interfaces when LAN discovery is on: the routes a LAN peer calls (all
/// behind `lan_or_full_auth_middleware`), plus health and the WhatsApp
/// webhook, which authenticate themselves. Nothing that requires the full
/// `auth_key` is served off-host, so a LAN listener has no full-key surface.
pub struct SrvRouters {
    pub full: Router,
    pub lan: Router,
}

/// Build the full router (see [`build_routers`]).
pub fn build_router(state: AppState) -> Router {
    build_routers(state).full
}

/// Build both routers with all routes, auth middleware, and CORS.
pub fn build_routers(state: AppState) -> SrvRouters {
    build_routers_with(state, crate::headless::frontend_dir())
}

/// [`build_routers`], with the frontend directory passed in (headless
/// `--frontend-dir`, or `None`).
pub(crate) fn build_routers_with(state: AppState, frontend_dir: Option<&std::path::Path>) -> SrvRouters {
    // CORS: reflect only loopback origins, plus any headless `--allowed-origin`.
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
            origin.to_str().is_ok_and(is_allowed_origin)
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
            // A video poster reads a file's first bytes (stream-local-file,
            // SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §5). The request also
            // carries X-AuthKey, so it's preflighted and Range must be listed.
            header::RANGE,
        ])
        // Custom response headers are invisible to cross-origin `fetch()`
        // callers (Response.headers.get(...) silently returns null) unless
        // explicitly exposed here — a browser CORS default, not an
        // allow_headers concern (that list governs REQUEST headers only).
        // `X-ZoneFileInfo` (files.rs's handle_mux_file) is read by
        // `fetchMuxFile` on every blockfile GET; without this, any 200
        // response is indistinguishable from a malformed one to the
        // frontend ("missing zone file info for ..." — the exact failure
        // mode `TermWrap.loadInitialTerminalData()` hit once terminal
        // scrollback write-through started actually producing real 200s
        // instead of always-404, see
        // SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1's
        // follow-up). Not dev-only — the CEF frontend's own origin is
        // cross-origin from this server in production too (see the
        // allow_origin comment above).
        // `Content-Range` carries a ranged stream-local-file response's
        // total size, which inline video/audio read before play (Codex P2 on
        // #4073).
        .expose_headers(vec!["X-ZoneFileInfo".parse().unwrap(), header::CONTENT_RANGE]);

    // The routes an LAN peer actually calls when forwarding a jekt or
    // looking up which agents this instance hosts
    // (`LanDiscoveryController::find_agent`, `server/reactive.rs`'s Tier-3
    // forward). Kept OUT of `reactive_routes`/`authed_routes` deliberately
    // — merged at the top level with their own `lan_or_full_auth_middleware`
    // instead, so a LAN peer's scoped `lan_key` (see `Config::lan_key`) is
    // accepted here without also being accepted by `authed_routes`'s much
    // broader surface (the general `auth_middleware` there requires the
    // full `auth_key` only). See `lan_or_full_auth_middleware`'s doc
    // comment for why this can't just be nested inside `authed_routes`.
    let lan_forward_routes = Router::new()
        .route("/agentmux/reactive/inject", post(reactive::handle_reactive_inject))
        .route("/agentmux/reactive/agent", get(reactive::handle_reactive_agent))
        // Names only — see `handle_reactive_agent_names`' doc comment for why
        // this is deliberately not `/agentmux/reactive/agents` (which stays
        // behind full auth because it serializes internal routing fields).
        .route(
            "/agentmux/reactive/agent-names",
            get(reactive::handle_reactive_agent_names),
        )
        // One live instance per agent, LAN tier (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24 §4.4).
        .route("/agentmux/agent/holding", get(agent_takeover::handle_agent_holding))
        // Identity M4a: inner to the auth layer (route_layer order: the
        // last one added runs first), so it sees which key authenticated.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            caller::caller_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            lan_or_full_auth_middleware,
        ));

    // Reactive routes. Previously registered without auth on the
    // assumption that localhost is a trust boundary; the 2026-05-11
    // security audit (C1 + C2) showed that any local process — or a
    // web page driving 127.0.0.1 via the permissive CORS layer — could
    // drive `/agentmux/reactive/inject` and reconfigure the cloud
    // muxbus poller. These routes are now merged into `authed_routes`
    // below and gated by `auth_middleware`. (`/inject` and `/agent` are
    // NOT here — see `lan_forward_routes` above.)
    let reactive_routes = Router::new()
        .route("/agentmux/reactive/agents", get(reactive::handle_reactive_agents))
        .route("/agentmux/reactive/audit", get(reactive::handle_reactive_audit))
        .route("/agentmux/reactive/register", post(reactive::handle_reactive_register))
        // Holder side of a takeover — full auth only, never LAN (spec §4.6).
        .route("/agentmux/agent/release", post(agent_takeover::handle_agent_release))
        .route(
            "/agentmux/reactive/unregister",
            post(reactive::handle_reactive_unregister),
        )
        // Identity M3 (spec §5): the one place a typed agent name is
        // interpreted. Called by agentmux-mcp before WorkEnqueue/CronCreate.
        .route(
            "/agentmux/agents/resolve",
            post(name_resolution::handle_resolve_agent_name),
        )
        .route(
            "/agentmux/reactive/ensure-signing-key",
            post(reactive::handle_reactive_ensure_signing_key),
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
        )
        .route(
            "/agentmux/reactive/transcript",
            get(reactive::handle_reactive_transcript),
        )
        .route(
            "/agentmux/reactive/supervisor-decision",
            post(reactive::handle_reactive_supervisor_decision),
        )
        // SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md — search an agent's own past
        // conversations on disk. Distinct from `/transcript`, which is the LIVE
        // session's tail only. Full-auth route (not in the LAN-scoped set):
        // this reads conversation content, which is exactly what the LAN key
        // must not reach.
        .route(
            "/agentmux/reactive/history/search",
            get(reactive::handle_reactive_history_search),
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
        .route(
            "/ws",
            get(websocket::handle_ws).route_layer(middleware::from_fn(ws_origin_guard)),
        )
        .route("/agentmux/service", post(service::handle_service))
        .route("/agentmux/file", get(files::handle_mux_file))
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
        // Bash command is running; we forward to the in-process MPS broker.
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
        // Real PTY-backed shell, for genuinely interactive programs the
        // piped /api/v1/shell/* family above can't drive (see
        // docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md).
        // agentmux-mcp's `PtyShell*` tools POST here.
        .route("/api/v1/ptyshell/create", post(handle_pty_shell_create))
        .route("/api/v1/ptyshell/input", post(handle_pty_shell_input))
        .route("/api/v1/ptyshell/resize", post(handle_pty_shell_resize))
        .route("/api/v1/ptyshell/read", post(handle_pty_shell_read))
        .route("/api/v1/ptyshell/status", post(handle_pty_shell_status))
        .route("/api/v1/ptyshell/stop", post(handle_pty_shell_stop))
        // Open a pane (editor/term/browser/…) from an agent tool call.
        // agentmux-mcp's OpenEditor tool POSTs `{view:"editor", file, …}` here;
        // shares the exact pane.open logic with the WebSocket RPC handler
        // (app_api::open_pane). See ANALYSIS_AGENT_APP_API_OPEN_IN_EDITOR_2026_05_30.
        .route("/api/v1/pane/open", post(handle_pane_open))
        // Open (launch) an agent into a pane from an agent tool call —
        // agentmux-mcp's OpenAgent tool POSTs `{agent_id, tab_id?, …}` here.
        // Shares the exact agent.open logic (incl. its AGENT_OPEN_LOCKS
        // TOCTOU serialization) with the WebSocket RPC handler via
        // app_api::open_agent_impl. Until this route existed, automation
        // could STOP an agent (/api/v1/fleet/bulk-stop, cross-channel) but
        // could not START one anywhere — see
        // REPORT_AGENT_OPEN_API_GAP_2026_09_06.md.
        .route("/api/v1/agent/open", post(handle_agent_open))
        // One live instance per agent, Phase 2 (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24 §4.6).
        .route("/api/v1/agent/takeover", post(agent_takeover::handle_agent_takeover))
        // Voice speech-to-text: the renderer POSTs mic audio (one
        // silence-bounded utterance per request); we forward to a Whisper
        // backend and return the transcript. Key stays server-side.
        // See SPEC_VOICE_STT_ENGINE_2026_06_20.md and #1591.
        .route("/api/v1/voice/transcribe", post(voice::handle_voice_transcribe))
        // Image attachments in the agent composer: serve stored files and
        // accept the paste fallback's streamed upload.
        // SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.2, §6.5.
        .route("/api/v1/attachments/upload", post(attachments::handle_attachment_upload))
        .route("/api/v1/attachments/:id/:kind", get(attachments::handle_attachment_file))
        // First-class agent API (SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md).
        // `GET /api/v1/self?block_id=` resolves the caller's place in the tree;
        // `POST /api/v1/window/name` sets the window display name (taskbar title).
        // agentmux-mcp's `WhoAmI` / `SetWindowName` tools call these.
        .route("/api/v1/self", get(handle_self))
        // The connections an agent's Shell/PtyShell may name (`ConnList` tool,
        // SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md §8.1).
        .route("/api/v1/conn/list", get(app_api::connections::handle_conn_list))
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
        // Live-state introspection for the `muxspect` CLI (Phase 1 of
        // docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md) —
        // read-only, diagnostic-only, reached the same way agentmux-mcp
        // reaches every other /api/v1/* route (X-AuthKey from the caller's
        // own environment, no new IPC).
        .route("/api/v1/muxspect/list", get(muxspect_handlers::handle_muxspect_list))
        .route("/api/v1/muxspect/layout", get(muxspect_handlers::handle_muxspect_layout))
        .route("/api/v1/muxspect/describe", get(muxspect_handlers::handle_muxspect_describe))
        // Cross-instance lookup — Ext 4 of
        // docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md.
        // Checks this instance first, then every other channel via the shared
        // reactive registry (same mechanism `conversations` below already
        // uses) — see the handler's own doc comment.
        .route("/api/v1/muxspect/find", get(muxspect_handlers::handle_muxspect_find))
        // Dock diagnosis/remediation extension (2026-08-06 spec) — `dock` is
        // read-only like the two routes above; `dock/clear` is this module's
        // first mutating route (narrowly scoped — see the handler's own doc
        // comment for why).
        .route("/api/v1/muxspect/dock", get(muxspect_handlers::handle_muxspect_dock))
        .route("/api/v1/muxspect/dock/clear", post(muxspect_handlers::handle_muxspect_dock_clear))
        // Durable declared-background task registry — see the handler's own
        // doc comment for why this is a separate, SQLite-backed source of
        // truth from `dock` above rather than another view over the same
        // ephemeral cache.
        .route(
            "/api/v1/muxspect/background-tasks",
            get(muxspect_handlers::handle_muxspect_background_tasks),
        )
        // Migration doctor — `migrate --verify`'s report from inside a live
        // instance (SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md Phase 1c).
        // Read-only; see the handler's own doc comment.
        .route(
            "/api/v1/muxspect/migrations",
            get(muxspect_handlers::handle_muxspect_migrations),
        )
        // Sender-liveness verdict for a JEKT's claimed FROM — see the
        // handler's own doc comment (SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md).
        .route(
            "/api/v1/muxspect/verify-sender",
            get(muxspect_handlers::handle_muxspect_verify_sender),
        )
        // All-tier conversation glance (host/cross-channel previews, LAN/WAN
        // liveness) — see the handler's own doc comment
        // (SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md Phase A).
        .route(
            "/api/v1/muxspect/conversations",
            get(muxspect_handlers::handle_muxspect_conversations),
        )
        // Agent App API — identity / preset / memory namespaces, the MCP-facing
        // slice of the app-API RPC surface (SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28).
        // The agent identity (`agent_id`) is supplied by agentmux-mcp from its
        // AGENTMUX_AGENT_ID env. Host agents do hold the instance key, so the
        // owner comes from the caller's token where there is one
        // (`SelfOwner::of`); a container token is always pinned to its own
        // agent (backend::container_credential). Each handler calls the
        // same `app_api::*_impl` the WebSocket RPC handlers use.
        .route("/api/v1/agent/memory/list", get(handle_agent_memory_list))
        .route("/api/v1/agent/memory/read", get(handle_agent_memory_read))
        .route("/api/v1/agent/memory/write", post(handle_agent_memory_write))
        .route("/api/v1/agent/memory/history", get(handle_agent_memory_history))
        .route("/api/v1/agent/memory/diff", get(handle_agent_memory_diff))
        .route("/api/v1/agent/memory/revert", post(handle_agent_memory_revert))
        // Global Memory (agent-facing) — SPEC_AGENT_FACING_GLOBAL_MEMORY_
        // API_2026_09_15.md. Unlike the agent/memory/* routes above (each
        // agent's own private native memory), these touch a single,
        // shared, workspace-wide list every agent inherits at launch. Same
        // trust model as every other route in this block: `agent_id` comes
        // from agentmux-mcp's trusted env, never forgeable from an agent's
        // own PTY. Never exposes or accepts a system-tier (`is_system`)
        // entry — see `global_memory_write_impl`'s own doc comment.
        .route("/api/v1/agent/globalmemory/write", post(handle_agent_globalmemory_write))
        .route("/api/v1/agent/globalmemory/list", get(handle_agent_globalmemory_list))
        .route("/api/v1/agent/globalmemory/read", get(handle_agent_globalmemory_read))
        .route("/api/v1/agent/globalmemory/remove", post(handle_agent_globalmemory_remove))
        // Global Memory audit trail (read side) — SPEC_AGENT_FACING_GLOBAL_
        // MEMORY_API_2026_09_15.md Phase 3 follow-up: `bundle_version_list`/
        // `bundle_version_get` (`bundle_versions.rs`) already existed and were
        // tested, but nothing exposed them via MCP/REST until now. Same trust
        // model and same system-tier exclusion as the four routes above.
        .route("/api/v1/agent/globalmemory/history", get(handle_agent_globalmemory_history))
        .route("/api/v1/agent/globalmemory/diff", get(handle_agent_globalmemory_diff))
        .route("/api/v1/agent/globalmemory/revert", post(handle_agent_globalmemory_revert))
        // An agent's memory delivered through Claude Code's `SessionStart`
        // hook, one part per hook command, then an acknowledgement per part —
        // SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2. Called by
        // `agentmux-bashwrap sessionstart`, whose `X-Agent-Token` names the
        // agent whose Personal Memory is delivered.
        .route(
            "/api/v1/agent/memory/session-start/part",
            post(memory_delivery_handlers::handle_session_start_part),
        )
        .route(
            "/api/v1/agent/memory/session-start/ack",
            post(memory_delivery_handlers::handle_session_start_ack),
        )
        .route("/api/v1/agent/preset/list", get(handle_agent_preset_list))
        .route("/api/v1/agent/preset/get", get(handle_agent_preset_get))
        .route("/api/v1/agent/identity/accounts", get(handle_agent_identity_accounts))
        .route("/api/v1/agent/identity/validate", post(handle_agent_identity_validate))
        // Agent UI automation (SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md)
        // — proxies to the paired CEF host's browser_api CDP routes. `block_id`
        // is stamped by agentmux-mcp from AGENTMUX_BLOCKID, same trust model as
        // the agent/memory/preset/identity routes just above.
        .route("/api/v1/ui/screenshot", post(ui_handlers::handle_ui_screenshot))
        .route("/api/v1/ui/click", post(ui_handlers::handle_ui_click))
        .route("/api/v1/ui/query", post(ui_handlers::handle_ui_query))
        // Browser-pane deep control (SPEC_AGENT_BROWSER_PANE_DEEP_CONTROL_2026_09_20.md)
        // — own-pane-only, same identity model as the ui/* routes above.
        // navigate/back/forward/reload/eval additionally require the
        // caller's own pane to be a dedicated browser pane (checked
        // host-side in browser_api::routes::reject_if_shared_target).
        .route("/api/v1/ui/browser/navigate", post(ui_handlers::handle_ui_browser_navigate))
        .route("/api/v1/ui/browser/back", post(ui_handlers::handle_ui_browser_back))
        .route("/api/v1/ui/browser/forward", post(ui_handlers::handle_ui_browser_forward))
        .route("/api/v1/ui/browser/reload", post(ui_handlers::handle_ui_browser_reload))
        .route("/api/v1/ui/browser/eval", post(ui_handlers::handle_ui_browser_eval))
        .route("/api/v1/ui/browser/dispatch_key", post(ui_handlers::handle_ui_browser_dispatch_key))
        .route("/api/v1/ui/browser/focus_element", post(ui_handlers::handle_ui_browser_focus_element))
        .route("/api/v1/ui/browser/focus_info", post(ui_handlers::handle_ui_browser_focus_info))
        // Pane lifecycle (SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md) —
        // `ClosePane`. Own-pane identity is verified the same way as the
        // ui/* routes above (`verified_block_id`); the target pane, when
        // acting on another agent's pane, is fleet-tier (see the
        // fleet/bulk-stop comment below) — no ownership check on the
        // target, but the caller is never anonymous in the audit log the
        // way `FleetBulkStop`'s calls are today.
        .route("/api/v1/agent/pane/close", post(app_api::pane::handle_close_pane))
        .route("/api/v1/agent/self/quit", post(app_api::pane::handle_quit_self))
        // The outcome of a shutdown waiting on the user's override (§6.5):
        // the MCP tools poll it after their 202.
        // axum 0.7: a path parameter is `:name`; `{name}` would be a literal.
        .route("/api/v1/agent/shutdown/:request_id", get(app_api::pane::handle_shutdown_status))
        // Native dev-proxy registration (SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md)
        // — `RegisterDevServer`. Same `verified_block_id` identity model as
        // the ui/* and pane/close routes above; the backend address it
        // actually stores is resolved server-side from the CALLER's own
        // container, never taken from the request body.
        .route("/api/v1/agent/dev_server/register", post(app_api::dev_server::handle_register_dev_server))
        // Fleet control (SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md) —
        // bulk-stop is the one fleet action exposed to agentmux-mcp (see
        // `FleetBulkStop`): stopping a controller involves no jekt signing,
        // unlike broadcast (which an agent instead sends by looping the
        // existing single-target signed SendMessage path client-side —
        // see `server/app_api/fleet.rs`'s module doc comment for why no
        // `/api/v1/fleet/broadcast` route exists). Same MCP-facing App-API
        // trust model as the routes above: gated by the general X-AuthKey
        // middleware wrapping this whole router, no additional per-agent
        // scoping — fleet actions target OTHER agents' panes by design, so
        // "own pane only" doesn't apply here the way it does to `ui/*`.
        .route("/api/v1/fleet/bulk-stop", post(handle_fleet_bulk_stop))
        // Cross-channel bulk-stop forward target
        // (SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md): when
        // `fleet_bulk_stop_impl` can't find a target block_id in ITS OWN
        // in-process controller registry, it checks the shared
        // cross-channel registry and forwards here — same trust model as
        // `/agentmux/reactive/inject`'s own cross-channel forward (same
        // host, same user, the target channel's own auth_key from the
        // shared registry). Loopback-only by construction: the caller only
        // ever forwards to a `local_url` it already verified is loopback.
        .route("/agentmux/agent/stop", post(handle_agent_stop_forward))
        // The same forward, for an agent's FleetBulkStop: this instance's
        // user gets the 15 s override window (SPEC_AGENT_SELF_QUIT §6.5).
        .route("/agentmux/agent/stop-pending", post(handle_agent_stop_pending_forward))
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
        // Muxqueue — the universal agent work queue, cron's readiness-triggered
        // sibling (docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md).
        // Same auth gate as the cron routes above.
        .route("/agentmux/work", post(work_queue::handle_work_enqueue))
        .route(
            "/agentmux/identity/fallbacks",
            get(caller::handle_identity_fallbacks),
        )
        .route("/agentmux/work", get(work_queue::handle_work_list))
        .route("/agentmux/work/claim", post(work_queue::handle_work_claim))
        .route("/agentmux/work/:id/heartbeat", post(work_queue::handle_work_heartbeat))
        .route("/agentmux/work/:id/complete", post(work_queue::handle_work_complete))
        .route("/agentmux/work/:id/release", post(work_queue::handle_work_release))
        .route("/agentmux/work/:id", delete(work_queue::handle_work_cancel))
        .merge(bus_routes)
        .merge(reactive_routes)
        // Identity M4a: attribute the (already authenticated) request.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            caller::caller_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Health endpoint (no auth). `/health` always; `/` too, except on the
    // full router when headless `--frontend-dir` serves the frontend there
    // (`with_frontend`). The LAN router's health never changes.
    let health = Router::new()
        .route("/", get(health_handler))
        .route("/health", get(health_handler));
    let full_health = if frontend_dir.is_some() {
        Router::new().route("/health", get(health_handler))
    } else {
        health.clone()
    };

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

    // Ext 5 of docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md:
    // stamp every response with this instance's own version, so a stale-
    // build 404 (this session hit exactly this: `muxspect conversations`
    // 404ing because the running srv predated that route, with nothing
    // saying so) is self-diagnosing instead of a bare, unexplained 404.
    // Captured by value (not via AppState extraction) so this applies to
    // EVERY route uniformly, including the unauthenticated health/webhook
    // ones, without needing `AppState: Clone`.
    let version_for_header = state.version.clone();
    let version_header = axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
        let version = version_for_header.clone();
        async move {
            let mut response = next.run(req).await;
            if let Ok(value) = axum::http::HeaderValue::from_str(&version) {
                response.headers_mut().insert("x-agentmux-srv-version", value);
            }
            response
        }
    });

    let lan = Router::new()
        .merge(health.clone())
        .merge(whatsapp_webhooks.clone())
        .merge(lan_forward_routes.clone())
        .layer(version_header.clone())
        .layer(cors.clone())
        .with_state(state.clone());

    let full = Router::new()
        .merge(full_health)
        .merge(whatsapp_webhooks)
        .merge(lan_forward_routes)
        .merge(authed_routes);
    // Before the layers: `Router::layer` wraps only the routes and fallback
    // that exist when it's called, so a fallback added afterwards would skip
    // CORS and the version header (ReAgent P1 on #3900).
    let full = match frontend_dir {
        Some(dir) => with_frontend(full, dir),
        None => full,
    };
    let full = full.layer(version_header).layer(cors).with_state(state);

    SrvRouters { full, lan }
}

/// Serve the built frontend in `dir` for every path no route claims, `/`
/// included. Unauthenticated, like the CEF host's static server: the bundle
/// holds no secrets, and srv's key still guards every API route. Unknown
/// paths get `index.html`. Full router only; the LAN router never serves it.
pub(super) fn with_frontend(router: Router<AppState>, dir: &std::path::Path) -> Router<AppState> {
    use tower_http::services::{ServeDir, ServeFile};
    let index = ServeFile::new(dir.join("index.html"));
    router.fallback_service(ServeDir::new(dir).fallback(index))
}
