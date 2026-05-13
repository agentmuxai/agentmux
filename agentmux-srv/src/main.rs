mod agents;
mod backend;
mod config;
mod event_log;
mod identity;
mod persist;
mod persist_subscriber;
mod reducer;
mod registry;
mod sagas;
mod server;
mod srv_ipc;
mod state;
mod workflows;
#[cfg(windows)]
mod crash_monitor;

use std::future::IntoFuture;
use std::sync::Arc;

use clap::Parser;
use config::CliArgs;
use server::{AppState, build_router};
use tokio::net::TcpListener;
use tokio::signal;

use backend::eventbus::EventBus;
use backend::reactive::{self, Poller, PollerConfig};
use backend::storage::filestore::FileStore;
use backend::storage::wstore::WaveStore;
use backend::wps::Broker;
use backend::wconfig;
use backend::{docsite, sysinfo, base, wcore};

/// Start a ppid polling watchdog on Linux/macOS.
/// If the parent process dies, getppid() changes (reparented to init/launchd).
/// This is safer than PR_SET_PDEATHSIG which tracks the parent *thread*, not process,
/// and can fire spuriously with async runtimes like Tokio.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn start_ppid_watchdog() {
    let original_ppid = unsafe { libc::getppid() };
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let current_ppid = unsafe { libc::getppid() };
            if current_ppid != original_ppid {
                eprintln!(
                    "parent process died (ppid changed {} -> {}), shutting down",
                    original_ppid, current_ppid
                );
                std::process::exit(0);
            }
        }
    });
}

/// Event-driven parent process watcher using kqueue (macOS) or pidfd (Linux).
/// Monitors a specific PID and exits when that process terminates.
/// Falls back to PPID polling on older Linux kernels without pidfd support.
#[cfg(target_os = "macos")]
fn start_parent_watcher(parent_pid: u32) {
    std::thread::spawn(move || {
        unsafe {
            let kq = libc::kqueue();
            if kq < 0 {
                eprintln!(
                    "kqueue() failed (errno={}), falling back to ppid watchdog",
                    *libc::__error()
                );
                let _ = kq;
                start_ppid_watchdog();
                return;
            }

            // Register EVFILT_PROC + NOTE_EXIT on the parent PID.
            let mut changelist: [libc::kevent; 1] = std::mem::zeroed();
            changelist[0] = libc::kevent {
                ident: parent_pid as usize,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ONESHOT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            };

            let ret = libc::kevent(
                kq,
                changelist.as_ptr(),
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            );

            if ret < 0 {
                let errno = *libc::__error();
                libc::close(kq);
                if errno == libc::ESRCH {
                    // Parent already dead
                    eprintln!(
                        "parent process {} already exited (ESRCH during kqueue registration), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                eprintln!(
                    "kevent() registration failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog();
                return;
            }

            eprintln!("kqueue EVFILT_PROC registered for parent pid {}", parent_pid);

            // Race condition guard: check if the parent is still alive after registering.
            // If it died between our registration and this check, we might miss the event.
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__error() == libc::ESRCH {
                libc::close(kq);
                eprintln!(
                    "parent process {} already exited (post-registration check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            // Block until the parent exits.
            let mut eventlist: [libc::kevent; 1] = std::mem::zeroed();
            let n = libc::kevent(
                kq,
                std::ptr::null(),
                0,
                eventlist.as_mut_ptr(),
                1,
                std::ptr::null(),
            );
            libc::close(kq);

            if n > 0 {
                eprintln!(
                    "parent process {} exited (kqueue EVFILT_PROC), shutting down",
                    parent_pid
                );
            } else {
                eprintln!(
                    "kevent() wait returned {} (errno={}), shutting down",
                    n,
                    *libc::__error()
                );
            }
            std::process::exit(0);
        }
    });
}

/// Event-driven parent process watcher using pidfd_open (Linux 5.3+).
/// Falls back to PPID polling on older kernels without pidfd support.
#[cfg(target_os = "linux")]
fn start_parent_watcher(parent_pid: u32) {
    std::thread::spawn(move || {
        unsafe {
            // Try pidfd_open (syscall 434 on x86_64, 434 on aarch64)
            let pidfd = libc::syscall(libc::SYS_pidfd_open, parent_pid as libc::c_int, 0 as libc::c_int);

            if pidfd < 0 {
                let errno = *libc::__errno_location();
                if errno == libc::ESRCH {
                    // Parent already dead
                    eprintln!(
                        "parent process {} already exited (ESRCH from pidfd_open), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                // ENOSYS means kernel doesn't support pidfd_open — fall back
                eprintln!(
                    "pidfd_open() failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog();
                return;
            }

            let pidfd = pidfd as libc::c_int;

            // Race condition guard: verify parent is still alive
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__errno_location() == libc::ESRCH {
                libc::close(pidfd);
                eprintln!(
                    "parent process {} already exited (post-pidfd check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            // poll() on the pidfd — blocks until the process exits
            let mut pfd = libc::pollfd {
                fd: pidfd,
                events: libc::POLLIN,
                revents: 0,
            };

            let ret = libc::poll(&mut pfd, 1, -1); // infinite timeout
            libc::close(pidfd);

            if ret > 0 {
                eprintln!(
                    "parent process {} exited (pidfd poll), shutting down",
                    parent_pid
                );
            } else {
                eprintln!(
                    "poll() on pidfd returned {} (errno={}), shutting down",
                    ret,
                    *libc::__errno_location()
                );
            }
            std::process::exit(0);
        }
    });
}

#[tokio::main]
async fn main() {
    // -1. Crash monitor branch — must be checked before any other initialization.
    //     The monitor process runs a blocking minidumper::Server and exits when the
    //     main process disconnects. It does not run any backend logic.
    #[cfg(windows)]
    if std::env::args().any(|a| a == "--crash-monitor") {
        crash_monitor::run_monitor();
        return;
    }

    // 0. Start parent process watcher BEFORE tokio runtime does real work (Linux/macOS only).
    // On Windows, the frontend uses a Job Object with KILL_ON_JOB_CLOSE instead.
    // Uses getppid() to get the parent PID, then kqueue/pidfd to watch it (event-driven,
    // zero CPU). Falls back to PPID polling if kqueue/pidfd setup fails or parent is init/launchd.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let ppid = unsafe { libc::getppid() } as u32;
        if ppid <= 1 {
            // Parent is init/launchd — can't meaningfully watch it, use polling fallback
            start_ppid_watchdog();
        } else {
            start_parent_watcher(ppid);
        }
    }

    // 0b. Attach out-of-process crash dump handler (Windows only).
    //     Spawns self with --crash-monitor and installs a VEH handler.
    //     _crash_guard must stay alive — dropping it uninstalls the VEH handler.
    //     Non-fatal: if the monitor fails to start, the process continues normally
    //     and WER LocalDumps still captures __fastfail crashes independently.
    #[cfg(windows)]
    let _crash_guard = crash_monitor::spawn_and_attach();

    // 1. Init tracing (stderr + rolling file)
    let _log_guard = init_logging();

    // 2. Parse CLI args and build config
    let args = CliArgs::parse();
    let config = config::Config::from_env_and_args(&args).unwrap_or_else(|e| {
        tracing::error!("Failed to load config: {}", e);
        std::process::exit(1);
    });

    let version = config.version.to_string();
    let build_time = config.build_time.to_string();

    // Make the per-launch auth_key available to the cross-instance agent
    // registry writer. Peers performing an HTTP forward of a missed inject
    // use this to authenticate against the writing instance's sidecar.
    // Must happen after Config::from_env_and_args (which removes
    // AGENTMUX_AUTH_KEY from the process env) but before anything calls
    // `agent_registry::write`.
    crate::backend::reactive::registry::init_local_auth_key(&config.auth_key);

    // 4. Initialize backend (matching Go cmd/server/main-server.go:374-590)
    base::set_version(&version);
    base::set_build_time(&build_time);

    // Migrate ~/.waveterm → ~/.agentmux if needed (one-time, non-destructive)
    base::migrate_legacy_data_dir();

    // Set up data directory (uses AGENTMUX_DATA_HOME or default)
    if !config.data_home.is_empty() {
        std::env::set_var("AGENTMUX_DATA_HOME", &config.data_home);
    }
    if !config.config_home.is_empty() {
        std::env::set_var("AGENTMUX_CONFIG_HOME", &config.config_home);
    }
    if !config.app_path.is_empty() {
        std::env::set_var("AGENTMUX_APP_PATH", &config.app_path);
    }

    base::ensure_wave_data_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure data dir: {}", e);
        std::process::exit(1);
    });
    base::ensure_wave_db_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure db dir: {}", e);
        std::process::exit(1);
    });

    // Startup diagnostics
    tracing::info!(
        data_dir = %base::get_wave_data_dir().display(),
        db_dir = %base::get_wave_db_dir().display(),
        app_path = %config.app_path,
        instance_id = %config.instance_id,
        "backend directories initialized"
    );

    // Open databases
    let db_dir = base::get_wave_db_dir();
    let wstore_raw = WaveStore::open(&db_dir.join("objects.db")).unwrap_or_else(|e| {
        tracing::error!("Failed to open object store: {}", e);
        std::process::exit(1);
    });
    // Attach the cross-version named-agent registry. Falls back to a
    // disabled registry when the shared home can't be resolved (CI,
    // unusual envs); mutations still hit SQLite, just don't mirror.
    // See docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md.
    if let Some(root) = registry::resolve_shared_registry_dir() {
        match registry::Registry::open(root.clone()) {
            Ok(reg) => {
                // PR B — one-shot backfill from every per-version
                // objects.db into the registry. Idempotent via marker
                // file in the registry root. Read-only on SQLite.
                //
                // Gating: the registry is only attached to wstore if
                // migration completes (Ok) — that way the read path
                // never serves a partial view. On Err, mirror writes
                // are also disabled (registry stays detached); SQLite
                // remains authoritative and the next launch retries
                // the migration via the same marker logic.
                let shared_home = root
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf());
                let migration_ok = match shared_home {
                    Some(home) => match registry::migrate_from_sqlite_once(&home, &reg) {
                        Ok(stats) => {
                            if stats.versions_scanned > 0 || stats.records_written > 0 {
                                tracing::info!(
                                    versions_scanned = stats.versions_scanned,
                                    rows_seen = stats.rows_seen,
                                    records_written = stats.records_written,
                                    records_skipped_existing = stats.records_skipped_existing,
                                    records_skipped_unmappable = stats.records_skipped_unmappable,
                                    complete = stats.complete,
                                    "registry: one-shot SQLite migration finished"
                                );
                            }
                            // Gate attach on `complete` — partial
                            // migration leaves the registry detached
                            // so the read path serves SQLite (full,
                            // current-version-only view) rather than
                            // a half-populated registry.
                            stats.complete
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "registry: SQLite migration errored — leaving registry detached; SQLite stays authoritative, next launch retries"
                            );
                            false
                        }
                    },
                    None => {
                        tracing::warn!(
                            root = %root.display(),
                            "registry: cannot resolve shared home (root has fewer than 2 ancestors) — leaving registry detached"
                        );
                        false
                    }
                };
                if migration_ok {
                    tracing::info!(root = %root.display(), "registry: shared agent registry attached");
                    wstore_raw.set_registry(Arc::new(reg));
                }
            }
            Err(e) => tracing::warn!(
                root = %root.display(),
                error = %e,
                "registry: failed to open shared agent registry — SQLite remains authoritative"
            ),
        }
    } else {
        tracing::warn!("registry: could not resolve shared registry dir — mirror disabled");
    }
    let wstore = Arc::new(wstore_raw);
    let filestore = Arc::new(FileStore::open(&db_dir.join("filestore.db")).unwrap_or_else(|e| {
        tracing::error!("Failed to open file store: {}", e);
        std::process::exit(1);
    }));
    // Saga durability — see SPEC_SAGA_DURABILITY_2026-05-01.md.
    // Backed by its own SQLite file (`sagas.db`) so saga writes
    // commit independently of the wstore connection. Failure here
    // is fatal: without the log, a srv crash mid-saga leaves
    // unrecoverable state divergence.
    let saga_log = Arc::new(
        crate::sagas::log::SagaLog::open(&db_dir.join("sagas.db")).unwrap_or_else(|e| {
            tracing::error!("Failed to open saga log: {}", e);
            std::process::exit(1);
        }),
    );
    // Seed `saga_id_alloc` from the highest persisted saga_id so
    // restarts don't reuse IDs from prior runs (reagent P1 + codex
    // P1 PR #631). With this seed + the plain INSERT (no OR REPLACE)
    // in `start_saga`, ID collisions become impossible by
    // construction.
    let saga_id_seed = saga_log.max_saga_id().unwrap_or_else(|e| {
        tracing::warn!(
            "[saga] failed to read MAX(saga_id) for allocator seed: {} — defaulting to 0; ID collisions on restart possible until next successful query",
            e
        );
        0
    });
    if saga_id_seed > 0 {
        tracing::info!(
            "[saga] seeded saga_id_alloc from durable log: next saga_id = {}",
            saga_id_seed + 1
        );
    }

    // Bootstrap data (creates Client/Window/Workspace/Tab on first launch)
    let first_launch = wcore::ensure_initial_data(&wstore).unwrap_or_else(|e| {
        tracing::error!("Failed to ensure initial data: {}", e);
        std::process::exit(1);
    });
    if first_launch {
        tracing::info!("First launch: created initial data");
    }

    // Seed ~/.agentmux/.gitignore so accidental git operations inside the
    // data directory (e.g. an agent running `git init` or `git clone` in its
    // cwd) don't stage anything by default. Idempotent — written once per
    // install; we don't overwrite an existing user-customized file.
    if let Some(home) = dirs::home_dir() {
        let data_dir = home.join(".agentmux");
        if data_dir.is_dir() {
            let gitignore = data_dir.join(".gitignore");
            if !gitignore.exists() {
                let _ = std::fs::write(&gitignore, "*\n!.gitignore\n");
            }
        }
    }

    // Self-heal layouts: remove orphaned block nodes that cause blank panes.
    // Runs on every startup to catch any corruption from prior sessions.
    heal_all_layouts(&wstore);

    // Session recovery (Phase 4.2): scan for agent blocks that still have
    // `session:active_pid` from a previous run — those sessions were killed
    // by a crash/reboot. Transfer to `session:was_interrupted` so the
    // frontend can show a reconnect banner.
    let orphan_count = backend::blockcontroller::session_recovery::scan_orphans(&wstore);
    if orphan_count > 0 {
        tracing::info!(
            orphan_count = orphan_count,
            "session_recovery: flagged {} interrupted sessions for user reconnect",
            orphan_count
        );
    }

    // Auto-seed Forge agents on first launch (or empty DB)
    backend::forge_seed::auto_seed_on_startup(&wstore);

    // Event infrastructure
    let event_bus = Arc::new(EventBus::new());
    let broker = Arc::new(Broker::new());

    // Bridge WPS events to WebSocket clients via EventBus
    let bridge = backend::eventbus::EventBusBridge::new(event_bus.clone());
    broker.set_client(Box::new(bridge));

    // Config watcher (created before sysinfo loop so it can read telemetry:interval)
    let config_watcher = Arc::new(wconfig::ConfigWatcher::with_config(wconfig::build_default_config()));

    // Load user's settings.json from disk (merges with defaults)
    backend::config_watcher_fs::load_settings_from_disk(&config_watcher);

    // Watch settings.json for changes and broadcast to WebSocket clients
    let _settings_watcher = backend::config_watcher_fs::spawn_settings_watcher(
        config_watcher.clone(),
        event_bus.clone(),
    );

    // Start sysinfo collection loop (interval configurable via telemetry:interval)
    let sysinfo_broker = broker.clone();
    let sysinfo_config = config_watcher.clone();
    tokio::spawn(async move {
        sysinfo::run_sysinfo_loop(sysinfo_broker, sysinfo_config, "local".to_string()).await;
    });

    // Start agent process watchdog (kills panes that exceed max-runtime or idle-output limits)
    let watchdog_config = config_watcher.clone();
    tokio::spawn(async move {
        backend::blockcontroller::watchdog::run_watchdog_loop(watchdog_config).await;
    });

    // Reactive handler (global singleton) + poller
    let reactive_handler = reactive::get_global_handler();
    reactive_handler.set_input_sender(Arc::new(|block_id: &str, data: &[u8]| {
        backend::blockcontroller::send_input(
            block_id,
            backend::blockcontroller::BlockInputUnion::data(data.to_vec()),
        )
    }));
    let poller = Arc::new(Poller::new(
        PollerConfig {
            agentmux_url: None,
            agentmux_token: None,
            poll_interval_secs: reactive::DEFAULT_POLL_INTERVAL_SECS,
        },
        reactive_handler,
    ));

    // Set up docsite directory
    if let Some(app_path) = base::get_wave_app_path() {
        let docsite_dir = app_path.join("docsite");
        docsite::set_docsite_dir(docsite_dir);
    }

    // Local MessageBus for inter-agent communication
    let messagebus = Arc::new(backend::messagebus::MessageBus::new());

    // Subagent watcher — monitors Claude Code session dirs for spawned subagents
    let subagent_watcher = backend::subagent_watcher::SubagentWatcher::spawn(event_bus.clone());

    // History service — discovers and indexes past CLI agent conversations
    let history_service = Arc::new(backend::history::HistoryService::new());

    // Session archiver — auto-archive sessions inactive for >7 days, cap at 2 GB.
    // Skip if home directory can't be determined (would otherwise fall back to a
    // relative path and create archives under the current working directory).
    if let Some(archive_dir) = backend::session_archive::default_archive_dir() {
        let archiver = Arc::new(backend::session_archive::SessionArchiver::new(
            wstore.clone(),
            filestore.clone(),
            7,                              // inactive days
            2 * 1024 * 1024 * 1024,         // 2 GB max
            archive_dir,
        ));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                match archiver.sweep().await {
                    Ok(stats) => tracing::info!(?stats, "session archiver sweep complete"),
                    Err(e) => tracing::warn!(error = %e, "session archiver sweep failed"),
                }
            }
        });
    } else {
        tracing::warn!("session archiver: home dir unavailable, archiver disabled");
    }

    // 5. Bind 2 TCP listeners on 127.0.0.1:0 (web + ws — separate ports matching Go)
    let web_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind web listener");
    let ws_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind ws listener");

    let web_addr = web_listener.local_addr().unwrap();
    let ws_addr = ws_listener.local_addr().unwrap();
    let local_web_url = format!("http://{}", web_addr);

    // Make local backend URL available to child processes (PTY shells).
    // agentbus-client reads AGENTMUX_LOCAL_URL and uses it for local PTY delivery
    // instead of routing through the cloud agentbus.
    std::env::set_var("AGENTMUX_LOCAL_URL", &local_web_url);

    // LAN discovery via mDNS — opt-in to avoid Windows Firewall prompt.
    // mDNS binds 0.0.0.0:5353 UDP which triggers the firewall dialog.
    // Only start if explicitly enabled in settings.
    let lan_discovery_enabled = config_watcher.get_settings().network_lan_discovery;
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    let lan_discovery = if lan_discovery_enabled {
        match backend::lan_discovery::LanDiscovery::start(
            config.instance_id.clone(),
            hostname,
            version.clone(),
            web_addr.port(),
            event_bus.clone(),
        ) {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::warn!("LAN discovery unavailable: {e}");
                None
            }
        }
    } else {
        tracing::info!("LAN discovery disabled (enable via network:lan_discovery setting)");
        None
    };

    // Clean up stale cross-instance agent registry entries (entries older than 4h).
    backend::reactive::registry::cleanup_stale(
        &base::get_wave_data_dir(),
        4 * 60 * 60 * 1000,
    );

    // Tracks agent-spawned OS processes per block. Registered trackers
    // live as long as their agent pane; the background poller emits
    // delta events (`agent:process-added`/`-exited`) to the frontend.
    let process_tracker = std::sync::Arc::new(
        backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
    );
    backend::process_tracker::registry::set_global(process_tracker.clone());
    backend::process_tracker::registry::spawn_poller(process_tracker.clone());

    // Phase E.2 / E.2c.2 — srv reducer plumbing, hoisted out of the
    // (conditional) pipe-IPC bind block so HTTP/WS RPC handlers in
    // dispatch_service can route through the reducer. State, event
    // bus, event log, and persist subscriber all live unconditionally;
    // the pipe IPC server is still conditional on
    // `AGENTMUX_SRV_PIPE_PATH` being set (absent in `task dev` mode).
    let wstore_for_persist = Arc::clone(&wstore);
    let srv_state = std::sync::Arc::new(tokio::sync::Mutex::new(state::State::default()));
    let (srv_events_tx, _) =
        tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(1024);
    let srv_event_log = std::sync::Arc::new(event_log::EventLog::new(Some(
        base::get_wave_data_dir().join("srv-events.log"),
    )));

    // Bootstrap reducer state from SQLite. Always runs (even in
    // `task dev` where there's no pipe IPC server) so RPC handlers
    // dispatching through the reducer see populated state.
    persist::bootstrap_state_from_wstore(&srv_state, &wstore_for_persist).await;

    // Spawn the disk writer (forensic log of every reducer event)
    // and the persist subscriber (idempotent SQLite write-back).
    let disk_writer_rx = srv_events_tx.subscribe();
    let log_for_writer = std::sync::Arc::clone(&srv_event_log);
    tokio::spawn(event_log::run_disk_writer(log_for_writer, disk_writer_rx));
    let subscriber_rx = srv_events_tx.subscribe();
    persist_subscriber::spawn_persist_subscriber(
        subscriber_rx,
        std::sync::Arc::clone(&wstore_for_persist),
        std::sync::Arc::clone(&srv_state),
    );

    let state = AppState {
        auth_key: config.auth_key.clone(),
        version: version.clone(),
        app_path: config.app_path.clone(),
        wstore,
        filestore,
        event_bus,
        broker,
        reactive_handler,
        poller,
        config_watcher,
        messagebus,
        subagent_watcher,
        history_service,
        lan_discovery,
        local_web_url: local_web_url.clone(),
        http_client: reqwest::Client::new(),
        process_tracker,
        // Phase E.2c.2 — reducer state + event bus exposed to HTTP/WS
        // dispatch handlers. Workspace handlers route through the
        // reducer and publish events to `srv_events_tx`; the persist
        // subscriber writes back to SQLite asynchronously.
        srv_state: std::sync::Arc::clone(&srv_state),
        srv_events_tx: srv_events_tx.clone(),
        // Phase E.5.5 — saga-id allocator. Seeded from
        // `SagaLog::max_saga_id()` so restarts don't collide with
        // prior runs' IDs. First new saga after restart gets
        // `seed + 1`; on a fresh DB seed=0, first saga gets id 1.
        saga_id_alloc: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(saga_id_seed)),
        saga_log: Arc::clone(&saga_log),
    };

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

    // Phase E.1b — srv pipe IPC server. Bound when launcher passes
    // `AGENTMUX_SRV_PIPE_PATH`; absent in `task dev` mode (no
    // launcher in the loop).
    //
    // Phase E.2 — bootstrap reducer state from SQLite at startup
    // so the session-only projection starts populated. The persist
    // subscriber that mirrors pipe-event effects back to SQLite is
    // deferred to E.2c (alongside the RPC-through-reducer migration);
    // until then, HTTP/WS RPC continues writing directly via wcore
    // and pipe commands only mutate the reducer's session-only state.
    //
    // Bind happens BEFORE the AGENTMUXSRV-ESTART line so the
    // launcher knows the pipe is ready when host starts. Non-fatal
    // if the bind fails — srv keeps running with HTTP/WS only.
    #[cfg(target_os = "windows")]
    if let Ok(srv_pipe_path) = std::env::var("AGENTMUX_SRV_PIPE_PATH") {
        if !srv_pipe_path.is_empty() {
            match srv_ipc::server::bind_first_pipe_instance(&srv_pipe_path) {
                Ok(first_pipe) => {
                    // Phase E.2c.2 — pipe IPC server reuses the
                    // hoisted srv_state / events_tx / event_log so
                    // pipe-originated commands and HTTP/WS-originated
                    // commands mutate the same canonical state.
                    let srv_ctx = srv_ipc::ServerCtx {
                        srv_pid: std::process::id(),
                        srv_version: version.clone(),
                        state: std::sync::Arc::clone(&srv_state),
                        events_tx: srv_events_tx.clone(),
                        event_log: std::sync::Arc::clone(&srv_event_log),
                    };
                    let _srv_ipc_handle = srv_ipc::run_srv_ipc_server(
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

    // 6. Emit AGENTMUXSRV-ESTART on stderr (exact format from cmd/server/main-server.go:617)
    eprintln!(
        "AGENTMUXSRV-ESTART ws:{} web:{} version:{} buildtime:{} instance:{}",
        ws_addr, web_addr, version, build_time, config.instance_id
    );

    // 7. Build router and serve on both listeners
    let router = build_router(state);

    let web_server = axum::serve(web_listener, router.clone());
    let ws_server = axum::serve(ws_listener, router);

    // 8. Spawn stdin watch thread (exit on EOF — matching Go's stdinReadWatch)
    let stdin_token = tokio_util::sync::CancellationToken::new();
    let stdin_shutdown = stdin_token.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 1024];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => {
                    eprintln!("stdin closed, shutting down");
                    stdin_shutdown.cancel();
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("stdin read error: {}, shutting down", e);
                    stdin_shutdown.cancel();
                    break;
                }
            }
        }
    });

    // 9. Spawn signal handler (SIGINT/SIGTERM → graceful shutdown)
    let signal_token = stdin_token.clone();
    tokio::spawn(async move {
        let ctrl_c = signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut sigterm =
                signal::unix::signal(signal::unix::SignalKind::terminate()).unwrap();
            tokio::select! {
                _ = ctrl_c => {
                    tracing::info!("received SIGINT, shutting down");
                }
                _ = sigterm.recv() => {
                    tracing::info!("received SIGTERM, shutting down");
                }
            }
        }
        #[cfg(not(unix))]
        {
            ctrl_c.await.ok();
            tracing::info!("received Ctrl+C, shutting down");
        }
        signal_token.cancel();
    });

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

/// Initialize tracing with dual output: JSON rolling file + human-readable stderr.
/// Returns a guard that must be held for the lifetime of the app to ensure log flushing.
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    // Always log to ~/.agentmux/logs/ so all logs (host + sidecar) land in one
    // discoverable directory. AGENTMUX_DATA_HOME controls the data dir, not logs.
    // Version is embedded in the filename for side-by-side coexistence.
    let version = env!("CARGO_PKG_VERSION");
    let log_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".agentmux")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    // Delete log files older than 7 days to prevent unbounded growth.
    cleanup_old_logs(&log_dir, 7);

    // Rolling daily log file with JSON structured output
    let log_prefix = format!("agentmuxsrv-v{}.log", version);
    let file_appender = tracing_appender::rolling::daily(&log_dir, &log_prefix);
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    // Write pointer to current log file for zero-lookup agent discovery.
    // Version-qualified name so multi-instance doesn't clobber pointers.
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let current_filename = format!("{}.{}", log_prefix, today);
    let pointer_name = format!("current-srv-v{}.path", version);
    let _ = std::fs::write(log_dir.join(&pointer_name), &current_filename);

    // Spawn a background thread to refresh the pointer on UTC date rollover.
    // tracing_appender::rolling::daily creates a new file at midnight UTC.
    {
        let log_dir = log_dir.clone();
        let log_prefix = log_prefix.clone();
        let pointer_name = pointer_name.clone();
        std::thread::Builder::new()
            .name("srv-log-pointer".into())
            .spawn(move || {
                let mut last_date = chrono::Utc::now().format("%Y-%m-%d").to_string();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                    if last_date != today {
                        last_date = today.clone();
                        let filename = format!("{}.{}", log_prefix, today);
                        let _ = std::fs::write(log_dir.join(&pointer_name), &filename);
                    }
                }
            })
            .ok();
    }

    let subscriber = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("agentmuxsrv=info,info")),
        )
        .with(
            fmt::layer()
                .json()
                .with_writer(non_blocking_file)
                .with_target(true)
                .with_thread_ids(true),
        )
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(true),
        );

    tracing::subscriber::set_global_default(subscriber).ok();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        log_dir = %log_dir.display(),
        "agentmuxsrv starting"
    );

    guard
}

/// Delete log files (*.log.*) older than `days` to prevent unbounded growth.
/// Only touches files with `.log.` in the name — pointer files and other data are safe.
fn cleanup_old_logs(log_dir: &std::path::Path, days: u64) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(days * 86400);
    let Ok(entries) = std::fs::read_dir(log_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.to_string_lossy().contains(".log.") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

/// Walk all tabs and heal their layouts by removing orphaned block references.
fn heal_all_layouts(store: &WaveStore) {
    use backend::obj::Tab;

    let tabs: Vec<Tab> = match store.get_all::<Tab>() {
        Ok(tabs) => tabs,
        Err(e) => {
            tracing::warn!(error = %e, "heal_all_layouts: failed to list tabs");
            return;
        }
    };

    let mut healed = 0;
    for tab in &tabs {
        match backend::wcore::heal_layout(store, &tab.oid) {
            Ok(true) => {
                tracing::info!(tab_id = %tab.oid, tab_name = %tab.name, "layout healed on startup");
                healed += 1;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(tab_id = %tab.oid, error = %e, "heal_layout failed");
            }
        }
    }
    if healed > 0 {
        tracing::info!(tabs_healed = healed, "layout self-healing complete");
    } else {
        tracing::info!("layout self-healing: all layouts clean");
    }
}
