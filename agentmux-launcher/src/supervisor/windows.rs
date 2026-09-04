// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::job_object::{create_job_object, JobHandle};
use crate::host_spawn::spawn_host_supervised;
use crate::logging::log;
use crate::second_instance::{forward_open_new_window, ForwardError};
use crate::show_fatal_dialog;
use crate::supervisor::{
    HOST_RESTART_BUDGET, HOST_RESTART_WINDOW, SRV_RESTART_BUDGET, SRV_RESTART_WINDOW,
};
use crate::{
    data_dir, event_log, hash, host_pipe, ipc, mem_supervisor, other_instances, saga,
    srv_spawner, startup_events, state,
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
///
/// `seq` (reagent P2, PR #2032, 2026-07-08) makes each restart's Win32 event
/// + window class name genuinely unique (`AgentMuxSplash-{dir_hash}-r{seq}`),
/// rather than reusing the SAME name every restart. Reusing the name is
/// unsafe in exactly the repeated-crash-loop scenario `HOST_RESTART_BUDGET`
/// exists to bound: if a newly-respawned host crashes again before ever
/// calling `on_load_end` (so the prior splash thread is still blocked in its
/// `WaitForSingleObject` loop, `splash.rs`'s dismiss check, holding the
/// event open), `CreateEventW` with an already-open name returns a handle to
/// that SAME still-live object instead of a fresh one — both splash threads
/// then wait on one shared event and can render stacked/duplicate splash
/// windows simultaneously. A per-restart unique name sidesteps the collision
/// entirely; the caller passes a monotonic counter that only increases.
///
/// The unique name alone only stops two splashes from *sharing one event* —
/// it does nothing to stop the PREVIOUS restart's splash from being
/// orphaned if its host crashed again before calling `on_load_end` (reagent
/// P1, PR #2032, 2026-07-08). Every call site MUST call
/// `crate::splash::dismiss_splash` on the outgoing `splash_event_name`
/// immediately before calling this function, so the old thread tears itself
/// down via its own normal dismiss path instead of leaking forever.
#[cfg(target_os = "windows")]
fn respawn_splash_for_restart(dir_hash: &str, seq: u32) -> Option<String> {
    if crate::splash_config::splash_disabled() {
        return None;
    }
    let (_sink, rx) = startup_events::StartupEventSink::new();
    let unique_id = format!("{}-r{}", dir_hash, seq);
    crate::splash::spawn_splash(&unique_id, rx, true)
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

    // Task #35 (SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md P1) — read-only
    // detection of other, OLDER AgentMux instances still running (the
    // scenario that stacks multiple CEF processes' memory/commit-charge
    // overhead across an upgrade). We only reach this point once we've WON
    // the single-instance pipe bind above, so we know we're not about to
    // exit as a forwarded second instance. Detection + logging ONLY — no
    // prompt, no dialog, no IPC to the other instance beyond the liveness
    // probe itself; see `other_instances.rs` doc comment for why the
    // cross-instance-quit half is deliberately out of scope.
    //
    // reagent (PR #2117 round 1): the body is synchronous I/O (`fs::read_dir`,
    // named-pipe connect probes) with no `.await` anywhere in it — running it
    // under plain `tokio::spawn` occupies a multi-thread runtime worker
    // instead of yielding it, worse than usual here since `channels/local-*`
    // dirs are documented to accumulate unpruned (CLAUDE.md), so a dev
    // machine can have many stale siblings to walk/probe. `spawn_blocking`
    // runs it on the blocking-task pool instead, alongside every other
    // synchronous-I/O task in this codebase, so it can never starve the
    // async worker pool the IPC accept loop / host supervision share.
    {
        let channels_root = paths.common.home_dir.join("channels");
        let own_channel = paths.common.channel.clone();
        let own_version = version.to_string();
        tokio::task::spawn_blocking(move || {
            other_instances::log_older_running_instances(&channels_root, &own_channel, &own_version);
        });
    }

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

    // Pillar 1 Step 6 (SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16) — the
    // saga registry is in-memory now. The durable SQLite log, its startup
    // recovery walker (which never compensated anything — it only wrote
    // failed_compensation tombstones), and the retention vacuum are gone;
    // retention is a bounded in-memory cap inside the registry. Best-effort
    // cleanup of the legacy on-disk files so operators don't mistake a
    // stale sagas.db for live state.
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

    // Saga coordinator-setup/IPC-server-startup run concurrently with the
    // srv boot below (tokio::join!) instead of sequentially before it.
    // Neither branch depends on the other's output: this setup never reads
    // srv_result, and srv never connects back to the launcher's IPC pipe
    // during its own startup. See SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_
    // TELEMETRY_2026_07_02.md §A.4 item 5.
    let launcher_setup = async {
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

        // `host_pipe` is also returned to the supervisor loop for the
        // UI-liveness prober (SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 1).
        (saga_coord, ipc_handle, host_pipe)
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
    let (saga_coord, _ipc_handle, host_pipe) = setup_result;
    // `mut`: SPEC_SRV_SUPERVISION_RECYCLE — an srv respawn rebinds this to
    // the new endpoints; the subsequent (deliberate) host restart then picks
    // them up through the existing `spawn_host_supervised(..., &srv_result,
    // ...)` call sites with no further plumbing.
    let (mut srv_result, mut srv_child) = match srv_spawn_result {
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
    // `mut`: SPEC_SRV_SUPERVISION_RECYCLE — an srv respawn must park the NEW
    // child's stdin here too, or the very next `srv_child.wait()` poll drops
    // it and the fresh srv shuts down within milliseconds of starting
    // (live-reproduced 2026-07-11: respawned srv logged "stdin closed,
    // shutting down" 7ms after binding its listeners).
    let mut srv_stdin_keepalive = srv_child.stdin.take();
    // SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03 — start the liveness
    // prober's counters clean for this srv instance.
    crate::srv_liveness::reset();

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
            // Issue #2940: the gap this is meant to explain — suspended
            // host process exists, zero instructions executed, up to ~30s
            // before it registers over the IPC pipe — happens entirely
            // AFTER this stage already ended (spawn_host_supervised
            // returning is fast; the actual wait is Windows Defender's
            // on-execution cloud reputation check on the newly-extracted
            // exes, confirmed via the Defender operational event log,
            // 2026-09-04). Nothing else reports progress during that gap
            // today. Deliberately does NOT claim "Windows Defender is
            // scanning" — that can't actually be known from here (no
            // event-log/process-state access in this path), and a wrong
            // guess is worse than a generic one; see the tracking issue
            // comment for why detection was ruled out. A launcher-side
            // timer, not host cooperation: the host executes zero code
            // during this gap, so it cannot report progress itself.
            //
            // 5s threshold: comfortably above a normal launch's ~1s host-
            // registration time (confirmed via a same-machine relaunch
            // immediately after a slow first launch — the SAME gap was
            // 23s on the fresh-machine run vs. 1s on the immediate
            // relaunch, same binaries) so this never appears on the fast
            // path, while still showing well before this gap's own
            // observed 18-30s+ worst case ends.
            //
            // Gated on `host_pipe.has_registered_host()`, NOT on splash
            // dismiss timing — Codex P2, PR #2967: host IPC registration
            // (this timer's actual target) happens well before the splash
            // dismisses (that waits for CEF's `on_load_end`, i.e. full
            // init + first paint). An earlier version of this comment
            // assumed those two events were close enough together that a
            // plain fire-and-forget timer would only ever land in the
            // pre-registration gap or a torn-down channel; that's false
            // whenever registration succeeds quickly but CEF init/first
            // paint alone runs past 5s (slow disk, cold caches, etc.) — a
            // real, if unrelated, delay this message must not be shown
            // for, since it isn't the first-run-scan gap it exists to
            // explain. Checking registration state directly, right before
            // sending, fixes that regardless of how long the splash stays
            // up afterward.
            if splash_event_name.is_some() {
                let delayed_sink = startup_sink.clone();
                let delayed_host_pipe = std::sync::Arc::clone(&host_pipe);
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if !delayed_host_pipe.has_registered_host().await {
                        delayed_sink.sub_begin(
                            "host",
                            "first-run-wait",
                            "First run can take longer",
                        );
                    }
                });
            }
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
    // exit (code 0) ends the loop. srv IS supervised too (#942 Phase 2,
    // SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11): an unexpected srv exit
    // respawns srv and recycles the host through this same restart path,
    // bounded by its own SRV_RESTART_BUDGET.
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
    // SPEC_PILLAR1_STEP4 Phase 4 (reagent P2, PR #2032) — monotonic, only
    // ever incremented, never reset: guarantees `respawn_splash_for_restart`
    // never reuses a Win32 event/class name a still-alive prior splash
    // thread might still be holding open. See that function's doc comment.
    let mut restart_splash_seq: u32 = 0;
    let mut last_abnormal_code: Option<i32> = None;
    let mut host_degraded = false;
    // SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11 (#942 Phase 2) — srv crash
    // budget + the recycle-kill flag. When srv dies unexpectedly, the srv
    // arm respawns it, rebinds `srv_result`, sets this flag, and kills the
    // host deliberately; the host arm (next iteration) consumes the flag to
    // SKIP the deterministic-crash classification (the host didn't fault —
    // a recycle must not step the retry ladder down to --disable-gpu),
    // while still counting against the host budget as runaway protection.
    let mut srv_restarts: Vec<std::time::Instant> = Vec::new();
    // Separate budget for system-OOM srv exits, mirroring `oom_restarts`
    // above — see the srv arm's classification comment
    // (docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md).
    let mut srv_oom_restarts: Vec<std::time::Instant> = Vec::new();
    let mut srv_recycle_kill = false;
    // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 Phase 1 — observe-only
    // UI-thread liveness prober. Low rate (60s); the first tick is delayed
    // a full interval so a booting host (whose UI thread isn't pumping yet
    // — the known pre-ready `post_task` silent drop) isn't logged as a
    // false miss. Phase 2's armed teardown rule will consume
    // `ui_liveness::last_alive()`; nothing here acts on the result.
    let mut ui_probe_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(60),
    );
    ui_probe_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ui_probe_nonce: u64 = 0;
    // SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 2 — armed J0 teardown check.
    // Low-rate poll of the state machine (armed by the IPC server's
    // post-reducer event hook on PoolDrained / OrphanInstance, disarmed on
    // WindowOpened and on every host exit below). The teardown decision
    // itself lives in `teardown_backstop::should_teardown` (grace elapsed
    // AND ≥2 consecutive unanswered UI-thread probes); this tick only
    // evaluates and executes. 5s granularity is plenty against a 30s grace.
    let mut teardown_check_interval =
        tokio::time::interval(std::time::Duration::from_secs(5));
    teardown_check_interval
        .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Distinct exit code so the wedged-host teardown is unambiguous in any
    // log/exit-code triage (spec: "launcher exit with a distinct code").
    const TEARDOWN_BACKSTOP_EXIT_CODE: i32 = 86;
    // SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03 (#942 family) — srv
    // liveness prober. Unlike the host's UI-thread probe, srv exposes a
    // synchronous HTTP health endpoint, so each tick gets a pass/fail
    // answer within that same tick — no cross-tick nonce matching needed.
    // First tick delayed one interval for the same reason as
    // `ui_probe_interval`: give a just-(re)spawned srv a moment before its
    // first probe.
    const SRV_PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
    const SRV_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
    let mut srv_probe_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + SRV_PROBE_INTERVAL,
        SRV_PROBE_INTERVAL,
    );
    srv_probe_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let exit_code = loop {
        tokio::select! {
            _ = teardown_check_interval.tick() => {
                let misses = crate::ui_liveness::consecutive_misses();
                if crate::teardown_backstop::should_teardown(misses) {
                    log(&format!(
                        "[teardown-backstop] host wedged with zero user windows — terminating job \
                         (armed > {}s grace, {} consecutive unanswered UI-thread probes)",
                        crate::teardown_backstop::TEARDOWN_GRACE.as_secs(),
                        misses,
                    ));
                    // I2/I3 hold by construction: J0 is this launcher's own
                    // unnamed job; the blast radius is exactly the processes
                    // it spawned (host + srv + their descendants — the
                    // launcher itself is never assigned to J0). The one
                    // deliberate exception to "never kill what a saga can
                    // reconcile": zero user windows remain and the UI thread
                    // is provably dead, so there is nothing to reconcile.
                    unsafe {
                        windows_sys::Win32::System::JobObjects::TerminateJobObject(
                            job_handle,
                            TEARDOWN_BACKSTOP_EXIT_CODE as u32,
                        );
                    }
                    break TEARDOWN_BACKSTOP_EXIT_CODE;
                }
            }
            _ = ui_probe_interval.tick() => {
                ui_probe_nonce += 1;
                if let Some((missed, sent_at)) = crate::ui_liveness::record_probe_sent(ui_probe_nonce) {
                    log(&format!(
                        "[ui-liveness] probe nonce={} unanswered after {}s — UI thread did not pump in that window",
                        missed,
                        sent_at.elapsed().as_secs()
                    ));
                }
                // Fail-fast send (reagent P1, round 2): the default
                // `send_command` BUFFERS while disconnected and returns Ok,
                // so a probe sent during a crash-restart gap would sit in
                // the pending buffer (or expire there — probes carry no
                // saga_id, so the drop paths can't report the loss) while
                // its outstanding-probe entry aged into a false "did not
                // pump" miss. `try_send_command_no_buffer` turns the
                // disconnected case into an immediate error instead; the
                // retract below then keeps the telemetry clean. A down
                // pipe has its own supervision — a failed send is
                // transport evidence, never liveness evidence.
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
            _ = srv_probe_interval.tick() => {
                if crate::srv_liveness::probe(&srv_result.web_endpoint, SRV_PROBE_TIMEOUT).await {
                    crate::srv_liveness::record_success();
                } else {
                    let misses = crate::srv_liveness::record_failure();
                    log(&format!(
                        "[srv-liveness] missed health probe ({} consecutive)",
                        misses
                    ));
                    if crate::srv_liveness::should_recycle() {
                        log(&format!(
                            "[srv-liveness] srv wedged (alive, unresponsive to {} consecutive health \
                             probes) — forcing recycle",
                            misses
                        ));
                        crate::srv_liveness::reset();
                        // Kill srv directly (not via J0/TerminateJobObject —
                        // this is scoped to srv alone, not a whole-tree
                        // teardown). The srv_status = srv_child.wait() arm
                        // (next iteration) picks up the exit and runs the
                        // already-shipped #2107 respawn/rebind/host-recycle
                        // path unmodified — this arm's only job is deciding
                        // "treat this as a crash".
                        let _ = srv_child.start_kill();
                    }
                }
            }
            host_status = host_child.wait() => {
                // SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 2 — the host exiting
                // (cleanly, crashing, or recycled) always stands the armed
                // teardown down: an Armed machine's whole premise is "the
                // host is alive but wedged", and this is the crash-restart-gap
                // false-positive guard — the machine stays suspended across
                // the restart and can only re-arm from a fresh drain report
                // by the NEW host.
                if crate::teardown_backstop::disarm() {
                    log("[teardown-backstop] disarmed — host exited (supervised-exit path owns it now)");
                }
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
                //
                // SPEC_SRV_SUPERVISION_RECYCLE (reagent P1, PR #2107) — a
                // launcher-inflicted recycle kill is consumed BEFORE
                // classification and forced onto the Abnormal path: we KNOW
                // why the host died, so classifying its exit is meaningless
                // and actively harmful — a recycle landing in a low-commit
                // moment would otherwise route to the SystemOom branch,
                // which always relaunches degraded (--disable-gpu) AND never
                // resets the flag, leaving the NEXT genuine crash silently
                // mistreated as a recycle.
                let recycle_kill = std::mem::replace(&mut srv_recycle_kill, false);
                let commit_free = mem_supervisor::commit_free_mb();
                let exit_class = if recycle_kill {
                    mem_supervisor::HostExitClass::Abnormal
                } else {
                    mem_supervisor::classify_host_exit(code, commit_free)
                };
                match exit_class {
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
                            r = mem_supervisor::await_commit_recovery("host", log) => r,
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
                        // Tear down the PREVIOUS restart's splash before spawning a
                        // new one (reagent P1, PR #2032, 2026-07-08): the unique
                        // per-restart event name (above) stops two splashes from
                        // sharing one Win32 event, but does nothing to stop the old
                        // one from being orphaned — if the host that was supposed to
                        // dismiss it crashed again first, its thread would otherwise
                        // block in `WaitForSingleObject` for the rest of the
                        // launcher's life and its window could still be on screen.
                        if let Some(prev_event) = splash_event_name.as_deref() {
                            crate::splash::dismiss_splash(prev_event);
                        }
                        restart_splash_seq += 1;
                        splash_event_name = respawn_splash_for_restart(&dir_hash, restart_splash_seq);
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
                        //
                        // SPEC_SRV_SUPERVISION_RECYCLE — a launcher-inflicted
                        // recycle kill is NOT a host fault: skip the ladder
                        // bookkeeping so an srv-driven recycle can never mark
                        // the host deterministic-crashing or degrade it to
                        // --disable-gpu. (It still counted against the budget
                        // above — runaway protection stays. The flag itself
                        // was consumed before classification — reagent P1.)
                        if !recycle_kill {
                            if last_abnormal_code == Some(code) {
                                host_degraded = true;
                            }
                            last_abnormal_code = Some(code);
                        }
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
                        // Tear down the PREVIOUS restart's splash before spawning a
                        // new one (reagent P1, PR #2032, 2026-07-08): the unique
                        // per-restart event name (above) stops two splashes from
                        // sharing one Win32 event, but does nothing to stop the old
                        // one from being orphaned — if the host that was supposed to
                        // dismiss it crashed again first, its thread would otherwise
                        // block in `WaitForSingleObject` for the rest of the
                        // launcher's life and its window could still be on screen.
                        if let Some(prev_event) = splash_event_name.as_deref() {
                            crate::splash::dismiss_splash(prev_event);
                        }
                        restart_splash_seq += 1;
                        splash_event_name = respawn_splash_for_restart(&dir_hash, restart_splash_seq);
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
                // SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11 (#942 Phase 2) —
                // recycle, don't rewire: srv's endpoints only reach the host
                // via spawn-time env, and the host is DISPOSABLE (Pillar 1
                // Step 4), so the supervision story is: respawn srv, rebind
                // the endpoints, deliberately kill the host, and let the
                // existing host-restart machinery (splash + crash-reproject)
                // rebuild the session against the new srv. srv's SQLite
                // state is durable (WAL) — reproject sees everything.
                let code = match srv_status {
                    Ok(s) => s.code().unwrap_or(1),
                    Err(e) => {
                        log(&format!("FATAL: srv wait failed: {}", e));
                        break 1;
                    }
                };
                // Classify like the host arm above (SPEC_MEMORY_PRESSURE_
                // SUPERVISION_2026_06_16 §5.B) instead of unconditionally
                // burning the fast SRV_RESTART_BUDGET. srv is a plain Rust
                // process, so it never emits Chromium's exact OOM code
                // (`CHROMIUM_OOM_EXIT_CODE`) — but `classify_host_exit`'s
                // low-commit-at-exit-time fallback still catches a genuine
                // system-OOM srv abort (Rust's Windows fail-fast abort on
                // allocation failure surfaces as the generic 0xC0000409, not
                // 0xE0000008). Before this, EVERY srv exit — OOM or not —
                // consumed the fixed, fast budget; a live incident hit that
                // budget 3x in under 10 seconds (each respawned srv re-OOMing
                // near-instantly into the still-starved system) and killed
                // the whole launcher instead of waiting out a few seconds of
                // transient commit pressure. See
                // docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md.
                let commit_free = mem_supervisor::commit_free_mb();
                if matches!(
                    mem_supervisor::classify_host_exit(code, commit_free),
                    mem_supervisor::HostExitClass::SystemOom
                ) {
                    let now = std::time::Instant::now();
                    if mem_supervisor::budget_exhausted(
                        &mut srv_oom_restarts,
                        now,
                        mem_supervisor::OOM_RESTART_WINDOW,
                        mem_supervisor::OOM_RESTART_BUDGET,
                    ) {
                        log(&format!(
                            "srv hit system OOM (code {}, {} MB commit-free); srv OOM restart \
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
                        "srv hit system OOM (code {}, {} MB commit-free) — waiting for memory \
                         to recover before respawning srv",
                        code, commit_free
                    ));
                    // Race the wait against the host dying too, same pattern
                    // as the host arm's nested select above — without this,
                    // a host crash during the (up to OOM_RELAUNCH_DEADLINE)
                    // wait would go unnoticed until the wait finished.
                    let recovered = tokio::select! {
                        r = mem_supervisor::await_commit_recovery("srv", log) => r,
                        host_status = host_child.wait() => {
                            match host_status {
                                Ok(s) => log(&format!(
                                    "CEF host exited UNEXPECTEDLY during srv-OOM wait with code {} — \
                                     terminating launcher",
                                    s.code().unwrap_or(1)
                                )),
                                Err(e) => log(&format!(
                                    "FATAL: host wait failed during srv-OOM wait: {}", e
                                )),
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
                    log(&format!(
                        "commit recovered — respawning srv (OOM restart {}/{})",
                        srv_oom_restarts.len(),
                        mem_supervisor::OOM_RESTART_BUDGET
                    ));
                } else {
                    if mem_supervisor::budget_exhausted(
                        &mut srv_restarts,
                        std::time::Instant::now(),
                        SRV_RESTART_WINDOW,
                        SRV_RESTART_BUDGET,
                    ) {
                        log(&format!(
                            "srv exited (code {}); srv restart budget exhausted ({} in {}s) — terminating launcher",
                            code,
                            SRV_RESTART_BUDGET,
                            SRV_RESTART_WINDOW.as_secs()
                        ));
                        break 1;
                    }
                    log(&format!(
                        "srv exited UNEXPECTEDLY (code {}) — respawning srv + recycling host (restart {}/{})",
                        code,
                        srv_restarts.len(),
                        SRV_RESTART_BUDGET
                    ));
                }
                match srv_spawner::spawn_srv(
                    launcher_exe_dir,
                    &paths,
                    &srv_pipe_path,
                    job_handle,
                    &startup_sink,
                )
                .await
                {
                    Ok((new_result, new_child)) => {
                        srv_result = new_result;
                        srv_child = new_child;
                        // Park the NEW srv's stdin exactly like cold boot does
                        // (see `srv_stdin_keepalive`'s comment above): the next
                        // `srv_child.wait()` poll would otherwise drop it, and
                        // srv reads stdin-EOF as "parent died" and exits within
                        // milliseconds — live-reproduced on the first version
                        // of this arm.
                        srv_stdin_keepalive = srv_child.stdin.take();
                        // A freshly respawned srv must not inherit its
                        // predecessor's miss count (whether this respawn was
                        // triggered by a real crash or by the liveness
                        // prober's own recycle-kill above).
                        crate::srv_liveness::reset();
                        log(&format!(
                            "srv respawned (pid {}) — new endpoints ws={} web={}; recycling host",
                            srv_result.pid, srv_result.ws_endpoint, srv_result.web_endpoint
                        ));
                        // The running host is wired to the DEAD srv's
                        // endpoints — every backend connection it holds is
                        // broken. Kill it deliberately; the host arm's
                        // supervised restart (next select iteration) spawns
                        // the replacement against the rebound `srv_result`.
                        srv_recycle_kill = true;
                        let _ = host_child.start_kill();
                    }
                    Err(e) => {
                        log(&format!(
                            "FATAL: srv respawn failed: {} — terminating launcher",
                            e
                        ));
                        break 1;
                    }
                }
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
