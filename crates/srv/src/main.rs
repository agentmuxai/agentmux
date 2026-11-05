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
mod connectivity;
mod event_log;
mod headless;
mod identity;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod malloc_arenas;
mod migrations;
mod persist;
mod persist_subscriber;
mod proc_name;
mod reducer;
mod registry;
mod sagas;
mod server;
#[cfg(target_os = "windows")]
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

use server::build_routers;

fn main() {
    // Before anything is spawned or any thread exists: the launcher's secrets leave
    // the environment, so no child inherits them and /proc/<pid>/environ shows none.
    agentmux_common::secret_env::stash_startup_secrets();
    // Linux (glibc): cap malloc arenas before any thread exists, so the runtime's
    // workers below share them (malloc_arenas.rs).
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    malloc_arenas::apply();
    // What a bare `#[tokio::main]` expands to (tokio-macros 2.7: the multi-thread
    // flavor, `enable_all`, no worker count, thread name or other setting, and the
    // same panic message), built by hand so the call above runs first: the runtime
    // starts its worker threads when it is built. This is on EVERY platform.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed building the Runtime")
        .block_on(app_main());
}

/// Runs the document-reading or file-icon child srv was started as, and exits; returns when
/// this is an ordinary start.
fn exit_if_helper_child() {
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
    // -2b. File icon child: srv re-runs itself to ask the operating system for a file type's
    //      icon, since the shell loads third-party code that can hang or crash; the parent
    //      kills it at its deadline (SPEC_FILES_SYSTEM_FILE_ICONS_2026_10_27.md §4.2).
    {
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some(backend::file_icons::CHILD_ARG) {
            proc_name::set(proc_name::ICON_CHILD);
            std::process::exit(backend::file_icons::child::run_child());
        }
    }
}

/// Prepares the process for headless or launcher-owned running; true when headless.
fn prepare_process_mode() -> bool {
    let headless = headless::requested();
    agentmux_common::process_hardening::deny_memory_access(headless);
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
    headless
}

async fn app_main() {
    exit_if_helper_child();
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
    let headless = prepare_process_mode();

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

    let (stores, bg) = bootstrap::open_stores_and_background(&config, &version, &build_time);

    // 5. Bind TCP listeners, bring up LAN discovery / LSP supervisor / process tracker.
    let net = boot_timing::time_async(
        "bind_listeners_and_network",
        bootstrap::bind_listeners_and_network(&config, &bg.event_bus, &bg.broker, &version),
    )
    .await;

    // Phase E.2 — srv reducer plumbing (state, event bus, event log, persist subscriber).
    let reducer = bootstrap::spawn_reducer_plumbing(&stores.mstore, &bg.event_bus, &bg.subagent_watcher).await;

    let state = bootstrap::build_app_state(&config, version.clone(), stores, bg, &net, &reducer);

    bootstrap::install_state_hooks(&state);
    bootstrap::start_job_ledger_and_cron(&state).await;
    bootstrap::resume_unresolved_sagas(&state).await;
    bootstrap::announce_ready(&state, &net, &reducer, &config, &version, &build_time);

    let viewer_router = server::build_viewer_router(state.clone());
    bootstrap::spawn_state_services(&state);
    let routers = build_routers(state.clone());
    backend::container::listener::set_router(routers.container);
    let router = routers.full;

    bootstrap::attach_listeners(&state, &net, routers.lan, viewer_router);

    // 8 & 9. Spawn stdin watch thread + SIGINT/SIGTERM handler (graceful shutdown).
    let stdin_token = bootstrap::install_shutdown_handlers(!headless);

    bootstrap::spawn_serving_loops(&state, headless, &stdin_token);

    bootstrap::serve_until_shutdown(net.web_listener, net.ws_listener, router, &stdin_token).await;

    bootstrap::shut_down(&state, &net.lan_discovery).await;
}

// This test binary must hold this checkout's code, not another clone's from
// a shared target directory (`agentmux_common::checkout`).
#[cfg(test)]
agentmux_common::checkout_guard!(agentmux_remote::CHECKOUT_DIR, agentmux_procstats::CHECKOUT_DIR);
