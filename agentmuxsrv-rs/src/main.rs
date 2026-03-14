mod backend;
mod config;
mod server;

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
use backend::{docsite, sysinfo, wavebase, wcore};

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

#[tokio::main]
async fn main() {
    // 0. Start ppid watchdog BEFORE tokio runtime does real work (Linux/macOS only).
    // On Windows, the frontend uses a Job Object with KILL_ON_JOB_CLOSE instead.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    start_ppid_watchdog();

    // 1. Init tracing (stderr + rolling file)
    let _log_guard = init_logging();

    // 2. Parse CLI args
    let args = CliArgs::parse();

    // 3. Build config from env + args
    let config = config::Config::from_env_and_args(&args).unwrap_or_else(|e| {
        tracing::error!("Failed to load config: {}", e);
        std::process::exit(1);
    });

    let version = config.version.to_string();
    let build_time = config.build_time.to_string();

    // 4. Initialize backend (matching Go cmd/server/main-server.go:374-590)
    wavebase::set_version(&version);
    wavebase::set_build_time(&build_time);

    // Migrate ~/.waveterm → ~/.agentmux if needed (one-time, non-destructive)
    wavebase::migrate_legacy_data_dir();

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

    wavebase::ensure_wave_data_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure data dir: {}", e);
        std::process::exit(1);
    });
    wavebase::ensure_wave_db_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure db dir: {}", e);
        std::process::exit(1);
    });

    // Startup diagnostics
    tracing::info!(
        data_dir = %wavebase::get_wave_data_dir().display(),
        db_dir = %wavebase::get_wave_db_dir().display(),
        app_path = %config.app_path,
        instance_id = %config.instance_id,
        "backend directories initialized"
    );

    // Open databases
    let db_dir = wavebase::get_wave_db_dir();
    let wstore = Arc::new(WaveStore::open(&db_dir.join("wave.db")).unwrap_or_else(|e| {
        tracing::error!("Failed to open wave store: {}", e);
        std::process::exit(1);
    }));
    let filestore = Arc::new(FileStore::open(&db_dir.join("filestore.db")).unwrap_or_else(|e| {
        tracing::error!("Failed to open file store: {}", e);
        std::process::exit(1);
    }));

    // Bootstrap data (creates Client/Window/Workspace/Tab on first launch)
    let first_launch = wcore::ensure_initial_data(&wstore).unwrap_or_else(|e| {
        tracing::error!("Failed to ensure initial data: {}", e);
        std::process::exit(1);
    });
    if first_launch {
        tracing::info!("First launch: created initial data");
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
    if let Some(app_path) = wavebase::get_wave_app_path() {
        let docsite_dir = app_path.join("docsite");
        docsite::set_docsite_dir(docsite_dir);
    }

    // Local MessageBus for inter-agent communication
    let messagebus = Arc::new(backend::messagebus::MessageBus::new());

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

    // Clean up stale cross-instance agent registry entries (entries older than 4h).
    backend::reactive::registry::cleanup_stale(
        &wavebase::get_wave_data_dir(),
        4 * 60 * 60 * 1000,
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
        local_web_url: local_web_url.clone(),
        http_client: reqwest::Client::new(),
    };

    // 6. Emit WAVESRV-ESTART on stderr (exact format from cmd/server/main-server.go:617)
    eprintln!(
        "WAVESRV-ESTART ws:{} web:{} version:{} buildtime:{} instance:{}",
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

    // Determine log directory: {AGENTMUX_DATA_HOME}/logs/ or ~/.agentmux/logs/
    // Include version in filename so multiple versions can run side-by-side.
    let version = env!("CARGO_PKG_VERSION");
    let log_dir = std::env::var("AGENTMUX_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".agentmux"))
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    // Rolling daily log file with JSON structured output
    let log_prefix = format!("agentmuxsrv-v{}.log", version);
    let file_appender = tracing_appender::rolling::daily(&log_dir, &log_prefix);
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

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
