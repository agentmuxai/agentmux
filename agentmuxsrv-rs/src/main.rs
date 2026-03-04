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

#[tokio::main]
async fn main() {
    // 1. Init tracing (stderr writer)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

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

    // Event infrastructure
    let event_bus = Arc::new(EventBus::new());
    let broker = Arc::new(Broker::new());

    // Bridge WPS events to WebSocket clients via EventBus
    let bridge = backend::eventbus::EventBusBridge::new(event_bus.clone());
    broker.set_client(Box::new(bridge));

    // Start sysinfo collection loop (CPU/memory/network metrics at 1s intervals)
    let sysinfo_broker = broker.clone();
    tokio::spawn(async move {
        sysinfo::run_sysinfo_loop(sysinfo_broker, "local".to_string()).await;
    });

    // Reactive handler (global singleton) + poller
    let reactive_handler = reactive::get_global_handler();
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

    let config_watcher = Arc::new(wconfig::ConfigWatcher::with_config(wconfig::build_default_config()));

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
    };

    // 5. Bind 2 TCP listeners on 127.0.0.1:0 (web + ws — separate ports matching Go)
    let web_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind web listener");
    let ws_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind ws listener");

    let web_addr = web_listener.local_addr().unwrap();
    let ws_addr = ws_listener.local_addr().unwrap();

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
