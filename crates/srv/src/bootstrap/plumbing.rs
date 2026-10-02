// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of bootstrap.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

use super::*;

/// Output of [`spawn_reducer_plumbing`].
pub struct ReducerPlumbing {
    pub srv_state: Arc<tokio::sync::Mutex<state::State>>,
    pub srv_events_tx: tokio::sync::broadcast::Sender<agentmux_common::ipc::Event>,
    pub srv_event_log: Arc<event_log::EventLog>,
}

/// Phase E.2 / E.2c.2 — srv reducer plumbing, hoisted out of the
/// (conditional) pipe-IPC bind block so HTTP/WS RPC handlers in
/// dispatch_service can route through the reducer. State, event
/// bus, event log, and persist subscriber all live unconditionally;
/// the pipe IPC server is still conditional on
/// `AGENTMUX_SRV_PIPE_PATH` being set (absent in `task dev` mode).
///
/// Bootstraps reducer state from SQLite, spawns the disk writer (forensic
/// log of every reducer event), the persist subscriber (idempotent SQLite
/// write-back), the MuxObjUpdate bridge, and the subagent-watcher block-delete
/// cascade backstop.
pub async fn spawn_reducer_plumbing(
    mstore: &Arc<Store>,
    event_bus: &Arc<EventBus>,
    subagent_watcher: &Arc<backend::subagent_watcher::SubagentWatcher>,
) -> ReducerPlumbing {
    let mstore_for_persist = Arc::clone(mstore);
    let srv_state = std::sync::Arc::new(tokio::sync::Mutex::new(state::State::default()));
    let (srv_events_tx, _) =
        tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(1024);
    let srv_event_log = std::sync::Arc::new(event_log::EventLog::new(Some(
        base::get_mux_data_dir().join("srv-events.log"),
    )));

    // Bootstrap reducer state from SQLite. Always runs (even in
    // `task dev` where there's no pipe IPC server) so RPC handlers
    // dispatching through the reducer see populated state.
    persist::bootstrap_state_from_mstore(&srv_state, &mstore_for_persist).await;

    // Spawn the disk writer (forensic log of every reducer event)
    // and the persist subscriber (idempotent SQLite write-back).
    let disk_writer_rx = srv_events_tx.subscribe();
    let log_for_writer = std::sync::Arc::clone(&srv_event_log);
    tokio::spawn(event_log::run_disk_writer(log_for_writer, disk_writer_rx));
    let subscriber_rx = srv_events_tx.subscribe();
    persist_subscriber::spawn_persist_subscriber(
        subscriber_rx,
        std::sync::Arc::clone(&mstore_for_persist),
        std::sync::Arc::clone(&srv_state),
    );

    // Phase 1 of the MuxObjUpdate bridge: subscribe to srv_events_tx and
    // translate workspace mutations into `waveobj:update` WS broadcasts.
    // Fixes the workspace-rename reactivity gap where UpdateWorkspace
    // returned `success_empty()` and the response loop had nothing to
    // broadcast — see docs/specs/SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md.
    //
    // Watchdog: capture the JoinHandle and observe it from a sibling task
    // so a panic in the bridge's loop scaffolding (vs. an inner
    // dispatch_event panic, which is already caught per-event) is logged
    // loudly. Without this, a silent bridge death would manifest as
    // "renaming a workspace stopped propagating" with no log evidence.
    // (Per ReAgent P2 follow-up on PR #852.)
    let bridge_rx = srv_events_tx.subscribe();
    let bridge_handle = server::mux_obj_bridge::spawn_mux_obj_bridge(
        bridge_rx,
        std::sync::Arc::clone(&mstore_for_persist),
        std::sync::Arc::clone(event_bus),
    );
    tokio::spawn(async move {
        match bridge_handle.await {
            Ok(()) => tracing::info!(
                target: "wave-obj-bridge",
                "bridge task exited normally (events channel closed at srv shutdown)"
            ),
            Err(e) if e.is_panic() => tracing::error!(
                target: "wave-obj-bridge",
                "bridge task PANICKED at top level — frontend MOS will stop receiving updates until srv restart. Panic: {}",
                e
            ),
            Err(e) => tracing::error!(
                target: "wave-obj-bridge",
                "bridge task terminated unexpectedly (non-panic JoinError): {}",
                e
            ),
        }
    });

    // Block-delete cascade backstop for the subagent watcher: prunes a
    // closed block's subagents/dispatches on Event::BlockDeleted/TabDeleted/
    // WorkspaceDeleted, independent of whether the frontend's normal
    // /agentmux/reactive/unregister teardown path fires for this close (see
    // SubagentWatcher::prune_block's doc comment). Without this, closing an
    // agent pane left a ghost row in the Swarm pane until srv restart.
    let block_prune_rx = srv_events_tx.subscribe();
    backend::subagent_watcher::spawn_block_prune_subscriber(subagent_watcher.clone(), block_prune_rx);

    ReducerPlumbing {
        srv_state,
        srv_events_tx,
        srv_event_log,
    }
}

/// Assembles the final `AppState` from every bundle produced above, plus the
/// handful of state pieces (auth session manager, install sessions, container
/// manager, shell sessions, cron scheduler) that only ever existed inline in
/// the `AppState` struct literal.
pub fn build_app_state(
    config: &config::Config,
    version: String,
    stores: Stores,
    bg: BackgroundSubsystems,
    net: &NetworkBundle,
    reducer: &ReducerPlumbing,
) -> AppState {
    // Clone before move into AppState for cron_scheduler construction.
    let shared_store_for_cron = stores.shared_store.clone();
    // Live settings for the agent CPU-priority policy (process_tracker).
    net.process_tracker.set_config(bg.config_watcher.clone());
    let broker = bg.broker;

    // Built before `container_manager` below so its constructor block can
    // wire the two together (`set_dev_proxy_registry`) — `stop`/`remove`
    // then clear an agent's routes when its container goes away. See
    // docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md.
    let dev_proxy = crate::backend::dev_proxy::DevProxyRegistry::new();

    AppState {
        auth_key: config.auth_key.clone(),
        lan_key: config.lan_key.clone(),
        boot_id: Arc::from(uuid::Uuid::new_v4().to_string()),
        version,
        hostname: net.hostname.clone(),
        app_path: config.app_path.clone(),
        mstore: stores.mstore,
        shared_store: stores.shared_store,
        id_store: stores.id_store,
        identity_store: stores.identity_store,
        filestore: stores.filestore,
        global_transcript_store: stores.global_transcript_store,
        event_bus: bg.event_bus,
        broker: broker.clone(),
        reactive_handler: bg.reactive_handler,
        poller: bg.poller,
        config_watcher: bg.config_watcher,
        messagebus: bg.messagebus,
        subagent_watcher: bg.subagent_watcher,
        history_service: bg.history_service,
        lan_discovery: net.lan_discovery.clone(),
        lan_listeners: net.lan_listeners.clone(),
        lsp_supervisor: net.lsp_supervisor.clone(),
        local_web_url: net.local_web_url.clone(),
        // Bounded request timeout: cross-instance reactive-inject forwards
        // (Tier 2/2b/3) chain through this client, and an unbounded client
        // combined with a forwarding cycle (two channels each holding a
        // stale-but-PID-alive shared-registry entry pointing at the other)
        // could otherwise hang a request indefinitely. The hop-count guard
        // in server/reactive.rs bounds the CYCLE; this bounds each
        // individual HOP (reagent P1 on PR #2350).
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "failed to build http_client with timeout, using default");
                reqwest::Client::new()
            }),
        host_ipc: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        host_reg_secret: config.host_reg_secret.clone(),
        process_tracker: net.process_tracker.clone(),
        process_broker: net.process_broker.clone(),
        dock_snapshots: std::sync::Arc::new(crate::backend::dock_snapshot::DockSnapshotCache::new()),
        pending_background_pids: std::sync::Arc::new(crate::backend::pending_background_pids::PendingBackgroundPids::new()),
        narrated_events: Arc::new(crate::backend::narrated_events::NarratedEvents::new()),
        // Phase E.2c.2 — reducer state + event bus exposed to HTTP/WS
        // dispatch handlers. Workspace handlers route through the
        // reducer and publish events to `srv_events_tx`; the persist
        // subscriber writes back to SQLite asynchronously.
        srv_state: std::sync::Arc::clone(&reducer.srv_state),
        srv_events_tx: reducer.srv_events_tx.clone(),
        // Phase E.5.5 — saga-id allocator. Seeded from
        // `SagaLog::max_saga_id()` so restarts don't collide with
        // prior runs' IDs. First new saga after restart gets
        // `seed + 1`; on a fresh DB seed=0, first saga gets id 1.
        saga_id_alloc: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(stores.saga_id_seed)),
        saga_log: stores.saga_log,
        auth_session_manager: std::sync::Arc::new(
            crate::identity::auth_session::AuthSessionManager::new(),
        ),
        install_sessions: crate::server::install_handlers::InstallSessionRegistry::new(),
        container_manager: {
            // Self-healing: unlike a plain `Option<ContainerManager>` fixed at
            // boot, `ContainerRuntimeHandle` retries the connect on demand, so
            // a daemon that starts after this point is picked up by later
            // calls without an app restart. See
            // docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
            let handle = std::sync::Arc::new(
                crate::backend::container::ContainerRuntimeHandle::connect_at_startup(),
            );
            handle.set_dev_proxy_registry(dev_proxy.clone());
            let handle_check = handle.clone();
            tokio::spawn(async move {
                if handle_check.is_available().await {
                    tracing::info!("Docker daemon available — container agent panes enabled");
                } else {
                    tracing::warn!(
                        "Docker daemon not reachable at startup; container agent panes will \
                         become available automatically once Docker is running"
                    );
                }
            });
            handle
        },
        dev_proxy,
        shell_sessions: crate::backend::shell_node::ShellSessionRegistry::new(),
        cron_scheduler: crate::backend::cron::CronScheduler::new(
            shared_store_for_cron,
            reqwest::Client::new(),
            net.local_web_url.clone(),
            config.auth_key.clone(),
            Arc::clone(&broker),
        ),
        editor_file_watcher: bg.editor_file_watcher,
        media_file_watcher: bg.media_file_watcher,
        files_watcher: bg.files_watcher,
        fs_watch_pool: bg.fs_watch_pool,
    }
}

/// Phase E.1b — srv pipe IPC server. Bound when launcher passes
/// `AGENTMUX_SRV_PIPE_PATH`; absent in `task dev` mode (no
/// launcher in the loop).
///
/// Bind happens BEFORE the AGENTMUXSRV-ESTART line so the
/// launcher knows the pipe is ready when host starts. Non-fatal
/// if the bind fails — srv keeps running with HTTP/WS only.
#[cfg(target_os = "windows")]
pub fn bind_srv_pipe_ipc(
    version: &str,
    srv_state: std::sync::Arc<tokio::sync::Mutex<state::State>>,
    srv_events_tx: tokio::sync::broadcast::Sender<agentmux_common::ipc::Event>,
    srv_event_log: std::sync::Arc<event_log::EventLog>,
) {
    if let Ok(srv_pipe_path) = std::env::var("AGENTMUX_SRV_PIPE_PATH") {
        if !srv_pipe_path.is_empty() {
            match crate::srv_ipc::server::bind_first_pipe_instance(&srv_pipe_path) {
                Ok(first_pipe) => {
                    // Phase E.2c.2 — pipe IPC server reuses the
                    // hoisted srv_state / events_tx / event_log so
                    // pipe-originated commands and HTTP/WS-originated
                    // commands mutate the same canonical state.
                    let srv_ctx = crate::srv_ipc::ServerCtx {
                        srv_pid: std::process::id(),
                        srv_version: version.to_string(),
                        state: srv_state,
                        events_tx: srv_events_tx,
                        event_log: srv_event_log,
                    };
                    let _srv_ipc_handle = crate::srv_ipc::run_srv_ipc_server(
                        srv_pipe_path.clone(),
                        first_pipe,
                        srv_ctx,
                    );
                    tracing::info!(
                        target: "srv-ipc",
                        "[srv-ipc] bound + spawned on {}",
                        srv_pipe_path
                    );
                }
                Err(e) => {
                    tracing::error!(
                        target: "srv-ipc",
                        "[srv-ipc] bind failed on {}: {} — srv runs without pipe IPC",
                        srv_pipe_path,
                        e
                    );
                }
            }
        }
    }
}
