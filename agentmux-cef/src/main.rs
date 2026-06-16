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
mod drag_stash;
mod events;
mod ipc;
mod launcher_event_bridge;
mod launcher_ipc;
mod parent_process;
mod srv_event_bridge;
mod srv_ipc;
mod memory_heartbeat;
mod memory_pressure;
mod browser_pane;
#[cfg(target_os = "windows")]
mod floating_pane;
mod reducer;
mod saga_dispatch;
mod sidecar;
mod state;
mod ui_tasks;
mod wrr;
#[cfg(target_os = "macos")]
mod macos_menu;

use std::sync::Arc;

use cef::*;

/// Suppress the Windows "Application Error" / WER crash dialog so an unhandled
/// fault (a Chromium `LOG(FATAL)`, an `abort()`, a breakpoint) terminates the
/// process immediately instead of wedging it behind a modal the user must
/// dismiss. While that dialog is up the process is frozen and cannot be
/// auto-recovered. No-op off Windows. Spec:
/// docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md.
#[cfg(target_os = "windows")]
fn suppress_os_crash_dialogs() {
    use windows_sys::Win32::System::Diagnostics::Debug::{SetErrorMode, SEM_FAILCRITICALERRORS};
    use windows_sys::Win32::System::ErrorReporting::{WerSetFlags, WER_FAULT_REPORTING_NO_UI};
    // Process-wide; also covers the CEF subprocesses.
    unsafe {
        // Suppress the WER crash-dialog UI WITHOUT disabling WER itself —
        // SEM_NOGPFAULTERRORBOX would also kill WER/LocalDumps crash-dump
        // collection, the postmortem diagnostics this stability work needs.
        // WER_FAULT_REPORTING_NO_UI is the documented "no UI, keep
        // reports" path.
        let _ = WerSetFlags(WER_FAULT_REPORTING_NO_UI);
        // SEM_FAILCRITICALERRORS suppresses the critical-error handler
        // (e.g. "no disk in drive" popups) — unrelated to crash reporting.
        SetErrorMode(SEM_FAILCRITICALERRORS);
    }
}

#[cfg(not(target_os = "windows"))]
fn suppress_os_crash_dialogs() {}

/// Resolve CEF's `browser_subprocess_path` (the executable CEF spawns for
/// renderer / GPU / utility processes).
///
/// On a packaged macOS `.app`, return the dedicated `AgentMux Helper`
/// executable inside `Contents/Frameworks/AgentMux Helper.app` — re-execing
/// the main bundle binary for subprocesses is rejected by the macOS process
/// model (every child would inherit the main bundle's identity), which makes
/// the renderers crash-loop. When no Helper.app is present (dev: a bare binary
/// with no bundle) fall back to the current exe — self-reexec works there.
/// Windows/Linux always use the current exe (self-reexec is fine off-bundle).
#[cfg(target_os = "macos")]
fn resolve_browser_subprocess_path() -> String {
    let exe = std::env::current_exe().unwrap();
    // exe = AgentMux.app/Contents/MacOS/agentmux-cef
    // → AgentMux.app/Contents/Frameworks/AgentMux Helper.app/Contents/MacOS/AgentMux Helper
    if let Some(contents) = exe.parent().and_then(|macos| macos.parent()) {
        let helper = contents
            .join("Frameworks")
            .join("AgentMux Helper.app")
            .join("Contents")
            .join("MacOS")
            .join("AgentMux Helper");
        if helper.exists() {
            return helper.to_string_lossy().into_owned();
        }
    }
    exe.to_string_lossy().into_owned()
}

#[cfg(not(target_os = "macos"))]
fn resolve_browser_subprocess_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_owned()))
        .unwrap_or_default()
}

/// True when a launcher is THIS host's actual parent process this run.
///
/// The launcher (which spawns the host directly) stamps
/// `AGENTMUX_LAUNCHER_PID` with its own pid; since we are its direct
/// child, that pid equals our `getppid()`. This distinguishes a fresh,
/// authoritative launcher hand-off from a STALE `AGENTMUX_BACKEND_WS`
/// inherited down the environment from a parent agentmux pane (whose
/// launcher pid will not match our real parent). Used only to relax the
/// dev-build "ignore the env hand-off" rule for the macOS/Linux Phase 1
/// launcher dev integration.
///
/// Unix-only — on other platforms it returns `false`, leaving the
/// existing dev-build behavior unchanged.
#[cfg(unix)]
fn launcher_is_genuine_parent() -> bool {
    match std::env::var("AGENTMUX_LAUNCHER_PID")
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
    {
        // SAFETY: getppid() takes no arguments, touches no memory, and is
        // documented as always succeeding.
        Some(pid) => pid == unsafe { libc::getppid() },
        None => false,
    }
}

#[cfg(not(unix))]
fn launcher_is_genuine_parent() -> bool {
    false
}

fn main() {
    // Phase 0 (service supervision & recovery): suppress the Windows crash
    // modal so a fault terminates the process immediately instead of freezing
    // it behind an "Application Error" dialog. Must be the first statement —
    // set before anything can fault.
    suppress_os_crash_dialogs();

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
        let exe = std::env::current_exe().unwrap();
        // Inside a packaged .app, renderer/GPU/utility subprocesses run as the
        // bundled "AgentMux Helper" at
        // …/Contents/Frameworks/AgentMux Helper.app/Contents/MacOS/AgentMux Helper
        // — 4 levels below the framework — so it must resolve the framework via
        // the deeper `../../../../Frameworks/…` path (helper=true). The main
        // host (Contents/MacOS/) and the bare dev binary use `../Frameworks/…`
        // (helper=false). Detect the helper by its exe path; the main bundle
        // binary is never under Contents/Frameworks/. See
        // docs/specs/SPEC_MACOS_PACKAGING_2026_05_30.md.
        let is_helper = exe.to_string_lossy().contains(".app/Contents/Frameworks/");
        let loader = library_loader::LibraryLoader::new(&exe, is_helper);
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

    // Read paths + mode from the launcher-injected env vars. Two
    // reachable configurations:
    //   a) Launcher-managed startup → env vars present, from_env()
    //      returns Some.
    //   b) Standalone `task dev` → env absent. We re-derive via
    //      `RuntimeMode::current` + `DataPaths::resolve` (symmetric
    //      with sidecar.rs::spawn_backend's fallback so they agree on
    //      the disk layout).
    let host_exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    // Dev builds NEVER inherit AGENTMUX_* env vars from a parent process.
    // `task dev` is routinely launched from inside an AgentMux terminal
    // pane, which means the child host inherits the parent instance's
    // AGENTMUX_DATA_DIR pointing at the parent's version-isolated dir.
    // Without this guard the dev build would resolve its data dir to the
    // running portable's path and trip CEF's process-singleton lock,
    // routing every "open" back to the existing window — the user would
    // never see the dev code run. Path-based detection is authoritative
    // for dev builds; for installed/portable we still honor the
    // launcher-provided env (it's the launcher's job to publish them).
    let common_paths = if agentmux_common::is_dev_build_exe(&host_exe_dir) {
        let mode = agentmux_common::RuntimeMode::current_path_only(&host_exe_dir);
        // resolve_path_only mirrors current_path_only's env-isolation:
        // ignore inherited AGENTMUX_CHANNEL so a dev host launched from
        // inside a parent agentmux instance doesn't redirect into the
        // parent's channel (would trip the channel single-instance lock).
        agentmux_common::DataPaths::resolve_path_only(version, &mode).ok()
    } else {
        agentmux_common::DataPaths::from_env().or_else(|| {
            let mode = agentmux_common::RuntimeMode::current(&host_exe_dir);
            agentmux_common::DataPaths::resolve(version, &mode).ok()
        })
    };
    let is_dev = match &common_paths {
        Some(p) => matches!(p.mode, agentmux_common::RuntimeMode::Dev { .. }),
        None => false,
    };

    let (data_dir, log_dir) = match &common_paths {
        Some(p) => (p.cef_cache_dir.clone(), p.logs_dir.clone()),
        None => {
            // Both env-read AND fallback resolution failed (no home
            // dir on disk, or platform unsupported). Use a degraded
            // path so log init at least works; the runtime-startup
            // check below will surface the underlying error.
            (
                std::path::PathBuf::from("."),
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".agentmux")
                    .join("logs"),
            )
        }
    };
    std::fs::create_dir_all(&data_dir).ok();

    // Initialize dual-output tracing: rolling log file + stderr.
    // The log file guard must live for the entire process to ensure flushing.
    let _log_guard = init_logging(&log_dir);

    tracing::info!(
        version,
        runtime_mode = ?common_paths.as_ref().map(|p| p.mode.to_env_string()),
        data_dir = %data_dir.display(),
        log_dir = %log_dir.display(),
        "Initializing CEF browser process"
    );

    // macOS 26 Tahoe compat: CEF 146 calls private NSApplication selectors
    // (e.g. `isHandlingSendEvent`, `isSendingEvent`) during NSDraggingSession
    // setup. Apple removed them in macOS 26, so the calls go through
    // `___forwarding___`, find nothing, and `objc_exception_throw` fires —
    // AppKit's default uncaught-exception handler then calls `_objc_terminate`
    // and the host dies (EXC_BREAKPOINT / SIGTRAP on CrBrowserMain). Inject
    // `+[NSApplication resolveInstanceMethod:]` to add typed stubs *before*
    // the forwarding machinery is entered. Must run before `cef::initialize`.
    // See docs/specs/SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md and PR #403.
    #[cfg(target_os = "macos")]
    unsafe { patch_nsapp_unrecognized_selector() };

    // macOS tear-off polish: suppress AppKit's native drag slide-back
    // ("poof" / fly-back-to-origin) animation. When a pane/tab is torn
    // off, the pointer is released OUTSIDE any DOM drop target (the new
    // floating window is created on mouseup), so blink hands AppKit
    // NSDragOperationNone and AppKit animates the drag image sliding
    // back into the source window — the very "rejection" animation the
    // tear-off is trying to avoid. `preventUnhandled` (PR #1186) only
    // covers in-document drops, not this out-of-window case. Disable the
    // session-level animation flag globally; we never want a
    // drop-rejected slide-back anywhere in the app.
    #[cfg(target_os = "macos")]
    unsafe { disable_macos_drag_slideback() };

    // Name the app early (sets CFBundleName on the main bundle, which AppKit
    // may read when it first builds a menu). Also re-run post-init below, since
    // Chromium can reset the process name during cef::initialize.
    #[cfg(target_os = "macos")]
    unsafe { set_macos_app_display_name() };

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
    // Dev builds inherit AGENTMUX_DATA_DIR from the parent pane they were
    // launched from. Writing ipc-port there would overwrite the parent
    // instance's port:token and break its single-instance forwarding.
    // In dev mode there is no launcher so port forwarding isn't wired
    // anyway — use the dev data dir directly.
    let port_file_dir = if agentmux_common::is_dev_build_exe(&host_exe_dir) {
        data_dir.clone()
    } else {
        std::env::var_os("AGENTMUX_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| data_dir.clone())
    };
    let _ = std::fs::create_dir_all(&port_file_dir);
    // Use a version-scoped filename so two concurrent release versions
    // don't overwrite each other's port file (codex P1 on #1227).
    // AGENTMUX_IPC_HASH is set by the launcher to hash(data_dir, version).
    let port_file_name = std::env::var("AGENTMUX_IPC_HASH")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| format!("ipc-port-{}", h))
        .unwrap_or_else(|| "ipc-port".to_string());
    let port_file = port_file_dir.join(&port_file_name);

    // Create shared application state.
    let app_state = Arc::new(state::AppState::default());

    // Start tokio runtime for async operations (IPC server, sidecar management).
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    // Install the runtime Handle into browser_pane::auth so the
    // CEF `get_auth_credentials` callback (which runs on CEF's IO
    // thread) can spawn the parked-auth TTL timer. A bare
    // `tokio::spawn` there would panic with "there is no reactor
    // running" because that thread has no `Handle::current()`.
    browser_pane::auth::set_runtime_handle(runtime.handle().clone());

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
    //
    // Env-isolation guard: a dev build inheriting
    // `AGENTMUX_LAUNCHER_PIPE` from a parent AgentMux pane (e.g. a
    // shell inside an active pane that re-invokes the host directly)
    // would otherwise connect to that parent's launcher pipe and
    // route its host events into the parent's launcher state.
    //
    // Discriminator: connect when our parent process IS the launcher
    // (production portable, installed build, OR post-#SPEC_LAUNCHER_DEV_INTEGRATION
    // `task dev` which spawns the host via the launcher). Skip when
    // it isn't.
    //
    // Older path-only guard (`is_dev_build_exe`) over-fired in dev
    // mode after launcher integration shipped — see
    // docs/specs/SPEC_DEV_MODE_LAUNCHER_IPC_2026_05_16.md.
    let parent_is_launcher = parent_process::parent_is_agentmux_launcher();
    let should_connect_launcher = match parent_is_launcher {
        Some(true) => true,
        Some(false) => false,
        // Parent detection failed — fall back to the path-based guard
        // so production builds still connect (they would otherwise
        // silently lose the launcher IPC) and dev builds still skip.
        None => !agentmux_common::is_dev_build_exe(&host_exe_dir),
    };
    let _launcher_ipc = if should_connect_launcher {
        runtime.block_on(launcher_ipc::connect_to_launcher(app_state.clone()))
    } else {
        None
    };

    // Phase E.2c.5a — connect to the srv reducer's pipe. Forwards
    // srv events (workspace / tab / block lifecycle) to every
    // top-level renderer via the JS bridge. Renderer-side handler
    // (`window.__agentmux_srv_event`) lands in E.2c.5b. Non-fatal
    // if absent: `AGENTMUX_SRV_PIPE_PATH` is only set on the srv
    // child by the launcher (`agentmux-launcher/src/srv_spawner.rs`),
    // not on the host spawn — so today the host never has the env
    // var and `connect_to_srv` short-circuits to None at
    // `srv_ipc.rs:62-68`. Path-based dev guard is the right gate
    // for this branch; restoring full srv-IPC parity in dev needs
    // the launcher to propagate the env var to the host first.
    // See spec §11 of SPEC_DEV_MODE_LAUNCHER_IPC_2026_05_16.md.
    let _srv_ipc = if agentmux_common::is_dev_build_exe(&host_exe_dir) {
        None
    } else {
        runtime.block_on(srv_ipc::connect_to_srv(app_state.clone()))
    };

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
        // Dev builds inherit AGENTMUX_BACKEND_WS from the parent pane.
        // Consuming it would connect to the parent's srv instead of
        // spawning our own, so the dev frontend runs against the wrong
        // (parent's) backend and no dev-version srv is ever started.
        //
        // EXCEPTION (macOS/Linux Phase 1 launcher dev integration,
        // SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30): when a launcher
        // is our GENUINE parent this run (it stamped AGENTMUX_LAUNCHER_PID
        // with its pid == our getppid), the env it set is fresh + ours —
        // adopt its launcher-owned srv instead of double-spawning. The
        // getppid match can't be satisfied by a value merely inherited
        // down the env from a parent pane, so `task dev:standalone`
        // (direct host invoke) still spawns its own srv.
        let launcher_provided = if agentmux_common::is_dev_build_exe(&host_exe_dir)
            && !launcher_is_genuine_parent()
        {
            None
        } else {
            sidecar::use_launcher_endpoints(&app_state)
        };
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
    // Write authkey.dev for ALL runtime modes (dev, portable, installed).
    // The file lets bench-term-echo.mjs and the PowerShell test harnesses
    // discover the running instance without manual --ws-url / --auth-key flags.
    // Security: the WS server is loopback-only; any same-user process already
    // has equivalent TCP access. See SPEC_TEST_API_ACCESS.md §3 and
    // SPEC_BENCHMARK_PORTABLE_DISCOVERY_2026_05_20.md for rationale.
    {
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
            Ok(p) => tracing::info!("Wrote authkey file: {}", p.display()),
            Err(e) => tracing::warn!("Failed to write authkey file: {}", e),
        }
    }

    // Create the App handler with state.
    let mut cef_app = app::AgentMuxApp::new(app_state.clone(), ipc_port);

    // Resolve resource directories for portable layout. In portable
    // mode the CEF host is IN runtime/, so resources are flat
    // alongside it. In dev mode they are also flat in dist/cef-dev/.
    // Reuses `host_exe_dir` from the startup mode-detection block.
    //
    // macOS is the exception: Chromium ships icudtl.dat + the .pak
    // files + locales inside the framework bundle's `Resources/`
    // directory, not flat next to the binary. cef-rs's LibraryLoader
    // already finds the framework via `../Frameworks/...`; we point
    // CefSettings at the same place so Chromium's icu_util loader,
    // pack loader, and locale loader resolve correctly. See
    // docs/specs/SPEC_MACOS_CEF_FRAMEWORK_BUNDLING_2026_05_28.md.
    let runtime_dir = host_exe_dir.join("runtime");
    let base_dir = if runtime_dir.exists() {
        runtime_dir
    } else {
        host_exe_dir.clone()
    };
    #[cfg(not(target_os = "macos"))]
    let (resources_path, locales_path) = (base_dir.clone(), base_dir.join("locales"));
    #[cfg(target_os = "macos")]
    let (framework_path, resources_path, locales_path) = {
        let framework = base_dir
            .join("..")
            .join("Frameworks")
            .join("Chromium Embedded Framework.framework");
        let resources = framework.join("Resources");
        (framework, resources.clone(), resources)
    };
    let resources_dir = CefString::from(resources_path.to_str().unwrap_or(""));
    let locales_dir = CefString::from(locales_path.to_str().unwrap_or(""));
    #[cfg(target_os = "macos")]
    let framework_dir = CefString::from(framework_path.to_str().unwrap_or(""));

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
        // ARGB: alpha=0 → SK_AlphaTRANSPARENT → triggers the transparency
        // cascade in the patched libcef.so (see cef commits b921ffe18 +
        // 68e0dc668). The CSS layer's rgba(_,_,_,<1) body bg then composites
        // with the desktop instead of being clamped to opaque white.
        // Pair: BrowserSettings.background_color must also be 0 (app.rs).
        // Pair: WindowDelegate must return is_frameless=true (already does
        // for the main window).
        // Spec: docs/research/cef-transparency-research-2026-05-10.md.
        background_color: 0x00000000,
        remote_debugging_port: debug_port as i32,
        root_cache_path: cache_dir,
        resources_dir_path: resources_dir,
        locales_dir_path: locales_dir,
        // macOS-only: tell CEF where the framework bundle lives so it can
        // resolve icudtl.dat, *.pak, and helper-process binaries through
        // NSBundle. Without this, Chromium's icu_util loader looks via
        // [NSBundle mainBundle] (the host exe's pseudo-bundle) which has
        // no Resources/ and fails with "icudtl.dat not found in bundle".
        #[cfg(target_os = "macos")]
        framework_dir_path: framework_dir,
        log_file: cef_log_file,
        log_severity: LogSeverity::INFO,
        // CEF subprocess (renderer, GPU, utility) executable. On a packaged
        // macOS .app this is the dedicated "AgentMux Helper" (distinct bundle
        // id, LSUIElement) — re-execing the main bundle binary is rejected by
        // the macOS process model and the renderers crash-loop. In dev (bare
        // binary, no Helper.app) and on Windows/Linux it's the current exe
        // (self-reexec, which works there). See
        // docs/specs/SPEC_MACOS_PACKAGING_2026_05_30.md.
        browser_subprocess_path: CefString::from(resolve_browser_subprocess_path().as_str()),
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
                // Surface a user-facing error instead of a silent splash-then-exit.
                // The most common cause is a bundled CEF runtime whose version does
                // not match the linked `cef` crate (e.g. a stale libcef.dll) — that
                // path logs "Request for unsupported CEF API version NNNNN" to
                // cef-debug.log and would otherwise just vanish after the splash.
                // See docs/specs/SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03.md.
                #[cfg(target_os = "windows")]
                {
                    use windows_sys::Win32::UI::WindowsAndMessaging::{
                        MessageBoxW, MB_ICONERROR, MB_OK,
                    };
                    let title: Vec<u16> =
                        "AgentMux — startup failed\0".encode_utf16().collect();
                    let body = format!(
                        "AgentMux couldn't start its browser engine (CEF init failed, code {}).\n\n\
                         This usually means the bundled browser runtime is incompatible with this \
                         build — for example a stale or mismatched libcef.dll from an incomplete build.\n\n\
                         Details were written to:\n    %USERPROFILE%\\.agentmux\\logs\\cef-debug.log\n\n\
                         If you built this locally, run:  task clean:cef && task build:host",
                        exit_code
                    );
                    let body_w: Vec<u16> =
                        body.encode_utf16().chain(std::iter::once(0)).collect();
                    // SAFETY: null parent HWND with valid NUL-terminated wide strings.
                    unsafe {
                        MessageBoxW(
                            std::ptr::null_mut(),
                            body_w.as_ptr(),
                            title.as_ptr(),
                            MB_OK | MB_ICONERROR,
                        );
                    }
                }
                std::process::exit(exit_code);
            }
        }
    }

    tracing::info!("CEF initialized, entering message loop");

    // Claim a Dock tile, THEN paint the icon onto it. A bare Mach-O launched
    // outside an `.app` bundle (both `task dev` direct-invoke and the launcher
    // flat layout) defaults to a background/accessory activation policy — no
    // Dock tile, no menu bar — so the AgentMux instance never shows in the
    // taskbar and the icon set below has nothing to land on. Force
    // NSApplicationActivationPolicyRegular first so the instance is a normal,
    // Dock-visible app. macOS-only; no-op elsewhere. Order matters: policy
    // before icon.
    #[cfg(target_os = "macos")]
    unsafe {
        // Friendly Dock + app-menu name ("AgentMux DEV" in dev) instead of the
        // raw process name "agentmux-cef". Done POST-init: Chromium overwrites
        // the process name during cef::initialize, so a pre-init set didn't
        // stick — set it here, right before the activation policy + our menu
        // bar are established.
        set_macos_app_display_name();
        set_macos_activation_policy_regular();
        set_macos_dock_icon();
        // Layer 1 of SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03: govern
        // Chromium's macOS accessibility activation so a window manager / KVM
        // poking the AX tree (Magnet, Synergy, …) can't force the crash-prone
        // web-content AX mode (CEF #3512). Must run after cef::initialize so
        // the CEF NSApplication subclass (which owns the legacy AX setter)
        // exists.
        install_macos_accessibility_governor();
    }

    // Native macOS menu bar (File/Edit/View/Window/Help) — Phase 1 of
    // SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03. After cef::initialize (NSApplication
    // exists) and after set_macos_app_display_name (the app-menu title follows
    // the process name). Standard Edit/Window items route to the focused web
    // view; custom items dispatch through the frontend command registry.
    #[cfg(target_os = "macos")]
    macos_menu::install_menu_bar(app_state.clone());

    // Start memory heartbeat — logs system/process memory stats every 20s.
    // Provides forensic data if the process later crashes from OOM / VA
    // exhaustion, and drives the debounced mem_pressure level + low-memory
    // banner (SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.A/§5.F).
    memory_heartbeat::start(app_state.clone());

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

    // Phase B.6 (post-fix): clean up the forwarding hint so a stale
    // file doesn't survive a graceful exit. (Hard crashes will leave
    // it behind; harmless because pipe-bind on next launch is
    // authoritative — see comment at the port_file declaration.)
    let _ = std::fs::remove_file(&port_file);

    // `drop(runtime)` blocks forever here on macOS after the last window
    // closes: the multi-thread runtime's drop waits for every background
    // task/blocking thread to finish, and at least one `spawn_blocking`
    // (a pipe/PTY reader) is parked on a blocking read that never returns.
    // The window is already destroyed at this point, so the user sees it
    // close — but the host wedges in the runtime drop and never exits, and
    // the launcher (which waits for the host to exit) wedges with it → the
    // instance "stays open hidden". Reaching this code at all means the CEF
    // message loop returned, which only happens on the intended
    // LastWindowClosed quit — so a hard exit here is correct, not a crash.
    //
    // `shutdown_background()` initiates runtime teardown without blocking;
    // then exit the process explicitly so a parked blocking thread can't
    // keep it resident. macOS-only; other platforms keep the plain drop.
    #[cfg(target_os = "macos")]
    {
        runtime.shutdown_background();
        tracing::info!("AgentMux host shutdown complete (fast exit)");
        std::process::exit(0);
    }
    #[cfg(not(target_os = "macos"))]
    {
        drop(runtime);
        tracing::info!("AgentMux host shutdown complete");
    }
}

/// macOS 26 Tahoe compatibility shim.
///
/// CEF 146 calls private `NSApplication` selectors (e.g. `isHandlingSendEvent`,
/// `isSendingEvent`) during `NSDraggingSession` setup that Apple removed in
/// macOS 26. Without a handler the ObjC runtime walks `___forwarding___`,
/// finds nothing, and `objc_exception_throw`s; AppKit's default uncaught
/// handler calls `_objc_terminate()` and the host dies with `EXC_BREAKPOINT`
/// before Rust panic machinery runs.
///
/// We hook `+[NSApplication resolveInstanceMethod:]` on the metaclass —
/// the earliest point in the ObjC dispatch chain — and install typed stubs
/// for any unknown selector *before* the forwarding machinery is entered.
/// Swizzling `doesNotRecognizeSelector:` would not work: that method is
/// invoked FROM inside `___forwarding___`, and returning normally without
/// throwing corrupts forwarding state and causes a secondary crash there.
///
/// Return-type matters: `isHandlingSendEvent` and `isSendingEvent` return
/// `BOOL` and callers act on the value. A `void` stub leaves `x0 = self`
/// (truthy) on ARM64, making CEF think the app is already inside a
/// `sendEvent:` call and skip normal event routing — which breaks window
/// drag silently. A maintained allowlist of `BOOL`-returning selectors
/// gets a `BOOL_no_stub` returning `0` (NO); everything else gets a void
/// stub, which is safe for the unbounded set of removed Apple-private APIs.
///
/// Safety: Called once, before `cef::initialize`. `NSApplication` is a
/// singleton; adding a `+resolveInstanceMethod:` implementation on its
/// metaclass at startup is documented Apple behavior. No allocations, no
/// crossings of language boundaries that hold Rust references.
///
/// Ported from PR #403 (a5af, 2026-04-15) with rationale comments expanded.
/// See `docs/specs/SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md` and
/// `docs/analysis/REPORT_MACOS_TEAROFF_DRAG_CRASH_2026_05_29.md`.
#[cfg(target_os = "macos")]
unsafe fn patch_nsapp_unrecognized_selector() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn object_getClass(obj: Id) -> Class; // on a Class obj → returns metaclass
        fn sel_registerName(name: *const c_char) -> Sel;
        fn sel_getName(sel: Sel) -> *const c_char;
        fn class_addMethod(cls: Class, sel: Sel, imp: usize, types: *const c_char) -> u8;
    }

    // Void stub — safe for unknown selectors whose return value isn't read.
    unsafe extern "C" fn void_stub(_self: Id, _cmd: Sel) {}

    // BOOL stub returning 0 (NO). On ARM64 the return value lives in `x0`;
    // a void stub leaves `x0 = self` (truthy), breaking CEF's sendEvent: guard.
    unsafe extern "C" fn bool_no_stub(_self: Id, _cmd: Sel) -> u8 { 0 }

    // +resolveInstanceMethod: injected onto NSApplication's metaclass.
    // The ObjC runtime calls us the first time a selector is sent to an
    // NSApplication instance that has no implementation. We `class_addMethod`
    // a typed stub and return YES; the runtime retries the original message
    // against the freshly added method.
    unsafe extern "C" fn resolve_instance_method_impl(
        cls:  Class,
        _cmd: Sel,
        sel:  Sel,
    ) -> u8 {
        let name = {
            let ptr = sel_getName(sel);
            if ptr.is_null() {
                "<unknown>".to_owned()
            } else {
                std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };

        // BOOL-returning getters whose value callers act on. The truthy
        // garbage a void stub would leave in x0 breaks event routing.
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

    // +resolveInstanceMethod: is a class method; it lives on the metaclass.
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

/// Suppress AppKit's native drag slide-back animation app-wide.
///
/// When an `NSDraggingSession` ends without a successful drop (the drag
/// operation resolves to `NSDragOperationNone`), AppKit animates the drag
/// image sliding back to where the drag began. For a pane/tab tear-off the
/// pointer is released outside any DOM drop target — the floating window is
/// created on mouseup — so blink reports `NSDragOperationNone` and the user
/// sees the drag image fly back into the source window before the new
/// window appears. That "rejection" animation is exactly what tear-off
/// wants gone; the frontend `preventUnhandled` guard (PR #1186) only
/// suppresses the WebKit-level snapback for *in-document* drops and can't
/// reach this AppKit-level animation.
///
/// `NSDraggingSession` exposes `animatesToStartingPositionsOnCancelOrFail`
/// (default `YES`) to control exactly this. CEF/Chromium starts every drag
/// via `-[NSView beginDraggingSessionWithItems:event:source:]` and never
/// flips the flag, so we swizzle that method: call the original, then set
/// the flag to `NO` on the returned session. Done at the `NSView` level so
/// it covers whichever Chromium content view initiates the drag. The flag
/// only affects the cancel/fail slide-back — successful in-window drops
/// (e.g. tab reorder) are unaffected — so disabling it globally is safe;
/// there is no place in the app where a drop-rejected slide-back is wanted.
///
/// Safety: called once at startup, before `cef::initialize`, on the main
/// thread. Mirrors the raw-libobjc FFI of `patch_nsapp_unrecognized_selector`.
#[cfg(target_os = "macos")]
unsafe fn disable_macos_drag_slideback() {
    use std::ffi::{c_char, c_void};

    type Id     = *mut c_void;
    type Sel    = *const c_void;
    type Class  = *mut c_void;
    type Method = *mut c_void;
    type Imp    = *const c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn class_getInstanceMethod(cls: Class, sel: Sel) -> Method;
        fn method_getImplementation(m: Method) -> Imp;
        fn method_setImplementation(m: Method, imp: Imp) -> Imp;
        fn objc_msgSend();
    }

    // IMP of the original beginDraggingSessionWithItems:event:source:, saved
    // so our replacement can chain to it. Single-threaded startup write +
    // main-thread-only drag reads, so a plain static is sufficient.
    static mut ORIGINAL_BEGIN_DRAG: Imp = std::ptr::null();

    // Replacement IMP: call the original to create the session, then clear
    // the slide-back flag on it before returning.
    unsafe extern "C" fn begin_drag_no_slideback(
        this:   Id,
        cmd:    Sel,
        items:  Id,
        event:  Id,
        source: Id,
    ) -> Id {
        let orig: extern "C" fn(Id, Sel, Id, Id, Id) -> Id =
            std::mem::transmute(ORIGINAL_BEGIN_DRAG);
        let session = orig(this, cmd, items, event, source);
        if !session.is_null() {
            // [session setAnimatesToStartingPositionsOnCancelOrFail:NO]
            let sel = sel_registerName(
                b"setAnimatesToStartingPositionsOnCancelOrFail:\0".as_ptr() as _,
            );
            let set_flag: extern "C" fn(Id, Sel, u8) =
                std::mem::transmute(objc_msgSend as *const c_void);
            set_flag(session, sel, 0); // NO
        }
        session
    }

    let cls = objc_getClass(b"NSView\0".as_ptr() as _);
    if cls.is_null() {
        tracing::warn!("drag slide-back: NSView class not found");
        return;
    }
    let sel = sel_registerName(
        b"beginDraggingSessionWithItems:event:source:\0".as_ptr() as _,
    );
    let method = class_getInstanceMethod(cls, sel);
    if method.is_null() {
        tracing::warn!("drag slide-back: beginDraggingSessionWithItems:event:source: not found");
        return;
    }
    ORIGINAL_BEGIN_DRAG = method_getImplementation(method);
    method_setImplementation(method, begin_drag_no_slideback as Imp);
    tracing::info!(
        "macOS drag polish: swizzled NSView beginDraggingSession to disable cancel/fail slide-back"
    );
}

/// Set the macOS app display name (Dock tile + app-menu title).
///
/// A bundle-less binary (`task dev` direct-invoke, the launcher's flat
/// `dist/cef-dev/` layout) has no `Info.plist`, so AppKit derives the app name
/// from the process name — `agentmux-cef` — which is what shows in the Dock and
/// the menu bar's app menu. Override it with a friendly name: **dev** builds get
/// `AgentMux DEV` (so they're visibly distinct from a packaged `AgentMux` and
/// from each other when several instances run), everything else gets `AgentMux`.
/// A packaged `.app` carries `CFBundleName` in its `Info.plist`; this still runs
/// and simply matches it.
///
/// Uses `-[NSProcessInfo setProcessName:]`, which AppKit reads for the Dock
/// label and the app-menu title. Must run AFTER `cef::initialize` — Chromium
/// overwrites the process name during init, so a pre-init set is clobbered;
/// setting it here (right before our menu bar is built) is what sticks. Raw
/// libobjc FFI, mirroring the other macOS shims.
#[cfg(target_os = "macos")]
unsafe fn set_macos_app_display_name() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    // Resolve dev vs. not by the exe PATH (`is_dev_self`), matching
    // `commands::platform::get_is_dev` and the menu name in `macos_menu.rs`.
    // NOT `AGENTMUX_RUNTIME_MODE`: a parent dev AgentMux leaks that env into
    // descendants, which would otherwise set the Dock / app-menu process name
    // to "AgentMux DEV" on a packaged build launched from inside a dev
    // instance. Build identity is a property of the binary on disk.
    let name = if agentmux_common::is_dev_self() { "AgentMux DEV" } else { "AgentMux" };

    // NSString *ns = [NSString stringWithUTF8String:name]
    let cls_str = objc_getClass(b"NSString\0".as_ptr() as _);
    if cls_str.is_null() {
        return;
    }
    let sel_with = sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _);
    let make: extern "C" fn(Class, Sel, *const c_char) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let cname = match std::ffi::CString::new(name) {
        Ok(c) => c,
        Err(_) => return,
    };
    let ns_name = make(cls_str, sel_with, cname.as_ptr());
    if ns_name.is_null() {
        return;
    }

    // pi = [NSProcessInfo processInfo]
    let cls_pi = objc_getClass(b"NSProcessInfo\0".as_ptr() as _);
    if cls_pi.is_null() {
        return;
    }
    let sel_pi = sel_registerName(b"processInfo\0".as_ptr() as _);
    let get_pi: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let pi = get_pi(cls_pi, sel_pi);
    if pi.is_null() {
        return;
    }

    // [pi setProcessName:ns_name]
    let sel_set = sel_registerName(b"setProcessName:\0".as_ptr() as _);
    let set_name: extern "C" fn(Id, Sel, Id) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_name(pi, sel_set, ns_name);

    // The app-menu title + Dock label for an unbundled binary come from the
    // main bundle's CFBundleName/CFBundleDisplayName, NOT the process name
    // (setProcessName above doesn't move them). Set them on the main bundle's
    // info dictionary, which is backed by a mutable dictionary. Guard on
    // isKindOfClass:NSMutableDictionary so an unexpected immutable dict is a
    // skip rather than a throw (an uncaught ObjC exception would terminate the
    // host on macOS 26).
    let cls_bundle = objc_getClass(b"NSBundle\0".as_ptr() as _);
    if !cls_bundle.is_null() {
        let sel_main = sel_registerName(b"mainBundle\0".as_ptr() as _);
        let get_main: extern "C" fn(Class, Sel) -> Id =
            std::mem::transmute(objc_msgSend as *const c_void);
        let bundle = get_main(cls_bundle, sel_main);
        if !bundle.is_null() {
            let sel_info = sel_registerName(b"infoDictionary\0".as_ptr() as _);
            let get_info: extern "C" fn(Id, Sel) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            let info = get_info(bundle, sel_info);
            let cls_mut = objc_getClass(b"NSMutableDictionary\0".as_ptr() as _);
            let sel_kind = sel_registerName(b"isKindOfClass:\0".as_ptr() as _);
            let is_kind: extern "C" fn(Id, Sel, Class) -> u8 =
                std::mem::transmute(objc_msgSend as *const c_void);
            if !info.is_null() && !cls_mut.is_null() && is_kind(info, sel_kind, cls_mut) != 0 {
                let sel_set_obj = sel_registerName(b"setObject:forKey:\0".as_ptr() as _);
                let set_obj: extern "C" fn(Id, Sel, Id, Id) =
                    std::mem::transmute(objc_msgSend as *const c_void);
                let k_name = make(cls_str, sel_with, b"CFBundleName\0".as_ptr() as _);
                let k_disp = make(cls_str, sel_with, b"CFBundleDisplayName\0".as_ptr() as _);
                set_obj(info, sel_set_obj, ns_name, k_name);
                set_obj(info, sel_set_obj, ns_name, k_disp);
                tracing::info!("macOS: set CFBundleName/CFBundleDisplayName on main bundle");
            } else {
                tracing::warn!("macOS: main bundle info dict not mutable; app name unchanged");
            }
        }
    }

    tracing::info!(app_name = name, "macOS: set app display name (Dock + app menu)");
}

/// macOS accessibility activation governor — Layer 1 of
/// `docs/specs/SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03.md`.
///
/// Chromium enables its web-content accessibility tree the moment an AX client
/// sets `AXEnhancedUserInterface` on the application. That tree's macOS
/// implementation faults under external iteration on macOS-26 / CEF M114+
/// (CEF #3512: `AXPlatformNodeCocoa::AXChildren()` SEGV; here it surfaced as an
/// `EXC_BREAKPOINT` through the legacy `NSAccessibility…` accessor when a user
/// clicked the title-bar menu). The trigger attribute is **overloaded**:
/// VoiceOver sets it, but so do ordinary window managers / KVMs — Magnet,
/// Synergy — see Firefox bug 1664992. So a window manager merely doing its job
/// forces AgentMux into the crash-prone full-AX mode.
///
/// This swizzles the application's legacy `accessibilitySetValue:forAttribute:`
/// and applies a policy:
///   * `AXEnhancedUserInterface` (the window-manager/KVM path) does **not**
///     auto-enable web-content AX — unless `AGENTMUX_A11Y_HONOR_ENHANCED=1`.
///   * `AXManualAccessibility` (explicit assistive-technology / app intent —
///     the path Electron documents for enabling AX) **is** honored.
///   * every set is logged so the real activation path is observable in the
///     field (this is also how we confirm the fix against Magnet/Synergy).
///
/// Window-level AX (windows, buttons, title) is unaffected — only the descent
/// into the crash-prone web-content tree is gated. Full screen-reader support
/// returns unconditionally once the AX path itself is made non-fatal (Phase 2 /
/// Layer 2 of the spec). Not a blanket `--disable-renderer-accessibility`: that
/// would make AgentMux permanently inaccessible; this keeps the explicit
/// (`AXManualAccessibility`) enable path working.
///
/// Must run AFTER `cef::initialize` — the CEF `NSApplication` subclass that
/// implements the legacy AX setter only exists then. FFI mirrors
/// `disable_macos_drag_slideback`. Idempotent enough for once-at-startup.
#[cfg(target_os = "macos")]
unsafe fn install_macos_accessibility_governor() {
    use std::ffi::{c_char, c_void};

    type Id     = *mut c_void;
    type Sel    = *const c_void;
    type Class  = *mut c_void;
    type Method = *mut c_void;
    type Imp    = *const c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn object_getClass(obj: Id) -> Class;
        fn class_getName(cls: Class) -> *const c_char;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn class_getInstanceMethod(cls: Class, sel: Sel) -> Method;
        fn method_getImplementation(m: Method) -> Imp;
        fn method_setImplementation(m: Method, imp: Imp) -> Imp;
        fn objc_msgSend();
    }

    // Original Chromium IMP, saved so the governed replacement can chain to it.
    // Single-threaded startup write; main-thread-only AX reads afterward.
    static mut ORIGINAL_SET_AX: Imp = std::ptr::null();
    // Read the override once at install (env reads in the hot path are wasteful
    // and AX sets are rare, but a static keeps the IMP allocation-free).
    static mut HONOR_ENHANCED: bool = false;

    // [attr isEqualToString:@literal] without bringing in a string crate.
    unsafe fn attr_is(attr: Id, literal: &[u8]) -> bool {
        if attr.is_null() {
            return false;
        }
        let objc_get_class: extern "C" fn(*const c_char) -> Class =
            std::mem::transmute(objc_getClass as *const c_void);
        let cls = objc_get_class(b"NSString\0".as_ptr() as _);
        if cls.is_null() {
            return false;
        }
        let sel_with = sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _);
        let make: extern "C" fn(Class, Sel, *const c_char) -> Id =
            std::mem::transmute(objc_msgSend as *const c_void);
        let lit = make(cls, sel_with, literal.as_ptr() as *const c_char);
        if lit.is_null() {
            return false;
        }
        let sel_eq = sel_registerName(b"isEqualToString:\0".as_ptr() as _);
        let eq: extern "C" fn(Id, Sel, Id) -> u8 =
            std::mem::transmute(objc_msgSend as *const c_void);
        eq(attr, sel_eq, lit) != 0
    }

    // Replacement for -[<app> accessibilitySetValue:(id)value forAttribute:(NSString*)attr].
    unsafe extern "C" fn governed_set_ax(this: Id, cmd: Sel, value: Id, attribute: Id) {
        if attr_is(attribute, b"AXEnhancedUserInterface\0") {
            if !HONOR_ENHANCED {
                tracing::warn!(
                    "a11y governor: blocked AXEnhancedUserInterface activation \
                     (window-manager/KVM path — CEF #3512). \
                     Set AGENTMUX_A11Y_HONOR_ENHANCED=1 to allow."
                );
                return; // swallow → web-content AX stays off
            }
            tracing::warn!("a11y governor: honoring AXEnhancedUserInterface (override enabled)");
        } else if attr_is(attribute, b"AXManualAccessibility\0") {
            tracing::info!("a11y governor: honoring AXManualAccessibility (explicit enable)");
        } else {
            tracing::debug!("a11y governor: passthrough accessibilitySetValue:forAttribute:");
        }
        let orig: extern "C" fn(Id, Sel, Id, Id) = std::mem::transmute(ORIGINAL_SET_AX);
        orig(this, cmd, value, attribute);
    }

    // Honor only an explicit truthy value — keying on presence would make
    // `AGENTMUX_A11Y_HONOR_ENHANCED=0` *enable* the override (reagent P2).
    HONOR_ENHANCED = std::env::var("AGENTMUX_A11Y_HONOR_ENHANCED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // app = [NSApplication sharedApplication]
    let cls_nsapp = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls_nsapp.is_null() {
        tracing::warn!("a11y governor: NSApplication class not found; skipping");
        return;
    }
    let sel_shared = sel_registerName(b"sharedApplication\0".as_ptr() as _);
    let shared: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let app = shared(cls_nsapp, sel_shared);
    if app.is_null() {
        tracing::warn!("a11y governor: sharedApplication nil; skipping");
        return;
    }

    let app_cls = object_getClass(app);
    let cls_name = {
        let p = class_getName(app_cls);
        if p.is_null() {
            "<unknown>".to_owned()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };

    // --- Layer 2a guard (LOAD-BEARING): -[NSApplication accessibilityParent] → nil ---
    //
    // Installed FIRST and INDEPENDENTLY of the Layer 1 setter swizzle below, so a
    // macOS/CEF build that lacks the legacy setter still gets this fix (reagent P1
    // — the setter was previously a gate that skipped this guard on early return).
    //
    // The observed crash (EXC_BREAKPOINT, both reports) is an *incoming* AX READ —
    // an external client (Magnet/Synergy) calls CopyAttributeValue on the app,
    // which routes through:
    //   -[NSApplication accessibilityParent]
    //     → NSAccessibilityGetObjectForAttributeUsingLegacyAPI
    //     → NSAccessibilitySetUnsupportedAttributeError
    //     → +[NSString stringWithFormat:]  → CF string trap (with a CEF AX object
    //        as the %@ arg — CEF #3512).
    // NSApplication is the AX root; its parent is legitimately nil. Returning nil
    // DIRECTLY short-circuits before the crashy legacy bridge runs. Safe and
    // semantically correct, and it does not disable accessibility — windows/title
    // still answer.
    unsafe extern "C" fn accessibility_parent_nil(_this: Id, _cmd: Sel) -> Id {
        std::ptr::null_mut()
    }
    let sel_parent = sel_registerName(b"accessibilityParent\0".as_ptr() as _);
    let m_parent = class_getInstanceMethod(app_cls, sel_parent);
    if !m_parent.is_null() {
        method_setImplementation(m_parent, accessibility_parent_nil as Imp);
        tracing::info!(app_class = %cls_name, "a11y governor: guarded -[NSApplication accessibilityParent] → nil (SPEC L2a)");
    } else {
        tracing::warn!(app_class = %cls_name, "a11y governor: accessibilityParent not found on app class");
    }

    // --- Layer 1 (defense in depth): govern AXEnhancedUserInterface activation ---
    // Independent of L2a above; if the legacy setter is absent we just log and the
    // load-bearing parent guard still stands.
    let sel_set = sel_registerName(b"accessibilitySetValue:forAttribute:\0".as_ptr() as _);
    let method = class_getInstanceMethod(app_cls, sel_set);
    if !method.is_null() {
        ORIGINAL_SET_AX = method_getImplementation(method);
        method_setImplementation(method, governed_set_ax as Imp);
        tracing::info!(
            honor_enhanced = HONOR_ENHANCED,
            "a11y governor: swizzled accessibilitySetValue:forAttribute: (SPEC L1)"
        );
    } else {
        tracing::warn!(
            "a11y governor: accessibilitySetValue:forAttribute: not found on app class — \
             activation governor inactive (parent guard still installed)"
        );
    }
}

/// Promote the process to a regular, Dock-visible macOS app.
///
/// A bare Mach-O launched outside an `.app` bundle — both `task dev`
/// direct-invoke and the launcher's flat `dist/cef-dev/` layout — has no
/// `Info.plist`, so LaunchServices leaves it as a background/accessory
/// process: it can open windows but gets **no Dock tile and no menu bar**, so
/// the AgentMux instance is invisible in the taskbar (`lsappinfo` reports it
/// `type="BackgroundOnly"`). `-[NSApplication setActivationPolicy:]` with
/// `NSApplicationActivationPolicyRegular` (raw value `0`) flips it to a normal
/// foreground app so it shows in the Dock; this must run before
/// `set_macos_dock_icon` (the icon needs a tile to land on). Harmless in a
/// future packaged `.app` (already regular there). Idempotent.
///
/// Must run on the main thread after `cef::initialize` (NSApplication exists
/// by then). FFI mirrors `set_macos_dock_icon` — raw libobjc, no extra crate.
#[cfg(target_os = "macos")]
unsafe fn set_macos_activation_policy_regular() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    // NSApplicationActivationPolicyRegular == 0 (Accessory == 1, Prohibited == 2).
    const NS_ACTIVATION_POLICY_REGULAR: isize = 0;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    let cls_nsapp = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls_nsapp.is_null() {
        tracing::warn!("activation-policy: NSApplication class not found; skipping");
        return;
    }

    // NSApplication *app = [NSApplication sharedApplication]
    let sel_shared = sel_registerName(b"sharedApplication\0".as_ptr() as _);
    let msg_shared: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let app = msg_shared(cls_nsapp, sel_shared);
    if app.is_null() {
        tracing::warn!("activation-policy: NSApplication.sharedApplication unavailable");
        return;
    }

    // BOOL ok = [app setActivationPolicy:NSApplicationActivationPolicyRegular]
    let sel_set = sel_registerName(b"setActivationPolicy:\0".as_ptr() as _);
    let msg_set: extern "C" fn(Id, Sel, isize) -> u8 =
        std::mem::transmute(objc_msgSend as *const ());
    let ok = msg_set(app, sel_set, NS_ACTIVATION_POLICY_REGULAR);
    tracing::info!(ok = ok != 0, "activation-policy: set NSApplication to Regular (Dock-visible)");
}

/// Set the macOS Dock icon for the running process.
///
/// `task dev` launches the bare `agentmux-cef` Mach-O directly (not inside an
/// `.app` bundle — see `Taskfile.yml::dev:serve`), so macOS has no
/// `CFBundleIconFile` to read and shows the generic executable tile in the
/// Dock. Rather than restructure the dev launch around a bundle, we set the
/// icon at runtime via `-[NSApplication setApplicationIconImage:]`, which
/// works whether or not we're in a bundle and also overrides a bundle icon
/// in any future packaged build — one code path for all launch modes.
///
/// The PNG is embedded at compile time (`include_bytes!`) so there's no
/// dependency on the `dist/` layout or a resource-path lookup at runtime.
/// It's the SAME normal AgentMux logo the Linux taskbar uses
/// (`assets/linux/icons/hicolor/512x512/apps/agentmux.png`, wired up in
/// `scripts/install-linux-desktop.sh`), keeping the Dock/taskbar icon
/// identical across platforms.
///
/// Must run on the main thread after `cef::initialize` (NSApplication exists
/// by then). FFI mirrors `patch_nsapp_unrecognized_selector` — raw libobjc,
/// no `objc2`/`cocoa` crate dependency. The created NSImage is intentionally
/// leaked (one per process lifetime): `setApplicationIconImage:` retains it
/// and the icon lives as long as the app does.
#[cfg(target_os = "macos")]
unsafe fn set_macos_dock_icon() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    // The normal AgentMux logo (panel layout, not the brain-alternate),
    // matching the Linux taskbar source.
    const ICON_PNG: &[u8] =
        include_bytes!("../../assets/linux/icons/hicolor/512x512/apps/agentmux.png");

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        // objc_msgSend is declared bare and transmuted to the exact prototype
        // at each call site — the ARM64 calling convention requires the real
        // signature, not a variadic stand-in.
        fn objc_msgSend();
    }

    let cls_nsdata  = objc_getClass(b"NSData\0".as_ptr() as _);
    let cls_nsimage = objc_getClass(b"NSImage\0".as_ptr() as _);
    let cls_nsapp   = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls_nsdata.is_null() || cls_nsimage.is_null() || cls_nsapp.is_null() {
        tracing::warn!("dock-icon: an AppKit class was not found; skipping");
        return;
    }

    // NSData *data = [NSData dataWithBytes:ICON_PNG.ptr length:ICON_PNG.len]
    let sel_data = sel_registerName(b"dataWithBytes:length:\0".as_ptr() as _);
    let msg_data: extern "C" fn(Class, Sel, *const c_void, usize) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let data = msg_data(cls_nsdata, sel_data, ICON_PNG.as_ptr() as *const c_void, ICON_PNG.len());
    if data.is_null() {
        tracing::warn!("dock-icon: NSData creation failed");
        return;
    }

    // NSImage *img = [[NSImage alloc] initWithData:data]
    let sel_alloc = sel_registerName(b"alloc\0".as_ptr() as _);
    let msg_alloc: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let img_alloc = msg_alloc(cls_nsimage, sel_alloc);
    let sel_init = sel_registerName(b"initWithData:\0".as_ptr() as _);
    let msg_init: extern "C" fn(Id, Sel, Id) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let image = msg_init(img_alloc, sel_init, data);
    if image.is_null() {
        tracing::warn!("dock-icon: NSImage creation failed (corrupt PNG?)");
        return;
    }

    // NSApplication *app = [NSApplication sharedApplication]
    let sel_shared = sel_registerName(b"sharedApplication\0".as_ptr() as _);
    let msg_shared: extern "C" fn(Class, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let app = msg_shared(cls_nsapp, sel_shared);
    if app.is_null() {
        tracing::warn!("dock-icon: NSApplication.sharedApplication unavailable");
        return;
    }

    // [app setApplicationIconImage:img]
    let sel_set = sel_registerName(b"setApplicationIconImage:\0".as_ptr() as _);
    let msg_set: extern "C" fn(Id, Sel, Id) =
        std::mem::transmute(objc_msgSend as *const ());
    msg_set(app, sel_set, image);

    tracing::info!("dock-icon: set NSApplication icon to embedded AgentMux logo");
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
    let absolute_path = log_dir.join(&current_filename);
    let pointer_name = format!("current-host-v{}.path", version);

    // Pointer #1: local — inside the instance's log dir. The basename
    // is enough here since the reader is colocated.
    let _ = std::fs::write(log_dir.join(&pointer_name), &current_filename);

    // Pointer #2: global — at `<root>/logs/<pointer_name>`. Writes the
    // ABSOLUTE PATH so legacy tooling (`muxlog host`) that lives outside
    // the instance dir can `cat $pointer | xargs tail -f` and reach the
    // real file. Skipped silently if the global dir can't be derived
    // (e.g. AGENTMUX_HOME_OVERRIDE unset in some test setups).
    if let Some(global_logs_dir) = log_dir.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).map(|p| p.join("logs")) {
        let _ = std::fs::create_dir_all(&global_logs_dir);
        let _ = std::fs::write(
            global_logs_dir.join(&pointer_name),
            absolute_path.to_string_lossy().as_bytes(),
        );
    }

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
