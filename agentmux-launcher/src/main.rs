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
// specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md.
//
// Process tree after B.1:
//   launcher (J0)
//     ├── srv     (assigned to J0; survives host crash)
//     └── host    (assigned to J0; CEF render workers inherit J0)

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod config;
mod data_dir;
mod diag;
mod event_log;
mod hash;
mod host_pipe;
mod ipc;
mod mem_supervisor;
mod reducer;
mod saga;
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
mod srv_spawner;
mod state;
mod wrr;

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

    // macOS: paint the splash FIRST, on the main thread, before any heavy work
    // — this is the whole reason the splash lives in the small fast launcher
    // rather than the slow CEF host. AppKit must own the main thread, so the
    // srv+host supervisor (`launcher_main`) runs on a worker thread with its
    // own Tokio runtime; the splash pumps a CoreFoundation runloop on main
    // until the host signals first paint. See `splash_mac`.
    #[cfg(target_os = "macos")]
    {
        // Splash disabled → no AppKit splash; run the supervisor directly on the
        // main thread (there's no runloop to pump without a splash window).
        if splash_config::splash_disabled() {
            tokio::runtime::Runtime::new()
                .expect("failed to build Tokio runtime")
                .block_on(launcher_main());
            return;
        }
        let splash = splash_mac::Splash::show();
        std::thread::Builder::new()
            .name("launcher-supervisor".into())
            .spawn(|| {
                // Catch panics so a supervisor crash always exits the process
                // rather than leaving the main-thread AppKit runloop spinning
                // as an invisible orphan.
                let result = std::panic::catch_unwind(|| {
                    tokio::runtime::Runtime::new()
                        .expect("failed to build Tokio runtime")
                        .block_on(launcher_main());
                });
                if result.is_err() {
                    eprintln!("AgentMux launcher supervisor panicked — exiting");
                    std::process::exit(1);
                }
                // Supervisor finished cleanly (host exited / fatal).
                std::process::exit(0);
            })
            .expect("failed to spawn launcher supervisor thread");
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
        if !splash_config::splash_disabled() {
            splash_linux::spawn();
        }

        tokio::runtime::Runtime::new()
            .expect("failed to build Tokio runtime")
            .block_on(launcher_main());
    }
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
        splash_linux::spawn();
        std::thread::sleep(hold);
    }
    #[cfg(target_os = "macos")]
    {
        let splash = splash_mac::Splash::show();
        let _ = splash; // run_until_dismissed parks; selftest just holds then exits
        std::thread::sleep(hold);
    }
    #[cfg(target_os = "windows")]
    {
        let _ = splash::spawn_splash("selftest");
        std::thread::sleep(hold);
    }
}

async fn launcher_main() {
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
        run_windows(exe_dir, &real_exe, &args).await;
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Phase 1 (SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30):
        // the launcher now owns srv + host on macOS/Linux too — it
        // spawns the backend, hands the host its endpoints via env,
        // and supervises both with the same crash budget Windows uses.
        // The legacy exec-into-host escape hatch lives in
        // `task dev:standalone` (host invoked directly, no launcher).
        run_unix(exe_dir, &real_exe, &args).await;
    }
}

/// Phase 1 host supervision (spec
/// `docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md`): on an
/// abnormal host exit the launcher relaunches the host, but at most
/// `HOST_RESTART_BUDGET` times within `HOST_RESTART_WINDOW` — a crash budget
/// so a deterministic crash cannot spin forever (spec §10-A). Shared by
/// the Windows (`run_windows`) and Unix (`run_unix`) supervisors.
const HOST_RESTART_BUDGET: usize = 3;
const HOST_RESTART_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Spawn the CEF host suspended, assign it to the launcher's Job Object, and
/// resume it. Returns the running child, or `None` if any step failed — the
/// caller decides (fatal on first launch, give-up on a restart). `splash_event`
/// is passed on every launch — including restarts — so a relaunched host can
/// still dismiss a splash left pending by a host that crashed pre-first-frame.
/// `disable_gpu` is the retry ladder's rung-2 degraded mode (spec §7): when set
/// the host is launched with `--disable-gpu` (software rendering).
#[cfg(target_os = "windows")]
fn spawn_host_supervised(
    real_exe: &std::path::Path,
    args: &[String],
    srv: &srv_spawner::SrvSpawnResult,
    host_env: &[(&'static str, std::ffi::OsString)],
    pipe_path: &str,
    job_present: bool,
    job_handle: windows_sys::Win32::Foundation::HANDLE,
    splash_event: Option<&str>,
    disable_gpu: bool,
) -> Option<tokio::process::Child> {
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    // The launcher's resolved `real_exe` lives in `runtime/`; pass its
    // parent dir as AGENTMUX_HOME so the host can anchor asset lookups
    // (frontend/index.html, etc.) on something stable rather than
    // `std::env::current_exe()`. Windows's GetModuleFileName keeps
    // returning the original load-time path even after the parent dir
    // is renamed or unlinked out from under it — the 2026-05-28
    // incident pattern. AGENTMUX_HOME is whatever path *we* (the
    // launcher) successfully resolved real_exe through, so it always
    // points at the runtime dir that actually contains the binaries.
    // See docs/retro/retro-portable-rm-running-install-2026-05-28.md.
    let host_runtime_dir = real_exe.parent().map(|p| p.to_path_buf());

    let mut host_cmd = tokio::process::Command::new(real_exe);
    host_cmd
        .args(args)
        .env("AGENTMUX_BACKEND_WS", &srv.ws_endpoint)
        .env("AGENTMUX_BACKEND_WEB", &srv.web_endpoint)
        .env("AGENTMUX_BACKEND_PID", srv.pid.to_string())
        .env("AGENTMUX_AUTH_KEY", &srv.auth_key)
        .env("AGENTMUX_INSTANCE_ID", &srv.instance_id)
        .envs(host_env.iter().cloned())
        .env("AGENTMUX_LAUNCHER_PIPE", pipe_path)
        .creation_flags(CREATE_SUSPENDED)
        .kill_on_drop(false); // J0 handles cleanup.
    if let Some(dir) = host_runtime_dir {
        host_cmd.env("AGENTMUX_HOME", dir);
    }
    if let Some(name) = splash_event {
        host_cmd.env("AGENTMUX_SPLASH_EVENT", name);
    }
    // Retry-ladder rung 2 (spec §7): software rendering — no GPU process to
    // crash. A Chromium switch the host forwards to CEF.
    if disable_gpu {
        host_cmd.arg("--disable-gpu");
    }

    let mut host_child = match host_cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log(&format!("failed to spawn CEF host: {}", e));
            return None;
        }
    };
    let host_pid = host_child.id().unwrap_or(0);
    log(&format!("spawned CEF host pid={} (suspended)", host_pid));

    // Assign to J0 BEFORE resuming so CEF render children inherit the job.
    if job_present && host_pid != 0 {
        match srv_spawner::assign_pid_to_job(host_pid, job_handle) {
            Ok(()) => log(&format!(
                "Job Object assigned to host pid={}, KILL_ON_JOB_CLOSE active",
                host_pid
            )),
            Err(e) => log(&format!(
                "WARN: AssignProcessToJobObject(host) failed: {} — host children may escape job",
                e
            )),
        }
    }

    // Resume the suspended main thread.
    if let Err(e) = resume_main_thread(host_pid) {
        log(&format!("failed to resume host pid={}: {}", host_pid, e));
        let _ = host_child.start_kill();
        return None;
    }
    Some(host_child)
}

/// Spawn the CEF host on Unix with the srv endpoints + canonical
/// data-dir env handed off, mirroring `spawn_host_supervised` minus the
/// Windows-only machinery (Job Object assignment, CREATE_SUSPENDED /
/// resume, named-pipe handle, splash event). `disable_gpu` is the retry
/// ladder's degraded rung (software rendering). Returns the running
/// child or `None` if spawn failed.
///
/// AGENTMUX_HOME is intentionally NOT set: on the flat dev layout the
/// host's `current_exe().parent()` fallback resolves to the same dir the
/// launcher would export, so omitting it keeps asset + framework lookup
/// byte-identical to the legacy direct-invoke (`task dev:standalone`)
/// path. Phase 2 will set it once the production runtime/ layout lands
/// on macOS.
#[cfg(not(target_os = "windows"))]
fn spawn_host_unix(
    real_exe: &std::path::Path,
    args: &[String],
    srv: &srv_spawner::SrvSpawnResult,
    host_env: &[(&'static str, std::ffi::OsString)],
    disable_gpu: bool,
) -> Option<tokio::process::Child> {
    let mut host_cmd = tokio::process::Command::new(real_exe);
    host_cmd
        .args(args)
        .env("AGENTMUX_BACKEND_WS", &srv.ws_endpoint)
        .env("AGENTMUX_BACKEND_WEB", &srv.web_endpoint)
        .env("AGENTMUX_BACKEND_PID", srv.pid.to_string())
        .env("AGENTMUX_AUTH_KEY", &srv.auth_key)
        .env("AGENTMUX_INSTANCE_ID", &srv.instance_id)
        // Parent-identity stamp: our pid == the host's getppid (we spawn it
        // directly). A dev-build host normally ignores AGENTMUX_BACKEND_WS
        // (it could be a stale value inherited from a parent agentmux pane);
        // this lets the host verify the hand-off is genuinely ours THIS run
        // and adopt our launcher-owned srv instead of double-spawning. See
        // agentmux-cef/src/main.rs::launcher_is_genuine_parent.
        .env("AGENTMUX_LAUNCHER_PID", std::process::id().to_string())
        .envs(host_env.iter().cloned())
        // We reap children ourselves on shutdown (SIGTERM, then SIGKILL
        // backstop) — kill_on_drop would SIGKILL the host the moment the
        // Child is dropped, robbing CEF of the chance to reap its render
        // subprocesses cleanly.
        .kill_on_drop(false);
    if disable_gpu {
        host_cmd.arg("--disable-gpu");
    }
    // Linux process-tree reap: PR_SET_PDEATHSIG asks the kernel to SIGKILL
    // this child the moment its parent (the launcher) dies, even abnormally
    // (e.g. launcher panic, OOM, SIGKILL from outside). This is the Linux
    // analogue of Windows' Job Object KILL_ON_JOB_CLOSE that A0 wires up
    // (SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05 §3 step 3 and
    // §A1.4). Without it, a launcher crash orphans the CEF host onto PID 1
    // and the user is left with a zombie tree until manual cleanup.
    //
    // The closure runs between fork() and execve() in the child; per
    // POSIX it MUST NOT allocate, take locks, or call async-signal-unsafe
    // functions. `prctl(2)` is async-signal-safe. macOS lacks prctl, so
    // this is gated to Linux; macOS uses NSTask's auto-reap behavior.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;
        // Safety: prctl is async-signal-safe. No allocation, no syscalls
        // beyond prctl itself. Errors from prctl are non-fatal — the host
        // still spawns; we just lose the auto-reap guarantee.
        unsafe {
            host_cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
                Ok(())
            });
        }
    }
    match host_cmd.spawn() {
        Ok(c) => {
            let pid = c.id().unwrap_or(0);
            log(&format!("spawned CEF host pid={} (unix)", pid));
            Some(c)
        }
        Err(e) => {
            log(&format!("failed to spawn CEF host: {}", e));
            None
        }
    }
}

/// Send SIGTERM to a child so it can shut down gracefully — for the CEF
/// host that means reaping its render subprocesses (a tokio `start_kill`
/// SIGKILL would orphan them); for srv it's a clean shutdown. No-op if the
/// child has already exited (its pid may have been reaped). Best-effort:
/// the SIGKILL grace-window backstop in `run_unix` catches anything that
/// ignores SIGTERM.
#[cfg(not(target_os = "windows"))]
fn terminate_child_gracefully(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        // SAFETY: kill(2) with a process-scoped pid + a constant signal —
        // no memory is touched. A stale pid just returns ESRCH (ignored).
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

/// Await the next delivery of a Unix signal, or never resolve if the
/// signal stream couldn't be installed. Lets `run_unix`'s `select!`
/// treat an absent handler as "this branch is dormant" rather than
/// special-casing `Option` at every poll.
#[cfg(not(target_os = "windows"))]
async fn next_signal(s: &mut Option<tokio::signal::unix::Signal>) {
    match s {
        Some(sig) => {
            sig.recv().await;
        }
        None => std::future::pending::<()>().await,
    }
}

/// Bind the launcher's IPC socket with single-instance enforcement +
/// crash-safe stale-socket recovery, serialized across concurrent
/// launchers via `flock(2)`.
///
/// Returns a bound `UnixListener` on success. Calls `std::process::exit`
/// on:
///   * second-instance detection (exit code 0)
///   * a hard bind failure that isn't `EADDRINUSE` (exit code 2)
///   * unable to acquire the recovery lock (exit code 2)
///
/// Why the lockfile (codex P1 + reagent P1 on PR #1288): see the call-
/// site comment. Two-launcher concurrent stale-cleanup would otherwise
/// produce two live launchers for one data dir.
#[cfg(not(target_os = "windows"))]
fn bind_socket_with_recovery(
    socket_path: &str,
    data_dir: &std::path::Path,
    dir_hash: &str,
) -> tokio::net::UnixListener {
    use std::os::unix::io::AsRawFd as _;

    // Fast path: bind without contention.
    match ipc::server::bind_first_unix_socket(socket_path) {
        Ok(l) => return l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => { /* slow path below */ }
        Err(e) => {
            log(&format!("FATAL: bind {} failed: {}", socket_path, e));
            eprintln!(
                "AgentMux failed to start: could not bind IPC socket.\n\nSocket: {}\nError: {}",
                socket_path, e
            );
            std::process::exit(2);
        }
    }

    // Slow path: contention. Acquire the recovery lock so only one
    // launcher at a time does the connect-probe + unlink + rebind.
    let lock_path = format!("{}.lock", socket_path);
    let lock_file = match std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            log(&format!(
                "FATAL: could not open recovery lockfile {}: {}",
                lock_path, e
            ));
            eprintln!(
                "AgentMux failed to start: could not open IPC recovery lockfile.\n\nLockfile: {}\nError: {}",
                lock_path, e
            );
            std::process::exit(2);
        }
    };
    // Block until we have the lock — another launcher's recovery
    // window is bounded by a single bind + a single connect-probe;
    // we won't wait long. flock(2) is auto-released on close (when
    // the OS reaps the launcher process), so a SIGKILL'd holder
    // doesn't leak the lock.
    if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        let errno = std::io::Error::last_os_error();
        log(&format!(
            "FATAL: flock({}, LOCK_EX) failed: {}",
            lock_path, errno
        ));
        eprintln!(
            "AgentMux failed to start: could not acquire IPC recovery lock.\n\nLockfile: {}\nError: {}",
            lock_path, errno
        );
        std::process::exit(2);
    }

    // Retry the bind under the lock — another launcher may have
    // already cleaned up the stale file while we were waiting on
    // flock, leaving us free to bind directly.
    match ipc::server::bind_first_unix_socket(socket_path) {
        Ok(l) => return l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => { /* probe below */ }
        Err(e) => {
            log(&format!(
                "FATAL: post-lock bind {} failed: {}",
                socket_path, e
            ));
            eprintln!(
                "AgentMux failed to start: post-lock IPC bind failed.\n\nSocket: {}\nError: {}",
                socket_path, e
            );
            std::process::exit(2);
        }
    }

    // Disambiguate: is the existing socket a real running launcher,
    // or a stale file?
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => {
            // Real second-instance. Forward an `open_new_window` request to the
            // already-running launcher's host (Windows-parity — main.rs:1292),
            // then exit cleanly. SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md.
            eprintln!(
                "AgentMux is already running for this data directory.\n\nSocket: {}",
                socket_path
            );
            forward_open_new_window_or_log(data_dir, dir_hash);
            log(&format!(
                "[ipc] second-instance detected — existing launcher owns {}",
                socket_path
            ));
            std::process::exit(0);
        }
        Err(connect_err)
            if connect_err.kind() == std::io::ErrorKind::ConnectionRefused
                || connect_err.raw_os_error() == Some(libc::ENOENT) =>
        {
            // Stale socket file from a crashed launcher. Unlink and
            // rebind. The lock serializes us against other launchers
            // ALSO doing recovery, but it does NOT block a fresh
            // launcher taking the fast-path bind() above — that
            // launcher is lock-free and can win the socket in the
            // microsecond window between our `remove_file` and our
            // `bind`. If that happens, AddrInUse means a real
            // launcher just claimed the socket and WE are now the
            // losing second instance, not a failed start.
            // (Reagent P2 on PR #1288.)
            log(&format!(
                "[ipc] stale socket file at {} — unlinking and rebinding (under recovery lock)",
                socket_path
            ));
            let _ = std::fs::remove_file(socket_path);
            match ipc::server::bind_first_unix_socket(socket_path) {
                Ok(l) => l,
                Err(retry_e) if retry_e.kind() == std::io::ErrorKind::AddrInUse => {
                    eprintln!(
                        "AgentMux is already running for this data directory.\n\nSocket: {}",
                        socket_path
                    );
                    forward_open_new_window_or_log(data_dir, dir_hash);
                    log(&format!(
                        "[ipc] post-recovery bind lost the race to a fresh launcher on {} — exiting as second instance",
                        socket_path
                    ));
                    std::process::exit(0);
                }
                Err(retry_e) => {
                    log(&format!(
                        "FATAL: bind retry after stale-socket unlink failed: {}",
                        retry_e
                    ));
                    eprintln!(
                        "AgentMux failed to start: IPC rebind after stale cleanup failed.\n\nSocket: {}\nError: {}",
                        socket_path, retry_e
                    );
                    std::process::exit(2);
                }
            }
        }
        Err(other) => {
            log(&format!(
                "[ipc] AddrInUse but connect probe failed in an unexpected way: {} — \
                 treating as second instance and exiting cleanly",
                other
            ));
            std::process::exit(0);
        }
    }
    // `lock_file` drops here; flock auto-released on close.
}

/// Unix (macOS/Linux) main flow: resolve paths → bind launcher IPC
/// socket (single-instance signal) → set up reducer / event log / saga
/// coordinator / IPC server → spawn srv → spawn host with srv endpoints
/// AND the launcher socket path in env → supervised wait → cleanup.
///
/// As of A1 (SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05.md §4)
/// the launcher's window/pool/instance reducer + durable saga
/// coordinator are now live on Linux. The 17 host-side `report_*` IPC
/// calls reach the reducer; the saga log persists at
/// `<data-dir>/db/launcher-sagas.db`; second-instance launches are
/// detected via socket-bind contention.
///
/// Differs from `run_windows` only where the OS forces it:
///   * No Job Object — A0 (`PR_SET_PDEATHSIG` in `spawn_host_unix` and
///     `srv_spawner`) gives the equivalent kernel-side reap when the
///     launcher dies abnormally. Terminal Ctrl+C reaches the process
///     group; explicit SIGINT/SIGTERM handlers cover the
///     `kill <launcher-pid>` case.
///   * Unix-domain-socket IPC (A1) instead of named pipes; protocol on
///     the wire is identical (newline-delimited JSON Command/Event).
///     The launcher-side server uses `tokio::net::UnixListener`; the
///     host-side client uses `tokio::net::UnixStream`. See
///     `ipc::server::run_ipc_server` (Unix arm) and
///     `agentmux-cef/src/launcher_ipc.rs::connect_to_launcher` (Unix arm).
///   * srv-side IPC is still skipped on Linux (srv is launched with an
///     empty `srv_pipe_path`); follow-up PR will bring srv's Unix
///     socket online too.
///   * Cleanup is SIGTERM-then-SIGKILL (we own the reap) rather than
///     KILL_ON_JOB_CLOSE.
///
/// srv's stdin write-end is held for the launcher's lifetime — its EOF is
/// srv's parent-death backstop if the launcher is SIGKILLed (the one case
/// our signal handlers can't cover).
#[cfg(not(target_os = "windows"))]
async fn run_unix(
    launcher_exe_dir: &std::path::Path,
    real_exe: &std::path::Path,
    args: &[String],
) {
    use tokio::signal::unix::{signal, SignalKind};

    let version = env!("CARGO_PKG_VERSION");

    // 1. Resolve + create data dirs (same authority as run_windows: srv +
    //    host receive these via env so they can't drift).
    let paths = match data_dir::resolve_paths(launcher_exe_dir, version) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("FATAL: path resolution failed: {}", e));
            eprintln!("Failed to resolve AgentMux data directories: {}", e);
            std::process::exit(1);
        }
    };
    log(&format!(
        "paths resolved: data={} config={} user_home={} portable={}",
        paths.data_dir.display(),
        paths.config_dir.display(),
        paths.user_home_dir.display(),
        paths.portable_root.is_some(),
    ));
    if let Err(e) = data_dir::ensure_dirs(&paths) {
        log(&format!("FATAL: failed to create data dirs: {}", e));
        eprintln!("{}", e);
        std::process::exit(1);
    }

    // -----------------------------------------------------------------
    // A1.1 — IPC server setup on Linux (the wire is up). Mirrors the
    // Windows path at the equivalent point in `run_windows`. Once this
    // lands the host's 17 `report_*` IPC calls reach the (already
    // platform-neutral) reducer, the window-pool / single-instance /
    // instance-numbering logic activates, sagas get persisted to the
    // launcher_saga.db SQLite log, and `--diag sagas` reads something
    // useful.
    //
    // Spec: docs/specs/SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05.md §4
    // -----------------------------------------------------------------

    // Compute the socket path from a data-dir hash + version, identical
    // scoping rule to the Windows pipe namespace. `pipe_name` is now
    // pure on both platforms — the side-effecting ensure step lives
    // in `ensure_ipc_socket_dir` and is invoked separately below so
    // that any future read-only inspector (e.g. a Linux `--diag`
    // port) can call `pipe_name` without mutating the filesystem.
    let pipe_version = option_env!("AGENTMUX_BUILD_LABEL")
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    let dir_hash = hash::data_dir_hash16(&paths.data_dir, pipe_version);
    let socket_path = ipc::pipe_name(&dir_hash);
    log(&format!(
        "launcher IPC socket = {} (data_dir={} pipe_version={})",
        socket_path,
        paths.data_dir.display(),
        pipe_version
    ));
    // Ensure the socket dir exists with safe ownership/perms BEFORE
    // any bind attempts. This may call std::process::exit(2) on a
    // hostile-dir-on-disk scenario (cross-user squatting attack).
    let _ = ipc::ensure_ipc_socket_dir();

    // Single-instance handshake (A1.6). The bind is the authoritative
    // signal: a second launcher pointing at the same data dir gets
    // `EADDRINUSE`. The Windows path enjoys atomic `first_pipe_
    // instance(true)`; Unix bind isn't atomic with respect to stale-
    // file recovery, so we serialize that recovery via an `flock(2)`
    // lockfile (codex P1 + reagent P1 on PR #1288).
    //
    // Concurrent-cleanup race the lockfile defends against:
    //   1. Stale socket file remains from a crashed prior launcher.
    //   2. Launcher A and B start concurrently. Both `bind` returns
    //      EADDRINUSE because the file exists.
    //   3. Without the lock: both A and B `connect` → ECONNREFUSED
    //      (no listener on the stale file), both `unlink`, both
    //      `bind` succeeds → two live launchers for one data dir,
    //      and B's `unlink` may have removed A's freshly-bound
    //      socket file before B bound its own.
    //   4. With the lock: A holds the lock → does probe + unlink +
    //      rebind atomically. B blocks on the lock. When A releases,
    //      B's retry-bind sees A's running listener (file exists) and
    //      B's `connect` succeeds → B exits cleanly as second
    //      instance. No double-recovery.
    //
    // The lockfile lives next to the socket as `<socket>.lock`. It is
    // intentionally NOT removed on clean shutdown — the inode is
    // tiny, and leaving it lets the next launcher reuse it without a
    // create-race. (Stale-lock-file scenarios are bounded: flock(2)
    // is auto-released on process exit even for SIGKILL.)
    let first_socket = bind_socket_with_recovery(&socket_path, &paths.data_dir, &dir_hash);

    // Broadcast bus for reducer-emitted events. Same capacity (1024)
    // and rationale as the Windows path.
    let (events_tx, _) =
        tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(1024);

    // Event log: in-memory ring + disk persistence at
    // <data-dir>/launcher-events.log. Disk writer task spawned next.
    let log_disk_path = paths.data_dir.join("launcher-events.log");
    let event_log = std::sync::Arc::new(event_log::EventLog::new(Some(log_disk_path)));
    let event_log_for_writer = std::sync::Arc::clone(&event_log);
    let disk_writer_rx = events_tx.subscribe();
    tokio::spawn(event_log::run_disk_writer(
        event_log_for_writer,
        disk_writer_rx,
    ));

    // Canonical state shared between IPC server + saga coordinator.
    let state = std::sync::Arc::new(tokio::sync::Mutex::new(state::State::default()));

    // Durable saga log at <data-dir>/db/launcher-sagas.db.
    let saga_log_path = data_dir::launcher_saga_log_path(&paths.data_dir);
    let saga_log = match saga::LauncherSagaLog::open(&saga_log_path) {
        Ok(l) => std::sync::Arc::new(l),
        Err(e) => {
            log(&format!(
                "FATAL: failed to open launcher saga log at {:?}: {}",
                saga_log_path, e
            ));
            std::process::exit(2);
        }
    };

    // Startup recovery walker: mark any saga left running from a prior
    // crashed run as failed_compensation. Must run BEFORE coordinator
    // spawn (LSD-3).
    if let Err(e) = saga::compensate_unresolved_launcher_sagas(&saga_log).await {
        log(&format!(
            "[saga-recovery] WARN: walker failed: {} — coordinator will still spawn",
            e
        ));
    }

    // Startup retention vacuum (LSD-4).
    let retention_days = config::load_saga_retention_days(&paths.user_home_dir, |w| log(w));
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days);
    match saga_log.vacuum_older_than(cutoff) {
        Ok(removed) => log(&format!(
            "[saga-log] vacuumed {} sagas older than {} (retention {} days)",
            removed, cutoff, retention_days
        )),
        Err(e) => log(&format!("[saga-log] WARN: vacuum failed: {}", e)),
    }

    // Host pipe wrapper for saga-issued Commands → host. The IPC
    // server's per-connection handler installs the host's writer once
    // the host registers (see ipc/server.rs handle_connection's
    // ClientKind::Host branch).
    let host_pipe = std::sync::Arc::new(host_pipe::HostPipe::new(
        events_tx.clone(),
        std::sync::Arc::clone(&state),
    ));

    // Saga coordinator. Same construction + error handling as the
    // Windows path; the coordinator itself is platform-neutral.
    let saga_coord_inner =
        saga::SagaCoordinator::new(events_tx.clone(), std::sync::Arc::clone(&state))
            .with_log(std::sync::Arc::clone(&saga_log))
            .unwrap_or_else(|e| {
                log(&format!(
                    "[main] FATAL: failed to seed saga_id allocator: {}",
                    e
                ));
                std::process::exit(1);
            })
            .with_host_pipe(std::sync::Arc::clone(&host_pipe));
    let saga_coord = std::sync::Arc::new(saga_coord_inner);
    let saga_rx = events_tx.subscribe();
    tokio::spawn(saga::run_coordinator(
        std::sync::Arc::clone(&saga_coord),
        saga_rx,
    ));

    let _ipc_handle = ipc::run_ipc_server(
        socket_path.clone(),
        first_socket,
        ipc::server::ServerCtx {
            launcher_pid: std::process::id(),
            launcher_version: env!("CARGO_PKG_VERSION").to_string(),
            state,
            events_tx,
            event_log,
            host_pipe: std::sync::Arc::clone(&host_pipe),
        },
    );
    log(&format!("IPC server started on {}", socket_path));

    // 2. Spawn srv. The srv pipe path is the launcher-owned socket
    //    path scope (srv will gain its own Unix-socket bind in a
    //    follow-up; for now we still pass an empty string so srv's
    //    Windows-only IPC code stays disabled).
    let srv_pipe_path = String::new();
    let (srv_result, mut srv_child) =
        match srv_spawner::spawn_srv(launcher_exe_dir, &paths, &srv_pipe_path).await {
            Ok(pair) => pair,
            Err(e) => {
                log(&format!("FATAL: srv spawn failed: {}", e));
                eprintln!("Failed to start backend: {}", e);
                std::process::exit(1);
            }
        };

    // CRITICAL (same rationale as run_windows): take srv's stdin out of
    // the Child so tokio's wait() can't close it and trip srv's
    // parent-watch EOF. Held until launcher exit.
    let _srv_stdin_keepalive = srv_child.stdin.take();

    // 3. Spawn the host with srv endpoints in env.
    // dir_hash was computed once above for socket_path; reuse it here
    // instead of re-hashing the same paths.data_dir + version.
    let mut host_env = paths.common.to_env_vars();
    host_env.push(("AGENTMUX_IPC_HASH", std::ffi::OsString::from(&dir_hash)));
    // A1.3 — IPC env handshake. Tell the host where to find the
    // launcher socket. The env var name `AGENTMUX_LAUNCHER_PIPE` is
    // reused from the Windows side even though the underlying resource
    // is a Unix-domain socket — keeps the 17 `report_*` call sites in
    // `agentmux-cef/src/launcher_ipc.rs` unchanged and avoids touching
    // the host's connect-on-startup code in `agentmux-cef/src/app.rs`.
    host_env.push((
        "AGENTMUX_LAUNCHER_PIPE",
        std::ffi::OsString::from(&socket_path),
    ));
    // AGENTMUX_HOME = the host's runtime directory (siblings of the
    // host binary — libcef.so, paks, locales, etc). Mirrors the
    // Windows path so the host's asset-resolution code finds its
    // co-located data without searching $PATH-like fallbacks.
    if let Some(host_runtime) = real_exe.parent() {
        host_env.push((
            "AGENTMUX_HOME",
            std::ffi::OsString::from(host_runtime),
        ));
    }
    let mut host_child = match spawn_host_unix(real_exe, args, &srv_result, &host_env, false) {
        Some(c) => c,
        None => {
            log("FATAL: could not start CEF host — terminating");
            eprintln!("Failed to launch AgentMux.");
            terminate_child_gracefully(&srv_child);
            let _ = srv_child.start_kill();
            std::process::exit(1);
        }
    };

    // 4. Signal handlers for `kill <launcher-pid>` (a terminal Ctrl+C
    //    already signals the whole foreground group; these cover the
    //    launcher-only case and make Ctrl+C deterministic too).
    let mut sigint = signal(SignalKind::interrupt()).ok();
    let mut sigterm = signal(SignalKind::terminate()).ok();
    if sigint.is_none() || sigterm.is_none() {
        log("WARN: failed to install one or more signal handlers — \
             relying on default termination + srv stdin-EOF backstop");
    }

    // 5. Supervised wait loop — host crash budget mirrors run_windows.
    log("entering supervised host + srv wait (unix)");
    let mut host_restarts: Vec<std::time::Instant> = Vec::new();
    // Separate budget for system-OOM host exits (memory-aware relaunch); see
    // mem_supervisor + SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.
    let mut oom_restarts: Vec<std::time::Instant> = Vec::new();
    let mut last_abnormal_code: Option<i32> = None;
    let mut host_degraded = false;
    let exit_code = loop {
        tokio::select! {
            host_status = host_child.wait() => {
                use std::os::unix::process::ExitStatusExt;
                let status = match host_status {
                    Ok(s) => s,
                    Err(e) => {
                        log(&format!("FATAL: host wait failed: {}", e));
                        break 1;
                    }
                };
                // Host killed by a signal. On a terminal Ctrl+C the host gets
                // SIGINT DIRECTLY (it shares our foreground process group), so
                // host_child.wait() can win the select! race against our own
                // SIGINT arm. Without this guard the host's signal-death has no
                // exit code → unwrap_or(1) → it looks like a crash and we'd
                // relaunch a replacement host that the SIGINT arm then has to
                // kill (reagent #1193 P2). Treat the group-shutdown signals
                // (SIGINT/SIGTERM/SIGHUP) as a clean shutdown; real crash
                // signals (SIGSEGV, SIGABRT, …) still fall through to the
                // crash-budget relaunch below.
                if let Some(sig) = status.signal() {
                    if sig == libc::SIGINT || sig == libc::SIGTERM || sig == libc::SIGHUP {
                        log(&format!("CEF host terminated by signal {} (group shutdown) — shutting down", sig));
                        break 0;
                    }
                    log(&format!("CEF host killed by signal {} (crash) — entering crash-budget relaunch", sig));
                }
                let code = status.code().unwrap_or(1);
                if code == 0 {
                    log("CEF host exited cleanly (code 0) — shutting down");
                    break 0;
                }
                // Classify system-OOM (wait it out) vs a genuine host fault
                // (existing fast budget), mirroring run_windows. SPEC_MEMORY_
                // PRESSURE_SUPERVISION_2026_06_16 §5.B. On Linux a kernel-OOM-kill
                // arrives as a SIGKILL (code 1 here) and is caught by the low-
                // commit reading (SPEC §9.4).
                let commit_free = mem_supervisor::commit_free_mb();
                match mem_supervisor::classify_host_exit(code, commit_free) {
                    mem_supervisor::HostExitClass::SystemOom => {
                        let now = std::time::Instant::now();
                        if mem_supervisor::budget_exhausted(
                            &mut oom_restarts,
                            now,
                            mem_supervisor::OOM_RESTART_WINDOW,
                            mem_supervisor::OOM_RESTART_BUDGET,
                        ) {
                            log(&format!(
                                "CEF host hit system OOM (code {}, {} MB commit-free); OOM restart \
                                 budget exhausted ({} in {}s) — giving up",
                                code,
                                commit_free,
                                mem_supervisor::OOM_RESTART_BUDGET,
                                mem_supervisor::OOM_RESTART_WINDOW.as_secs()
                            ));
                            show_fatal_dialog(
                                mem_supervisor::OOM_GIVEUP_TITLE,
                                mem_supervisor::OOM_GIVEUP_BODY,
                            );
                            break code;
                        }
                        log(&format!(
                            "CEF host hit system OOM (code {}, {} MB commit-free) — waiting for \
                             memory to recover before relaunch",
                            code, commit_free
                        ));
                        // Race the commit-recovery wait against shutdown + srv
                        // death so the supervisor stays responsive during the
                        // (up to OOM_RELAUNCH_DEADLINE) wait — without this the
                        // SIGINT/SIGTERM + srv arms are starved for the whole
                        // wait (reagent P2). Mirrors the outer select! arms.
                        let recovered = tokio::select! {
                            r = mem_supervisor::await_commit_recovery(log) => r,
                            srv_status = srv_child.wait() => {
                                use std::os::unix::process::ExitStatusExt;
                                match srv_status {
                                    Ok(s) => {
                                        let group_shutdown = matches!(
                                            s.signal(),
                                            Some(sig) if sig == libc::SIGINT || sig == libc::SIGTERM || sig == libc::SIGHUP
                                        );
                                        if s.success() || group_shutdown {
                                            log("srv exited as part of shutdown (during OOM wait) — shutting down");
                                            break 0;
                                        }
                                        log(&format!(
                                            "srv exited UNEXPECTEDLY during OOM wait with code {} — terminating launcher",
                                            s.code().unwrap_or(1)
                                        ));
                                    }
                                    Err(e) => log(&format!("FATAL: srv wait failed during OOM wait: {}", e)),
                                }
                                break 1;
                            }
                            _ = next_signal(&mut sigint) => {
                                log("received SIGINT during OOM wait — shutting down");
                                break 0;
                            }
                            _ = next_signal(&mut sigterm) => {
                                log("received SIGTERM during OOM wait — shutting down");
                                break 0;
                            }
                        };
                        if !recovered {
                            show_fatal_dialog(
                                mem_supervisor::OOM_GIVEUP_TITLE,
                                mem_supervisor::OOM_GIVEUP_BODY,
                            );
                            break code;
                        }
                        match spawn_host_unix(real_exe, args, &srv_result, &host_env, true) {
                            Some(c) => host_child = c,
                            None => {
                                log("host relaunch failed to spawn — giving up");
                                break code;
                            }
                        }
                    }
                    mem_supervisor::HostExitClass::Abnormal => {
                        let now = std::time::Instant::now();
                        host_restarts.retain(|t| now.duration_since(*t) < HOST_RESTART_WINDOW);
                        if host_restarts.len() >= HOST_RESTART_BUDGET {
                            log(&format!(
                                "CEF host exited abnormally (code {}); restart budget exhausted \
                                 ({} in {}s) — giving up",
                                code,
                                host_restarts.len(),
                                HOST_RESTART_WINDOW.as_secs()
                            ));
                            break code;
                        }
                        host_restarts.push(now);
                        if last_abnormal_code == Some(code) {
                            host_degraded = true;
                        }
                        last_abnormal_code = Some(code);
                        log(&format!(
                            "CEF host exited abnormally (code {}) — relaunching (restart {}/{}{})",
                            code,
                            host_restarts.len(),
                            HOST_RESTART_BUDGET,
                            if host_degraded { ", degraded: --disable-gpu" } else { "" }
                        ));
                        match spawn_host_unix(real_exe, args, &srv_result, &host_env, host_degraded) {
                            Some(c) => host_child = c,
                            None => {
                                log("host relaunch failed to spawn — giving up");
                                break code;
                            }
                        }
                    }
                }
            }
            srv_status = srv_child.wait() => {
                use std::os::unix::process::ExitStatusExt;
                match srv_status {
                    Ok(s) => {
                        // Mirror the host arm's group-shutdown guard. On a
                        // terminal Ctrl+C srv gets SIGINT DIRECTLY (it shares
                        // our foreground process group); its own signal handler
                        // shuts it down gracefully and it exits with code 0
                        // (agentmux-srv/src/main.rs — SIGINT/SIGTERM → cancel
                        // token → clean exit). srv_child.wait() can win this
                        // select! race against our own SIGINT arm, so a clean
                        // (code 0) or signal-killed exit is a group teardown,
                        // NOT an unexpected srv death — don't log a scary
                        // message and break 1 (reagent #1193 P2).
                        let group_shutdown = matches!(
                            s.signal(),
                            Some(sig) if sig == libc::SIGINT || sig == libc::SIGTERM || sig == libc::SIGHUP
                        );
                        if s.success() || group_shutdown {
                            log("srv exited as part of shutdown — shutting down");
                            break 0;
                        }
                        log(&format!(
                            "srv exited UNEXPECTEDLY (host still running) with code {} — terminating launcher",
                            s.code().unwrap_or(1)
                        ));
                    }
                    Err(e) => log(&format!("FATAL: srv wait failed: {}", e)),
                }
                break 1;
            }
            _ = next_signal(&mut sigint) => {
                log("received SIGINT — shutting down");
                break 0;
            }
            _ = next_signal(&mut sigterm) => {
                log("received SIGTERM — shutting down");
                break 0;
            }
        }
    };

    // 6. Cleanup. SIGTERM both children so the host reaps its render
    //    subprocesses (and srv shuts down cleanly), wait a short grace
    //    window, then SIGKILL any survivor. Dropping the stdin keepalive
    //    is srv's secondary shutdown trigger (parent-watch EOF).
    log("terminating children (SIGTERM → grace → SIGKILL)");
    terminate_child_gracefully(&host_child);
    terminate_child_gracefully(&srv_child);
    drop(_srv_stdin_keepalive);
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(1500),
        async {
            let _ = host_child.wait().await;
            let _ = srv_child.wait().await;
        },
    )
    .await;
    let _ = host_child.start_kill();
    let _ = srv_child.start_kill();
    log(&format!("launcher exiting with code {}", exit_code));
    std::process::exit(exit_code);
}

/// Windows main flow: resolve paths → create J0 → spawn srv → spawn
/// host with srv endpoints in env → supervised wait → cleanup.
#[cfg(target_os = "windows")]
async fn run_windows(
    launcher_exe_dir: &std::path::Path,
    real_exe: &std::path::Path,
    args: &[String],
) {

    let version = env!("CARGO_PKG_VERSION");

    // 1. Resolve data_dir / config_dir / user_home_dir. Both srv and
    // host receive these via env so they don't recompute (and so they
    // can't drift). Host's existing data_dir computation in sidecar.rs
    // still runs as a fallback for `task dev` mode where the launcher
    // is not in the loop.
    let paths = match data_dir::resolve_paths(launcher_exe_dir, version) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("FATAL: path resolution failed: {}", e));
            eprintln!("Failed to resolve AgentMux data directories: {}", e);
            std::process::exit(1);
        }
    };
    log(&format!(
        "paths resolved: data={} config={} user_home={} portable={}",
        paths.data_dir.display(),
        paths.config_dir.display(),
        paths.user_home_dir.display(),
        paths.portable_root.is_some(),
    ));
    if let Err(e) = data_dir::ensure_dirs(&paths) {
        log(&format!("FATAL: failed to create data dirs: {}", e));
        eprintln!("{}", e);
        std::process::exit(1);
    }

    // 2. Create the launcher's Job Object J0 BEFORE any spawn. Both
    // srv and host will be assigned to it (so they're siblings under
    // a single OS-enforced cleanup contract). Failure here drops us
    // into "degraded mode" — children spawn but won't be reaped on
    // launcher death.
    let job: Option<JobHandle> = match create_job_object() {
        Ok(handle) => {
            log("Job Object created (KILL_ON_JOB_CLOSE active)");
            Some(JobHandle(handle))
        }
        Err(e) => {
            log(&format!(
                "WARN: Job Object setup failed: {} (process-tree cleanup degraded)",
                e
            ));
            None
        }
    };
    let job_handle: windows_sys::Win32::Foundation::HANDLE =
        job.as_ref().map(|j| j.0).unwrap_or(std::ptr::null_mut());

    // Phase B.2: start the named-pipe IPC server BEFORE spawning
    // any children. Host connects to this pipe at startup using the
    // AGENTMUX_LAUNCHER_PIPE env var the launcher passes below.
    //
    // The server runs in its own Tokio task; the JoinHandle is held
    // for the rest of run_windows so the task isn't cancelled mid-
    // accept. Server owns the namespace `\\.\pipe\agentmux-{hash}\
    // command` per data dir, so multi-instance launchers (different
    // data dirs) get distinct pipes.
    //
    // Phase B.6: the bind itself is the single-instance signal.
    // `bind_first_pipe_instance` synchronously reserves the pipe;
    // a second launcher pointing at the same data dir gets
    // ERROR_ACCESS_DENIED. We surface that to the user as
    // "AgentMux is already running for this data directory" and
    // exit cleanly BEFORE spawning srv/host (otherwise the second
    // host would briefly contend on the CEF cache lockfile).
    // For release builds, CARGO_PKG_VERSION (semver) is the isolation key —
    // two different versions on the same channel get distinct pipes.
    // For local builds, package.sh bakes AGENTMUX_BUILD_LABEL (which includes
    // a per-build timestamp stamp), so each successive `task package` run gets
    // its own single-instance domain and can start a fresh window even while a
    // previous local build is running. Session data is still shared (data_dir
    // is keyed on channel+semver, not the label), so agents/auth carry over.
    let pipe_version = option_env!("AGENTMUX_BUILD_LABEL")
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    let dir_hash = hash::data_dir_hash16(&paths.data_dir, pipe_version);
    let pipe_path = ipc::pipe_name(&dir_hash);
    // Isolation telemetry: record exactly which keyed resources this instance
    // claims, so a cross-instance collision is diagnosable from the log alone
    // (two live PIDs claiming the same dir_hash) instead of inferred after a
    // vanished window. The launcher's job object is unnamed, so there is no
    // shared lifecycle handle to log. See
    // docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md.
    log(&format!(
        "instance_claim pid={} version={} data_dir={} dir_hash={} pipe={}",
        std::process::id(),
        pipe_version,
        paths.data_dir.display(),
        dir_hash,
        pipe_path
    ));
    let first_pipe = match ipc::server::bind_first_pipe_instance(&pipe_path) {
        Ok(p) => p,
        Err(e) => {
            // ERROR_ACCESS_DENIED (5) means another launcher already
            // owns this pipe — i.e., another AgentMux is running for
            // this data dir. The user-facing behavior matches the
            // status-bar version popup's "new window": forward an
            // `open_new_window` IPC POST to the existing host and
            // exit 0. The named-pipe bind is the AUTHORITATIVE
            // single-instance signal; this HTTP call is just the
            // forwarding hint. Other errors (namespace misconfig,
            // security descriptor failure) genuinely fail — show
            // the dialog and exit 2.
            const ERROR_ACCESS_DENIED: i32 = 5;
            let already_running = e.raw_os_error() == Some(ERROR_ACCESS_DENIED);
            log(&format!(
                "pipe bind failed (already_running={}): {} pipe={}",
                already_running, e, pipe_path
            ));
            if already_running {
                match forward_open_new_window(&paths.data_dir, &dir_hash) {
                    Ok(()) => {
                        log("forwarded open_new_window to existing instance — exiting 0");
                        std::process::exit(0);
                    }
                    Err(ForwardError::Transient(reason)) => {
                        // Transient race: the host is alive (pipe is
                        // held by the first launcher) but its
                        // forwarding hint isn't readable yet —
                        // typically because the host is mid-CEF-init
                        // and hasn't written `<data-dir>/ipc-port`
                        // yet. Silent exit so the user isn't punished
                        // for double-clicking quickly.
                        log(&format!("forward transient: {} — exiting 0 silently", reason));
                        std::process::exit(0);
                    }
                    Err(ForwardError::Fatal(reason)) => {
                        // Fatal forward failure: the port file IS
                        // readable, so the host got far enough to
                        // publish it, but the HTTP path is dead
                        // (connect refused, write failed). Could be
                        // a hung host, a port collision, or
                        // ERROR_ACCESS_DENIED that wasn't really
                        // "another instance" (namespace conflict).
                        // Surface the dialog so the user sees that
                        // something is genuinely broken rather than
                        // a silent no-op. (codex P2 PR #598.)
                        log(&format!("forward fatal: {} — surfacing dialog", reason));
                        show_fatal_dialog(
                            "AgentMux",
                            &format!(
                                "AgentMux appears to already be running but isn't responding.\n\nData dir: {}\nReason: {}\n\nClose any leftover AgentMux processes and try again. If the problem persists, check the launcher log.",
                                paths.data_dir.display(),
                                reason
                            ),
                        );
                        std::process::exit(2);
                    }
                }
            }
            // Genuine bind failure (not "already running"). Surface
            // it loudly because it indicates a system-level problem.
            show_fatal_dialog(
                "AgentMux",
                &format!(
                    "AgentMux failed to start: could not bind IPC pipe.\n\nPipe: {}\nError: {}\n\nIf the problem persists, check the launcher log.",
                    pipe_path, e
                ),
            );
            std::process::exit(2);
        }
    };
    // Spawn the native pre-splash immediately after claiming the
    // single-instance pipe — before srv spawn and CEF init.
    // The event name is forwarded to the CEF host as
    // AGENTMUX_SPLASH_EVENT so it can signal dismiss from on_load_end.
    #[cfg(target_os = "windows")]
    let splash_event_name = if splash_config::splash_disabled() {
        None // splash:disabled / AGENTMUX_SPLASH=0 — no event, no window (SPEC §6)
    } else {
        splash::spawn_splash(&dir_hash)
    };
    #[cfg(not(target_os = "windows"))]
    let splash_event_name: Option<String> = None;

    // Phase B.8 — broadcast bus for reducer-emitted events. Capacity
    // 1024 is comfortable headroom for the launcher's event volume
    // (~10–50 events per user action × handful of subscribers); a
    // lagging client gets `RecvError::Lagged` and reconnects.
    let (events_tx, _) = tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(1024);

    // Phase D.2 — event log: in-memory ring (replay source for D.3's
    // GetEvents) + optional disk persistence at
    // `<data-dir>/launcher-events.log` for crash forensics.
    let log_disk_path = paths.data_dir.join("launcher-events.log");
    let event_log = std::sync::Arc::new(event_log::EventLog::new(Some(log_disk_path)));
    let event_log_for_writer = std::sync::Arc::clone(&event_log);
    let disk_writer_rx = events_tx.subscribe();
    tokio::spawn(event_log::run_disk_writer(event_log_for_writer, disk_writer_rx));

    // Phase E.1a — canonical state shared between IPC server + saga
    // coordinator (and, in E.5, individual sagas). Single Mutex
    // owner, multiple readers via Arc.
    let state = std::sync::Arc::new(tokio::sync::Mutex::new(state::State::default()));

    // LSD-2 — open the durable launcher saga log at
    // `<data-dir>/db/launcher-sagas.db` (separate file from
    // `launcher-events.log`; the saga log is structured SQLite, the
    // event log is append-only JSONL). Failure to open is a launcher
    // startup error — without the log, sagas have no crash-recovery
    // story (LSD-3 walks `unresolved_sagas` to mark interrupted
    // sagas `failed_compensation`). Spec
    // `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` §3.1.
    //
    // `launcher_saga_log_path` performs the back-compat move from
    // the pre-AUDIT_SQLITE_SYSTEMS_2026_05_19.md location
    // (`<data-dir>/launcher-sagas.db` — outside `db/`) into the
    // canonical `db/` subdir alongside srv's SQLite files.
    let saga_log_path = data_dir::launcher_saga_log_path(&paths.data_dir);
    let saga_log = match saga::LauncherSagaLog::open(&saga_log_path) {
        Ok(l) => std::sync::Arc::new(l),
        Err(e) => {
            log(&format!(
                "FATAL: failed to open launcher saga log at {:?}: {}",
                saga_log_path, e
            ));
            std::process::exit(2);
        }
    };

    // LSD-3 — startup recovery walker. Walks the durable saga log,
    // marks any saga still in `running` / `compensating` / `failed`
    // (left over from a crashed prior run) as `failed_compensation`
    // so operators see them in `--diag sagas` and the next coordinator
    // run can't accidentally double-act on partially-applied effects.
    // MUST run BEFORE `tokio::spawn(saga::run_coordinator(..))` below
    // (LSD spec §5 risk #5: don't spawn while recovery is in progress).
    // Runs BEFORE LSD-4 vacuum so just-recovered sagas land in their
    // failed_compensation state for the operator to see — vacuum
    // honors the 7-day retention window and won't immediately purge.
    // Spec `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` §3.5.
    if let Err(e) = saga::compensate_unresolved_launcher_sagas(&saga_log).await {
        log(&format!(
            "[saga-recovery] WARN: walker failed: {} — coordinator will still spawn; prior crashed sagas remain unresolved until next restart",
            e
        ));
    }

    // LSD-4 — startup retention vacuum. Runs once per launcher boot,
    // before the coordinator subscribes, so any rows it deletes are
    // already terminal and can't possibly belong to an in-flight saga
    // the coordinator is about to drive (see `vacuum_older_than` SQL —
    // `running` and `compensating` rows are unreachable by the DELETE
    // regardless of timing). Failure is non-fatal.
    // Spec §3.6.
    let retention_days =
        config::load_saga_retention_days(&paths.user_home_dir, |w| log(w));
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days);
    match saga_log.vacuum_older_than(cutoff) {
        Ok(removed) => log(&format!(
            "[saga-log] vacuumed {} sagas older than {} (retention {} days)",
            removed, cutoff, retention_days
        )),
        Err(e) => log(&format!("[saga-log] WARN: vacuum failed: {}", e)),
    }

    // CPD-2 — launcher → host pipe wrapper. Owns the writer half of
    // the host's IPC connection (installed by the per-connection
    // handler in `ipc::server` once the host registers) and exposes
    // `send_command` / `send_event` to the rest of the launcher.
    // CPD-2 wires the wrapper + refactors event fanout for the host
    // connection to flow through here. CPD-3 wires this into the
    // saga coordinator's `apply_action` so `IssueCmd::Host` actions
    // dispatch live (no longer log-only).
    let host_pipe = std::sync::Arc::new(host_pipe::HostPipe::new(
        events_tx.clone(),
        std::sync::Arc::clone(&state),
    ));

    // Phase E.1a — saga coordinator task. Subscribes to the broadcast
    // bus, drives in-flight sagas. E.1a registry is empty — framework
    // only. E.5 adds the first concrete saga consumer (tear-off).
    // LSD-2 — durable saga log is now installed; every lifecycle
    // transition is persisted.
    // CPD-3 — install `host_pipe` so saga `IssueCmd::Host` actions
    // dispatch through the launcher → host wire instead of being
    // log-only.
    //
    // Subscribe BEFORE spawning so the race window between construction
    // and first `recv()` doesn't drop early events. (reagent P2 PR #609.)
    // Same pattern as the disk writer above.
    // with_log() can fail if max_saga_id() fails (e.g. corrupted SQLite
    // file). Treat as fatal — continuing with a default next_saga_id=1
    // while the log is attached would let the coordinator silently
    // mutate prior saga history on restart. Better to crash loudly so
    // operators see + investigate. (codex P1 PR #645 round 2.)
    let saga_coord_inner = saga::SagaCoordinator::new(events_tx.clone(), std::sync::Arc::clone(&state))
        .with_log(std::sync::Arc::clone(&saga_log))
        .unwrap_or_else(|e| {
            log(&format!(
                "[main] FATAL: failed to seed saga_id allocator from launcher_saga.max(saga_id): {} — refusing to start with degraded coordinator",
                e
            ));
            std::process::exit(1);
        })
        .with_host_pipe(std::sync::Arc::clone(&host_pipe));
    let saga_coord = std::sync::Arc::new(saga_coord_inner);
    let saga_rx = events_tx.subscribe();
    tokio::spawn(saga::run_coordinator(
        std::sync::Arc::clone(&saga_coord),
        saga_rx,
    ));

    let _ipc_handle = ipc::run_ipc_server(
        pipe_path.clone(),
        first_pipe,
        ipc::server::ServerCtx {
            launcher_pid: std::process::id(),
            launcher_version: env!("CARGO_PKG_VERSION").to_string(),
            state,
            events_tx,
            event_log,
            host_pipe: std::sync::Arc::clone(&host_pipe),
        },
    );
    log(&format!("IPC server started on {}", pipe_path));

    // 3. Spawn srv first. Host needs srv's endpoints to skip its own
    // spawn_backend path. Srv signals readiness via AGENTMUXSRV-ESTART on
    // stderr; the spawner returns once we see that line (or after a
    // 30s timeout).
    // Phase E.1b — pre-compute srv's pipe path (same data-dir hash
    // as launcher's pipe) and pass via env so srv binds it on
    // startup. Launcher is the sole authority for the data-dir hash.
    let srv_pipe_path = ipc::srv_pipe_name(&dir_hash);
    log(&format!("[ipc] srv pipe path = {}", srv_pipe_path));

    let (srv_result, mut srv_child) = match srv_spawner::spawn_srv(
        launcher_exe_dir,
        &paths,
        &srv_pipe_path,
        job_handle,
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            log(&format!("FATAL: srv spawn failed: {}", e));
            eprintln!("Failed to start backend: {}", e);
            drop(job);
            std::process::exit(1);
        }
    };

    // CRITICAL: tokio::process::Child::wait() proactively drops
    // self.stdin before waiting (tokio source comment: "Ensure stdin
    // is closed so the child can't read from it any more"). agentmux-
    // srv has a parent-watch loop on its own stdin — when stdin reads
    // 0 bytes (EOF from a closed write end), it interprets that as
    // "parent died" and shuts itself down. tokio's wait() would
    // trigger that within milliseconds, causing srv to exit before
    // the host even mounts its first browser. Move srv's stdin out
    // of the Child into a launcher-scope binding so tokio can't see
    // it (its take() returns None) and the pipe stays open for the
    // launcher's lifetime. (Smoke test on v0.33.447 caught this.)
    let _srv_stdin_keepalive = srv_child.stdin.take();

    // 4-6. Spawn the host (suspended) → assign to J0 → resume, via
    // spawn_host_supervised(). The splash event is passed on every launch
    // (including restarts) so a relaunched host still dismisses a pending
    // splash if the first host crashed before its first frame.
    let mut host_env = paths.common.to_env_vars();
    // Pass the version-scoped IPC hash to the host so it writes the
    // port file to `ipc-port-{hash}` rather than the shared `ipc-port`.
    // Prevents two running releases from overwriting each other's port
    // file (codex P1 on #1227).
    host_env.push(("AGENTMUX_IPC_HASH", std::ffi::OsString::from(&dir_hash)));
    let mut host_child = match spawn_host_supervised(
        real_exe,
        args,
        &srv_result,
        &host_env,
        &pipe_path,
        job.is_some(),
        job_handle,
        splash_event_name.as_deref(),
        false,
    ) {
        Some(c) => c,
        None => {
            // First-launch failure is fatal. Happy path: drop(job) →
            // KILL_ON_JOB_CLOSE reaps srv. Degraded path (J0 absent):
            // kill srv explicitly or it orphans (kill_on_drop is false).
            log("FATAL: could not start CEF host — terminating");
            eprintln!("Failed to launch AgentMux.");
            if job.is_none() {
                let _ = srv_child.start_kill();
            }
            drop(job);
            std::process::exit(1);
        }
    };

    // 7. Supervised wait loop (Phase 1 — host supervision). The host is
    // auto-restarted on abnormal exit, bounded by a crash budget so a
    // deterministic crash can't spin forever (spec §10-A). A clean host
    // exit (code 0) ends the loop. srv is NOT yet supervised — an srv
    // exit still terminates the launcher; srv supervision is Phase 2.
    //
    // We don't manually kill the surviving child in the happy path:
    // dropping `job` below triggers KILL_ON_JOB_CLOSE which reaps the
    // entire J0 membership. The explicit start_kill is the backstop for
    // degraded mode (job == None) only.
    log("entering supervised host + srv wait");
    let mut host_restarts: Vec<std::time::Instant> = Vec::new();
    // Separate budget for system-OOM host exits (memory-aware relaunch); see
    // mem_supervisor + SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.
    let mut oom_restarts: Vec<std::time::Instant> = Vec::new();
    let mut last_abnormal_code: Option<i32> = None;
    let mut host_degraded = false;
    let exit_code = loop {
        tokio::select! {
            host_status = host_child.wait() => {
                let code = match host_status {
                    Ok(s) => s.code().unwrap_or(1),
                    Err(e) => {
                        log(&format!("FATAL: host wait failed: {}", e));
                        break 1;
                    }
                };
                if code == 0 {
                    log("CEF host exited cleanly (code 0) — shutting down");
                    break 0;
                }
                // Classify: a *system-OOM* exit (the OS ran out of commit) is
                // transient and must be WAITED OUT, not hammered into the same
                // wall on the fast wedged-host budget — that just re-OOMs and
                // burns the budget into a silent give-up
                // (docs/retro/retro-oom-crash-2026-06-16.md,
                // SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.B). A genuine
                // host fault still takes the existing path below, unchanged.
                let commit_free = mem_supervisor::commit_free_mb();
                match mem_supervisor::classify_host_exit(code, commit_free) {
                    mem_supervisor::HostExitClass::SystemOom => {
                        let now = std::time::Instant::now();
                        if mem_supervisor::budget_exhausted(
                            &mut oom_restarts,
                            now,
                            mem_supervisor::OOM_RESTART_WINDOW,
                            mem_supervisor::OOM_RESTART_BUDGET,
                        ) {
                            log(&format!(
                                "CEF host hit system OOM (code {}, {} MB commit-free); OOM restart \
                                 budget exhausted ({} in {}s) — giving up",
                                code,
                                commit_free,
                                mem_supervisor::OOM_RESTART_BUDGET,
                                mem_supervisor::OOM_RESTART_WINDOW.as_secs()
                            ));
                            show_fatal_dialog(
                                mem_supervisor::OOM_GIVEUP_TITLE,
                                mem_supervisor::OOM_GIVEUP_BODY,
                            );
                            break code;
                        }
                        log(&format!(
                            "CEF host hit system OOM (code {}, {} MB commit-free) — waiting for \
                             memory to recover before relaunch",
                            code, commit_free
                        ));
                        // Commit-gated, backed-off wait. Relaunching into a
                        // starved system just re-OOMs; waiting is the only lever.
                        // Race it against srv death so the supervisor isn't blind
                        // to a concurrent srv exit during the wait (reagent P2).
                        // run_windows has no signal arms (shutdown flows via the
                        // host/srv), so srv is the only concurrent event here.
                        let recovered = tokio::select! {
                            r = mem_supervisor::await_commit_recovery(log) => r,
                            srv_status = srv_child.wait() => {
                                match srv_status {
                                    Ok(s) => log(&format!(
                                        "srv exited UNEXPECTEDLY during OOM wait with code {} — terminating launcher",
                                        s.code().unwrap_or(1)
                                    )),
                                    Err(e) => log(&format!("FATAL: srv wait failed during OOM wait: {}", e)),
                                }
                                break 1;
                            }
                        };
                        if !recovered {
                            show_fatal_dialog(
                                mem_supervisor::OOM_GIVEUP_TITLE,
                                mem_supervisor::OOM_GIVEUP_BODY,
                            );
                            break code;
                        }
                        // Relaunch degraded: the GPU process is a large commit
                        // consumer, so skip straight to software rendering for an
                        // OOM relaunch (SPEC §5.B.4).
                        match spawn_host_supervised(
                            real_exe,
                            args,
                            &srv_result,
                            &host_env,
                            &pipe_path,
                            job.is_some(),
                            job_handle,
                            splash_event_name.as_deref(),
                            true, // disable_gpu
                        ) {
                            Some(c) => host_child = c,
                            None => {
                                log("host relaunch failed to spawn — giving up");
                                break code;
                            }
                        }
                    }
                    mem_supervisor::HostExitClass::Abnormal => {
                        // Abnormal exit — relaunch within the crash budget.
                        let now = std::time::Instant::now();
                        host_restarts.retain(|t| now.duration_since(*t) < HOST_RESTART_WINDOW);
                        if host_restarts.len() >= HOST_RESTART_BUDGET {
                            log(&format!(
                                "CEF host exited abnormally (code {}); restart budget exhausted \
                                 ({} in {}s) — giving up",
                                code,
                                host_restarts.len(),
                                HOST_RESTART_WINDOW.as_secs()
                            ));
                            break code;
                        }
                        host_restarts.push(now);
                        // Crash classification + retry ladder (spec §7): a crash that
                        // reproduces the previous abnormal exit code is deterministic —
                        // step down to a degraded (--disable-gpu) relaunch so the retry
                        // isn't "the same thing again". Degraded is sticky; the ladder
                        // only steps down.
                        if last_abnormal_code == Some(code) {
                            host_degraded = true;
                        }
                        last_abnormal_code = Some(code);
                        log(&format!(
                            "CEF host exited abnormally (code {}) — relaunching (restart {}/{}{})",
                            code,
                            host_restarts.len(),
                            HOST_RESTART_BUDGET,
                            if host_degraded { ", degraded: --disable-gpu" } else { "" }
                        ));
                        match spawn_host_supervised(
                            real_exe,
                            args,
                            &srv_result,
                            &host_env,
                            &pipe_path,
                            job.is_some(),
                            job_handle,
                            splash_event_name.as_deref(),
                            host_degraded,
                        ) {
                            Some(c) => host_child = c,
                            None => {
                                log("host relaunch failed to spawn — giving up");
                                break code;
                            }
                        }
                    }
                }
            }
            srv_status = srv_child.wait() => {
                match srv_status {
                    Ok(s) => log(&format!(
                        "srv exited UNEXPECTEDLY (host still running) with code {} — terminating launcher",
                        s.code().unwrap_or(1)
                    )),
                    Err(e) => log(&format!("FATAL: srv wait failed: {}", e)),
                }
                break 1;
            }
        }
    };

    // 8. Cleanup. Happy path: drop(job) → KILL_ON_JOB_CLOSE reaps
    // the surviving child + CEF renderers. Degraded path (job is
    // None): explicit start_kill on both — neither will be reaped
    // by the OS, so we have to terminate them ourselves to avoid
    // orphans. (gemini PR #570 round-1 MEDIUM L105 / round-2 P1
    // backstop pattern.)
    if job.is_none() {
        log("WARN: J0 absent — explicitly killing surviving children");
        let _ = host_child.start_kill();
        let _ = srv_child.start_kill();
    }
    drop(job);
    log(&format!("launcher exiting with code {}", exit_code));
    std::process::exit(exit_code);
}

/// Append a timestamped line to ~/.agentmux/logs/agentmux-launcher.log.
/// Best-effort — silently no-ops if the log dir doesn't exist yet.
pub(crate) fn log(msg: &str) {
    let log_dir = dirs_fallback_home().join(".agentmux").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("agentmux-launcher.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] v{} {}", secs, env!("CARGO_PKG_VERSION"), msg);
    }
}

/// Home dir without depending on `dirs` for THIS specific lookup.
/// Kept to avoid a dirs dep cycle from log() — log() is called from
/// data_dir::resolve_paths via failure paths, and we want it to work
/// even if `dirs` itself is mid-failure.
fn dirs_fallback_home() -> std::path::PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Owns a Windows Job Object handle. CloseHandle on drop. The job's
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` flag means closing the last handle
/// terminates every assigned process — which is what we want as a backstop
/// if this launcher dies abruptly.
#[cfg(target_os = "windows")]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
unsafe impl Send for JobHandle {}

#[cfg(target_os = "windows")]
impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

/// Create a Job Object J0 with `KILL_ON_JOB_CLOSE`. Caller assigns
/// processes to it via `srv_spawner::assign_pid_to_job(pid, job)`.
#[cfg(target_os = "windows")]
fn create_job_object() -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::*;

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err("CreateJobObjectW returned null".into());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            CloseHandle(job);
            return Err("SetInformationJobObject failed".into());
        }
        Ok(job)
    }
}

/// Resume the (single) main thread of a CREATE_SUSPENDED process.
///
/// Walks a Toolhelp32 thread snapshot to find the one thread belonging
/// to `pid` (a freshly-spawned suspended process has only its main
/// thread), opens it with THREAD_SUSPEND_RESUME, and ResumeThread's it.
///
/// Errors come from snapshot creation, OpenThread, or ResumeThread
/// returning `(DWORD)-1`. A `ResumeThread` return of 0 means the thread
/// was already running (impossible if the process was just created
/// suspended) — treated as success.
#[cfg(target_os = "windows")]
pub(crate) fn resume_main_thread(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
        THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{
        OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err("CreateToolhelp32Snapshot failed".into());
        }

        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

        let mut found = false;
        if Thread32First(snap, &mut entry) != 0 {
            loop {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        let prev = ResumeThread(thread);
                        CloseHandle(thread);
                        if prev == u32::MAX {
                            CloseHandle(snap);
                            return Err(format!(
                                "ResumeThread failed for tid={}",
                                entry.th32ThreadID
                            ));
                        }
                        found = true;
                        break;
                    }
                }
                entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
                if Thread32Next(snap, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snap);
        if !found {
            return Err(format!("no thread found for pid={}", pid));
        }
        Ok(())
    }
}

/// Phase B.6 (post-fix) — forward an `open_new_window` request to
/// the already-running host and let this launcher exit 0.
///
/// The host writes `<data-dir>/ipc-port` after CEF init as
/// `port:token`. We open a TCP connection to 127.0.0.1:port, send a
/// minimal HTTP/1.1 POST to /ipc with the bearer token and a JSON
/// body, and bail. We deliberately do NOT pull in reqwest: the
/// launcher binary should stay tiny (~325 KB) and the protocol is
/// fixed, so a hand-rolled request is the right tool.
///
/// Failure classification (codex P2 PR #598):
/// - `Transient` — port file missing / unreadable / malformed.
///   The host is alive (pipe held) but mid-startup; caller exits
///   0 silently so the user isn't punished for double-clicking
///   quickly.
/// - `Fatal` — port file is readable, but the HTTP path failed
///   (connect refused, write failed, timeout). Either a hung
///   host or a non-running-instance source of
///   `ERROR_ACCESS_DENIED` (namespace conflict, security
///   descriptor failure). Caller surfaces the dialog so the user
///   sees a real problem rather than a silent no-op.
enum ForwardError {
    Transient(String),
    Fatal(String),
}

fn forward_open_new_window(data_dir: &std::path::Path, dir_hash: &str) -> Result<(), ForwardError> {
    // Read the version-scoped port file so we reach THIS version's host,
    // not a concurrent release's host that may have overwritten "ipc-port".
    let port_file_name = format!("ipc-port-{}", dir_hash);
    let port_file = data_dir.join(&port_file_name);
    let contents = std::fs::read_to_string(&port_file).map_err(|e| {
        ForwardError::Transient(format!("read {}: {}", port_file.display(), e))
    })?;
    let trimmed = contents.trim();
    let (port_str, token) = trimmed.split_once(':').ok_or_else(|| {
        ForwardError::Transient(format!(
            "malformed port file (expected port:token): {}",
            trimmed
        ))
    })?;
    let port: u16 = port_str
        .parse()
        .map_err(|e| ForwardError::Transient(format!("invalid port {:?}: {}", port_str, e)))?;

    // From here on the file was readable: any failure is a fatal
    // forward (the host got far enough to publish but isn't
    // serving the IPC port).
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2))
        .map_err(|e| ForwardError::Fatal(format!("connect 127.0.0.1:{}: {}", port, e)))?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(2)))
        .ok();

    let body = r#"{"cmd":"open_new_window"}"#;
    let req = format!(
        "POST /ipc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        token,
        body.len(),
        body
    );
    use std::io::{Read, Write};
    stream
        .write_all(req.as_bytes())
        .map_err(|e| ForwardError::Fatal(format!("write request: {}", e)))?;
    // CRITICAL: read at least the status line. The host's axum
    // handler is async — if the launcher closes the TCP socket
    // before axum has finished parsing + dispatching to
    // `open_new_window`, the request can be dropped (smoke caught
    // exactly this on v0.33.481: the launcher logged "forwarded"
    // but no second window appeared because the process exited
    // before axum ran the handler). We don't care about the body
    // — `Connection: close` lets the server drop the socket once
    // the response is written, so a single short read is enough
    // to keep the connection alive past handler dispatch.
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .ok();
    let mut sink = [0u8; 64];
    let _ = stream.read(&mut sink);
    Ok(())
}

/// Best-effort `open_new_window` forward for the unix second-instance path.
/// Unlike the Windows path (which pops a dialog on a fatal forward), unix just
/// logs and lets the caller exit 0: the existing instance is alive (its socket
/// answered our connect probe), so a transient/fatal forward failure shouldn't
/// block — at worst the relaunch is a silent no-op instead of a new window.
/// SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md.
#[cfg(not(target_os = "windows"))]
fn forward_open_new_window_or_log(data_dir: &std::path::Path, dir_hash: &str) {
    match forward_open_new_window(data_dir, dir_hash) {
        Ok(()) => log("forwarded open_new_window to existing instance"),
        Err(ForwardError::Transient(reason)) => {
            log(&format!("open_new_window forward transient (host mid-startup?): {}", reason))
        }
        Err(ForwardError::Fatal(reason)) => {
            log(&format!("open_new_window forward failed: {}", reason))
        }
    }
}

/// Show a modal error dialog before the launcher exits. Used for
/// genuine bind failures (NOT the "already running" path — that
/// silently forwards via `forward_open_new_window`). Without this,
/// the launcher exit is silent (it has the `windows` subsystem in
/// release, so eprintln! goes nowhere).
#[cfg(target_os = "windows")]
fn show_fatal_dialog(title: &str, body: &str) {
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
fn show_fatal_dialog(_title: &str, body: &str) {
    eprintln!("{}", body);
}

/// Find the CEF host binary in the runtime directory.
/// Tries versioned name first (agentmux-X.Y.Z.exe), then the old
/// agentmux-cef-X.Y.Z.exe pattern for backwards compat, then plain
/// agentmux-cef.exe (dev mode).
fn find_cef_binary(runtime_dir: &std::path::Path) -> std::path::PathBuf {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };

    let versioned = format!("agentmux-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_path = runtime_dir.join(&versioned);
    if versioned_path.exists() {
        return versioned_path;
    }

    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        let prefix = "agentmux-";
        let cef_prefix = "agentmux-cef";
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix)
                && !name.starts_with(cef_prefix)
                && !name.starts_with("agentmux-srv")
                // CRITICAL for the flat dev layout (macOS/Linux Phase 1):
                // launcher + host + srv share one dir, so the launcher
                // binary itself matches `agentmux-*`. Without this guard
                // the launcher resolves ITSELF as the host and spawns a
                // recursive launcher fork bomb. On Windows the launcher
                // lives at the root (not in runtime/), so this is a no-op.
                && !name.starts_with("agentmux-launcher")
                && name.ends_with(ext)
            {
                return entry.path();
            }
        }
    }

    let versioned_old = format!("agentmux-cef-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_old_path = runtime_dir.join(&versioned_old);
    if versioned_old_path.exists() {
        return versioned_old_path;
    }

    runtime_dir.join(format!("agentmux-cef{}", ext))
}
