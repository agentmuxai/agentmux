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

    // Initialize tracing (stderr + optional env filter).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "AgentMux CEF host starting"
    );

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
        tracing::error!("Failed to parse command line arguments");
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
        tracing::info!("CEF subprocess exiting: type={}", process_type);
        assert!(ret >= 0, "execute_process failed for subprocess");
        std::process::exit(ret);
    }

    // -----------------------------------------------------------------------
    // Browser process initialization
    // -----------------------------------------------------------------------

    tracing::info!("Initializing CEF browser process");

    // Create shared application state.
    let app_state = Arc::new(state::AppState::default());

    // Start tokio runtime for async operations (IPC server, sidecar management).
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    // Start the IPC HTTP server and get the assigned port.
    let ipc_port = runtime.block_on(ipc::start_ipc_server(app_state.clone()));
    *app_state.ipc_port.lock().unwrap() = ipc_port;

    tracing::info!("IPC server started on port {}", ipc_port);

    // Spawn the backend sidecar asynchronously.
    let state_for_sidecar = app_state.clone();
    runtime.spawn(async move {
        match sidecar::spawn_backend(&state_for_sidecar).await {
            Ok(result) => {
                // Store endpoints in state
                {
                    let mut endpoints = state_for_sidecar.backend_endpoints.lock().unwrap();
                    endpoints.ws_endpoint = result.ws_endpoint.clone();
                    endpoints.web_endpoint = result.web_endpoint.clone();
                }

                // Emit backend-ready event to frontend
                let payload = serde_json::json!({
                    "ws": result.ws_endpoint,
                    "web": result.web_endpoint,
                });
                events::emit_event_from_state(&state_for_sidecar, "backend-ready", &payload);

                tracing::info!(
                    "Backend ready: ws={} web={}",
                    result.ws_endpoint,
                    result.web_endpoint
                );
            }
            Err(e) => {
                tracing::error!("Failed to spawn backend: {}", e);
                let payload = serde_json::json!({
                    "error": format!("{}", e),
                });
                events::emit_event_from_state(&state_for_sidecar, "backend-spawn-error", &payload);
            }
        }
    });

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

    // Resolve cache path — dev mode uses a separate cache to avoid singleton conflict
    // with any running portable build.
    let is_dev = std::env::var("AGENTMUX_DEV").is_ok();
    let cache_dir = if is_dev {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("ai.agentmux.cef.dev");
        std::fs::create_dir_all(&dir).ok();
        CefString::from(dir.to_str().unwrap_or(""))
    } else {
        CefString::default()
    };

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

    // Run the CEF message loop. This blocks until quit_message_loop() is called
    // (triggered when all browser windows are closed in client.rs).
    run_message_loop();

    tracing::info!("CEF message loop exited, shutting down");

    // Kill the backend sidecar on shutdown.
    {
        let mut sidecar = app_state.sidecar_child.lock().unwrap();
        if let Some(ref mut child) = *sidecar {
            tracing::info!("Killing backend sidecar");
            let _ = child.kill();
        }
    }

    // Clean shutdown.
    shutdown();

    // Drop the tokio runtime after CEF shutdown.
    drop(runtime);

    tracing::info!("AgentMux CEF host shutdown complete");
}
