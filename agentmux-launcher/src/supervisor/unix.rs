// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::host_spawn::{spawn_host_unix, terminate_child_gracefully};
use crate::logging::log;
use crate::second_instance::bind_socket_with_recovery;
use crate::show_fatal_dialog;
use crate::supervisor::{HOST_RESTART_BUDGET, HOST_RESTART_WINDOW};
use crate::{
    data_dir, event_log, hash, host_pipe, ipc, mem_supervisor, other_instances, saga,
    srv_spawner, startup_events, state,
};

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
pub(crate) async fn run_unix(
    launcher_exe_dir: &std::path::Path,
    real_exe: &std::path::Path,
    args: &[String],
    // Linux: pre-created sink whose rx was handed to the splash thread.
    // macOS / other Unix: None; a fresh sink (rx dropped) is created below.
    startup_sink_opt: Option<startup_events::StartupEventSink>,
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
    let first_socket = bind_socket_with_recovery(&socket_path, &paths.data_dir, &dir_hash, &paths.common.channel);

    // macOS: we are the first instance (second-instance launches exit inside
    // bind_socket_with_recovery). Publish this instance's bound-socket identity
    // to the splash's reopen handler so a Finder double-click / `open` (no -n)
    // forwards open_new_window to exactly THIS (channel, version) instance —
    // never a recomputed hash. SPEC_MACOS_REOPEN_NEW_WINDOW_2026_06_22.md.
    #[cfg(target_os = "macos")]
    crate::splash_mac::set_reopen_target(paths.data_dir.clone(), dir_hash.clone());

    // Task #35 (SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md P1) — read-only
    // detection of other, OLDER AgentMux instances still running. Mirrors
    // the Windows call site: we only reach this point once we've WON the
    // single-instance socket bind above. Detection + logging ONLY — see
    // `other_instances.rs` doc comment.
    //
    // reagent (PR #2117 round 1): synchronous I/O (fs::read_dir, socket
    // connect probes), no `.await` in the body — spawn_blocking instead of
    // plain tokio::spawn so it runs on the blocking-task pool and can never
    // occupy/starve an async worker the IPC accept loop or host supervision
    // needs, especially given channels/local-* dirs are documented to
    // accumulate unpruned (more siblings to walk/probe over time).
    {
        let channels_root = paths.common.home_dir.join("channels");
        let own_channel = paths.common.channel.clone();
        let own_version = version.to_string();
        tokio::task::spawn_blocking(move || {
            other_instances::log_older_running_instances(&channels_root, &own_channel, &own_version);
        });
    }

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

    // Pillar 1 Step 6 — in-memory saga registry (see the Windows
    // supervisor's comment for the full rationale). Best-effort cleanup of
    // the legacy durable files.
    let saga_log = std::sync::Arc::new(saga::LauncherSagaLog::new());
    for legacy in [
        paths.data_dir.join("db").join("launcher-sagas.db"),
        paths.data_dir.join("launcher-sagas.db"),
    ] {
        for suffix in ["", "-wal", "-shm"] {
            let p = std::path::PathBuf::from(format!("{}{}", legacy.display(), suffix));
            if p.exists() && std::fs::remove_file(&p).is_ok() {
                log(&format!("[saga] removed legacy durable saga log file {:?} (Step 6: registry is in-memory)", p));
            }
        }
    }

    // Startup telemetry bus.
    // Linux: sink was created in main() before the splash thread launched;
    // its rx is already being drained by the splash — reuse it so stage
    // events flow live. macOS/other: create a fresh sink and drop rx
    // (macOS splash doesn't yet consume typed events).
    let startup_sink = startup_sink_opt.unwrap_or_else(|| {
        let (s, rx) = startup_events::StartupEventSink::new();
        drop(rx);
        s
    });

    // Saga coordinator-setup/IPC-server-startup run concurrently with the
    // srv boot below (tokio::join!) instead of sequentially before it.
    // Neither branch depends on the other's output: this setup never reads
    // srv_result, and srv never connects back to the launcher's IPC socket
    // during its own startup. See SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_
    // TELEMETRY_2026_07_02.md §A.4 item 5.
    let launcher_setup = async {
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

        let ipc_handle = ipc::run_ipc_server(
            socket_path.clone(),
            first_socket,
            ipc::server::ServerCtx {
                launcher_pid: std::process::id(),
                launcher_version: env!("CARGO_PKG_VERSION").to_string(),
                state,
                events_tx,
                event_log,
                host_pipe: std::sync::Arc::clone(&host_pipe),
                startup_sink: Some(startup_sink.clone()),
            },
        );
        log(&format!("IPC server started on {}", socket_path));

        // `host_pipe` is also returned to the supervisor loop — the Linux
        // teardown-backstop UI-liveness prober (SPEC_LAUNCHER_TEARDOWN_
        // BACKSTOP_2026_07_11, issue #2189) sends `Command::ProbeUiThread`
        // over it directly, mirroring `run_windows`.
        (saga_coord, ipc_handle, host_pipe)
    };

    // 2b. Spawn srv. The srv pipe path is the launcher-owned socket
    //    path scope (srv will gain its own Unix-socket bind in a
    //    follow-up; for now we still pass an empty string so srv's
    //    Windows-only IPC code stays disabled).
    let srv_pipe_path = String::new();
    let (setup_result, srv_spawn_result) = tokio::join!(
        launcher_setup,
        srv_spawner::spawn_srv(launcher_exe_dir, &paths, &srv_pipe_path, &startup_sink)
    );
    let (saga_coord, _ipc_handle, host_pipe) = setup_result;
    let (srv_result, mut srv_child) = match srv_spawn_result {
        Ok(pair) => pair,
        Err(e) => {
            log(&format!("FATAL: srv spawn failed: {}", e));
            eprintln!("Failed to start backend: {}", e);
            std::process::exit(1);
        }
    };

    // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 (Linux Phase 2, issue #2189):
    // srv's own pid IS the backstop process-group id — `srv_spawner::spawn_srv`
    // gave srv `.process_group(0)` (Linux-only), making it the leader of a
    // brand-new group separate from the launcher's own. host joins this group
    // at every spawn below. Captured once: srv is never restarted mid-session
    // (an unexpected srv exit always terminates the launcher — see the srv arm
    // of the select loop below), so this stays valid for the launcher's whole
    // lifetime.
    //
    // `None` (backstop disabled for this session, not "0") is used if srv's
    // `Child` somehow has no pid at this point — deliberately NOT
    // `unwrap_or(0)`: pgid 0 has a DIFFERENT special meaning to `killpg(2)`
    // ("my own process group"), so a stray literal 0 here could make a later
    // teardown kill the LAUNCHER's own group instead of doing nothing. Every
    // consumer below treats `None` as "skip the backstop, still spawn host
    // normally" — never as a fallback value.
    #[cfg(target_os = "linux")]
    let backstop_pgid: Option<i32> = srv_child.id().map(|p| p as i32);
    #[cfg(not(target_os = "linux"))]
    let backstop_pgid: Option<i32> = None;
    #[cfg(target_os = "linux")]
    if backstop_pgid.is_none() {
        log("WARN: srv spawned with no pid — Linux teardown-backstop disabled for this session");
    }

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
    // "host" stage currently covers process-spawn latency only (begin →
    // spawn_host_unix returning a live Child), not full first-paint — the
    // signal for "host painted its first frame" (AGENTMUX_SPLASH_READY_FILE)
    // is written by the host process itself and consumed exclusively by the
    // splash's own poll loop (splash_mac.rs / splash_linux); having the
    // supervisor also poll/remove that file risks a race with the splash's
    // consumer. Extending this stage to span to first-paint is a follow-up
    // once a race-safe signal exists (see SPEC_MACOS_LAUNCH_SPEED_AND_
    // SPLASH_TELEMETRY_2026_07_02.md §B.7). Spawn latency is still a real,
    // independently useful number.
    startup_sink.stage_begin("host", "Host startup");
    let host_spawn_t = std::time::Instant::now();
    let mut host_child = match spawn_host_unix(real_exe, args, &srv_result, &host_env, false, backstop_pgid) {
        Some(c) => {
            startup_sink.stage_end(
                "host",
                host_spawn_t.elapsed().as_millis() as u64,
                startup_events::StartupStatus::Ok,
                None,
            );
            c
        }
        None => {
            startup_sink.stage_end(
                "host",
                host_spawn_t.elapsed().as_millis() as u64,
                startup_events::StartupStatus::Error,
                Some("spawn failed".to_string()),
            );
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

    // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 (Linux Phase 2, issue #2189)
    // — mirrors `run_windows`'s two low-rate ticks exactly (see that file for
    // the full rationale); `teardown_backstop` / `ui_liveness` are already
    // platform-neutral and the arm/disarm hooks in `ipc/server.rs` already
    // fire for both platforms. Only the tick + the kill action are new here.
    //
    // NOT `#[cfg(target_os = "linux")]`-gated at this declaration/arm level —
    // `tokio::select!` doesn't support `#[cfg]` attributes on branches at
    // all. Instead these intervals + arms compile and tick on every Unix
    // platform, but do nothing on macOS: `backstop_pgid` is unconditionally
    // `None` there (captured above), and every consumer below branches on
    // that Option — never on `target_os`. This is deliberately the SAME
    // pattern as macOS today already: the shared reducer hooks in
    // `ipc/server.rs` can already `arm()` this state machine on any
    // platform (nothing about PoolDrained/OrphanInstance detection is
    // Linux-specific); only Windows previously consumed it. Disarming
    // unconditionally on host exit (below) is strictly better hygiene than
    // before this change, not new platform-specific behavior.
    let mut ui_probe_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(60),
    );
    ui_probe_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ui_probe_nonce: u64 = 0;
    let mut teardown_check_interval = tokio::time::interval(std::time::Duration::from_secs(5));
    teardown_check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Same sentinel exit code as `run_windows`'s `TerminateJobObject` path —
    // keeps wedge-teardown unambiguous in any log/exit-code triage regardless
    // of platform.
    const TEARDOWN_BACKSTOP_EXIT_CODE: i32 = 86;

    let exit_code = loop {
        tokio::select! {
            // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 (issue #2189). Only
            // ever fires (past the Option check) on Linux — `backstop_pgid`
            // is `None` on every other platform.
            _ = teardown_check_interval.tick() => {
                if let Some(pgid) = backstop_pgid {
                    let misses = crate::ui_liveness::consecutive_misses();
                    if crate::teardown_backstop::should_teardown(misses) {
                        log(&format!(
                            "[teardown-backstop] host wedged with zero user windows — terminating \
                             process group {} (armed > {}s grace, {} consecutive unanswered UI-thread probes)",
                            pgid,
                            crate::teardown_backstop::TEARDOWN_GRACE.as_secs(),
                            misses,
                        ));
                        // I2/I3 hold by construction: `pgid` is the group srv
                        // created for itself at spawn (`.process_group(0)`)
                        // and host joined (`.process_group(pgid)`) — the
                        // launcher was never a member. Blast radius is exactly
                        // {srv, host, host's CEF-spawned descendants}.
                        //
                        // reagent P1 (PR #2204): agent-spawned PTY shells
                        // (host's `run_command` via `setsid()`,
                        // `agentmux-srv/src/backend/shell_node.rs`'s own
                        // `.process_group(0)`) deliberately escape into THEIR
                        // OWN process groups — same mechanism this backstop
                        // uses, applied for a different scoping purpose — so
                        // they are NOT members of `pgid` and a bare
                        // `killpg(pgid, SIGKILL)` would orphan them. This is
                        // NOT a Linux-only gap: srv's own per-agent Windows
                        // `JobObjectTracker` (`backend/process_tracker/
                        // windows.rs`) creates a brand-new, unnested job per
                        // agent block — `TerminateJobObject(J0)` doesn't reach
                        // those either. Hard, kernel-guaranteed cleanup of
                        // every agent subprocess tree on ANY teardown path is
                        // out of scope here (it would mean duplicating
                        // `process_tracker`'s own pid/job bookkeeping).
                        //
                        // What we CAN cheaply do: srv (unlike host) is not
                        // presumed wedged by this scenario — only the host's
                        // UI thread failed to answer probes — so give the
                        // group a brief chance to shut down via SIGTERM
                        // first. srv's own signal handler
                        // (`agentmux-srv/src/main.rs`, SIGINT/SIGTERM →
                        // `shell_sessions.stop_all()`) already tears down its
                        // TRACKED agent shells correctly on a graceful exit —
                        // by PID/job, not by process-group membership, so it
                        // reaches the escaped subgroups this killpg can't.
                        // SIGKILL remains the hard backstop for whatever's
                        // still alive after a short grace (same 1500ms window
                        // `terminate_child_gracefully` uses elsewhere in this
                        // file) — covers a wedged-or-dead srv, or shells
                        // `stop_all()` itself couldn't reap in time.
                        let sigterm_ok = unsafe { libc::killpg(pgid, libc::SIGTERM) };
                        if sigterm_ok != 0 {
                            log(&format!(
                                "[teardown-backstop] killpg({}, SIGTERM) failed: {} — \
                                 group may already be empty, proceeding to SIGKILL",
                                pgid,
                                std::io::Error::last_os_error()
                            ));
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        let sigkill_ok = unsafe { libc::killpg(pgid, libc::SIGKILL) };
                        if sigkill_ok != 0 {
                            let err = std::io::Error::last_os_error();
                            if err.raw_os_error() == Some(libc::ESRCH) {
                                log(&format!(
                                    "[teardown-backstop] killpg({}, SIGKILL): group already empty \
                                     (SIGTERM + srv's own shutdown handler reaped it)",
                                    pgid
                                ));
                            } else {
                                log(&format!(
                                    "[teardown-backstop] killpg({}, SIGKILL) failed: {}",
                                    pgid, err
                                ));
                            }
                        }
                        break TEARDOWN_BACKSTOP_EXIT_CODE;
                    }
                }
            }
            // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 (issue #2189) —
            // Phase 1 UI-thread liveness prober, mirroring `run_windows`. The
            // `if` guard means this branch is simply skipped (re-polled next
            // tick) whenever the backstop is disabled (`backstop_pgid: None`
            // — every platform except Linux, or Linux with no srv pid).
            _ = ui_probe_interval.tick(), if backstop_pgid.is_some() => {
                ui_probe_nonce += 1;
                if let Some((missed, sent_at)) = crate::ui_liveness::record_probe_sent(ui_probe_nonce) {
                    log(&format!(
                        "[ui-liveness] probe nonce={} unanswered after {}s — UI thread did not pump in that window",
                        missed,
                        sent_at.elapsed().as_secs()
                    ));
                }
                if let Err(e) = host_pipe
                    .try_send_command_no_buffer(&agentmux_common::ipc::Command::ProbeUiThread {
                        nonce: ui_probe_nonce,
                    })
                    .await
                {
                    crate::ui_liveness::retract_probe(ui_probe_nonce);
                    log(&format!(
                        "[ui-liveness] probe send failed (transport, not liveness): {:?}",
                        e
                    ));
                }
            }
            host_status = host_child.wait() => {
                use std::os::unix::process::ExitStatusExt;
                // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 (issue #2189) —
                // the host exiting for ANY reason (clean, crash, or the
                // supervisor about to relaunch it) always stands an armed
                // teardown down: the machine's whole premise is "host alive
                // but wedged", which is now false regardless of why. Also the
                // crash-restart-gap false-positive guard — re-arming can only
                // come from a fresh drain report by the NEW host. Not
                // Linux-gated: the shared `ipc/server.rs` reducer hooks can
                // already `arm()` this on any platform, so disarming here
                // unconditionally is just better hygiene everywhere, not new
                // platform-specific behavior.
                if crate::teardown_backstop::disarm() {
                    log("[teardown-backstop] disarmed — host exited (supervised-exit path owns it now)");
                }
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
                        match spawn_host_unix(real_exe, args, &srv_result, &host_env, true, backstop_pgid) {
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
                        match spawn_host_unix(real_exe, args, &srv_result, &host_env, host_degraded, backstop_pgid) {
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

    // Close any open saga brackets before tearing down children so the
    // durable log doesn't carry dangling SagaStarted entries into the
    // next startup's LSD-3 compensation pass.
    saga_coord.cancel_all_in_flight("launcher shutting down").await;

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
