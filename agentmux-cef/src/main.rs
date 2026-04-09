// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AgentMux CEF Host — Entry point.
//
// This binary serves as both the browser process and CEF subprocess
// (renderer, GPU, utility). Subprocess mode is detected via the --type
// command-line argument injected by CEF.
//
// Phase 2: Includes IPC HTTP server, sidecar management, and command routing.
//
// Usage:
//   agentmux-cef                         # Load default URL (http://localhost:5173)
//   agentmux-cef --url=http://host:port  # Load custom URL
//   agentmux-cef --use-native            # Use native platform window instead of Views
//   agentmux-cef --use-alloy-style       # Use Alloy runtime style

// Hide console window in release mode on Windows (not sandbox).
#![cfg_attr(
    all(not(debug_assertions), not(feature = "sandbox"), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app;
mod client;
mod commands;
mod events;
mod ipc;
mod memory_heartbeat;
mod sidecar;
mod state;
mod ui_tasks;

use std::sync::Arc;

use cef::*;

fn main() {
    // Add runtime/ subdirectory to DLL search path so CEF can find libcef.dll
    // in the portable layout (agentmux.exe in root, libcef.dll in runtime/).
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let runtime_dir = dir.join("runtime");
                if runtime_dir.exists() {
                    unsafe {
                        use std::os::windows::ffi::OsStrExt;
                        let wide: Vec<u16> = runtime_dir.as_os_str().encode_wide().chain(Some(0)).collect();
                        windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr());
                    }
                }
            }
        }
    }

    // Tracing is initialized after the subprocess check below — browser process
    // gets dual file+stderr output; subprocesses exit before tracing is needed.

    // macOS: load the CEF framework library explicitly.
    #[cfg(target_os = "macos")]
    let _library = {
        let loader =
            library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), false);
        assert!(loader.load(), "Failed to load CEF framework");
        loader
    };

    // Initialize the CEF API hash for version verification.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    // Parse command-line arguments.
    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        eprintln!("agentmux-cef: Failed to parse command line arguments");
        std::process::exit(1);
    };

    // Detect subprocess mode: CEF injects --type=renderer|gpu-process|utility
    // for child processes. If --type is present, this is a subprocess.
    let type_switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&type_switch)) != 1;

    // Execute subprocess if applicable (exits here for non-browser processes).
    let ret = execute_process(
        Some(args.as_main_args()),
        None, // App can be None for subprocess
        std::ptr::null_mut(),
    );

    if is_browser_process {
        // Browser process: execute_process returns -1, we continue with initialization.
        assert_eq!(ret, -1, "execute_process should return -1 for browser process");
    } else {
        // Subprocess: execute_process returns the exit code.
        let process_type = CefString::from(&cmd_line.switch_value(Some(&type_switch)));
        eprintln!("agentmux-cef: subprocess exiting: type={}", process_type);
        assert!(ret >= 0, "execute_process failed for subprocess");
        std::process::exit(ret);
    }

    // -----------------------------------------------------------------------
    // Browser process initialization
    // -----------------------------------------------------------------------

    // Initialize dual-output tracing: rolling log file + stderr.
    // The log file guard must live for the entire process to ensure flushing.
    let _log_guard = init_logging();

    tracing::info!("Initializing CEF browser process");

    // Single-instance check: if another instance of the same version is
    // running, send it a "new window" request via its IPC server and exit.
    // Uses a named mutex for detection and a port file for communication.
    let version = env!("CARGO_PKG_VERSION");
    let is_dev = std::env::var("AGENTMUX_DEV").is_ok();
    let version_slug = version.replace('.', "-");
    let data_dir_name = if is_dev {
        "ai.agentmux.cef.dev".to_string()
    } else {
        format!("ai.agentmux.cef.v{}", version_slug)
    };
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(&data_dir_name);
    std::fs::create_dir_all(&data_dir).ok();
    let port_file = data_dir.join("ipc-port");

    // If port file exists and we can connect, another instance is running.
    // Send it a "new window" request and exit.
    if port_file.exists() {
        if let Ok(contents) = std::fs::read_to_string(&port_file) {
            let parts: Vec<&str> = contents.trim().splitn(2, ':').collect();
            if parts.len() == 2 {
                let addr: Result<std::net::SocketAddr, _> = format!("127.0.0.1:{}", parts[0]).parse();
                if let Ok(addr) = addr {
                if let Ok(mut stream) = std::net::TcpStream::connect_timeout(
                    &addr,
                    std::time::Duration::from_secs(2),
                ) {
                    use std::io::Write;
                    let body = r#"{"cmd":"open_new_window"}"#;
                    let req = format!(
                        "POST /ipc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        parts[1], body.len(), body
                    );
                    let _ = stream.write_all(req.as_bytes());
                    tracing::info!("Sent new-window request to existing instance");
                    std::process::exit(0);
                }
                // Connection failed — stale port file, continue with fresh launch
                tracing::info!("Stale port file (connection refused), launching fresh");
            }
            } // addr parse
        }
    }

    // Create shared application state.
    let app_state = Arc::new(state::AppState::default());

    // Start tokio runtime for async operations (IPC server, sidecar management).
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    // Start the IPC HTTP server and get the assigned port.
    let ipc_port = runtime.block_on(ipc::start_ipc_server(app_state.clone()));
    *app_state.ipc_port.lock() = ipc_port;

    tracing::info!("IPC server started on port {}", ipc_port);

    // Spawn the backend sidecar SYNCHRONOUSLY — block until it signals ready
    // (WAVESRV-ESTART) before creating the browser window. This eliminates the
    // race condition where CEF loads the frontend before the backend is available,
    // which causes a "raw browser" appearance on slow machines or first launch.
    let backend_ready = runtime.block_on(async {
        match sidecar::spawn_backend(&app_state).await {
            Ok(result) => {
                {
                    let mut endpoints = app_state.backend_endpoints.lock();
                    endpoints.ws_endpoint = result.ws_endpoint.clone();
                    endpoints.web_endpoint = result.web_endpoint.clone();
                }
                tracing::info!(
                    "Backend ready: ws={} web={}",
                    result.ws_endpoint,
                    result.web_endpoint
                );
                true
            }
            Err(e) => {
                tracing::error!("Failed to spawn backend: {}", e);
                false
            }
        }
    });

    if !backend_ready {
        tracing::error!("Backend failed to start — exiting");
        std::process::exit(1);
    }

    // Create the App handler with state.
    let mut cef_app = app::AgentMuxApp::new(app_state.clone(), ipc_port);

    // Resolve runtime directory for portable layout (resources in runtime/ subdir)
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    let runtime_dir = exe_dir.join("runtime");
    let base_dir = if runtime_dir.exists() {
        runtime_dir
    } else {
        // Dev mode: resources are flat alongside the exe in dist/cef/
        exe_dir.clone()
    };
    let resources_dir = CefString::from(base_dir.to_str().unwrap_or(""));
    let locales_dir = CefString::from(base_dir.join("locales").to_str().unwrap_or(""));

    // Reuse data_dir from single-instance check as CEF cache path.
    // Remove stale lockfile from a previous killed run.
    let lockfile = data_dir.join("lockfile");
    if lockfile.exists() {
        tracing::warn!("Removing stale CEF lockfile: {}", lockfile.display());
        let _ = std::fs::remove_file(&lockfile);
    }
    tracing::info!("CEF cache dir: {}", data_dir.display());
    let cache_dir = CefString::from(data_dir.to_str().unwrap_or(""));

    // Configure CEF settings.
    let settings = Settings {
        no_sandbox: 1,
        background_color: 0xFF000000,
        remote_debugging_port: if is_dev { 9223 } else { 9222 },
        root_cache_path: cache_dir,
        resources_dir_path: resources_dir,
        locales_dir_path: locales_dir,
        // CEF subprocess (renderer, GPU) uses the same exe
        browser_subprocess_path: CefString::from(
            std::env::current_exe().unwrap().to_str().unwrap_or("")
        ),
        ..Default::default()
    };

    // Initialize CEF.
    let init_result = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut cef_app),
        std::ptr::null_mut(),
    );
    assert_eq!(init_result, 1, "CEF initialization failed");

    tracing::info!("CEF initialized, entering message loop");

    // Start memory heartbeat — logs system/process memory stats every 20s.
    // Provides forensic data if the process later crashes from OOM / VA exhaustion.
    memory_heartbeat::start();

    // Write port + token to file AFTER CEF init so a second instance
    // only connects when we're ready to handle new-window requests.
    let _ = std::fs::write(
        &port_file,
        format!("{}:{}", ipc_port, app_state.ipc_token),
    );

    // Run the CEF message loop. This blocks until quit_message_loop() is called
    // (triggered when all browser windows are closed in client.rs).
    run_message_loop();

    tracing::info!("CEF message loop exited, shutting down");

    // Kill the backend sidecar on shutdown.
    {
        let mut sidecar = app_state.sidecar_child.lock();
        if let Some(ref mut child) = *sidecar {
            tracing::info!("Killing backend sidecar");
            let _ = child.kill();
        }
    }

    // Clean shutdown.
    shutdown();

    // Drop the tokio runtime after CEF shutdown.
    drop(runtime);

    // Clean up port file so stale data doesn't confuse future launches.
    let _ = std::fs::remove_file(&port_file);

    tracing::info!("AgentMux CEF host shutdown complete");
}

/// Initialize tracing with dual output: rolling daily log file + human-readable stderr.
/// Returns a guard that must be held for the lifetime of the process to ensure log flushing.
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    // Always log to ~/.agentmux/logs/ so all logs (host + sidecar) land in one
    // discoverable directory. Ignores AGENTMUX_DATA_HOME which may point to a
    // versioned AppData dir (or be inherited from a parent AgentMux instance).
    let version = env!("CARGO_PKG_VERSION");
    let log_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".agentmux")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    // Delete log files older than 7 days to prevent unbounded growth.
    cleanup_old_logs(&log_dir, 7);

    let log_prefix = format!("agentmux-host-v{}.log", version);
    let file_appender = tracing_appender::rolling::daily(&log_dir, &log_prefix);
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    // Write pointer to current log file for zero-lookup agent discovery.
    // Version-qualified name so multi-instance doesn't clobber pointers.
    // Uses UTC to match tracing_appender::rolling::daily's date suffix.
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let current_filename = format!("{}.{}", log_prefix, today);
    let pointer_name = format!("current-host-v{}.path", version);
    let _ = std::fs::write(log_dir.join(&pointer_name), &current_filename);

    let subscriber = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
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
        version,
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        log_dir = %log_dir.display(),
        "AgentMux CEF host starting"
    );

    guard
}

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
