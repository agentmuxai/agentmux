// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::job_object::{create_job_object, JobHandle};
use crate::host_spawn::spawn_host_supervised;
use crate::logging::log;
use crate::second_instance::{forward_open_new_window, ForwardError};
use crate::show_fatal_dialog;
use crate::supervisor::{HOST_RESTART_BUDGET, HOST_RESTART_WINDOW};
use crate::{
    config, data_dir, event_log, hash, host_pipe, ipc, mem_supervisor, saga, srv_spawner,
    startup_events, state,
};

/// SPEC_PILLAR1_STEP4 Phase 4 — re-arm the pre-splash for a crash-restart.
/// Not a no-op wrapper around `spawn_splash`: it exists so both restart
/// branches (OOM, Abnormal) share one call site rather than duplicating the
/// "new channel, new splash, restoring=true" triplet inline. Respects
/// `AGENTMUX_SPLASH=0` (`splash_disabled()`) the same as the cold-start path.
/// No stage-telemetry events are ever sent into the fresh channel — a
/// restart doesn't re-run saga recovery/migrations/etc., so the splash shows
/// only the "Restoring session..." headline and the pulsing brain, nothing
/// from the (empty) stage list.
#[cfg(target_os = "windows")]
fn respawn_splash_for_restart(dir_hash: &str) -> Option<String> {
    if crate::splash_config::splash_disabled() {
        return None;
    }
    let (_sink, rx) = startup_events::StartupEventSink::new();
    crate::splash::spawn_splash(dir_hash, rx, true)
}

/// Windows main flow: resolve paths → create J0 → spawn srv → spawn
/// host with srv endpoints in env → supervised wait → cleanup.
#[cfg(target_os = "windows")]
pub(crate) async fn run_windows(
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
    // Startup telemetry bus: events flow from each startup stage to the splash.
    let (startup_sink, startup_rx) = startup_events::StartupEventSink::new();

    // Spawn the native pre-splash immediately after claiming the
    // single-instance pipe — before srv spawn and CEF init.
    // The event name is forwarded to the CEF host as
    // AGENTMUX_SPLASH_EVENT so it can signal dismiss from on_load_end.
    // SPEC_PILLAR1_STEP4 Phase 4 — `mut`: a crash-restart branch below
    // re-spawns the splash (a fresh event + consumer thread, since this
    // first thread's own event is one-shot — see `spawn_splash`'s doc
    // comment) and reassigns this to the new event name.
    #[cfg(target_os = "windows")]
    let mut splash_event_name = if crate::splash_config::splash_disabled() {
        drop(startup_rx); // no splash — let senders fail silently
        None // splash:disabled / AGENTMUX_SPLASH=0 — no event, no window (SPEC §6)
    } else {
        crate::splash::spawn_splash(&dir_hash, startup_rx, false)
    };
    #[cfg(not(target_os = "windows"))]
    let splash_event_name: Option<String> = { drop(startup_rx); None };

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

    // Saga recovery/vacuum/coordinator-setup/IPC-server-startup run
    // concurrently with the srv boot below (tokio::join!) instead of
    // sequentially before it. Neither branch depends on the other's
    // output: this setup never reads srv_result, and srv never connects
    // back to the launcher's IPC pipe during its own startup — the
    // sequential ordering was incidental program order, not a real
    // dependency. LSD-3's "recovery MUST run before coordinator spawn"
    // requirement is still honored — both happen inside the same
    // `launcher_setup` future, in order, just no longer serialized
    // against srv. See SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_
    // 2026_07_02.md §A.4 item 5 (full parallelization with host spawn
    // was investigated and found genuinely blocked — see item 1 there —
    // this overlap is the safe subset that doesn't have that problem).
    let launcher_setup = async {
        // LSD-3 — startup recovery walker. Walks the durable saga log,
        // marks any saga still in `running` / `compensating` / `failed`
        // (left over from a crashed prior run) as `failed_compensation`
        // so operators see them in `--diag sagas` and the next coordinator
        // run can't accidentally double-act on partially-applied effects.
        // MUST run BEFORE `tokio::spawn(saga::run_coordinator(..))` below
        // (LSD spec §5 risk #5: don't spawn while recovery is in progress).
        // Spec `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` §3.5.
        startup_sink.stage_begin("saga", "Saga recovery");
        let saga_t = std::time::Instant::now();
        if let Err(e) = saga::compensate_unresolved_launcher_sagas(&saga_log).await {
            log(&format!(
                "[saga-recovery] WARN: walker failed: {} — coordinator will still spawn; prior crashed sagas remain unresolved until next restart",
                e
            ));
        }

        // Vacuum terminal saga rows older than the configured retention window.
        {
            let retention_days = config::load_saga_retention_days(&paths.user_home_dir, |w| log(w));
            let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days);
            match saga_log.vacuum_older_than(cutoff) {
                Ok(n) if n > 0 => log(&format!("[saga-vacuum] removed {} terminal rows older than {} days", n, retention_days)),
                Ok(_) => {}
                Err(e) => log(&format!("[saga-vacuum] WARN: vacuum failed: {}", e)),
            }
        }

        startup_sink.stage_end(
            "saga",
            saga_t.elapsed().as_millis() as u64,
            startup_events::StartupStatus::Ok,
            None,
        );

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

        let ipc_handle = ipc::run_ipc_server(
            pipe_path.clone(),
            first_pipe,
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
        log(&format!("IPC server started on {}", pipe_path));

        (saga_coord, ipc_handle)
    };

    // 3b. Spawn srv. Host needs srv's endpoints to skip its own
    // spawn_backend path. Srv signals readiness via AGENTMUXSRV-ESTART on
    // stderr; the spawner returns once we see that line (or after a
    // 30s timeout).
    // Phase E.1b — pre-compute srv's pipe path (same data-dir hash
    // as launcher's pipe) and pass via env so srv binds it on
    // startup. Launcher is the sole authority for the data-dir hash.
    let srv_pipe_path = ipc::srv_pipe_name(&dir_hash);
    log(&format!("[ipc] srv pipe path = {}", srv_pipe_path));

    let (setup_result, srv_spawn_result) = tokio::join!(
        launcher_setup,
        srv_spawner::spawn_srv(launcher_exe_dir, &paths, &srv_pipe_path, job_handle, &startup_sink)
    );
    let (saga_coord, _ipc_handle) = setup_result;
    let (srv_result, mut srv_child) = match srv_spawn_result {
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
    // "host" stage covers process-spawn latency (begin → spawn_host_supervised
    // returning a live Child), including the suspend → job-assign → resume
    // dance (see resume_main_thread's Toolhelp32 snapshot walk in
    // host_spawn.rs) — not full first-paint. The first-paint signal
    // (splash_event_name) is consumed exclusively by the splash's own wait;
    // having the supervisor also wait on it risks double-signaling semantics
    // on the named event. Extending this stage to span to first-paint is a
    // follow-up once a race-safe signal exists (see SPEC_MACOS_LAUNCH_SPEED_
    // AND_SPLASH_TELEMETRY_2026_07_02.md §B.7). Mirrors unix.rs's "host" stage.
    startup_sink.stage_begin("host", "Host startup");
    let host_spawn_t = std::time::Instant::now();
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
                        //
                        // SPEC_PILLAR1_STEP4 Phase 4 — re-spawn the splash (fresh
                        // event + consumer thread) with the "Restoring session..."
                        // headline rather than reusing `splash_event_name` as-is:
                        // the original splash thread already exited after the
                        // cold-start dismiss, so nothing was listening on that
                        // event name — the host would signal it and nothing would
                        // happen. See `respawn_splash_for_restart`.
                        splash_event_name = respawn_splash_for_restart(&dir_hash);
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
                        // SPEC_PILLAR1_STEP4 Phase 4 — see the OOM branch above for
                        // why this re-spawns the splash rather than reusing
                        // `splash_event_name` as-is.
                        splash_event_name = respawn_splash_for_restart(&dir_hash);
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

    // Close any open saga brackets before J0 is dropped — same reason
    // as the Unix path above (prevent spurious LSD-3 compensation).
    saga_coord.cancel_all_in_flight("launcher shutting down").await;

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
