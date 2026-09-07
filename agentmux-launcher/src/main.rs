// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// AgentMux Launcher — Sets DLL search path then spawns srv + the CEF host.
//
// Phase B.1: launcher now spawns srv directly (sibling of host) so srv
// survives host crashes. Both children are assigned to the launcher's
// Job Object J0 with KILL_ON_JOB_CLOSE; killing the launcher reaps
// the entire tree atomically via the OS.
//
// This was previously a tiny sync wrapper that just SetDllDirectoryW'd
// runtime/ then spawned the CEF host. Phase B grew it into the
// privileged owner per
// docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md.
//
// Process tree after B.1:
//   launcher (J0)
//     ├── srv     (assigned to J0; survives host crash)
//     └── host    (assigned to J0; CEF render workers inherit J0)

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod autostart;
mod binary_resolution;
mod data_dir;
mod diag;
mod event_log;
mod hash;
mod host_pipe;
mod host_spawn;
mod ipc;
mod job_object;
mod logging;
mod mem_supervisor;
mod other_instances;
mod reducer;
mod saga;
mod second_instance;
mod supervisor;
mod teardown_backstop;
mod tray;
mod ui_liveness;
#[cfg(target_os = "windows")]
mod splash;
#[cfg(target_os = "macos")]
mod splash_mac;
#[cfg(target_os = "linux")]
mod splash_linux;
// Splash footer support. The baked font + software text blitter are only used by
// the software-buffer backends (Linux, Windows); macOS renders native text.
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod splash_font;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod splash_text;
mod splash_config;
mod splash_info;
mod startup_events;
// srv hang-while-alive detection plugs into srv recycle-on-exit
// (SRV_RESTART_BUDGET, supervisor/mod.rs), which is Windows-only today.
#[cfg(target_os = "windows")]
mod srv_liveness;
mod srv_spawner;
mod state;
mod wrr;

use binary_resolution::find_cef_binary;
use logging::log;

/// Suppress the Windows "Application Error" / WER crash dialog so an unhandled
/// fault terminates the process immediately instead of wedging it behind a
/// modal. No-op off Windows. Spec:
/// docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md.
#[cfg(target_os = "windows")]
fn suppress_os_crash_dialogs() {
    use windows_sys::Win32::System::Diagnostics::Debug::{SetErrorMode, SEM_FAILCRITICALERRORS};
    use windows_sys::Win32::System::ErrorReporting::{WerSetFlags, WER_FAULT_REPORTING_NO_UI};
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

/// Process entry point. `suppress_os_crash_dialogs()` runs FIRST — before the
/// Tokio runtime is built. The runtime is built explicitly here (rather than
/// via `#[tokio::main]`, whose generated wrapper would construct it before any
/// of our code runs) so a fault during runtime construction can't surface the
/// Windows crash modal either. Spec:
/// docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md.
fn main() {
    suppress_os_crash_dialogs();

    // Dev/demo affordance: `--splash-selftest` shows the splash in isolation
    // (no srv/host), holds it briefly, then exits — for eyeballing the footer +
    // layout. See SPEC_SPLASH_USERINFO_AND_DISABLE_2026_06_21.md.
    if std::env::args().any(|a| a == "--splash-selftest") {
        splash_selftest();
        return;
    }

    // Auto-start control verbs (issue #2977 WS2). Handled before any heavy
    // startup work — they must not spawn srv/host or disturb a running
    // instance. Gives an uninstaller a callable removal path, which is what
    // Workstream 4 requires.
    {
        let args: Vec<String> = std::env::args().collect();
        if autostart::handle_cli(&args) {
            return;
        }
    }

    // macOS: paint the splash FIRST, on the main thread, before any heavy work
    // — this is the whole reason the splash lives in the small fast launcher
    // rather than the slow CEF host. AppKit must own the main thread, so the
    // srv+host supervisor (`launcher_main`) runs on a worker thread with its
    // own Tokio runtime; the splash pumps a CoreFoundation runloop on main
    // until the host signals first paint. See `splash_mac`.
    #[cfg(target_os = "macos")]
    {
        if splash_config::splash_disabled() {
            if tray::background_service_from_env() {
                // Splash disabled BUT background-service mode on: the main
                // thread must still pump AppKit, or there is no reopen
                // delegate and no menu-bar item once every window is closed
                // (design doc §7.5.1). Same thread layout as the splash path,
                // minus the window.
                splash_mac::prepare_headless_app();
                spawn_supervisor_thread(None);
                splash_mac::pump_forever();
            }
            // Splash disabled, ordinary mode → no AppKit at all; run the
            // supervisor directly on the main thread (there's no runloop to
            // pump without a window or a tray).
            tokio::runtime::Runtime::new()
                .expect("failed to build Tokio runtime")
                .block_on(launcher_main(None));
            return;
        }
        // Create the startup-event sink/receiver here (before the splash
        // window exists) and thread the sink into launcher_main on the
        // worker thread, the receiver into the splash on the main thread —
        // mirrors the Linux branch below. Closes the gap where macOS
        // previously always passed None and the receiver got dropped
        // unread in run_unix. See SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_
        // TELEMETRY_2026_07_02.md §B.3-B.4.
        let (startup_sink, startup_rx) = startup_events::StartupEventSink::new();
        let splash = splash_mac::Splash::show(startup_rx);
        spawn_supervisor_thread(Some(startup_sink));
        splash.run_until_dismissed(); // pumps the runloop, then parks forever
        return;
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Linux: paint the splash before any heavy work (mirrors the macOS path,
        // but on its own thread since neither X11 nor Wayland needs the main
        // thread). spawn() sets AGENTMUX_SPLASH_READY_FILE so the host — spawned
        // later inside launcher_main — inherits it and signals first paint.
        // Windows keeps spawning its splash inside launcher_main (event-name
        // model). See splash_linux/.
        #[cfg(target_os = "linux")]
        let linux_startup_sink = {
            let (sink, rx) = startup_events::StartupEventSink::new();
            if !splash_config::splash_disabled() {
                splash_linux::spawn(rx);
            } else {
                drop(rx);
            }
            Some(sink)
        };
        #[cfg(not(target_os = "linux"))]
        let linux_startup_sink: Option<startup_events::StartupEventSink> = None;

        tokio::runtime::Runtime::new()
            .expect("failed to build Tokio runtime")
            .block_on(launcher_main(linux_startup_sink));
    }
}

/// macOS: run the srv+host supervisor (`launcher_main`) on a worker thread with
/// its own Tokio runtime, leaving the main thread to AppKit. The thread owns
/// process lifetime: it `exit`s when the supervisor returns, which is what ends
/// the main thread's pump loop.
#[cfg(target_os = "macos")]
fn spawn_supervisor_thread(startup_sink: Option<startup_events::StartupEventSink>) {
    std::thread::Builder::new()
        .name("launcher-supervisor".into())
        .spawn(move || {
            // Catch panics so a supervisor crash always exits the process
            // rather than leaving the main-thread AppKit runloop spinning
            // as an invisible orphan.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tokio::runtime::Runtime::new()
                    .expect("failed to build Tokio runtime")
                    .block_on(launcher_main(startup_sink));
            }));
            if result.is_err() {
                eprintln!("AgentMux launcher supervisor panicked — exiting");
                std::process::exit(1);
            }
            // Supervisor finished cleanly (host exited / fatal).
            std::process::exit(0);
        })
        .expect("failed to spawn launcher supervisor thread");
}

/// `--splash-selftest`: show the splash with no srv/host behind it, hold it for a
/// few seconds (or `AGENTMUX_SPLASH_HOLD_MS`), then exit. A demo/dev affordance
/// for eyeballing the footer + centering without launching the whole app.
fn splash_selftest() {
    let hold = std::env::var("AGENTMUX_SPLASH_HOLD_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|ms| std::time::Duration::from_millis(ms.max(3000)))
        .unwrap_or_else(|| std::time::Duration::from_secs(6));

    #[cfg(target_os = "linux")]
    {
        let (sink, rx) = startup_events::StartupEventSink::new();
        // Fire fake startup events so the stage list is exercised.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            sink.stage_begin("prep", "Launcher setup");
            std::thread::sleep(std::time::Duration::from_millis(120));
            sink.stage_end("prep", 120, startup_events::StartupStatus::Ok, None);
            std::thread::sleep(std::time::Duration::from_millis(100));
            sink.stage_begin("migrations", "Migrations");
            sink.sub_begin("migrations", "0009", "cron_schema");
            std::thread::sleep(std::time::Duration::from_millis(80));
            sink.sub_end("migrations", "0009", 80, startup_events::StartupStatus::Ok, None);
            sink.sub_begin("migrations", "0010", "identity_dedup");
            std::thread::sleep(std::time::Duration::from_millis(40));
            sink.sub_end("migrations", "0010", 40, startup_events::StartupStatus::Ok, None);
            sink.stage_end("migrations", 220, startup_events::StartupStatus::Ok, None);
            std::thread::sleep(std::time::Duration::from_millis(200));
            sink.stage_begin("backend", "Backend startup");
            std::thread::sleep(std::time::Duration::from_millis(1500));
            sink.stage_end("backend", 1500, startup_events::StartupStatus::Ok, None);
        });
        splash_linux::spawn(rx);
        std::thread::sleep(hold);
    }
    #[cfg(target_os = "macos")]
    {
        let (sink, rx) = startup_events::StartupEventSink::new();
        // Fire fake startup events so the stage panel is exercised — same
        // fixture shape as the Linux branch above.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            sink.stage_begin("prep", "Launcher setup");
            std::thread::sleep(std::time::Duration::from_millis(120));
            sink.stage_end("prep", 120, startup_events::StartupStatus::Ok, None);
            std::thread::sleep(std::time::Duration::from_millis(100));
            sink.stage_begin("migrations", "Migrations");
            sink.sub_begin("migrations", "0009", "cron_schema");
            std::thread::sleep(std::time::Duration::from_millis(80));
            sink.sub_end("migrations", "0009", 80, startup_events::StartupStatus::Ok, None);
            sink.sub_begin("migrations", "0010", "identity_dedup");
            std::thread::sleep(std::time::Duration::from_millis(40));
            sink.sub_end("migrations", "0010", 40, startup_events::StartupStatus::Ok, None);
            sink.stage_end("migrations", 220, startup_events::StartupStatus::Ok, None);
            std::thread::sleep(std::time::Duration::from_millis(200));
            sink.stage_begin("backend", "Backend startup");
            std::thread::sleep(std::time::Duration::from_millis(1500));
            sink.stage_end("backend", 1500, startup_events::StartupStatus::Ok, None);
            std::thread::sleep(std::time::Duration::from_millis(100));
            sink.stage_begin("host", "Host startup");
            std::thread::sleep(std::time::Duration::from_millis(90));
            sink.stage_end("host", 90, startup_events::StartupStatus::Ok, None);
        });
        let splash = splash_mac::Splash::show(rx);
        if let Ok(p) = std::env::var("AGENTMUX_SPLASH_DUMP_PNG") {
            splash.dump_png(&p);
        }
        let _ = splash; // run_until_dismissed parks; selftest just holds then exits
        std::thread::sleep(hold);
    }
    #[cfg(target_os = "windows")]
    {
        let (_sink, rx) = startup_events::StartupEventSink::new();
        let _ = splash::spawn_splash("selftest", rx, false);
        std::thread::sleep(hold);
    }
}

async fn launcher_main(startup_sink: Option<startup_events::StartupEventSink>) {
    let exe_path = std::env::current_exe().expect("cannot resolve exe path");
    let exe_dir = exe_path.parent().expect("exe has no parent directory");
    // Production + Windows dev use a `runtime/` subdir (launcher at root,
    // host + libs + srv under runtime/). The macOS/Linux `task dev` flat
    // layout (Phase 1, SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30)
    // drops the launcher next to the host in dist/cef-dev/ so the host's
    // `../Frameworks` resolution and asset anchoring are byte-identical to
    // the legacy direct-invoke path — no `runtime/` to descend into. Fall
    // back to exe_dir when there's no runtime/ subdir. Windows always has
    // one, so its behavior is unchanged.
    let runtime_dir = {
        let rt = exe_dir.join("runtime");
        if rt.is_dir() {
            rt
        } else {
            exe_dir.to_path_buf()
        }
    };

    log(&format!(
        "starting — exe={} runtime={}",
        exe_path.display(),
        runtime_dir.display()
    ));

    // Set DLL search path so libcef.dll (in runtime/) is found by the
    // CEF host's load-time linker. SetDllDirectoryW is process-local
    // and inherited by child processes — both srv (which doesn't
    // need libcef but harmless) and host (which absolutely does).
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = runtime_dir
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr());
        }
    }
    log("SetDllDirectoryW done");

    let args: Vec<String> = std::env::args().skip(1).collect();

    // LSD-3 — `agentmux.exe --diag sagas` is OFFLINE: it reads the
    // launcher saga SQLite log directly, with no IPC and no running
    // launcher. So it MUST run BEFORE the CEF runtime existence
    // check below — the offline-diagnostic value is most needed
    // exactly when the launcher won't start (e.g. corrupt runtime
    // folder). (codex P1 + reagent P1 PR #647 round 3.)
    if matches!(
        (args.first().map(String::as_str), args.get(1).map(String::as_str)),
        (Some("--diag"), Some("sagas"))
    ) {
        match diag::run_sagas_diag(exe_dir).await {
            Ok(()) => std::process::exit(0),
            Err(msg) => {
                eprintln!("--diag sagas failed: {}", msg);
                std::process::exit(1);
            }
        }
    }

    let real_exe = find_cef_binary(&runtime_dir);
    log(&format!("resolved CEF binary: {}", real_exe.display()));
    // Self-spawn guard: if host resolution ever points back at the
    // launcher's own binary (the flat dev layout's failure mode —
    // launcher + host co-located), spawning it would recurse into an
    // unbounded launcher fork bomb. find_cef_binary excludes
    // `agentmux-launcher` by name; this is the loud backstop in case a
    // future binary slips past that filter. Compare canonicalized paths
    // so symlink/`./` differences don't defeat the check.
    if let (Ok(a), Ok(b)) = (
        std::fs::canonicalize(&real_exe),
        std::fs::canonicalize(&exe_path),
    ) {
        if a == b {
            log(&format!(
                "FATAL: host resolved to the launcher's own binary ({}) — refusing to self-spawn",
                a.display()
            ));
            eprintln!("AgentMux runtime is misconfigured (host == launcher). Aborting.");
            std::process::exit(1);
        }
    }
    if !real_exe.exists() {
        log(&format!(
            "FATAL: CEF binary not found at {}",
            real_exe.display()
        ));
        eprintln!(
            "AgentMux runtime not found in: {}\nMake sure the runtime/ folder is intact.",
            runtime_dir.display()
        );
        std::process::exit(1);
    }

    log(&format!("forwarding {} CLI args to host", args.len()));

    // Phase B.8 — `agentmux.exe --diag wrr` and `--diag srv` Tool
    // clients. Connect to the running launcher (or srv) over IPC,
    // capture events for a short window, print summary, exit.
    // (Note: --diag sagas is handled above, before the CEF runtime
    // check, since it doesn't need IPC.)
    if matches!(args.first().map(String::as_str), Some("--diag")) {
        let topic = args.get(1).map(String::as_str).unwrap_or("");
        match topic {
            "wrr" => match diag::run_wrr_diag(exe_dir).await {
                Ok(()) => std::process::exit(0),
                Err(msg) => {
                    eprintln!("--diag wrr failed: {}", msg);
                    std::process::exit(1);
                }
            },
            // Phase E.7 — operator visibility into the srv reducer's
            // canonical state (workspaces / tabs / blocks / sagas) +
            // recent activity. Same `Tool` IPC pattern as `--diag wrr`,
            // talks to the srv pipe instead of the launcher pipe.
            "srv" => match diag::run_srv_diag(exe_dir).await {
                Ok(()) => std::process::exit(0),
                Err(msg) => {
                    eprintln!("--diag srv failed: {}", msg);
                    std::process::exit(1);
                }
            },
            // sagas is handled above, before the runtime check.
            "sagas" => {
                // Should never reach here — `sagas` is matched + handled
                // above the CEF runtime check. Kept for completeness.
                unreachable!("--diag sagas is handled before runtime check");
            }
            "" => {
                eprintln!("usage: agentmux.exe --diag <topic>\nknown topics: wrr, srv, sagas");
                std::process::exit(2);
            }
            other => {
                eprintln!("unknown --diag topic: {} (known: wrr, srv, sagas)", other);
                std::process::exit(2);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        supervisor::run_windows(exe_dir, &real_exe, &args).await;
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Phase 1 (SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30):
        // the launcher now owns srv + host on macOS/Linux too — it
        // spawns the backend, hands the host its endpoints via env,
        // and supervises both with the same crash budget Windows uses.
        // The legacy exec-into-host escape hatch lives in
        // `task dev:standalone` (host invoked directly, no launcher).
        supervisor::run_unix(exe_dir, &real_exe, &args, startup_sink).await;
    }
}

/// Show a modal error dialog before the launcher exits. Used for
/// genuine bind failures (NOT the "already running" path — that
/// silently forwards via `forward_open_new_window`). Without this,
/// the launcher exit is silent (it has the `windows` subsystem in
/// release, so eprintln! goes nowhere).
#[cfg(target_os = "windows")]
pub(crate) fn show_fatal_dialog(title: &str, body: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let title_w: Vec<u16> = std::ffi::OsStr::new(title)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let body_w: Vec<u16> = std::ffi::OsStr::new(body)
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body_w.as_ptr(),
            title_w.as_ptr(),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn show_fatal_dialog(_title: &str, body: &str) {
    eprintln!("{}", body);
}

