// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

mod agents;
mod ambient;
mod backend;
mod bootstrap;
mod broker;
mod config;
mod event_log;
mod identity;
mod migrations;
mod persist;
mod persist_subscriber;
mod reducer;
mod registry;
mod sagas;
mod server;
mod srv_ipc;
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

use server::build_router;

#[tokio::main]
async fn main() {
    // -1. Crash monitor branch — must be checked before any other initialization.
    if bootstrap::maybe_run_crash_monitor() {
        return;
    }

    // 0. Start parent process watcher BEFORE tokio runtime does real work (Linux/macOS only).
    bootstrap::install_process_watchers();

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

    // 1b. Direct-launch PATH fallback.
    bootstrap::enrich_path();

    // 2. Parse CLI args and build config (dispatches `migrate` subcommand internally).
    let config = bootstrap::load_config();
    let version = config.version.to_string();
    let build_time = config.build_time.to_string();

    // 4. Initialize backend: data dir, in-process migrations, open every store,
    //    attach shared/global registries, run one-time seed/repair passes.
    let stores = bootstrap::open_stores_and_migrate(&config, &version, &build_time);

    // Event infrastructure + every background task that doesn't need AppState yet.
    let bg = bootstrap::spawn_background_subsystems(&stores.wstore, &stores.filestore, &stores.id_store);

    // 5. Bind TCP listeners, bring up LAN discovery / LSP supervisor / process tracker.
    let net = bootstrap::bind_listeners_and_network(
        &config,
        &bg.config_watcher,
        &bg.event_bus,
        &bg.broker,
        &version,
    )
    .await;

    // Phase E.2 — srv reducer plumbing (state, event bus, event log, persist subscriber).
    let reducer = bootstrap::spawn_reducer_plumbing(&stores.wstore, &bg.event_bus, &bg.subagent_watcher).await;

    let state = bootstrap::build_app_state(&config, version.clone(), stores, bg, &net, &reducer);

    // Upgrade reactive message delivery now that AppState exists, so agents on a
    // SubprocessController (every container agent, plus codex/gemini/qwen/kimi/
    // muxcode/antigravity on the host) can actually receive inter-agent
    // messages instead of having them dropped on a PTY fallback they reject.
    bootstrap::install_agent_turn_delivery(&state);

    // Out-of-band native-memory write detection (fast fs-watch path + slow
    // reconciliation-sweep path) — see
    // docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.5.
    backend::native_memory_drift::spawn(
        state.fs_watch_pool.clone(),
        state.wstore.clone(),
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
    let resumed = sagas::recovery::compensate_unresolved(&state)
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

    // 7. Build router and serve on both listeners
    // Clone Arcs that are needed after `state` is moved into build_router.
    let shell_sessions_shutdown = state.shell_sessions.clone();
    let wal_wstore = Arc::clone(&state.wstore);
    let wal_filestore = Arc::clone(&state.filestore);
    let router = build_router(state);

    let web_server = axum::serve(net.web_listener, router.clone());
    let ws_server = axum::serve(net.ws_listener, router);

    // 8 & 9. Spawn stdin watch thread + SIGINT/SIGTERM handler (graceful shutdown).
    let stdin_token = bootstrap::install_shutdown_handlers();

    // Periodic WAL checkpoint — prevents unbounded WAL file growth during
    // long-running sessions.
    bootstrap::spawn_wal_checkpoint_loop(stdin_token.clone(), wal_wstore, wal_filestore);

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

    // Shutdown cleanup — tree-kill any persistent shells so long-running
    // children (`task dev` → task.exe/node) don't orphan on srv exit. stop_all()
    // fires each shell's cancel handle; the kill_tasks run taskkill/killpg
    // asynchronously, so give them a brief grace to complete before we exit.
    // (`kill_on_drop` only reaps the wrapper shell and doesn't fire on a clean
    // process exit, so this is the real orphan guard.) [reagent #1422 P2]
    let live = shell_sessions_shutdown.stop_all();
    if live > 0 {
        tracing::info!(count = live, "shutdown: stopping persistent shells");
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }
}
