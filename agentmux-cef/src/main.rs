// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AgentMux Host — Entry point.
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
mod browser_api;
mod browser_panes;
mod client;
mod commands;
mod dev_authfile;
mod events;
mod ipc;
mod launcher_event_bridge;
mod launcher_ipc;
mod srv_event_bridge;
mod srv_ipc;
mod memory_heartbeat;
mod pane;
mod reducer;
mod saga_dispatch;
mod sidecar;
mod state;
mod ui_tasks;
mod wrr;

use std::sync::Arc;

use cef::*;

fn main() {
    // Set the DLL search path so CEF's runtime LoadLibrary calls (chrome_elf,
    // libEGL, libGLESv2, d3dcompiler_47, …) resolve against the directory that
    // actually holds libcef.dll. Two layouts exist:
    //
    //   Portable / installed: <root>/runtime/host.exe + libcef.dll alongside.
    //                         The launcher (agentmux-launcher) already sets
    //                         the path to <root>/runtime/ before spawning us;
    //                         this block is a no-op safety net for that mode.
    //
    //   Dev (`task dev`):     dist/cef-dev/agentmux-cef.exe + libcef.dll
    //                         alongside (flat layout). Taskfile launches the
    //                         host directly with no launcher, so nothing else
    //                         has set the DLL path. Without it, CEF's internal
    //                         LoadLibrary chain can fail and `cef::initialize`
    //                         returns 0 — the empty-chrome_debug.log mode.
    //
    // Fall back to the host's own directory whenever a runtime/ subdir isn't
    // present. Idempotent in portable mode (launcher set it first), correct
    // in dev mode.
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let runtime_dir = dir.join("runtime");
                let dll_dir = if runtime_dir.exists() {
                    runtime_dir
                } else {
                    dir.to_path_buf()
                };
                unsafe {
                    use std::os::windows::ffi::OsStrExt;
                    let wide: Vec<u16> = dll_dir.as_os_str().encode_wide().chain(Some(0)).collect();
                    windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr());
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

    // Set the Application User Model ID before any UI is created. This lets
    // Windows group our windows under one pinned identity and is required for
    // the `DeleteTab` + per-HWND AppID treatment used by the full-instance /
    // sub-window model (see docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md).
    // Use a VERSION-STABLE ID — never embed the patch number or pinning forks.
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        let aumid: Vec<u16> = "AgentMuxCorp.AgentMux\0".encode_utf16().collect();
        let _ = SetCurrentProcessExplicitAppUserModelID(aumid.as_ptr());
    }

    let version = env!("CARGO_PKG_VERSION");
    let is_dev = std::env::var("AGENTMUX_DEV").is_ok();
    let version_slug = version.replace('.', "-");

    // Detect portable mode: in portable builds the CEF host binary lives inside
    // <portable-root>/runtime/. If current_exe()'s parent directory is named
    // "runtime", we are in portable mode and the portable root is its parent.
    let host_exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    let portable_root: Option<std::path::PathBuf> = if host_exe_dir
        .file_name()
        .and_then(|n| n.to_str())
        == Some("runtime")
    {
        host_exe_dir.parent().map(|p| p.to_path_buf())
    } else {
        None
    };

    // Resolve the CEF cache directory and log directory.
    //
    // Portable:  all state lives under <portable-root>/data/
    //            → cef cache:  data/cef/
    //            → logs:       data/logs/
    //
    // Installed: state lives in platform AppData (version-isolated)
    //            → cef cache:  %LOCALAPPDATA%/ai.agentmux.cef.vX/
    //            → logs:       ~/.agentmux/logs/  (shared, easy to find)
    let (data_dir, log_dir) = if let Some(ref root) = portable_root {
        let base = root.join("data");
        (base.join("cef"), base.join("logs"))
    } else {
        let cef_name = if is_dev {
            "ai.agentmux.cef.dev".to_string()
        } else {
            format!("ai.agentmux.cef.v{}", version_slug)
        };
        let cef_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(&cef_name);
        let log_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".agentmux")
            .join("logs");
        (cef_dir, log_dir)
    };
    std::fs::create_dir_all(&data_dir).ok();

    // Initialize dual-output tracing: rolling log file + stderr.
    // The log file guard must live for the entire process to ensure flushing.
    let _log_guard = init_logging(&log_dir);

    tracing::info!(
        version,
        portable = portable_root.is_some(),
        data_dir = %data_dir.display(),
        log_dir = %log_dir.display(),
        "Initializing CEF browser process"
    );

    // Phase B.6 (post-fix): the named-pipe bind in the launcher is
    // the AUTHORITATIVE single-instance lock — a second launcher
    // hits ERROR_ACCESS_DENIED and never reaches the host. We still
    // publish `<launcher-shared-data-dir>/ipc-port` (port:token) so
    // the second launcher can FORWARD an `open_new_window` request
    // to the existing instance over HTTP and exit silently — the
    // legacy forwarding UX users expect when double-clicking the
    // exe twice. The pipe-bind-first ordering closes the stale-state
    // defect (gap #8 in
    // specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md):
    // a stale ipc-port file from a hard crash is irrelevant on the
    // FIRST-instance path because pipe-bind succeeds and the file is
    // overwritten; on the SECOND-instance path the live first
    // instance wrote a fresh port:token, so forwarding lands.
    //
    // CRITICAL: write the port file at the LAUNCHER-shared data dir
    // (`AGENTMUX_DATA_DIR`, == `paths.data_dir` in the launcher), NOT
    // the host-local CEF cache dir (`<portable>/data/cef/`). The two
    // diverge in portable mode (cef cache is one level deeper) and
    // the launcher's `forward_open_new_window` reads the launcher-
    // shared path. Falls back to the cef cache dir only when the env
    // is unset (`task dev` mode without launcher), where forwarding
    // wouldn't be wired anyway.
    let port_file_dir = std::env::var_os("AGENTMUX_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| data_dir.clone());
    let _ = std::fs::create_dir_all(&port_file_dir);
    let port_file = port_file_dir.join("ipc-port");

    // Create shared application state.
    let app_state = Arc::new(state::AppState::default());

    // Start tokio runtime for async operations (IPC server, sidecar management).
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    // Start the IPC HTTP server and get the assigned port.
    let ipc_port = runtime.block_on(ipc::start_ipc_server(app_state.clone()));
    *app_state.ipc_port.lock() = ipc_port;

    tracing::info!("IPC server started on port {}", ipc_port);

    // Phase B.2: connect to launcher's named-pipe IPC (if launcher
    // is in the loop) so the launcher can route Commands and Events
    // through us. The handle is held in main scope for the host's
    // lifetime — dropping it closes the pipe (logged by launcher).
    // Failure to connect is non-fatal in B.2 (host can still run);
    // B.5+ will tighten when the host depends on IPC for state.
    let _launcher_ipc = runtime.block_on(launcher_ipc::connect_to_launcher(app_state.clone()));

    // Phase E.2c.5a — connect to the srv reducer's pipe. Forwards
    // srv events (workspace / tab / block lifecycle) to every
    // top-level renderer via the JS bridge. Renderer-side handler
    // (`window.__agentmux_srv_event`) lands in E.2c.5b. Non-fatal
    // if absent: `task dev` mode doesn't run the launcher and so
    // doesn't set `AGENTMUX_SRV_PIPE_PATH` — host runs without the
    // bridge, frontend uses the legacy waveobj:update path.
    let _srv_ipc = runtime.block_on(srv_ipc::connect_to_srv(app_state.clone()));

    // Phase B.1: if launcher already spawned srv (the normal portable
    // / installed path post-PR-#570 + B.1), populate state from the
    // env vars launcher set — no need to re-spawn srv. Falls back to
    // spawn_backend() ONLY when env vars are absent (`task dev` mode
    // where the host runs without the launcher).
    //
    // Spawn the backend sidecar SYNCHRONOUSLY — block until it
    // signals ready (AGENTMUXSRV-ESTART) before creating the browser
    // window. This eliminates the race condition where CEF loads the
    // frontend before the backend is available, which causes a "raw
    // browser" appearance on slow machines or first launch.
    let backend_ready = runtime.block_on(async {
        let launcher_provided = sidecar::use_launcher_endpoints(&app_state);
        let result = match launcher_provided {
            Some(Ok(r)) => {
                tracing::info!(
                    "Using launcher-provided backend endpoints: ws={} web={} pid={}",
                    r.ws_endpoint,
                    r.web_endpoint,
                    r.instance_id
                );
                Ok(r)
            }
            Some(Err(e)) => {
                tracing::error!(
                    "Launcher set AGENTMUX_BACKEND_WS but env was malformed: {} — refusing to fall back to spawn_backend (would fight launcher's srv)",
                    e
                );
                Err(e)
            }
            None => {
                tracing::info!("No launcher-provided backend env (dev mode) — spawning srv ourselves");
                sidecar::spawn_backend(&app_state).await
            }
        };
        match result {
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
                tracing::error!("Failed to set up backend: {}", e);
                false
            }
        }
    });

    if !backend_ready {
        tracing::error!("Backend failed to start — exiting");
        std::process::exit(1);
    }

    // Dev-only: write authkey.dev so external test harnesses can call
    // the service API without polling logs or driving the UI. Gate is
    // runtime, not cfg(debug_assertions), because `task dev` builds
    // --release (Taskfile.yml `build:host:windows`) — a compile-time
    // gate would silently no-op the file write in dev mode, defeating
    // the purpose. Taskfile sets AGENTMUX_DEV=1 on the dev task; the
    // user's installed/portable builds do not, so the file is only
    // written when the operator has opted into dev mode for THIS run.
    // See docs/specs/SPEC_TEST_API_ACCESS.md §3 (threat model) — the
    // attacker class affected is "same-user local process", which we
    // do not defend against.
    if std::env::var("AGENTMUX_DEV").as_deref() == Ok("1") {
        let endpoints = app_state.backend_endpoints.lock().clone();
        let auth_key = app_state.auth_key.lock().clone();
        let ipc_token = app_state.ipc_token.clone();
        let data_dir_str = app_state
            .version_data_dir
            .lock()
            .clone()
            .unwrap_or_default();
        let data_dir_path = std::path::PathBuf::from(&data_dir_str);
        let ipc_endpoint = format!("127.0.0.1:{}", ipc_port);
        let instance = format!("v{}", env!("CARGO_PKG_VERSION"));
        let host_pid = std::process::id();
        match dev_authfile::write_dev_auth_file(
            &data_dir_path,
            &auth_key,
            &endpoints.web_endpoint,
            &endpoints.ws_endpoint,
            &ipc_endpoint,
            &ipc_token,
            &instance,
            host_pid,
        ) {
            Ok(p) => tracing::info!("Wrote dev authkey file: {}", p.display()),
            Err(e) => tracing::warn!("Failed to write dev authkey file: {}", e),
        }
    }

    // Create the App handler with state.
    let mut cef_app = app::AgentMuxApp::new(app_state.clone(), ipc_port);

    // Resolve resource directories for portable layout.
    // In portable mode the CEF host is IN runtime/, so resources are flat
    // alongside it. In dev mode they are also flat in dist/cef-dev/.
    // host_exe_dir is already computed above.
    let runtime_dir = host_exe_dir.join("runtime");
    let base_dir = if runtime_dir.exists() {
        runtime_dir
    } else {
        host_exe_dir.clone()
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
    let debug_port: u16 = if is_dev { 9223 } else { 9222 };
    *app_state.debug_port.lock() = debug_port;

    // Route CEF's internal Chromium logging into our log dir alongside the
    // tracing-subscriber file. Without this, init failures leave an empty
    // chrome_debug.log in the cache dir and we have nothing to read. INFO is
    // verbose enough to expose load-library / resource problems but quiet
    // enough not to swamp the file in normal operation.
    let cef_log_path = log_dir.join("cef-debug.log");
    let cef_log_file = CefString::from(cef_log_path.to_str().unwrap_or(""));

    let settings = Settings {
        no_sandbox: 1,
        background_color: 0xFF000000,
        remote_debugging_port: debug_port as i32,
        root_cache_path: cache_dir,
        resources_dir_path: resources_dir,
        locales_dir_path: locales_dir,
        log_file: cef_log_file,
        log_severity: LogSeverity::INFO,
        // CEF subprocess (renderer, GPU) uses the same exe
        browser_subprocess_path: CefString::from(
            std::env::current_exe().unwrap().to_str().unwrap_or("")
        ),
        ..Default::default()
    };

    // Initialize CEF.
    //
    // CefInitialize returns 1 on success and 0 either on real init failure OR
    // on "normal early exit" (process singleton, command-line forward, etc).
    // We can only tell the two apart by calling cef_get_exit_code() and
    // matching against cef_resultcode_t. Treat NORMAL_EXIT* codes as a clean
    // exit; everything else is a real failure that we surface via panic.
    //
    // Common early-exit codes (cef_resultcode_t):
    //   0  CEF_RESULT_CODE_NORMAL_EXIT
    //   24 CEF_RESULT_CODE_NORMAL_EXIT_PROCESS_NOTIFIED  ← singleton relaunch
    //   36 CEF_RESULT_CODE_NORMAL_EXIT_PACK_EXTENSION_SUCCESS
    //   38 CEF_RESULT_CODE_NORMAL_EXIT_AUTO_DE_ELEVATED
    let init_result = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut cef_app),
        std::ptr::null_mut(),
    );
    if init_result != 1 {
        let exit_code = get_exit_code();
        // Sidecar was spawned before cef_initialize(); std::process::exit()
        // bypasses the normal shutdown block, so kill it here first.
        {
            let mut sidecar = app_state.sidecar_child.lock();
            if let Some(ref mut child) = *sidecar {
                tracing::info!("CEF early exit: killing backend sidecar before exit");
                let _ = child.kill();
            }
        }
        match exit_code {
            0 | 24 | 36 | 38 => {
                tracing::info!(
                    exit_code,
                    "CEF early exit (process singleton or similar) — exiting cleanly"
                );
                std::process::exit(0);
            }
            _ => {
                tracing::error!(
                    exit_code,
                    "CEF initialization failed; see ~/.agentmux/logs/cef-debug.log for details"
                );
                std::process::exit(exit_code);
            }
        }
    }

    tracing::info!("CEF initialized, entering message loop");

    // Start memory heartbeat — logs system/process memory stats every 20s.
    // Provides forensic data if the process later crashes from OOM / VA exhaustion.
    memory_heartbeat::start();

    // Phase B.6 (post-fix): publish port:token AFTER CEF init so a
    // second launcher only forwards `open_new_window` when we're
    // actually ready to handle it. Single-instance enforcement is
    // the launcher's named-pipe bind — this file is purely a
    // forwarding hint.
    let _ = std::fs::write(
        &port_file,
        format!("{}:{}", ipc_port, app_state.ipc_token),
    );

    // Phase B.9.1 (WRR) — install Win32 event hooks. Must come
    // AFTER `connect_to_launcher` so the report_hwnd_* sync APIs
    // have a live `COMMAND_TX` to push into; AFTER CEF init so
    // any HWNDs CEF creates during initialize() are missed
    // (acceptable — they predate the user's session and are
    // accounted for by main-window startup paths). Idempotent;
    // safe to call multiple times. State arg lets the callback
    // peek `pending_window_creations` for `label_hint`.
    wrr::install_hooks(app_state.clone());

    // Run the CEF message loop. This blocks until quit_message_loop() is called
    // (triggered when all browser windows are closed in client.rs).
    run_message_loop();

    tracing::info!("CEF message loop exited, shutting down");

    // Phase B.9.1 (WRR) — tear down Win32 event hooks before any
    // further teardown. UnhookWinEvent is cheap; doing it early
    // prevents stray callbacks during shutdown from racing the
    // launcher_ipc channel close.
    wrr::uninstall_hooks();

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

    // Phase B.6 (post-fix): clean up the forwarding hint so a stale
    // file doesn't survive a graceful exit. (Hard crashes will leave
    // it behind; harmless because pipe-bind on next launch is
    // authoritative — see comment at the port_file declaration.)
    let _ = std::fs::remove_file(&port_file);

    tracing::info!("AgentMux host shutdown complete");
}

/// Initialize tracing with dual output: rolling daily log file + human-readable stderr.
/// `log_dir` is resolved by the caller: `<portable-root>/data/logs/` in portable mode,
/// `~/.agentmux/logs/` in installed mode.
/// Returns a guard that must be held for the lifetime of the process to ensure log flushing.
fn init_logging(log_dir: &std::path::Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    let version = env!("CARGO_PKG_VERSION");
    let _ = std::fs::create_dir_all(log_dir);

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

    // Synchronous init sentinel: append a single line directly to the
    // expected log path BEFORE the tracing subscriber is wired up. Without
    // this, a hang between subscriber-setup and the non-blocking writer's
    // first flush leaves the pointer file pointing at a never-created log
    // file (observed 2026-05-02 freeze investigation). The sentinel
    // guarantees the file exists once init_logging has run past
    // pointer-write — if the file is missing afterwards, we know
    // init_logging itself didn't get past this point.
    let sentinel_path = log_dir.join(&current_filename);
    let sentinel_line = format!(
        "{} INIT-SENTINEL agentmux-host v={} pid={} os={} arch={}\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        version,
        std::process::id(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&sentinel_path)
    {
        use std::io::Write;
        let _ = f.write_all(sentinel_line.as_bytes());
        let _ = f.flush();
    }

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
        "AgentMux host starting"
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
