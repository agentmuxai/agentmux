// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared server state: AppState and HostIpc.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// The paired CEF host's CDP-automation credentials, pushed once via
/// `host_ipc.Register` — see `AppState::host_ipc`.
#[derive(Clone, Debug)]
pub struct HostIpc {
    pub port: u16,
    pub token: String,
}

#[derive(Clone)]
pub struct AppState {
    pub auth_key: String,
    /// LAN peer-discovery credential — see `Config::lan_key`'s doc comment.
    /// Accepted only by `lan_or_full_auth_middleware`'s three routes, never
    /// the general `auth_middleware` gating everything else.
    pub lan_key: String,
    /// Random identifier generated once per process boot — NOT the
    /// `--instance` channel name (`config.instance_id`), which is
    /// stable across restarts and shared by every process on the same
    /// channel. Used as the owner id for cross-process session leases
    /// (`registry::LeaseStore`) so two live processes can be told
    /// apart even when they're the same channel/version.
    /// See `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`.
    pub boot_id: Arc<str>,
    pub version: String,
    /// OS hostname of the machine this instance runs on (e.g. "narko").
    /// Already computed at boot for LAN discovery's mDNS TXT records; kept
    /// here so `/agentmux/discovery` can name its own host too — `local_url`
    /// is always loopback, so without this a client has no way to say which
    /// machine it is talking to.
    pub hostname: String,
    pub app_path: String,
    pub mstore: Arc<Store>,
    /// GLOBAL shared store (`~/.agentmux/shared/store.db`). Holds durable
    /// user content that must survive version upgrades: identity accounts,
    /// memory bundles, drone definitions, and MuxBus credentials.
    /// `None` when the shared root can't be resolved (CI / unusual envs).
    /// See `docs/specs/SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md`.
    pub shared_store: Option<Arc<Store>>,
    /// Effective identity/memory/drone/muxbus store — `shared_store` when
    /// available, otherwise `mstore`. Handlers capture this instead of
    /// `mstore` for any operation that must survive across version upgrades.
    ///
    /// Deprecated for everything except `db_accounts` reads/writes as of
    /// `docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` — new call sites
    /// for agent→account links, memory bundles, drone definitions, muxbus
    /// creds, native memory, or cron jobs should use `identity_store`
    /// instead, which (unlike this field) is never redirected by
    /// `isolated_auth_enabled()`. `id_store` itself stays because
    /// `db_accounts` still legitimately needs an isolatable path (Armory
    /// testing) — see the spec's §3.2 for the account-specific split, not
    /// yet implemented.
    pub id_store: Arc<Store>,
    /// Permanently-global identity store — see `identity_store`'s own doc
    /// comment on `bootstrap::Stores` for the full explanation. Never
    /// `None`: falls back to `mstore` on the same best-effort terms as
    /// `id_store` if the shared root can't be resolved.
    pub identity_store: Arc<Store>,
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
    pub config_watcher: Arc<wconfig::ConfigState>,
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
    /// In-memory, latest-per-block cache of Activity Dock `ToolNode` status
    /// deltas pushed by the frontend. Backs `muxspect dock`/`muxspect dock
    /// clear`. Never persisted — see
    /// `docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md`.
    pub dock_snapshots: Arc<crate::backend::dock_snapshot::DockSnapshotCache>,
    /// Holding pen for a declared-background task's OS pid when it arrives
    /// (from bashwrap, over MPS) before its `db_background_tasks` row
    /// exists yet — closes the race `background_task_set_pid`'s silent
    /// no-op on a missing row would otherwise lose permanently. See
    /// `docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md` and the
    /// Codex/reagentx findings on PR #2681.
    pub pending_background_pids: Arc<crate::backend::pending_background_pids::PendingBackgroundPids>,
    /// Which events have already produced an ambient narration. Process-wide
    /// and bounded — a per-connection set is reset by every reconnect, which is
    /// precisely when the frontend re-emits nodes and would narrate them twice.
    /// See `backend::narrated_events`.
    pub narrated_events: Arc<crate::backend::narrated_events::NarratedEvents>,
    /// Live controller for mDNS-based LAN/host peer discovery. The controller
    /// owns a swappable daemon slot so the `network:lan_discovery` setting can
    /// be toggled at runtime without restarting the process.
    /// See `docs/specs/lan-discovery-toggle.md`.
    pub lan_discovery: Arc<LanDiscoveryController>,
    /// Owns the LAN-facing HTTP listeners. Paired with `lan_discovery`: that
    /// one controls whether we ADVERTISE on the LAN, this one controls whether
    /// we are actually REACHABLE there. Both must be driven from the same
    /// place — advertising without listening is the "discovered but
    /// unreachable" state described in `backend::lan_listeners`.
    pub lan_listeners: Arc<crate::backend::lan_listeners::LanListenerSupervisor>,
    /// The names-only fleet feed behind `/agentmux/fleet` and
    /// `/agentmux/fleet/events` (`backend::fleet_feed`). Its change-detection
    /// task is started by `main.rs` once the shutdown token exists.
    pub fleet_feed: Arc<crate::backend::fleet_feed::FleetFeed>,
    /// Paired devices' read-only view: pairing codes, the certificate, the
    /// feed hub and the feed caps (`backend::viewer`). Its routes are served
    /// only by the viewer listener (`server::build_viewer_router`).
    pub viewer: Arc<crate::backend::viewer::ViewerService>,
    /// Language Server Protocol supervisor — owns the lifecycle of LSP
    /// server child processes (one per workspace/language) and proxies
    /// LSP messages between the editor pane and the server.
    /// Spec: `docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md`.
    pub lsp_supervisor: Arc<LspSupervisor>,
    /// Local HTTP URL of this instance (e.g. "http://127.0.0.1:PORT").
    /// Used for cross-instance inject forwarding and file registry entries.
    pub local_web_url: String,
    /// Shared HTTP client for cross-instance inject forwarding.
    pub http_client: reqwest::Client,
    /// This instance's paired CEF host's `ipc_port` + `ipc_token`
    /// (`/agentmux/browser/*` on the host's own IPC server — CDP-based
    /// screenshot/click/query automation). `None` until the host calls
    /// `host_ipc.Register` once at its own startup (there is no way to
    /// know these values before then — the host generates `ipc_token`
    /// for itself and is the sole source of truth, see
    /// `crates/cef/src/client/helpers.rs::register_ipc_with_backend`).
    /// Backs the `/api/v1/ui/{screenshot,click,query}` proxy routes.
    /// See `docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`.
    pub host_ipc: Arc<tokio::sync::Mutex<Option<HostIpc>>>,
    /// Shared secret only the paired host (never an agent) is given —
    /// gates `host_ipc.Register` so an agent process can't spoof the
    /// host's own credential push (both share the general `auth_key`
    /// already, so that alone can't distinguish them). `None` if nobody
    /// configured one, in which case `handle_register` rejects every
    /// registration. See `Config::host_reg_secret`'s doc comment.
    pub host_reg_secret: Option<String>,
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
    /// `install_chunk` MPS events scoped by session id; the registry
    /// holds per-session cancel handles so `install.cancel` can abort
    /// an install mid-flight.
    /// See `SPEC_AGENT_INSTALL_STAGE_2026_05_17.md` §9.
    pub install_sessions: std::sync::Arc<crate::server::install_handlers::InstallSessionRegistry>,
    /// Docker container runtime handle for container-type agent panes
    /// (Phase 2). Self-healing — `.get()`/`.is_available()` retry the
    /// Docker connection on demand rather than being fixed at process
    /// boot, so a daemon that starts after AgentMux launched is picked up
    /// without an app restart. See `ContainerRuntimeHandle` and
    /// docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    pub container_manager: std::sync::Arc<crate::backend::container::ContainerRuntimeHandle>,
    /// Native dev-proxy routing table (`"<project>-<agent_id>"` →
    /// container-internal `ip:port`) — backs the `RegisterDevServer` MCP
    /// tool and the standalone dev-proxy HTTP server spawned in `main.rs`.
    /// Cheap to clone (one `Arc` inside); `container_manager` also holds a
    /// clone (wired via `ContainerRuntimeHandle::set_dev_proxy_registry`
    /// in `bootstrap::build_app_state`) so `stop`/`remove` can clear a
    /// dead container's routes. See
    /// docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md.
    pub dev_proxy: crate::backend::dev_proxy::DevProxyRegistry,
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
    /// stale. Migrated onto the shared `fs_watch_pool` below
    /// (SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md) — always constructs
    /// successfully now (a construction-time failure degrades into
    /// `fs_watch_pool.health()`'s `degraded_paths` instead of `None`), so
    /// unlike before this field is no longer optional.
    /// See docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md.
    pub editor_file_watcher: std::sync::Arc<crate::backend::editor_file_watcher::EditorFileWatcher>,
    /// Filesystem watcher for directories a Media pane is pointed at —
    /// publishes `EVENT_MEDIA_FILE_CHANGED` (scoped per-block) when a
    /// matching-extension file is created/modified. Same migration and
    /// no-longer-optional note as `editor_file_watcher` above.
    /// See docs/specs/SPEC_MEDIA_PANE_2026_07_26.md.
    pub media_file_watcher: std::sync::Arc<crate::backend::media_file_watcher::MediaFileWatcher>,
    /// Directory watcher behind `fs.watch` for Files panes — publishes
    /// `files:changed` (scoped per-block) when a watched folder's listing
    /// changes. Process-wide rather than per connection, so a watch outlives
    /// the WebSocket that made it, like the two watchers above.
    /// See docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3.
    pub files_watcher: std::sync::Arc<crate::backend::files_watcher::FilesWatcher>,
    /// Shared filesystem-watcher framework (retry/fallback/self-healing on
    /// top of `notify`) that `editor_file_watcher`/`media_file_watcher`
    /// above are built on. See
    /// docs/specs/SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md.
    pub fs_watch_pool: std::sync::Arc<crate::backend::fs_watch::FsWatchPool>,
}
