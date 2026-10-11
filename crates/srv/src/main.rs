// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

mod adoption;
mod agents;
mod ambient;
mod backend;
mod boot_timing;
mod bootstrap;
mod broker;
mod config;
mod event_log;
mod headless;
mod identity;
mod migrations;
mod persist;
mod persist_subscriber;
mod proc_name;
mod reducer;
mod registry;
mod sagas;
mod server;
mod srv_ipc;
mod srv_info;
mod state;
mod drone;
mod messaging;
mod muxbus;
mod util;
#[cfg(windows)]
mod crash_monitor;
#[cfg(test)]
mod test_support;

use std::future::IntoFuture;
use std::sync::Arc;

use server::build_routers;

#[tokio::main]
async fn main() {
    // -2. Document reading child: srv re-runs itself to parse one attached
    //     document (PDF, Word, Excel, PowerPoint, ODF, RTF) out of process,
    //     since parsers can crash, recurse or run long on hostile input and
    //     the parent kills the child at its deadline
    //     (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §6). Nothing else
    //     may start in this mode.
    {
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some(backend::attachments::extract::CHILD_ARG) {
            proc_name::set(proc_name::DOC_CHILD);
            std::process::exit(backend::attachments::extract::run_child(&args[2..]));
        }
    }
    // -1. Crash monitor branch — must be checked before any other initialization.
    if bootstrap::maybe_run_crash_monitor() {
        return;
    }
    // Linux: a stable name for ps/top instead of the versioned file name's
    // first 15 bytes. This runs on the main thread (tokio's block_on).
    proc_name::set(proc_name::MAIN);

    // Before anything changes the environment (srv_info.rs).
    srv_info::capture_home_dir();

    // 0. Headless (no launcher/host): prepare the env the launcher would have
    //    provided, and don't tie srv's life to a parent or stdin. Otherwise start
    //    the parent process watcher BEFORE tokio runtime does real work
    //    (Linux/macOS only). docs/specs/SPEC_SRV_HEADLESS_MODE_2026_09_26.md.
    let headless = headless::requested();
    if headless {
        match headless::prepare_env() {
            Ok(Some(key_file)) => eprintln!("agentmux-srv --headless: auth key written to {}", key_file.display()),
            Ok(None) => {}
            Err(e) => {
                eprintln!("agentmux-srv --headless: {e}");
                std::process::exit(1);
            }
        }
    } else {
        bootstrap::install_process_watchers();
    }

    // 0b. Attach out-of-process crash dump handler (Windows only).
    //     _crash_guard must stay alive — dropping it uninstalls the VEH handler.
    #[cfg(windows)]
    let _crash_guard = bootstrap::install_crash_guard();

    // 0c. Anchor the uptime clock before anything else can take time. Must be
    //     neither a wall-clock stamp (moves backwards on a clock step) nor an
    //     `Instant` (stops while suspended) — see `suspend_aware_now_ms` in
    //     `backend::sysinfo` for both bugs and the per-platform sources.
    backend::sysinfo::mark_process_start();

    // 1. Init tracing (stderr + rolling file)
    let _log_guard = bootstrap::init_logging();

    // 1a. Start the boot-path stopwatch. Logged as one summary line at
    //     ESTART below — see `boot_timing`'s module doc for why these numbers
    //     go to the log rather than the splash.
    boot_timing::init();

    // 1b. Direct-launch PATH fallback.
    bootstrap::enrich_path();

    // 2. Parse CLI args and build config (dispatches `migrate` subcommand internally).
    let config = bootstrap::load_config();
    let version = config.version.to_string();
    let build_time = config.build_time.to_string();

    // 4. Initialize backend: data dir, in-process migrations, open every store,
    //    attach shared/global registries, run one-time seed/repair passes.
    let stores = boot_timing::time("open_stores_and_migrate", || {
        bootstrap::open_stores_and_migrate(&config, &version, &build_time)
    });

    // Event infrastructure + every background task that doesn't need AppState yet.
    let bg = bootstrap::spawn_background_subsystems(
        &stores.mstore,
        &stores.filestore,
        &stores.id_store,
        &stores.identity_store,
    );

    // Widget packages: scan ~/.agentmux/widgets, apply this instance's
    // approvals, merge their widget-bar entries, watch the folder
    // (SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.1). After the user's
    // widgets.json loaded above, whose v1 module entries it lists too.
    backend::widget_packages::start(
        &config.data_home,
        config.auth_key.clone(),
        bg.fs_watch_pool.clone(),
        bg.config_watcher.clone(),
        bg.event_bus.clone(),
        bg.broker.clone(),
    );

    // 5. Bind TCP listeners, bring up LAN discovery / LSP supervisor / process tracker.
    let net = boot_timing::time_async(
        "bind_listeners_and_network",
        bootstrap::bind_listeners_and_network(&config, &bg.event_bus, &bg.broker, &version),
    )
    .await;

    // Phase E.2 — srv reducer plumbing (state, event bus, event log, persist subscriber).
    let reducer = bootstrap::spawn_reducer_plumbing(&stores.mstore, &bg.event_bus, &bg.subagent_watcher).await;

    let state = bootstrap::build_app_state(&config, version.clone(), stores, bg, &net, &reducer);

    // Upgrade reactive message delivery now that AppState exists, so agents on a
    // SubprocessController (every container agent, plus codex/gemini/qwen/kimi/
    // muxcode/antigravity on the host) can actually receive inter-agent
    // messages instead of having them dropped on a PTY fallback they reject.
    bootstrap::install_agent_turn_delivery(&state);

    // Cron fires through the same in-process inject path (identity M4c-3);
    // before `cron_scheduler.start()` below, so no fire takes the HTTP
    // fallback.
    bootstrap::install_cron_delivery(&state);

    // Durable jekt: replay messages held for an agent that was not running
    // (SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md).
    server::jekt_held::install(&state);

    // The helper install asks first (SPEC_REMOTES_PANE_2026_10_05.md §4.9).
    server::app_api::connections::install_helper_consent(&state);

    // Now that AppState exists, wire up close-on-exit so a shell pane can
    // actually close itself when its process exits — see
    // `bootstrap::install_close_on_exit_handler`'s doc comment.
    bootstrap::install_close_on_exit_handler(&state);

    // Backstop for agent blocks left in a tab but in no pane — see
    // `sagas::orphan_reaper`'s module doc.
    sagas::orphan_reaper::install(&state);

    // Out-of-band native-memory write detection (fast fs-watch path + slow
    // reconciliation-sweep path) — see
    // docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.5.
    backend::native_memory_drift::spawn(
        state.fs_watch_pool.clone(),
        state.mstore.clone(),
        state.id_store.clone(),
        state.broker.clone(),
    );

    // Retention/GC for native-memory version history — see
    // docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §7.1.
    backend::native_memory_retention::spawn(state.id_store.clone());

    // Start persistent cron scheduler — load enabled jobs from DB, fire any
    // that missed their window (FIRE_ONCE_NOW), schedule all for future fires.
    state.cron_scheduler.start().await;

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
    let resumed = boot_timing::time_async(
        "saga_resume_on_startup",
        sagas::recovery::compensate_unresolved(&state),
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

    // Phase E.1b — srv pipe IPC server (Windows only; conditional on
    // AGENTMUX_SRV_PIPE_PATH). Bind happens BEFORE the AGENTMUXSRV-ESTART
    // line so the launcher knows the pipe is ready when host starts.
    #[cfg(target_os = "windows")]
    bootstrap::bind_srv_pipe_ipc(
        &version,
        Arc::clone(&state.srv_state),
        state.srv_events_tx.clone(),
        Arc::clone(&reducer.srv_event_log),
    );

    // 6. Emit AGENTMUXSRV-ESTART on stderr (exact format from cmd/server/main-server.go:617)
    bootstrap::emit_estart(
        net.ws_addr.port(),
        net.web_addr.port(),
        &version,
        &build_time,
        &config.instance_id,
    );

    boot_timing::log_summary();

    // 7. Build router and serve on both listeners
    // Clone Arcs that are needed after `state` is moved into build_routers.
    let state_for_exit = state.clone();
    let wal_mstore = Arc::clone(&state.mstore);
    let wal_filestore = Arc::clone(&state.filestore);
    let config_watcher_for_lan = Arc::clone(&state.config_watcher);
    let dev_proxy_registry = state.dev_proxy.clone();
    let fleet_feed = Arc::clone(&state.fleet_feed);
    let fleet_source = backend::fleet_source::FleetSource::live(
        state.reactive_handler,
        Arc::clone(&state.mstore),
        state.local_web_url.clone(),
    );
    let presence_id_store = Arc::clone(&state.id_store);
    let presence_config = Arc::clone(&state.config_watcher);
    let viewer = Arc::clone(&state.viewer);
    let viewer_router = server::build_viewer_router(state.clone());
    // Keeps the host's copy of which panes agents own current (for its popup
    // decisions; docs/specs/SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §3).
    server::browser_host_sync::spawn(state.clone());
    // Overlap notes (an agent editing a file another agent is also changing
    // is told who): the progress watcher's edits are checked from here on.
    backend::work_facts::overlap_notes::install(
        Arc::clone(&state.mstore),
        Arc::clone(&state.identity_store),
        Arc::clone(&state.config_watcher),
        state.http_client.clone(),
        state.local_web_url.clone(),
    );
    let routers = build_routers(state);
    let router = routers.full;

    // Native dev-proxy (docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md)
    // — a small standalone HTTP server on its own fixed port, separate from
    // the web/ws listeners above (Host-header routing to container-internal
    // addresses, not part of AgentMux's own API surface). Detached: a bind
    // failure or later error is logged inside `serve` itself and never
    // fatal to srv startup, same posture as `lan_listeners`' reconcile loop.
    tokio::spawn(backend::dev_proxy::serve(dev_proxy_registry));

    // Hand the LAN listener supervisor its router now that one exists —
    // `build_routers` consumes `AppState`, which owns the supervisor, so this
    // cannot happen at construction time. Then honor the current setting and
    // start the self-healing sweep, which also picks up interface changes
    // (DHCP renewal, Wi-Fi↔Ethernet handoff, VPN up/down) without a restart.
    // See `backend::lan_listeners`.
    net.lan_listeners.set_router(routers.lan);
    // The viewer listener (backend::viewer) comes up beside the LAN
    // listeners, on its own port of the same range.
    net.lan_listeners
        .set_viewer(viewer_router, viewer, backend::lan_ports::LAN_PORT_RANGE);
    net.lan_listeners
        .apply(config_watcher_for_lan.get_settings().network_lan_discovery);
    Arc::clone(&net.lan_listeners).spawn_reconcile_loop();

    let web_server = axum::serve(net.web_listener, router.clone());
    let ws_server = axum::serve(net.ws_listener, router);

    // 8 & 9. Spawn stdin watch thread + SIGINT/SIGTERM handler (graceful shutdown).
    let stdin_token = bootstrap::install_shutdown_handlers(!headless);

    // Periodic WAL checkpoint — prevents unbounded WAL file growth during
    // long-running sessions.
    bootstrap::spawn_wal_checkpoint_loop(stdin_token.clone(), wal_mstore, wal_filestore);

    // The fleet feed's change detection (`backend::fleet_feed`): compares the
    // reachable agents, their kinds and states and this machine's channel
    // count once a second, stops with the same token.
    Arc::clone(&fleet_feed).spawn_change_detection(move || fleet_source.observe(), stdin_token.clone());

    // The cloud presence record, published from the fleet feed's snapshot
    // while signed in (`muxbus::wan_presence`). Here rather than beside
    // `wan_publish::spawn` because the feed exists only once AppState does.
    // A headless srv doesn't publish unless forced (`wan_presence::policy`).
    muxbus::wan_presence::spawn(fleet_feed, presence_id_store, presence_config, headless, stdin_token.clone());

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

    // This host's other LAN channels stop listing this one as a sibling.
    net.lan_discovery.withdraw_instance_record();

    // Shutdown cleanup: close every agent through the one teardown
    // (SPEC_AGENT_TEARDOWN_SINGLE_PATH_2026_10_01.md, `Policy::app_exit()`),
    // so each gets to end its turn and exit and nothing it started (`task dev`
    // → task.exe/node, `Shell()` sessions) orphans on srv exit. Capped at
    // `agent_teardown::APP_EXIT_CAP`; the launcher's backstop takes whatever
    // is left. Beside it, the cloud presence goodbye (at most 2 s, so it
    // never outlasts the teardown's own cap): devices show this computer as
    // offline at once instead of minutes later.
    let (closed, ()) = tokio::join!(
        sagas::agent_teardown::app_exit(&state_for_exit),
        muxbus::wan_presence::goodbye_on_shutdown(),
    );
    tracing::info!(agents = closed, "shutdown: agents closed");
}
