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
    pub started_at: String, // RFC3339
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
            Self::EstartTimeout => write!(f, "timeout waiting for WAVESRV-ESTART (30s)"),
            Self::EstartChannelClosed => {
                write!(f, "ESTART channel closed before srv signalled ready")
            }
        }
    }
}

/// Spawn srv as a child of the launcher, assigned to launcher's
/// Job Object J0 so it dies cleanly with the launcher tree.
///
/// `launcher_exe_dir` is used to locate the srv binary alongside the
/// launcher (or in `runtime/` for portable). `paths` carries the
/// data + config dirs and is propagated to srv via env vars.
/// `job_handle` is the launcher's Job Object so srv joins the same
/// kill-on-job-close contract as the host. Returns once srv prints
/// `WAVESRV-ESTART` (or the 30s timeout fires).
///
/// Caller keeps the returned `Child` alive — drop closes srv's
/// stdin and srv's existing PPID death-watcher takes over (already
/// part of agentmux-srv per `SPEC_BACKEND_LIFECYCLE.md`).
pub async fn spawn_srv(
    launcher_exe_dir: &Path,
    paths: &DataPaths,
    #[cfg(target_os = "windows")] job_handle: windows_sys::Win32::Foundation::HANDLE,
) -> Result<(SrvSpawnResult, Child), SrvSpawnError> {
    let backend_path = resolve_srv_binary(launcher_exe_dir)?;

    // Generate a fresh auth_key per run (UUID v4 — same as host did).
    // This is the launcher's responsibility now; host receives it via
    // AGENTMUX_AUTH_KEY env so srv + host + frontend agree on the key.
    let auth_key = uuid::Uuid::new_v4().to_string();
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
    .env("AGENTMUX_CONFIG_HOME", paths.config_dir.to_string_lossy().to_string())
    .env("AGENTMUX_DATA_HOME", paths.data_dir.to_string_lossy().to_string())
    .env("AGENTMUX_SETTINGS_DIR", paths.config_dir.to_string_lossy().to_string())
    .env("AGENTMUX_APP_PATH", &app_path_str)
    .env("AGENTMUX_DEV", if cfg!(debug_assertions) { "1" } else { "" })
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
        const CREATE_SUSPENDED: u32 = 0x00000004;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    }

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
        if let Err(e) = crate::resume_main_thread(pid) {
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

    // Parse stderr for WAVESRV-ESTART (the readiness signal). srv
    // writes other diagnostic lines too (WAVESRV-EVENT:..., plain
    // text); for B.1 we just log them. Phase B sub-PR B.2 will
    // forward WAVESRV-EVENT messages to subscribers via the IPC
    // event stream.
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SrvSpawnError::SpawnFailed("no stderr handle".to_string()))?;
    let (tx, mut rx) = mpsc::channel::<SrvSpawnResult>(1);
    let auth_key_for_estart = auth_key.clone();
    let started_at_for_estart = started_at.clone();
    let pid_for_log = pid;
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut estart_sent = false;
        while let Ok(Some(line)) = reader.next_line().await {
            if !estart_sent && line.starts_with("WAVESRV-ESTART") {
                let parsed = parse_estart(&line);
                let result = SrvSpawnResult {
                    pid: pid_for_log,
                    ws_endpoint: parsed.ws_endpoint,
                    web_endpoint: parsed.web_endpoint,
                    instance_id: parsed.instance_id,
                    auth_key: auth_key_for_estart.clone(),
                    started_at: started_at_for_estart.clone(),
                };
                crate::log(&format!(
                    "srv {} ready: ws={} web={} instance={}",
                    result.pid, result.ws_endpoint, result.web_endpoint, result.instance_id
                ));
                let _ = tx.send(result).await;
                estart_sent = true;
            } else if line.starts_with("WAVESRV-EVENT:") {
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
    let recv = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv()).await;
    match recv {
        Err(_) => {
            let _ = child.start_kill();
            Err(SrvSpawnError::EstartTimeout)
        }
        Ok(None) => {
            let _ = child.start_kill();
            Err(SrvSpawnError::EstartChannelClosed)
        }
        Ok(Some(result)) => Ok((result, child)),
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

/// Parsed fields out of a `WAVESRV-ESTART` line. Same shape as the
/// host's `parse_estart` (sidecar.rs:404-420).
struct EstartFields {
    ws_endpoint: String,
    web_endpoint: String,
    instance_id: String,
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
    EstartFields {
        ws_endpoint: get("ws:"),
        web_endpoint: get("web:"),
        instance_id: get("instance:"),
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
