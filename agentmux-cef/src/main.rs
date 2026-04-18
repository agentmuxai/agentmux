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
mod browser_panes;
mod client;
mod commands;
mod events;
mod ipc;
mod memory_heartbeat;
mod pane;
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
    let port_file = data_dir.join("ipc-port");

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

    // macOS 26 Tahoe compat: patch NSApplication before CEF initializes.
    // Injects resolveInstanceMethod: to add typed stubs for private selectors
    // removed in macOS 26 (isHandlingSendEvent, setEffectiveAppearance:, etc.)
    // that CEF 146 calls during NSDraggingSession and event routing setup.
    #[cfg(target_os = "macos")]
    unsafe { patch_nsapp_unrecognized_selector() };

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
    // On macOS, pak files and locale paks live inside the CEF framework's
    // Resources/ directory — not alongside the executable. The framework is
    // at {exe_dir}/../Frameworks/ (resolved by library_loader above).
    // Pass that path for both resources_dir and locales_dir so CEF finds
    // chrome_*.pak, resources.pak, icudtl.dat, and the *.lproj/locale.pak files.
    #[cfg(target_os = "macos")]
    let (resources_dir, locales_dir) = {
        let fw_resources = host_exe_dir
            .join("../Frameworks/Chromium Embedded Framework.framework/Resources");
        let fw_resources = fw_resources.canonicalize().unwrap_or(fw_resources);
        let s = fw_resources.to_str().unwrap_or("").to_owned();
        (CefString::from(s.as_str()), CefString::from(s.as_str()))
    };
    #[cfg(not(target_os = "macos"))]
    let (resources_dir, locales_dir) = (
        CefString::from(base_dir.to_str().unwrap_or("")),
        CefString::from(base_dir.join("locales").to_str().unwrap_or("")),
    );

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
    // On macOS, tell CEF exactly where the framework lives so it can load ICU
    // and register the bundle correctly — required when running outside a .app.
    #[cfg(target_os = "macos")]
    let framework_dir = {
        let p = host_exe_dir.join("../Frameworks/Chromium Embedded Framework.framework");
        p.canonicalize().unwrap_or(p)
    };

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
        #[cfg(target_os = "macos")]
        framework_dir_path: CefString::from(framework_dir.to_str().unwrap_or("")),
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

    tracing::info!("AgentMux host shutdown complete");
}

/// macOS 26 Tahoe compat: CEF 146 calls private NSApplication selectors (e.g.
/// `isHandlingSendEvent`) during NSDraggingSession setup that were removed in macOS 26.
///
/// The correct fix is to hook `+[NSApplication resolveInstanceMethod:]` — the earliest
/// point in the ObjC dispatch chain — so missing selectors get a void stub before the
/// forwarding machinery (`___forwarding___`) is invoked. Swizzling `doesNotRecognizeSelector:`
/// is wrong here: that method is called FROM inside `___forwarding___`, and returning
/// normally from it (without throwing) corrupts the forwarding state and causes a second
/// crash inside `___forwarding___` itself.
///
/// Return-type-aware stubs: `isHandlingSendEvent` and similar BOOL guard getters must
/// return 0 (NO). A void stub leaves x0 = self (truthy), causing CEF to think the app
/// is already handling a send event and skip normal event routing — breaking window drag.
/// All other unknown selectors get a void stub, which is safe.
///
/// Safety: Called once before CEF initializes. NSApplication is a singleton; adding a
/// `resolveInstanceMethod:` implementation on its metaclass is safe at startup.
#[cfg(target_os = "macos")]
unsafe fn patch_nsapp_unrecognized_selector() {
    use std::ffi::{c_char, c_void};

    type Id  = *mut c_void;
    type Sel = *const c_void;
    type Class = *mut c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn object_getClass(obj: Id) -> Class;           // on a Class obj → returns metaclass
        fn sel_registerName(name: *const c_char) -> Sel;
        fn sel_getName(sel: Sel) -> *const c_char;
        fn class_addMethod(
            cls: Class,
            sel: Sel,
            imp: usize,
            types: *const c_char,
        ) -> u8; // BOOL
    }

    // Generic void stub for unknown selectors that return nothing (or whose
    // return value is not used by callers).
    unsafe extern "C" fn void_stub(_self: Id, _cmd: Sel) {}

    // BOOL stub returning 0 (NO) for guard-style getters. On ARM64, a void stub
    // leaves x0 = self (non-nil = truthy), which breaks callers like CEF's
    // sendEvent: guard that skips event routing when isHandlingSendEvent returns YES.
    unsafe extern "C" fn bool_no_stub(_self: Id, _cmd: Sel) -> u8 { 0 }

    // +resolveInstanceMethod: injected into NSApplication metaclass.
    // Called by the ObjC runtime the first time an unknown selector is sent to
    // an NSApplication instance — before ___forwarding___ is ever entered.
    // We add a typed stub and return YES so the runtime retries the send.
    unsafe extern "C" fn resolve_instance_method_impl(
        cls:  Class,
        _cmd: Sel,
        sel:  Sel,
    ) -> u8 {
        let name = {
            let ptr = sel_getName(sel);
            if ptr.is_null() { "<unknown>".to_owned() }
            else { std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned() }
        };

        // These selectors return BOOL and callers act on the value.
        // Returning truthy (garbage from a void stub) breaks event routing and
        // prevents window drag from receiving mouse events.
        const BOOL_NO_SELECTORS: &[&str] = &[
            "isHandlingSendEvent",
            "isSendingEvent",
        ];

        if BOOL_NO_SELECTORS.contains(&name.as_str()) {
            tracing::warn!(selector = %name, "macOS 26 compat: adding BOOL(NO) stub");
            class_addMethod(cls, sel, bool_no_stub as usize, b"c@:\0".as_ptr() as _);
        } else {
            tracing::warn!(selector = %name, "macOS 26 compat: adding void stub");
            class_addMethod(cls, sel, void_stub as usize, b"v@:\0".as_ptr() as _);
        }
        1 // YES — resolved; runtime retries the original send
    }

    let cls = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls.is_null() {
        tracing::warn!("macOS 26 compat: NSApplication class not found");
        return;
    }

    // The metaclass is the "class object" of a class; class methods live there.
    let metacls = object_getClass(cls as Id);
    if metacls.is_null() {
        tracing::warn!("macOS 26 compat: NSApplication metaclass not found");
        return;
    }

    let sel = sel_registerName(b"resolveInstanceMethod:\0".as_ptr() as _);
    // "c@::" = BOOL return, id (Class), SEL (cmd), SEL (queried selector)
    let added = class_addMethod(
        metacls,
        sel,
        resolve_instance_method_impl as usize,
        b"c@::\0".as_ptr() as _,
    );
    if added != 0 {
        tracing::info!("macOS 26 compat: injected resolveInstanceMethod: into NSApplication metaclass");
    } else {
        tracing::warn!("macOS 26 compat: class_addMethod failed (method already exists?)");
    }
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
