// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Spawn the agentmux-srv backend sidecar from the LAUNCHER (Phase B.1).
//
// Today's flow (pre-Phase-B): launcher spawns host; host spawns srv;
// host owns a Job Object J1 wrapping srv; renderers inherit launcher's
// J0 via host. Result: when host crashes, J1 closes and srv dies.
//
// Phase B.1 flow: launcher spawns BOTH srv and host as siblings,
// assigns BOTH to launcher's J0 directly. Host's J1 on srv is
// deleted (it would actively defeat "srv survives host crash" if
// kept). Renderers continue to inherit J0 via host as before.
//
// The launcher passes srv's endpoints to the host via env vars
// (AGENTMUX_BACKEND_WS, _WEB, _PID). Host detects them and skips
// its own spawn_backend path (which is preserved for `task dev`
// fallback where launcher isn't in the loop).
//
// Adapted from `agentmux-cef/src/sidecar.rs::spawn_backend` —
// kept structurally similar so divergence is auditable. Key
// differences:
//   * Tokio process API (not std::process)
//   * CREATE_SUSPENDED + assign-to-job + ResumeThread (PR #570 race
//     pattern, applied to srv too)
//   * No separate Job Object on srv; launcher's J0 covers it
//   * Auth key generated here, not consumed from a shared AppState
//   * stderr ESTART parsing returns the result via tokio mpsc

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::data_dir::DataPaths;

/// What the launcher learns about srv after it signals ready.
/// Held by the launcher and used to populate env vars the host reads.
#[derive(Debug, Clone)]
pub struct SrvSpawnResult {
    pub pid: u32,
    pub ws_endpoint: String,
    pub web_endpoint: String,
    pub instance_id: String,
    pub auth_key: String,
    /// Shared secret proving to srv that a `host_ipc.Register` call really
    /// comes from the paired host, not an agent process (agents share
    /// `auth_key` too — see `agentmux-srv/src/server/service/host_ipc.rs`).
    /// Minted once per `spawn_srv` call (same lifetime as `auth_key`) and
    /// given to both host and srv's spawn env — see `host_spawn.rs`'s
    /// `AGENTMUX_HOST_REG_SECRET` env line and this file's own
    /// `AGENTMUX_HOST_REG_SECRET` env line below. Survives every host-only
    /// crash-restart automatically, exactly like `auth_key` does (this
    /// struct is held in the launcher's own stack frame across restarts —
    /// see `SrvSpawnResult`'s own doc comment above).
    pub host_reg_secret: String,
    /// Number of data migrations still pending after the in-process startup run.
    /// Non-zero means run_pending_migrations failed; status-bar shows a retry message.
    pub pending_migrations: usize,
    /// RFC3339 timestamp captured when ESTART arrived. Carried on the
    /// result for `--diag` / debug observability; not currently
    /// propagated into env. F.7 cleanup audit: keep with allow + this
    /// note rather than delete — a future `--diag srv` printer is the
    /// natural reader.
    #[allow(dead_code)]
    pub started_at: String,
}

/// Errors during srv spawn — granular enough that the launcher can
/// log the right diagnostic.
#[derive(Debug)]
pub enum SrvSpawnError {
    BinaryNotFound(String),
    SpawnFailed(String),
    JobAssignFailed(String),
    ResumeFailed(String),
    EstartTimeout,
    EstartChannelClosed,
}

impl std::fmt::Display for SrvSpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryNotFound(s) => write!(f, "srv binary not found: {}", s),
            Self::SpawnFailed(s) => write!(f, "spawn failed: {}", s),
            Self::JobAssignFailed(s) => write!(f, "AssignProcessToJobObject failed: {}", s),
            Self::ResumeFailed(s) => write!(f, "ResumeThread failed: {}", s),
            Self::EstartTimeout => write!(f, "timeout waiting for AGENTMUXSRV-ESTART (30s)"),
            Self::EstartChannelClosed => {
                write!(f, "ESTART channel closed before srv signalled ready")
            }
        }
    }
}

/// Run `agentmux-srv migrate` synchronously before spawning the daemon.
///
/// The migration runner exits 0 (success / nothing to do) or 1 (failure).
/// On failure the launcher should surface an error and not start the daemon.
/// stdout lines are newline-delimited JSON progress events — consumed inline
/// so sub-events are delivered to the splash before `stage_end` is sent.
///
/// Not called during normal startup — migrations now run in-process inside srv
/// via `run_pending_migrations` before ESTART is emitted. Preserved here as a
/// fallback subprocess path (e.g. for a future recovery flow) but has no active
/// callers.
#[allow(dead_code)]
pub async fn run_migrate(
    launcher_exe_dir: &Path,
    paths: &DataPaths,
    sink: &crate::startup_events::StartupEventSink,
) -> Result<(), SrvSpawnError> {
    sink.stage_begin("migrations", "Migrations");
    let t = std::time::Instant::now();

    let backend_path = resolve_srv_binary(launcher_exe_dir)?;

    let mut cmd = tokio::process::Command::new(&backend_path);
    cmd.args([
        "--wavedata",
        &paths.data_dir.to_string_lossy(),
        "migrate",
    ])
    .envs(paths.common.to_env_vars())
    // Auth key is not needed for migrate but the binary may check for it
    // before dispatching. Provide a placeholder so argument parsing passes.
    .env("AGENTMUX_AUTH_KEY", "migrate-placeholder")
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true);

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let mut child = cmd
        .spawn()
        .map_err(|e| SrvSpawnError::SpawnFailed(format!("migrate spawn: {}", e)))?;

    // Stderr to log only (non-blocking task).
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                crate::log(&format!("[migrate stderr] {}", line));
            }
        });
    }

    // Read stdout inline so sub-events arrive before stage_end is sent.
    // We break as soon as we see the "complete" event — the process may hang
    // in Tokio shutdown (crash-monitor task) after flushing its last line,
    // so waiting for stdout EOF would block forever.
    let mut applied = 0u32;
    let mut skipped = 0u32;
    let mut migration_complete = false;
    if let Some(stdout) = child.stdout.take() {
        let mut reader = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            crate::log(&format!("[migrate] {}", line));
            match parse_migration_line(&line) {
                Some(MigrationLine::Start { id, label }) => {
                    sink.sub_begin("migrations", id, label);
                }
                Some(MigrationLine::Done { id, duration_ms }) => {
                    sink.sub_end(
                        "migrations",
                        id,
                        duration_ms,
                        crate::startup_events::StartupStatus::Ok,
                        None,
                    );
                }
                Some(MigrationLine::Complete { applied: a, skipped: s }) => {
                    applied = a;
                    skipped = s;
                    migration_complete = true;
                    break; // Don't wait for EOF — kill below.
                }
                None => {}
            }
        }
    }

    // If the process signalled complete via stdout we know it succeeded.
    // Kill it now so we don't hang on wait() if its Tokio runtime shutdown
    // stalls (crash-monitor interaction, background tasks with long timeouts).
    if migration_complete {
        let _ = child.start_kill();
    }

    let status = child
        .wait()
        .await
        .map_err(|e| SrvSpawnError::SpawnFailed(format!("migrate wait: {}", e)))?;

    let duration_ms = t.elapsed().as_millis() as u64;
    // migration_complete means the process emitted {"event":"complete"} on
    // stdout before we killed it — that is the authoritative success signal.
    // status.success() may be false when we force-killed the process after
    // seeing complete (crash-monitor Tokio shutdown hung).
    if migration_complete || status.success() {
        let detail = if applied > 0 {
            Some(format!("{} applied, {} current", applied, skipped))
        } else if skipped > 0 {
            Some(format!("all {} current", skipped))
        } else {
            None
        };
        sink.stage_end("migrations", duration_ms, crate::startup_events::StartupStatus::Ok, detail);
        Ok(())
    } else {
        sink.stage_end(
            "migrations",
            duration_ms,
            crate::startup_events::StartupStatus::Error,
            Some("failed; see migration-error.log".into()),
        );
        Err(SrvSpawnError::SpawnFailed(format!(
            "agentmux-srv migrate exited with status {}; see ~/.agentmux/logs/migration-error.log",
            status.code().unwrap_or(-1)
        )))
    }
}

// ── Migration JSON parser ────────────────────────────────────────────────────

enum MigrationLine {
    Start { id: String, label: String },
    Done { id: String, duration_ms: u64 },
    Complete { applied: u32, skipped: u32 },
}

fn parse_migration_line(s: &str) -> Option<MigrationLine> {
    let event = json_str(s, "event")?;
    match event.as_str() {
        "migration_start" => Some(MigrationLine::Start {
            id: json_str(s, "id")?,
            label: json_str(s, "description")
                .unwrap_or_else(|| json_str(s, "id").unwrap_or_default()),
        }),
        "migration_done" => Some(MigrationLine::Done {
            id: json_str(s, "id")?,
            duration_ms: json_u64(s, "duration_ms").unwrap_or(0),
        }),
        "complete" => Some(MigrationLine::Complete {
            applied: json_u64(s, "applied").unwrap_or(0) as u32,
            skipped: json_u64(s, "skipped").unwrap_or(0) as u32,
        }),
        _ => None,
    }
}

fn json_str(s: &str, key: &str) -> Option<String> {
    let prefix = format!("\"{}\":\"", key);
    let start = s.find(&prefix)? + prefix.len();
    let rest = &s[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn json_u64(s: &str, key: &str) -> Option<u64> {
    let prefix = format!("\"{}\":", key);
    let start = s.find(&prefix)? + prefix.len();
    let rest = s[start..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    if end == 0 { return None; }
    rest[..end].parse().ok()
}

/// Spawn srv as a child of the launcher, assigned to launcher's
/// Job Object J0 so it dies cleanly with the launcher tree.
///
/// `launcher_exe_dir` is used to locate the srv binary alongside the
/// launcher (or in `runtime/` for portable). `paths` carries the
/// data + config dirs and is propagated to srv via env vars.
/// `job_handle` is the launcher's Job Object so srv joins the same
/// kill-on-job-close contract as the host. Returns once srv prints
/// `AGENTMUXSRV-ESTART` (or the 30s timeout fires).
///
/// Caller keeps the returned `Child` alive — drop closes srv's
/// stdin and srv's existing PPID death-watcher takes over (already
/// part of agentmux-srv per `SPEC_BACKEND_LIFECYCLE.md`).
// Phase E.1b — `srv_pipe_path` is the launcher-computed pipe path
// (same data-dir hash as launcher's own pipe, different leaf name).
// Passed via `AGENTMUX_SRV_PIPE_PATH` so srv doesn't have to
// recompute the hash; launcher is the single source of truth.
pub async fn spawn_srv(
    launcher_exe_dir: &Path,
    paths: &DataPaths,
    srv_pipe_path: &str,
    #[cfg(target_os = "windows")] job_handle: windows_sys::Win32::Foundation::HANDLE,
    sink: &crate::startup_events::StartupEventSink,
) -> Result<(SrvSpawnResult, Child), SrvSpawnError> {
    sink.stage_begin("backend", "Backend startup");
    let srv_t = std::time::Instant::now();
    let backend_path = resolve_srv_binary(launcher_exe_dir)?;

    // Generate a fresh auth_key per run (UUID v4 — same as host did).
    // This is the launcher's responsibility now; host receives it via
    // AGENTMUX_AUTH_KEY env so srv + host + frontend agree on the key.
    let auth_key = uuid::Uuid::new_v4().to_string();
    // Minted alongside auth_key, same lifetime/reuse rules — see
    // `SrvSpawnResult::host_reg_secret`'s doc comment.
    let host_reg_secret = uuid::Uuid::new_v4().to_string();
    let version = env!("CARGO_PKG_VERSION");
    let instance_id = format!("v{}", version);

    // app_path = launcher's exe dir (used by srv for finding bundled
    // tooling like jq.exe / rg.exe). In portable mode this is the
    // top of the portable folder; the runtime/tools/bin/ subdir
    // lives under runtime/, but srv's app_path lookup is currently
    // exe_dir-based per the host's code.
    //
    // For B.1 we keep parity: pass exe_dir of the LAUNCHER. If srv
    // tooling lookup breaks, follow up — log it loudly.
    let app_path_str = launcher_exe_dir.to_string_lossy().to_string();

    let mut cmd = Command::new(&backend_path);
    cmd.args([
        "--wavedata",
        &paths.data_dir.to_string_lossy(),
        "--instance",
        &instance_id,
    ])
    .env("AGENTMUX_AUTH_KEY", &auth_key)
    .env("AGENTMUX_HOST_REG_SECRET", &host_reg_secret)
    // Canonical AGENTMUX_* env vars (INSTANCE_DIR / DATA_DIR /
    // CONFIG_DIR / LOG_DIR / CEF_CACHE_DIR / AGENTS_DIR / INSTANCE_
    // RUNTIME_DIR / SHARED_DIR / RUNTIME_MODE). Replaces the old
    // AGENTMUX_DATA_HOME / AGENTMUX_DEV / AGENTMUX_CONFIG_HOME /
    // AGENTMUX_SETTINGS_DIR pre-unification names. srv reads them
    // via `DataPaths::from_env()` (or the raw var names directly).
    .envs(paths.common.to_env_vars())
    .env("AGENTMUX_APP_PATH", &app_path_str)
    .env("AGENTMUX_SRV_PIPE_PATH", srv_pipe_path)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(false); // Job Object handles cleanup; tokio's kill-on-drop would force-kill.

    // Windows: spawn suspended so we can assign-to-job before any
    // child code runs (PR #570 race pattern, now applied to srv).
    // Without this, srv could open files / sockets before joining
    // the job — those resources would survive launcher death.
    #[cfg(target_os = "windows")]
    {
        use agentmux_common::win32::CREATE_SUSPENDED;
        use agentmux_common::win32::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    }

    // Linux process-tree reap (A0): PR_SET_PDEATHSIG → SIGKILL on
    // launcher death so srv is reaped even if the launcher exits
    // abnormally (panic/OOM/external SIGKILL). Linux analogue of the
    // Windows Job Object cleanup the launcher already does. Same
    // safety contract as the host pre_exec — async-signal-safe call
    // only. macOS lacks prctl; the macOS launcher reap path is
    // handled by tokio's child supervision in `run_unix`.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
                Ok(())
            });
        }
    }

    // SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 unix parity — srv becomes
    // its own process-group leader too, same rationale as
    // `spawn_host_unix`'s `.process_group(0)`: the teardown backstop's
    // `kill_process_group_forcefully` needs an explicit, launcher-scoped
    // group for srv independent of host's (they spawn concurrently via
    // `tokio::join!` in `run_unix`, so srv can't join a group host hasn't
    // been assigned yet — each gets its own rather than coupling the two
    // spawn call sites).
    #[cfg(not(target_os = "windows"))]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| SrvSpawnError::SpawnFailed(e.to_string()))?;
    let pid = child
        .id()
        .ok_or_else(|| SrvSpawnError::SpawnFailed("child has no PID".to_string()))?;

    // Windows: assign srv to launcher's job, then resume.
    // Skip job assignment if launcher's job creation failed (J0 is
    // null) — that's the degraded mode logged by main.rs. Srv still
    // runs but won't be reaped on launcher death.
    //
    // Both error paths must explicitly start_kill the suspended
    // child before returning. We set kill_on_drop(false) (J0 normally
    // handles cleanup), so dropping the Child wouldn't terminate the
    // suspended srv — it would orphan as a permanent zombie holding
    // resources and the data dir lockfile, blocking subsequent
    // launches. (codex P1 @ srv_spawner.rs:161, PR #571 round-3.)
    #[cfg(target_os = "windows")]
    {
        if !job_handle.is_null() {
            if let Err(e) = assign_pid_to_job(pid, job_handle) {
                let _ = child.start_kill();
                return Err(SrvSpawnError::JobAssignFailed(e));
            }
        }
        if let Err(e) = crate::host_spawn::resume_main_thread(pid) {
            let _ = child.start_kill();
            return Err(SrvSpawnError::ResumeFailed(e));
        }
    }

    let started_at = chrono::Utc::now().to_rfc3339();

    // Forward srv stdout to our log (info level; the launcher's log
    // file is the same one srv-logs end up in once we wire log
    // forwarding properly).
    if let Some(stdout) = child.stdout.take() {
        let pid_for_log = pid;
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                crate::log(&format!("[srv {} stdout] {}", pid_for_log, line));
            }
        });
    }

    // Parse stderr for AGENTMUXSRV-ESTART (the readiness signal). srv
    // writes other diagnostic lines too (AGENTMUXSRV-EVENT:..., plain
    // text); for B.1 we just log them. Phase B sub-PR B.2 will
    // forward AGENTMUXSRV-EVENT messages to subscribers via the IPC
    // event stream.
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SrvSpawnError::SpawnFailed("no stderr handle".to_string()))?;
    let (tx, mut rx) = mpsc::channel::<SrvSpawnResult>(1);
    // Separate channel so the stderr reader can signal that migrations are running.
    // The ESTART waiter extends its deadline when it receives this ping so that a
    // large-dataset migration that takes longer than the normal 30s boot window
    // doesn't cause the launcher to kill srv prematurely.
    let (migration_tx, mut migration_rx) = mpsc::channel::<()>(4);
    let auth_key_for_estart = auth_key.clone();
    let host_reg_secret_for_estart = host_reg_secret.clone();
    let started_at_for_estart = started_at.clone();
    let pid_for_log = pid;
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut estart_sent = false;
        while let Ok(Some(line)) = reader.next_line().await {
            if !estart_sent && line.starts_with("AGENTMUXSRV-ESTART") {
                let parsed = parse_estart(&line);
                let result = SrvSpawnResult {
                    pid: pid_for_log,
                    ws_endpoint: parsed.ws_endpoint,
                    web_endpoint: parsed.web_endpoint,
                    instance_id: parsed.instance_id,
                    auth_key: auth_key_for_estart.clone(),
                    host_reg_secret: host_reg_secret_for_estart.clone(),
                    pending_migrations: parsed.pending_migrations,
                    started_at: started_at_for_estart.clone(),
                };
                crate::log(&format!(
                    "srv {} ready: ws={} web={} instance={} pending_migrations={}",
                    result.pid, result.ws_endpoint, result.web_endpoint, result.instance_id,
                    result.pending_migrations
                ));
                let _ = tx.send(result).await;
                estart_sent = true;
            } else if line.starts_with("AGENTMUXSRV-MIGRATING") {
                crate::log(&format!("[srv {} migrating] {}", pid_for_log, line));
                let _ = migration_tx.send(()).await;
            } else if line.starts_with("AGENTMUXSRV-EVENT:") {
                crate::log(&format!("[srv {} event] {}", pid_for_log, line));
                // Phase B.2 will forward these to subscribers.
            } else {
                crate::log(&format!("[srv {} stderr] {}", pid_for_log, line));
            }
        }
        // EOF on stderr → srv exited (or its stderr closed). Logged
        // by the wait-task in main; nothing else to do here.
    });

    // Wait for ESTART. Both error paths must explicitly start_kill
    // the child before returning — same kill_on_drop(false) leak
    // class as the assign/resume failures above. Without this, the
    // 30s timeout in degraded mode (J0 absent) would leak a fully-
    // running srv that keeps the data dir lockfile, blocking the
    // next launch. (codex P2 @ srv_spawner.rs:240, PR #571 round-4.)
    //
    // If srv emits AGENTMUXSRV-MIGRATING before ESTART, the deadline is
    // extended to 30 minutes to accommodate large-dataset migrations. A shorter
    // cap risks killing srv mid-migration: 0011_shared_store_backfill's skip-guards
    // check for non-empty shared tables, so a partial copy followed by a kill would
    // cause rows to be permanently stranded on the next retry.
    let normal_timeout = std::time::Duration::from_secs(30);
    let migration_timeout = std::time::Duration::from_secs(1800);
    let mut deadline = tokio::time::Instant::now() + normal_timeout;
    let mut sleep = tokio::time::sleep_until(deadline);
    tokio::pin!(sleep);

    let recv: Result<Option<SrvSpawnResult>, ()> = loop {
        tokio::select! {
            result = rx.recv() => break Ok(result),
            _ = &mut sleep => break Err(()),
            Some(()) = migration_rx.recv() => {
                // Migrations are running — extend the deadline generously.
                let new_deadline = tokio::time::Instant::now() + migration_timeout;
                if new_deadline > deadline {
                    deadline = new_deadline;
                    sleep.as_mut().reset(deadline);
                    crate::log("srv: migration in progress — ESTART deadline extended to 30 minutes");
                }
            }
        }
    };
    match recv {
        Err(()) => {
            let _ = child.start_kill();
            sink.stage_end(
                "backend",
                srv_t.elapsed().as_millis() as u64,
                crate::startup_events::StartupStatus::Error,
                Some("timeout (30s)".into()),
            );
            Err(SrvSpawnError::EstartTimeout)
        }
        Ok(None) => {
            let _ = child.start_kill();
            sink.stage_end(
                "backend",
                srv_t.elapsed().as_millis() as u64,
                crate::startup_events::StartupStatus::Error,
                None,
            );
            Err(SrvSpawnError::EstartChannelClosed)
        }
        Ok(Some(result)) => {
            sink.stage_end(
                "backend",
                srv_t.elapsed().as_millis() as u64,
                crate::startup_events::StartupStatus::Ok,
                None,
            );
            Ok((result, child))
        }
    }
}

/// Resolve the agentmux-srv binary path from the LAUNCHER's vantage
/// point.
///
/// Search order, mirroring the host's `resolve_backend_binary`
/// (sidecar.rs:318-402) but anchored at the launcher's exe dir:
///   1. `<launcher_dir>/runtime/agentmux-srv-{ver}-{os}.{arch}.exe`
///      (versioned portable layout)
///   2. `<launcher_dir>/runtime/agentmux-srv.exe` (dev fallback)
///   3. `<launcher_dir>/agentmux-srv-{ver}-{os}.{arch}.exe`
///      (launcher in same dir as srv — should not happen in portable
///      but covers cargo-built dev mode where launcher + srv both
///      land in target/release/)
///   4. `<launcher_dir>/agentmux-srv.exe` (dev fallback)
fn resolve_srv_binary(launcher_exe_dir: &Path) -> Result<PathBuf, SrvSpawnError> {
    let backend_name = "agentmux-srv";
    let exe_suffix = if cfg!(target_os = "windows") { ".exe" } else { "" };
    let version = env!("CARGO_PKG_VERSION");
    let (os_name, arch) = if cfg!(target_os = "macos") {
        ("darwin", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    } else if cfg!(target_os = "linux") {
        ("linux", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    } else {
        ("windows", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    };

    let candidates = [
        // Portable: srv lives in launcher_dir/runtime/
        launcher_exe_dir
            .join("runtime")
            .join(format!("{}-{}-{}.{}{}", backend_name, version, os_name, arch, exe_suffix)),
        launcher_exe_dir
            .join("runtime")
            .join(format!("{}{}", backend_name, exe_suffix)),
        // Dev: launcher and srv side-by-side in target/release/
        launcher_exe_dir
            .join(format!("{}-{}-{}.{}{}", backend_name, version, os_name, arch, exe_suffix)),
        launcher_exe_dir.join(format!("{}{}", backend_name, exe_suffix)),
    ];

    for p in &candidates {
        if p.exists() {
            return Ok(p.clone());
        }
    }

    Err(SrvSpawnError::BinaryNotFound(format!(
        "{} v{} not found. Searched:\n  {}",
        backend_name,
        version,
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    )))
}

/// Parsed fields out of a `AGENTMUXSRV-ESTART` line. Same shape as the
/// host's `parse_estart` (sidecar.rs:404-420).
struct EstartFields {
    ws_endpoint: String,
    web_endpoint: String,
    instance_id: String,
    pending_migrations: usize,
}

fn parse_estart(line: &str) -> EstartFields {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let get = |prefix: &str| -> String {
        parts
            .iter()
            .find_map(|p| p.strip_prefix(prefix))
            .unwrap_or_default()
            .to_string()
    };
    let pending_migrations = parts
        .iter()
        .find_map(|p| p.strip_prefix("pending_migrations:"))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    EstartFields {
        ws_endpoint: get("ws:"),
        web_endpoint: get("web:"),
        instance_id: get("instance:"),
        pending_migrations,
    }
}

/// Assign a process to the launcher's Job Object J0. Used by
/// `spawn_srv` for srv and exported for `main.rs` to use for the
/// host. Separated from job creation because both children join
/// the SAME job (only one J0 ever exists per launcher run).
#[cfg(target_os = "windows")]
pub fn assign_pid_to_job(
    pid: u32,
    job: windows_sys::Win32::Foundation::HANDLE,
) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };
    unsafe {
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            return Err(format!("OpenProcess({}) returned null", pid));
        }
        let ok = AssignProcessToJobObject(job, process);
        CloseHandle(process);
        if ok == 0 {
            return Err(format!(
                "AssignProcessToJobObject failed for pid={}",
                pid
            ));
        }
        Ok(())
    }
}
