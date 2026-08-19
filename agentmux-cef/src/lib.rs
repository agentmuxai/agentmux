// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AgentMux Host — Library entry point.
//
// This cdylib is the Windows DLL wrapper sandbox target (Phase 3, issue #1374).
// `bootstrap.exe` (renamed `agentmux-cef.exe`) loads this DLL and calls
// `RunWinMain` with a pre-initialized `cef_sandbox_info_t *` pointer, which we
// forward into `run()`.
//
// On non-Windows or sandbox-off builds, only the `[[bin]]` target is used;
// this file must still compile on those platforms.

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
mod logging;
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
mod macos_compat;
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

/// Run the AgentMux host.
///
/// `windows_sandbox_info` is the `cef_sandbox_info_t *` created by
/// `bootstrap.exe` before loading this DLL (Windows + sandbox feature), or
/// `null_mut()` on all other platforms / sandbox-off builds.
pub fn run(windows_sandbox_info: *mut std::ffi::c_void) -> i32 {
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

    // Escape hatch — read once here so both macOS subprocess sandbox init
    // (below) and the browser-process Settings construction share the same value.
    let force_no_sandbox = std::env::var("AGENTMUX_UNSAFE_NOSANDBOX")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);

    // macOS sandbox availability: probe for libcef_sandbox.dylib to detect
    // whether we are running inside a packaged .app bundle. Two uses:
    //   1. Seatbelt init (subprocess only) — skip if dylib not reachable.
    //   2. no_sandbox flag to CefInitialize — must be 1 when not in bundle,
    //      otherwise CEF kills GPU/network/renderer with a sandbox error.
    //
    // The path is relative to current_exe().parent() and differs by role
    // because the Host and Helper processes live at different depths:
    //
    //   Host   — Contents/MacOS/AgentMux
    //            → ../Frameworks/Chromium Embedded Framework.framework/…
    //
    //   Helper — Contents/Frameworks/AgentMux Helper.app/Contents/MacOS/AgentMux Helper
    //            → ../../../Chromium Embedded Framework.framework/…
    //
    // In task dev neither path exists (flat dist/cef-dev/ tree), so the probe
    // returns false for both roles → sandbox skipped, no crash.
    #[cfg(all(target_os = "macos", feature = "sandbox"))]
    let macos_sandbox_available = {
        let is_subprocess = std::env::args().any(|a| a.starts_with("--type="));
        let rel: &str = if is_subprocess {
            "../../../Chromium Embedded Framework.framework/Libraries/libcef_sandbox.dylib"
        } else {
            "../Frameworks/Chromium Embedded Framework.framework/Libraries/libcef_sandbox.dylib"
        };
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(rel)))
            .map(|p| p.exists())
            .unwrap_or(false)
    };

    // Non-macOS / sandbox-off: define macos_sandbox_available as a no-op
    // placeholder so the no_sandbox computation below compiles on all targets.
    // On those targets cfg!(all(target_os="macos",feature="sandbox")) is false,
    // so the `&& !macos_sandbox_available` branch is never reached at runtime.
    #[cfg(not(all(target_os = "macos", feature = "sandbox")))]
    let macos_sandbox_available = true; // irrelevant — guarded by cfg!() in no_sandbox

    // macOS: initialize Seatbelt sandbox context BEFORE the CEF framework is
    // loaded. This is the order required by CEF (cefsimple/process_helper_mac.cc):
    //   1. cef_sandbox_initialize     ← here, before dlopen
    //   2. LoadInHelper / _library    ← CEF framework loaded within the sandbox
    //   3. CefExecuteProcess
    // Subprocess mode is detected from raw argv (--type=…) because cef::Args
    // cannot be parsed until after the framework is loaded. Browser process
    // skips this entirely — the host process is unsandboxed by design.
    #[cfg(all(target_os = "macos", feature = "sandbox"))]
    let _macos_sandbox = if std::env::args().any(|a| a.starts_with("--type="))
        && !force_no_sandbox
        && macos_sandbox_available
    {
        let raw = cef::args::Args::new();
        let mut s = cef::sandbox::Sandbox::new();
        s.initialize(raw.as_main_args());
        Some(s)
    } else {
        None
    };

    // macOS: load the CEF framework library explicitly.
    // For subprocesses this load happens within the Seatbelt policy established above.
    //
    // Timed locally (Instant, not IPC) because this happens before
    // connect_to_launcher runs — there's no live launcher connection yet
    // to report through. The duration is sent retroactively as soon as
    // the connection exists (see the report_startup_stage_* call right
    // after connect_to_launcher below). Reported for every process role
    // that reaches here (browser + subprocesses), but only the browser
    // process ever has a live COMMAND_TX to actually send through —
    // report_startup_stage_* silently no-ops otherwise, so no extra
    // role-gating is needed here.
    #[cfg(target_os = "macos")]
    let dlopen_started_at = std::time::Instant::now();
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
    #[cfg(target_os = "macos")]
    let dlopen_ms = dlopen_started_at.elapsed().as_millis() as u64;

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

    // macOS 26 + dev build: ChromeWebAppShortcutCopierMain CHECK-aborts (SIGTRAP)
    // because the dev binary is not in a signed .app bundle.
    // --disable-features=MacAppCodeSignClone in on_before_command_line_processing
    // cannot prevent the spawn because the feature flag is checked before FeatureList
    // is initialized (same timing problem as MachPortRendezvous). Exit 0 immediately
    // for this subprocess type so the main process sees a "clean" failure and
    // continues without the code-sign clone (which is irrelevant for dev builds).
    // The main process logs "Failed to send Mojo invitation to web_app_shortcut_copier"
    // (harmless) and continues. This exit must happen BEFORE execute_process() which
    // calls ChromeWebAppShortcutCopierMain() and SIGTRAP-kills the subprocess.
    #[cfg(target_os = "macos")]
    if !is_browser_process {
        let pt = CefString::from(&cmd_line.switch_value(Some(&type_switch)));
        let pt_str = pt.to_string();
        if pt_str == "web-app-shortcut-copier" {
            eprintln!("agentmux-cef: intercepted web-app-shortcut-copier (dev build, not in .app bundle) — exiting 0");
            std::process::exit(0);
        }
    }

    // Execute subprocess if applicable (exits here for non-browser processes).
    let ret = execute_process(
        Some(args.as_main_args()),
        None, // App can be None for subprocess
        windows_sandbox_info as *mut u8,
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
    let _log_guard = logging::init_logging(&log_dir);

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
    unsafe { macos_compat::patch_nsapp_unrecognized_selector() };

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
    unsafe { macos_compat::disable_macos_drag_slideback() };

    // Name the app early (sets CFBundleName on the main bundle, which AppKit
    // may read when it first builds a menu). Also re-run post-init below, since
    // Chromium can reset the process name during cef::initialize.
    // NOTE: CFBundleIdentifier is NOT set here (pre-init) — setting it before
    // cef::initialize triggers MacAppCodeSignClone which SIGTRAP-crashes dev builds.
    #[cfg(target_os = "macos")]
    unsafe { macos_compat::set_macos_app_display_name_pre_init() };

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

    // Report the CEF framework dlopen timing now that a launcher
    // connection exists (it didn't when dlopen actually happened, above).
    // Sent as a StageBegin+StageEnd pair with the already-elapsed
    // duration on the End message — the launcher's splash renders this
    // like any other completed stage. No-op if _launcher_ipc is None
    // (dev:standalone, or this is a subprocess role that never connects).
    #[cfg(target_os = "macos")]
    {
        launcher_ipc::report_startup_stage_begin("dlopen", "Load CEF framework");
        launcher_ipc::report_startup_stage_end("dlopen", dlopen_ms, "ok", None);
    }

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
        // Captured before the match below consumes launcher_provided by value.
        // Phase 0f: distinguishes which path result.pending_migrations came
        // from, so we know when it's safe to trust a 0.
        let is_host_owned_spawn = launcher_provided.is_none();
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
                // On the host-owned spawn_backend path, result.pending_migrations
                // is the authoritative signal for THIS spawn — always trust it,
                // including 0, so a stale nonzero count cached from an earlier
                // spawn in this process's lifetime (e.g. before a restart) gets
                // cleared on a clean re-spawn instead of sticking forever
                // (Phase 0f, docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md
                // §1.6/F8). On the launcher path use_launcher_endpoints always
                // returns 0 here (meaningless stub) and the real count was
                // already seeded into AppState from AGENTMUX_PENDING_MIGRATIONS
                // at construction time — only ever overwrite it with a
                // genuinely positive value, never clobber it with this stub 0.
                if is_host_owned_spawn || result.pending_migrations > 0 {
                    *app_state.pending_migrations.lock() = result.pending_migrations;
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

    // Push this host's ipc_port/ipc_token to srv (host_ipc.Register) so
    // srv can proxy /api/v1/ui/{screenshot,click,query} (agent-facing UI
    // automation) to this host's /agentmux/browser/* routes. Must run
    // after backend_endpoints.web_endpoint is known (just above) — see
    // SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md. Runs off
    // the UI thread via spawn_blocking (raw-TCP, synchronous, same as
    // every other client::helpers call) so a slow/unreachable srv can't
    // stall startup.
    {
        let web_endpoint = app_state.backend_endpoints.lock().web_endpoint.clone();
        let auth_key = app_state.auth_key.lock().clone();
        let ipc_token = app_state.ipc_token.clone();
        let host_reg_secret = app_state.host_reg_secret.lock().clone();
        runtime.spawn_blocking(move || {
            client::register_ipc_with_backend(&web_endpoint, &auth_key, ipc_port, &ipc_token, &host_reg_secret);
        });
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
    // Expose root_cache_path. Per-window RequestContexts no longer place
    // anything under it (they are in-memory — see
    // `create_isolated_request_context`), but the legacy-litter sweep below
    // and future consumers still need the resolved path.
    *app_state.cef_cache_dir.lock() = Some(data_dir.to_string_lossy().to_string());
    // Remove dirs left by the earlier isolated-context path schemes
    // (`browser-contexts/`, `ctx-*`) — labels are per-run UUIDs, so they can
    // never be referenced again. SPEC_CEF_LOG_ROBUSTNESS_2026_06_20.md §1.6.
    crate::commands::cleanup_legacy_context_dirs(&data_dir.to_string_lossy());

    // Configure CEF settings.
    // Pick a FREE remote-debugging port instead of a fixed one: AgentMux runs
    // many instances in parallel (isolation I1–I6), so a hardcoded port collides
    // (WSAEADDRINUSE / 0x2740) and the 2nd+ instance gets no DevTools server →
    // the browser DOM API (`/agentmux/browser/*`) can't connect. Prefer the
    // conventional port (9223 dev / 9222 release) for muscle memory; fall back to
    // an OS-assigned free port. Store the ACTUAL port so `browser_api` targets
    // it. SPEC_CEF_LOG_ROBUSTNESS_2026_06_20.md §2.
    let preferred: u16 = if is_dev { 9223 } else { 9222 };
    let debug_port: u16 = {
        use std::net::TcpListener;
        if TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
            preferred
        } else {
            TcpListener::bind(("127.0.0.1", 0))
                .and_then(|l| l.local_addr())
                .map(|a| a.port())
                .unwrap_or(preferred)
        }
    };
    *app_state.debug_port.lock() = debug_port;
    tracing::info!("CEF remote-debugging port: {} (preferred {})", debug_port, preferred);

    // Route CEF's internal Chromium logging into our log dir alongside the
    // tracing-subscriber file. Without this, init failures leave an empty
    // chrome_debug.log in the cache dir and we have nothing to read. INFO is
    // verbose enough to expose load-library / resource problems but quiet
    // enough not to swamp the file in normal operation.
    let cef_log_path = log_dir.join("cef-debug.log");
    let cef_log_file = CefString::from(cef_log_path.to_str().unwrap_or(""));

    if force_no_sandbox {
        tracing::warn!(
            "AGENTMUX_UNSAFE_NOSANDBOX=1: renderer sandbox disabled. \
             Only use in environments where namespace/Seatbelt sandbox is known-incompatible."
        );
    }

    // Sandbox active on macOS (Seatbelt via libcef_sandbox.dylib), Linux
    // (kernel namespace isolation), and Windows (bootstrap.exe DLL wrapper,
    // Phase 3) when built with `--features sandbox` (the default).
    // Escape hatch: AGENTMUX_UNSAFE_NOSANDBOX=1 disables on all platforms.
    let no_sandbox: i32 = if cfg!(not(feature = "sandbox"))
        || force_no_sandbox
        || cfg!(all(target_os = "macos", feature = "sandbox")) && !macos_sandbox_available
    {
        1
    } else {
        0
    };

    let settings = Settings {
        no_sandbox,
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
    // Live-reportable on every platform: connect_to_launcher (above)
    // always runs before cef::initialize, so a launcher connection
    // exists by this point wherever one exists at all. No-op if it
    // doesn't (dev:standalone, subprocess roles).
    let cef_init_started_at = std::time::Instant::now();
    launcher_ipc::report_startup_stage_begin("cef_init", "CEF initialize");
    let init_result = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut cef_app),
        windows_sandbox_info as *mut u8,
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

    launcher_ipc::report_startup_stage_end(
        "cef_init",
        cef_init_started_at.elapsed().as_millis() as u64,
        "ok",
        None,
    );
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
        macos_compat::set_macos_app_display_name();
        macos_compat::set_macos_activation_policy_regular();
        macos_compat::set_macos_dock_icon();
        // Layer 1 of SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03: govern
        // Chromium's macOS accessibility activation so a window manager / KVM
        // poking the AX tree (Magnet, Synergy, …) can't force the crash-prone
        // web-content AX mode (CEF #3512). Must run after cef::initialize so
        // the CEF NSApplication subclass (which owns the legacy AX setter)
        // exists.
        macos_compat::install_macos_accessibility_governor();
    }

    // Native macOS menu bar (File/Edit/View/Window/Help) — Phase 1 of
    // SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03. After cef::initialize (NSApplication
    // exists) and after set_macos_app_display_name (the app-menu title follows
    // the process name). Standard Edit/Window items route to the focused web
    // view; custom items dispatch through the frontend command registry.
    #[cfg(target_os = "macos")]
    macos_menu::install_menu_bar(app_state.clone());

    // Reopen handler — a plain re-launch / Finder/Dock double-click of the
    // running app opens a new window (Windows-parity) instead of just focusing
    // it. Installs an NSApplication delegate (`applicationShouldHandleReopen:`);
    // a raw NSAppleEventManager handler was inert because CEF re-registers its
    // own. After menu install, NSApplication exists.
    // SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md.
    #[cfg(target_os = "macos")]
    macos_menu::install_reopen_handler(app_state.clone());

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

    // Kill the backend sidecar. (The launcher's Job Object also reaps it once
    // we exit; kill explicitly for promptness.)
    {
        let mut sidecar = app_state.sidecar_child.lock();
        if let Some(ref mut child) = *sidecar {
            tracing::info!("Killing backend sidecar");
            let _ = child.kill();
        }
    }

    // Phase B.6 (post-fix): clean up the forwarding hint so a stale file doesn't
    // survive a graceful exit. (Hard crashes leave it behind; harmless because
    // pipe-bind on next launch is authoritative.)
    let _ = std::fs::remove_file(&port_file);

    // Reaching here means run_message_loop() returned, which ONLY happens on the
    // intended LastWindowClosed quit — so a hard exit(0) is correct here, NOT a
    // crash. It is also NECESSARY. After a CEF Views last-window close the
    // browsers are HIDDEN/recycled (the close never fires on_before_close), so
    // they're still alive at shutdown; CEF's teardown then access-violates on
    // Windows (`cef::shutdown()` / `UnhookWinEvent`, exit 0xC0000005) and wedges
    // in the tokio runtime drop on macOS. Either way the launcher (which
    // classifies host exit) sees an ABNORMAL exit and RELAUNCHES the instance —
    // the "reopens on its own" symptom (Discussion #1680). exit(0) gives the
    // launcher a clean code-0 shutdown; it reaps the host's children via its Job
    // Object (KILL_ON_JOB_CLOSE).
    // Unhook the Win32 event hooks before exit (no-op off Windows; a cheap,
    // safe UnhookWinEvent — NOT a crash site).
    wrr::uninstall_hooks();
    #[cfg(target_os = "macos")]
    {
        // macOS keeps the prior sequence (works): cef::shutdown() then a
        // non-blocking runtime teardown (#1268).
        shutdown();
        runtime.shutdown_background();
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows/Linux: SKIP cef::shutdown() — the Windows crash site on the
        // still-alive recycled browsers. Drop the tokio runtime (safe +
        // non-blocking here) before the hard exit.
        drop(runtime);
    }
    tracing::info!("AgentMux host shutdown complete (fast exit)");

    // On Windows, hard-terminate so the C-runtime / CEF-DLL static teardown
    // (atexit handlers + DLL_PROCESS_DETACH) does NOT run: with the recycled
    // browsers still alive, that teardown raises a fail-fast (exit 0xC0000602)
    // even though we reached this line, which the launcher classifies as an
    // ABNORMAL exit and RELAUNCHES ("reopens on its own"). `std::process::exit`
    // still runs that cleanup, so use `TerminateProcess(self, 0)` for an
    // immediate, clean code-0 termination; the launcher then shuts down and
    // reaps the host's children via its Job Object. See Discussion #1680.
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        TerminateProcess(GetCurrentProcess(), 0);
    }
    std::process::exit(0)
}

/// Windows DLL wrapper sandbox entry point (Phase 3, issue #1374).
///
/// `bootstrap.exe` (renamed `agentmux-cef.exe`) calls this after:
///   1. `cef_sandbox_info_create()` → `sandbox_info`
///   2. `LoadLibraryW("agentmux-cef.dll")` → this DLL
///   3. `GetProcAddress(hDll, "RunWinMain")` → this function
///
/// We forward `sandbox_info` into `run()` so CEF's `CefExecuteProcess` and
/// `CefInitialize` receive the pre-initialized sandbox context.
#[cfg(all(target_os = "windows", feature = "sandbox"))]
#[no_mangle]
pub unsafe extern "system" fn RunWinMain(
    _h_instance:   windows_sys::Win32::Foundation::HINSTANCE,
    _lp_cmd_line:  *mut u16,
    _n_cmd_show:   i32,
    sandbox_info:  *mut std::ffi::c_void,
    _version_info: *mut std::ffi::c_void,
) -> i32 {
    run(sandbox_info)
}

