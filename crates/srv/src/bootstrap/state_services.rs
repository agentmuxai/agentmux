// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `app_main` starts once the app state exists, in the order it started them: the
//! hooks that let an agent be reached and resumed, the job run ledger and the cron
//! scheduler, the sagas an earlier run left unresolved, and the services that run beside
//! the server. Moved out of `app_main` without a change in behaviour.

use super::*;
use std::future::IntoFuture;

/// Wires delivery and the restart-time resumers to the app state, and starts the native
/// memory watchers.
pub fn install_state_hooks(state: &AppState) {
    // The relay's delivery path reaches the server's holds and file steps through this, before anything is claimed.
    crate::server::muxbus_hooks::install();
    // Upgrade reactive message delivery now that AppState exists, so agents on a
    // SubprocessController (every container agent, plus codex/gemini/qwen/kimi/
    // muxcode/antigravity on the host) can actually receive inter-agent
    // messages instead of having them dropped on a PTY fallback they reject.
    install_agent_turn_delivery(state);
    // What is held for an agent waits while its pane has no controller yet (every resumed pane, at start).
    crate::backend::reactive::controller_wait::set_has_controller(std::sync::Arc::new(|block_id: &str| crate::backend::blockcontroller::get_controller(block_id).is_some()));

    // Cron fires through the same in-process inject path (identity M4c-3);
    // before `cron_scheduler.start()` below, so no fire takes the HTTP
    // fallback.
    install_cron_delivery(state);
    crate::server::job_actions::install(state);
    // The Drone blocks that act on AgentMux (App query, Message, Task, Spawn, Notify) reach it here.
    crate::drone::host::install(state);

    // Durable jekt: replay messages held for an agent that was not running
    // (SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md).
    crate::server::jekt_held::install(state);
    // A routine message that came by the relay waits for a session gone cold, as a local one does (#2263).
    crate::server::jekt_cold::install(state);
    // A parked agent no pane shows is sent the input held for it while its model API was out.
    crate::server::held_input::install(state);

    // Agent panes no window reconnected to after the restart get their controller a minute in.
    crate::server::controller_backstop::install(state);

    // Idle agents with no view are stopped after the window (agents:always_available).
    crate::server::agent_lifecycle::install(state);

    // Parked agents that were running when srv stopped are resumed, a few seconds apart (agents:always_available).
    crate::server::restart_resume::install(state);

    // An agent has one pane: a second pane an older layout left is closed, pointing at the first.
    crate::server::app_api::agent_reconcile::install(state);

    // The helper install asks first (SPEC_REMOTES_PANE_2026_10_05.md §4.9).
    crate::server::app_api::connections::install_helper_consent(state);

    // Now that AppState exists, wire up close-on-exit so a shell pane can
    // actually close itself when its process exits — see
    // `install_close_on_exit_handler`'s doc comment.
    install_close_on_exit_handler(state);

    // Backstop for agent blocks left in a tab but in no pane — see
    // `crate::sagas::orphan_reaper`'s module doc.
    crate::sagas::orphan_reaper::install(state);

    // Out-of-band native-memory write detection (fast fs-watch path + slow
    // reconciliation-sweep path) — see
    // docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.5.
    backend::memory::native_drift::spawn(
        state.fs_watch_pool.clone(),
        state.mstore.clone(),
        state.id_store.clone(),
        state.broker.clone(),
    );

    // Retention/GC for native-memory version history — see
    // docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §7.1.
    backend::memory::native_retention::spawn(state.id_store.clone());
}

/// Opens the job run ledger, closes the runs a restart interrupted, then starts the
/// cron scheduler.
pub async fn start_job_ledger_and_cron(state: &AppState) {
    // The run ledger (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3): close the
    // runs a restart interrupted, then record every fire from here on.
    let run_ledger = backend::jobs::RunLedger::new(
        crate::server::cron::ledger_store(state),
        Some(state.mstore.clone()),
        state.broker.clone(),
    );
    // A `spawn` job's worker is closed when its run ends: its tab goes, so it
    // doesn't count against the leader's worker limit for the next run.
    let worker_state = state.clone();
    run_ledger.install_worker_stop(std::sync::Arc::new(move |block_id: &str| {
        crate::sagas::pending_shutdown::request(
            &worker_state,
            block_id,
            "jobs",
            "JobWorker",
            "its run ended",
            crate::sagas::pending_shutdown::Action::CloseTab {
                detail: "a job's worker, its run ended".to_string(),
            },
        );
    }));
    // Runs are weighed against Settings' spending caps before they start, and
    // stopped when they reach their own (jobs spec §5.3).
    crate::server::job_caps::install(state, &run_ledger);
    // A timeout or cancel of a run another channel follows is asked of that channel.
    crate::server::jobs_remote_stop::install(state, &run_ledger);
    // A failing job opens one incident for its owner (jobs spec §3.4).
    run_ledger.install_incidents(crate::server::cron::jobs_store(state));
    // Cloud jobs report their runs in over MuxBus (jobs spec §4).
    backend::jobs::cloud_hub::install(run_ledger.clone());
    // A cloud run that never reported, or stopped reporting, is given up on (jobs spec §4.4).
    if let Some(templates) = backend::job_templates::service() {
        backend::jobs::watchdog::start(run_ledger.clone(), std::sync::Arc::new(templates.clone()));
    }
    {
        let ledger = run_ledger.clone();
        let _ = tokio::task::spawn_blocking(move || ledger.recover_after_restart()).await;
    }
    // A paused job's slots are counted as skips (jobs spec §3.3).
    if let Some(jobs_store) = crate::server::cron::jobs_store(state) {
        backend::jobs::paused::spawn(jobs_store, run_ledger.clone());
    }
    state.cron_scheduler.install_runs(run_ledger);

    // Start persistent cron scheduler — load enabled jobs from DB, fire any
    // that missed their window (FIRE_ONCE_NOW), schedule all for future fires.
    state.cron_scheduler.start().await;
}

/// Compensates the sagas an earlier run left unresolved; non-fatal when it fails.
pub async fn resume_unresolved_sagas(state: &AppState) {
    // Saga durability PR 2 — resume-on-startup. Walk any sagas the
    // durable log says are unresolved (running / compensating /
    // failed) from a prior srv-process run, dispatch their inverse
    // commands, and mark them compensated. Runs AFTER reducer
    // bootstrap + persist subscriber spawn so the recovery's reducer
    // dispatches operate against fully-populated state, BUT BEFORE
    // the API server starts accepting requests so resumed
    // compensation can't interleave with new sagas.
    //
    // Failure here is non-fatal: the saga log read might be transient,
    // and starting up without recovery beats refusing to start.
    // Operator can still inspect via `--diag sagas` (PR 2 part 2).
    let resumed = crate::boot_timing::time_async(
        "saga_resume_on_startup",
        crate::sagas::recovery::compensate_unresolved(state),
    )
    .await
    .unwrap_or_else(|e| {
        tracing::error!(
            "[saga] resume-on-startup failed: {} — continuing; operator review needed",
            e
        );
        0
    });
    if resumed > 0 {
        tracing::info!(
            "[saga] resume-on-startup compensated {} unresolved saga(s) from prior run",
            resumed
        );
    }
}

/// What agents spend through the model gateway is kept across restarts, and a cap reached is acted on
/// (SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md §3).
fn spawn_spend_services(state: &AppState) {
    backend::model_gateway::spend::attach(Arc::clone(&state.mstore));
    crate::server::spend_watch::start(state.clone());
}

/// Background-task rows end when their command exits, whose process is gone, or that nothing has reported on for a day
/// (SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md §5).
fn spawn_background_task_sweep(state: &AppState) {
    backend::background_task_exit::install(Arc::clone(&state.broker), Arc::clone(&state.mstore));
    backend::background_task_sweep::start(Arc::clone(&state.mstore), Arc::clone(&state.broker));
}

/// Starts the services that run beside the server and need the app state.
/// The panes that watch messages are told as they happen: the Audit when a row is written, the Atlas map when
/// a message goes between two agents.
fn install_message_brokers(state: &AppState) {
    backend::audit_live::install_broker(Arc::clone(&state.broker));
    backend::message_flow::install(Arc::clone(&state.broker));
    backend::task_flow::install(Arc::clone(&state.broker));
}

pub fn spawn_state_services(state: &AppState) {
    // Keeps the host's copy of which panes agents own current (for its popup
    // decisions; docs/specs/SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §3).
    crate::server::browser_host_sync::spawn(state.clone());
    // Every agent's runtime state is looked at and each change published (`agentruntime:changed:<id>`).
    crate::server::agent_runtime::spawn(state.clone());
    spawn_spend_services(state);
    spawn_background_task_sweep(state);
    // Overlap notes (an agent editing a file another agent is also changing
    // is told who): the progress watcher's edits are checked from here on.
    backend::work_facts::overlap_notes::install(
        Arc::clone(&state.mstore),
        Arc::clone(&state.work_store),
        Arc::clone(&state.config_watcher),
        state.http_client.clone(),
        state.local_web_url.clone(),
    );
    // The Kanban: idle agents are reminded of the tasks waiting for them,
    // and a PR opened on a task is linked to it (SPEC_WARDEN_KANBAN_2026_10_20.md).
    backend::kanban::install(
        Arc::clone(&state.mstore),
        Arc::clone(&state.work_store),
        Arc::clone(&state.config_watcher),
        state.broker.clone(),
        state.http_client.clone(),
    );
    tokio::spawn(backend::kanban::run_reminder_loop());
    // A Vault entry's expiry date is reminded of, and its rotation filed as a task
    // (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.2).
    tokio::spawn(backend::vault::expiry::run(
        Arc::clone(&state.identity_store),
        Arc::clone(&state.work_store),
        state.broker.clone(),
    ));
    // HTTP sign-ins the keychain still holds move into the Vault, under Personal (same spec §5.4, §9.3).
    let vault = Arc::clone(&state.identity_store);
    tokio::task::spawn_blocking(move || backend::vault::signins::move_from_keychain(&vault));
    // An agent out of quota is woken at its reset (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §7).
    backend::limit_wake::install(
        Arc::clone(&state.mstore),
        Arc::clone(&state.work_store),
        Arc::clone(&state.event_bus),
    );
    tokio::spawn(backend::kanban::prs::run_merge_loop());
    // An agent's AgentMux MCP server that didn't connect is noticed and
    // repaired (`agent:mcpselfheal`).
    backend::mcp_health::install(Arc::clone(&state.config_watcher));
    // And one whose process is alive but has stopped answering, noticed by its
    // heartbeat going quiet (#2614).
    backend::mcp_health::spawn_watch(
        tokio_util::sync::CancellationToken::new(),
        |block_id, process, wedge| {
            use backend::blockcontroller::persistent::PersistentSubprocessController;
            if let Some(ctrl) = backend::blockcontroller::get_controller(block_id) {
                if let Some(ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<PersistentSubprocessController>()
                {
                    ctrl.mcp_wedged(process, wedge);
                }
            }
        },
    );
    backend::transcript::append::install_config(Arc::clone(&state.config_watcher));
    // Claude config dirs get the auto-compact window `agent:compactat` asks for.
    backend::providers::install_config(Arc::clone(&state.config_watcher));
    backend::context_reading::install_config(Arc::clone(&state.config_watcher));
    backend::container::limits::install_config(Arc::clone(&state.config_watcher));
    // The Claude CLI's own cross-session messaging is off unless `agent:climessaging` is true.
    backend::cli_messaging::install_config(Arc::clone(&state.config_watcher));
    // The Supervisor's switch and limits are read live from the settings.
    backend::supervisor::settings::install_config(Arc::clone(&state.config_watcher));
    backend::transcript::framer::install_config(Arc::clone(&state.config_watcher));
    backend::transcript::damage::install_broker(Arc::clone(&state.broker));
    install_message_brokers(state);
    crate::identity::accounts_changed::install_broker(Arc::clone(&state.broker));
    // The open workspace saves itself as it changes (SPEC_WORKSPACES_2026_10_23.md §4.1).
    crate::server::service::user_workspace::spawn_saver(Arc::clone(&state.mstore));
    // One notice per account its agents are out of quota or signed out on
    // (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6).
    crate::server::account_episodes::install(state.clone());
    // Fleet Global Memory: entries scoped to the fleet are sent to the machines on the LAN that are behind.
    crate::server::remote_ops::fleet_sync::install(state.clone());
    // Agents' cargo caches are cleared as tasks finish, and the disk is watched
    // (SPEC_BUILD_CACHE_CLEANUP_2026_10_26.md).
    backend::build_cache_service::install(
        Arc::clone(&state.mstore),
        Arc::clone(&state.config_watcher),
    );
    // The version folders and unpacked portables an upgrade leaves behind are listed daily, and cleared on a host that turns it on.
    backend::old_versions_service::install(Arc::clone(&state.config_watcher));
    // A leader hears from its workers (SPEC_AGENT_WORKERS_2026_10_15.md §3.3).
    backend::worker_notes::install(
        Arc::clone(&state.broker),
        state.srv_events_tx.subscribe(),
        Arc::clone(&state.mstore),
        Arc::clone(&state.filestore),
        Arc::clone(&state.config_watcher),
    );
}

/// Starts what is read from the data folder once the stores and watchers exist: widget
/// packages, job templates and the Vault's secret-file folder.
pub fn start_data_home_services(config: &config::Config, bg: &BackgroundSubsystems) {
    // Widget packages: scan ~/.agentmux/widgets, apply this instance's
    // approvals, merge their widget-bar entries, watch the folder
    // (SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.1). After the user's
    // widgets.json loaded above, whose v1 module entries it lists too.
    backend::widget::packages::start(
        &config.data_home,
        config.auth_key.clone(),
        bg.fs_watch_pool.clone(),
        bg.config_watcher.clone(),
        bg.event_bus.clone(),
        bg.broker.clone(),
    );
    // Job templates: this instance's approvals and pinned publishers, beside
    // the widgets' (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4).
    backend::job_templates::start(&config.data_home);
    // Where a command's Vault secret files go, emptied of any a crash left
    // (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.3).
    backend::vault::files::init(&config.data_home);
}

/// Tells the launcher srv is ready (the pipe on Windows, then `AGENTMUXSRV-ESTART`), logs the
/// boot summary and queues the saga log's pruning.
pub fn announce_ready(
    state: &AppState,
    net: &NetworkBundle,
    reducer: &ReducerPlumbing,
    config: &config::Config,
    version: &str,
    build_time: &str,
) {
    #[cfg(not(target_os = "windows"))]
    let _ = reducer;
    // Phase E.1b — srv pipe IPC server (Windows only; conditional on
    // AGENTMUX_SRV_PIPE_PATH). Bind happens BEFORE the AGENTMUXSRV-ESTART
    // line so the launcher knows the pipe is ready when host starts.
    #[cfg(target_os = "windows")]
    bind_srv_pipe_ipc(
        version,
        Arc::clone(&state.srv_state),
        state.srv_events_tx.clone(),
        Arc::clone(&reducer.srv_event_log),
    );

    // 6. Emit AGENTMUXSRV-ESTART on stderr (exact format from cmd/server/main-server.go:617)
    emit_estart(
        net.ws_addr.port(),
        net.web_addr.port(),
        version,
        build_time,
        &config.instance_id,
    );

    crate::boot_timing::log_summary();

    // After ESTART, off the boot path: drop sagas that finished cleanly over
    // 30 days ago, so sagas.db stops growing.
    let saga_log = Arc::clone(&state.saga_log);
    tokio::task::spawn_blocking(move || {
        match saga_log.prune_finished_older_than(std::time::Duration::from_secs(30 * 24 * 60 * 60))
        {
            Ok(0) => {}
            Ok(n) => tracing::info!("[saga] pruned {n} saga(s) that finished over 30 days ago"),
            Err(e) => tracing::warn!("[saga] pruning finished sagas failed: {e}"),
        }
    });
}

/// Hands the LAN listeners their routers and starts the dev proxy.
pub fn attach_listeners(
    state: &AppState,
    net: &NetworkBundle,
    lan_router: axum::Router,
    viewer_router: axum::Router,
) {
    // Native dev-proxy (docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md)
    // — a small standalone HTTP server on its own fixed port, separate from
    // the web/ws listeners above (Host-header routing to container-internal
    // addresses, not part of AgentMux's own API surface). Detached: a bind
    // failure or later error is logged inside `serve` itself and never
    // fatal to srv startup, same posture as `lan_listeners`' reconcile loop.
    tokio::spawn(backend::dev_proxy::serve(state.dev_proxy.clone()));

    // Hand the LAN listener supervisor its router now that one exists —
    // `build_routers` consumes `AppState`, which owns the supervisor, so this
    // cannot happen at construction time. Then honor the current setting and
    // start the self-healing sweep, which also picks up interface changes
    // (DHCP renewal, Wi-Fi↔Ethernet handoff, VPN up/down) without a restart.
    // See `backend::lan::listeners`.
    net.lan_listeners.set_router(lan_router);
    // The viewer listener (backend::viewer) comes up beside the LAN
    // listeners, on its own port of the same range.
    net.lan_listeners.set_viewer(
        viewer_router,
        Arc::clone(&state.viewer),
        backend::lan::ports::LAN_PORT_RANGE,
    );
    net.lan_listeners
        .apply(state.config_watcher.get_settings().network_lan_discovery);
    Arc::clone(&net.lan_listeners).spawn_reconcile_loop();
}

/// Starts the loops that run for as long as the servers do and stop with `stdin_token`.
pub fn spawn_serving_loops(
    state: &AppState,
    headless: bool,
    stdin_token: &tokio_util::sync::CancellationToken,
) {
    let fleet_source = backend::fleet_source::FleetSource::live(
        state.reactive_handler,
        Arc::clone(&state.mstore),
        Arc::clone(&state.config_watcher),
        state.local_web_url.clone(),
    );
    // Periodic WAL checkpoint — prevents unbounded WAL file growth during
    // long-running sessions.
    spawn_wal_checkpoint_loop(
        stdin_token.clone(),
        Arc::clone(&state.mstore),
        Arc::clone(&state.filestore),
    );
    spawn_global_transcripts_truncate_loop(
        stdin_token.clone(),
        state.global_transcript_store.clone(),
    );

    // The fleet feed's change detection (`backend::fleet_feed`): compares the
    // reachable agents, their kinds and states and this machine's channel
    // count once a second, stops with the same token.
    Arc::clone(&state.fleet_feed)
        .spawn_change_detection(move || fleet_source.observe(), stdin_token.clone());

    // The cloud presence record, published from the fleet feed's snapshot
    // while signed in (`muxbus::wan_presence`). Here rather than beside
    // `wan_publish::spawn` because the feed exists only once AppState does.
    // A headless srv doesn't publish unless forced (`wan_presence::policy`).
    crate::muxbus::wan_presence::spawn(
        Arc::clone(&state.fleet_feed),
        Arc::clone(&state.id_store),
        Arc::clone(&state.config_watcher),
        headless,
        stdin_token.clone(),
    );
}

/// What runs once the servers have stopped: this channel leaves the LAN, then every agent closes.
pub async fn shut_down(
    state: &AppState,
    lan_discovery: &backend::lan::discovery::LanDiscoveryController,
) {
    // This host's other LAN channels stop listing this one as a sibling.
    lan_discovery.withdraw_instance_record();
    crate::headless::withdraw_port_file();

    // Shutdown cleanup: close every agent through the one teardown
    // (SPEC_AGENT_TEARDOWN_SINGLE_PATH_2026_10_01.md, `Policy::app_exit()`),
    // so each gets to end its turn and exit and nothing it started (`task dev`
    // → task.exe/node, `Shell()` sessions) orphans on srv exit. Capped at
    // `agent_teardown::APP_EXIT_CAP`; the launcher's backstop takes whatever
    // is left. Beside it, the cloud presence goodbye (at most 2 s, so it
    // never outlasts the teardown's own cap): devices show this computer as
    // offline at once instead of minutes later.
    let (closed, ()) = tokio::join!(
        crate::sagas::agent_teardown::app_exit(state),
        crate::muxbus::wan_presence::goodbye_on_shutdown(),
    );
    tracing::info!(agents = closed, "shutdown: agents closed");
    #[cfg(unix)]
    backend::process_tracker::roots_file::finish();
}

/// Opens every store and starts the background subsystems that need no app state, then what
/// is read from the data folder.
pub fn open_stores_and_background(
    config: &config::Config,
    version: &str,
    build_time: &str,
) -> (Stores, BackgroundSubsystems) {
    // 4. Initialize backend: data dir, in-process migrations, open every store,
    //    attach shared/global registries, run one-time seed/repair passes.
    let stores = crate::boot_timing::time("open_stores_and_migrate", || {
        open_stores_and_migrate(config, version, build_time)
    });

    // Event infrastructure + every background task that doesn't need AppState yet.
    let bg = spawn_background_subsystems(
        &stores.mstore,
        &stores.filestore,
        &stores.id_store,
        &stores.identity_store,
        &stores.work_store,
    );

    start_data_home_services(config, &bg);
    (stores, bg)
}

/// Serves the web and WebSocket listeners until one fails or `stdin_token` is cancelled.
pub async fn serve_until_shutdown(
    web_listener: TcpListener,
    ws_listener: TcpListener,
    router: axum::Router,
    stdin_token: &tokio_util::sync::CancellationToken,
) {
    let web_server = axum::serve(web_listener, router.clone());
    let ws_server = axum::serve(ws_listener, router);

    // Run both servers until shutdown
    tokio::select! {
        result = web_server.into_future() => {
            if let Err(e) = result {
                tracing::error!("web server error: {}", e);
            }
        }
        result = ws_server.into_future() => {
            if let Err(e) = result {
                tracing::error!("ws server error: {}", e);
            }
        }
        _ = stdin_token.cancelled() => {
            tracing::info!("shutdown signal received, exiting");
        }
    }
}
