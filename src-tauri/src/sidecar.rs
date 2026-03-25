mod binary;
mod event_loop;

#[cfg(unix)]
use libc;
use std::path::PathBuf;
use tauri::Emitter;
use tauri::Manager;
/// The full target triple this binary was compiled for (e.g. "x86_64-pc-windows-msvc").
/// Emitted by build.rs so we can locate the bundled sidecar binary by name at runtime.
const AGENTMUX_TARGET_TRIPLE: &str = env!("AGENTMUX_TARGET_TRIPLE");

/// Copy a bundled sidecar to the version-isolated app local data dir.
///
/// Tauri's externalBin places sidecars at `src-tauri/binaries/{name}-{triple}`,
/// which is a fixed path shared across all versions. On Windows, a running
/// sidecar holds a read lock that blocks tauri-build during builds of new versions.
///
/// Fix: copy to `{app_local_data_dir}/sidecar/{name}(.exe)` on first launch and
/// run from there. The app local data dir is already version-isolated via the
/// Tauri identifier (e.g. `ai.agentmux.app.v0-32-63`).
///
/// The copy is skipped if the destination already exists with the same size+mtime.
pub(crate) fn ensure_versioned_sidecar(app: &tauri::AppHandle, sidecar_name: &str) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("data dir error: {e}"))?;
    let sidecar_dir = data_dir.join("sidecar");
    std::fs::create_dir_all(&sidecar_dir)
        .map_err(|e| format!("create sidecar dir: {e}"))?;

    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    let dest = sidecar_dir.join(format!("{}{}", sidecar_name, exe_suffix));

    // Source binary: Tauri places externalBin entries in the resource dir as
    // `{name}-{target-triple}(.exe)`
    let src_name = format!("{}-{}{}", sidecar_name, AGENTMUX_TARGET_TRIPLE, exe_suffix);
    let src = app
        .path()
        .resource_dir()
        .map_err(|e| format!("resource dir: {e}"))?
        .join(&src_name);

    if !src.exists() {
        return Err(format!(
            "bundled sidecar not found at {} — cannot copy to versioned data dir",
            src.display()
        ));
    }

    // Skip copy if dest exists and is already up-to-date (same size + mtime)
    if dest.exists() {
        let src_meta = std::fs::metadata(&src).ok();
        let dst_meta = std::fs::metadata(&dest).ok();
        if let (Some(s), Some(d)) = (src_meta, dst_meta) {
            if s.len() == d.len() && s.modified().ok() == d.modified().ok() {
                tracing::debug!(
                    "[sidecar] {} is up-to-date at {}",
                    sidecar_name,
                    dest.display()
                );
                return Ok(dest);
            }
        }
    }

    std::fs::copy(&src, &dest).map_err(|e| {
        format!(
            "copy {} → {}: {}",
            src.display(),
            dest.display(),
            e
        )
    })?;

    // Set executable bit on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).ok();
    }

    tracing::info!(
        "[sidecar] copied {} to versioned data dir: {}",
        sidecar_name,
        dest.display()
    );

    Ok(dest)
}

/// State returned after successfully spawning the backend.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackendSpawnResult {
    pub ws_endpoint: String,
    pub web_endpoint: String,
    pub auth_key: String,
    pub instance_id: String,  // version-namespaced, e.g. "v0.31.23"
    pub version: String,      // Backend version (e.g., "0.27.12")
}

/// Create a Windows Job Object and assign the child process to it.
/// The Job Object has KILL_ON_JOB_CLOSE set, so when the last handle closes
/// (including on crash/force-kill), Windows terminates all assigned processes.
/// Returns the Job Object handle which MUST be kept alive for the app's lifetime.
#[cfg(target_os = "windows")]
fn create_job_object_for_child(pid: u32) -> Result<*mut std::ffi::c_void, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::*;

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err("Failed to create job object".into());
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
            return Err("Failed to set job object info".into());
        }

        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            CloseHandle(job);
            return Err(format!("Failed to open process {}", pid));
        }

        let ok = AssignProcessToJobObject(job, process);
        CloseHandle(process);
        if ok == 0 {
            CloseHandle(job);
            return Err("Failed to assign process to job object".into());
        }

        Ok(job)
    }
}

/// Spawn the agentmuxsrv-rs Rust backend as a sidecar and wait for it to signal
/// readiness via a `WAVESRV-ESTART` line on stderr (30 s timeout).
///
/// On success, returns the WS/HTTP endpoints and auth key. The caller is
/// responsible for emitting `backend-ready` to the frontend.
pub async fn spawn_backend(app: &tauri::AppHandle) -> Result<BackendSpawnResult, String> {
    tracing::info!("spawn_backend() called");

    // 1. Resolve directories -----------------------------------------------
    let data_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("Failed to get local data dir: {}", e))?;
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Failed to get config dir: {}", e))?;

    tracing::info!("Using config_dir: {}", config_dir.display());
    tracing::info!("Using data_dir: {}", data_dir.display());

    let current_version = env!("CARGO_PKG_VERSION");
    let version_instance_id = format!("v{}", current_version);

    // 2. Cleanup stale processes/files ------------------------------------
    #[cfg(unix)]
    cleanup_stale_backends(current_version);
    cleanup_stale_endpoints(&config_dir);

    // 3. Ensure directory tree --------------------------------------------
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data dir: {}", e))?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    let version_data_home = data_dir.join("instances").join(&version_instance_id);
    let version_config_home = config_dir.join("instances").join(&version_instance_id);

    std::fs::create_dir_all(version_data_home.join("db"))
        .map_err(|e| format!("Failed to create version instance data dir: {}", e))?;
    std::fs::create_dir_all(&version_config_home)
        .map_err(|e| format!("Failed to create version instance config dir: {}", e))?;

    // 4. Spawn the process (binary resolution + wsh deploy + env vars) ----
    let auth_key = app.state::<crate::state::AppState>().auth_key.lock().unwrap().clone();
    tracing::info!("Spawning agentmuxsrv-rs with auth key: {}...", &auth_key[..8.min(auth_key.len())]);

    let spawned = binary::build_sidecar_command(
        app,
        &auth_key,
        &version_data_home,
        &version_config_home,
        &config_dir,
        &version_instance_id,
    )?;
    let (mut rx, child) = (spawned.rx, spawned.child);

    // 5. Store PID, child handle, and start time --------------------------
    let child_pid = child.pid();
    {
        let state = app.state::<crate::state::AppState>();
        *state.sidecar_child.lock().unwrap() = Some(child);
        *state.backend_pid.lock().unwrap() = Some(child_pid);
        *state.backend_started_at.lock().unwrap() = Some(chrono::Utc::now().to_rfc3339());
    }

    // 6. Windows: Job Object (KILL_ON_JOB_CLOSE) --------------------------
    #[cfg(target_os = "windows")]
    {
        match create_job_object_for_child(child_pid) {
            Ok(job_handle) => {
                tracing::info!("Created Job Object for backend (pid={}), KILL_ON_JOB_CLOSE active", child_pid);
                let state = app.state::<crate::state::AppState>();
                *state.job_handle.lock().unwrap() = Some(crate::state::JobHandle::new(job_handle));
            }
            Err(e) => {
                tracing::error!("Failed to create Job Object for backend: {}. Backend may orphan on crash.", e);
                // Non-fatal — graceful shutdown via child.kill() still works
            }
        }
    }

    // 7. Run event loop, wait for ESTART (30 s timeout) -------------------
    let (tx, mut endpoint_rx) = tokio::sync::mpsc::channel::<event_loop::EStartPayload>(1);
    tokio::spawn(event_loop::run(rx, app.clone(), tx));

    let estart = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        endpoint_rx.recv(),
    )
    .await
    .map_err(|_| "Timeout waiting for agentmuxsrv-rs to start (30s)".to_string())?
    .ok_or_else(|| "agentmuxsrv-rs channel closed before sending endpoints".to_string())?;

    let result = BackendSpawnResult {
        ws_endpoint: estart.ws,
        web_endpoint: estart.web,
        version: estart.version,
        instance_id: estart.instance_id,
        auth_key: auth_key.clone(),
    };

    tracing::info!(
        "Backend successfully started: ws={} web={} version={} instance={} auth_key={}...",
        result.ws_endpoint, result.web_endpoint, result.version, result.instance_id,
        &result.auth_key[..8.min(result.auth_key.len())]
    );

    Ok(result)
}

/// Find and kill stale agentmuxsrv-rs processes left behind by previous versions.
///
/// When the frontend crashes or is force-killed, the backend process may survive.
/// On the next launch we find all running agentmuxsrv-rs processes, inspect their
/// `--instance` argument, and kill any whose version differs from `current_version`.
#[cfg(unix)]
fn cleanup_stale_backends(current_version: &str) {
    let current_instance = format!("v{}", current_version);
    let my_pid = std::process::id();

    tracing::info!(
        "cleanup_stale_backends: looking for stale agentmuxsrv-rs processes (current instance={})",
        current_instance
    );

    // Step 1: Find all agentmuxsrv-rs PIDs via pgrep
    let pgrep_output = match std::process::Command::new("pgrep")
        .args(["-f", "agentmuxsrv-rs"])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            tracing::warn!("cleanup_stale_backends: failed to run pgrep: {}", e);
            return;
        }
    };

    if !pgrep_output.status.success() && pgrep_output.stdout.is_empty() {
        tracing::info!("cleanup_stale_backends: no agentmuxsrv-rs processes found");
        return;
    }

    let stdout = String::from_utf8_lossy(&pgrep_output.stdout);
    let pids: Vec<u32> = stdout
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect();

    if pids.is_empty() {
        tracing::info!("cleanup_stale_backends: no agentmuxsrv-rs PIDs parsed");
        return;
    }

    tracing::info!(
        "cleanup_stale_backends: found {} candidate PID(s): {:?}",
        pids.len(),
        pids
    );

    for pid in pids {
        if pid == my_pid {
            tracing::info!("cleanup_stale_backends: skipping our own PID {}", pid);
            continue;
        }

        // Step 2: Get the full command line for this PID
        let ps_output = match std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .output()
        {
            Ok(output) => output,
            Err(e) => {
                tracing::warn!(
                    "cleanup_stale_backends: failed to run ps for PID {}: {}",
                    pid, e
                );
                continue;
            }
        };

        if !ps_output.status.success() {
            tracing::info!(
                "cleanup_stale_backends: PID {} no longer exists (ps failed), skipping",
                pid
            );
            continue;
        }

        let cmdline = String::from_utf8_lossy(&ps_output.stdout).trim().to_string();
        if cmdline.is_empty() {
            tracing::info!(
                "cleanup_stale_backends: PID {} has empty command line, skipping",
                pid
            );
            continue;
        }

        // Step 3: Parse the --instance argument
        let instance_version = cmdline
            .split_whitespace()
            .collect::<Vec<&str>>()
            .windows(2)
            .find_map(|pair| {
                if pair[0] == "--instance" {
                    Some(pair[1].to_string())
                } else {
                    None
                }
            });

        let instance_version = match instance_version {
            Some(v) => v,
            None => {
                tracing::info!(
                    "cleanup_stale_backends: PID {} has no --instance arg, skipping",
                    pid
                );
                continue;
            }
        };

        // Step 4: Compare versions — skip if it matches the current version
        if instance_version == current_instance {
            tracing::info!(
                "cleanup_stale_backends: PID {} is current version ({}), keeping",
                pid, instance_version
            );
            continue;
        }

        tracing::info!(
            "cleanup_stale_backends: PID {} is stale (instance={}, current={}), terminating",
            pid, instance_version, current_instance
        );

        // Step 5: Send SIGTERM
        let pid_i32 = pid as i32;
        let term_result = unsafe { libc::kill(pid_i32, libc::SIGTERM) };
        if term_result != 0 {
            let err = std::io::Error::last_os_error();
            tracing::warn!(
                "cleanup_stale_backends: SIGTERM failed for PID {}: {}",
                pid, err
            );
            continue;
        }

        // Step 6: Wait up to 3 seconds for the process to exit
        let mut exited = false;
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            unsafe {
                if libc::kill(pid_i32, 0) != 0 {
                    exited = true;
                    break;
                }
            }
        }

        if exited {
            tracing::info!("cleanup_stale_backends: PID {} exited after SIGTERM", pid);
        } else {
            tracing::warn!(
                "cleanup_stale_backends: PID {} did not exit after SIGTERM, sending SIGKILL",
                pid
            );
            unsafe { libc::kill(pid_i32, libc::SIGKILL); }
            tracing::info!("cleanup_stale_backends: sent SIGKILL to PID {}", pid);
        }
    }
}

/// Remove stale wave-endpoints.json files from all instance directories.
/// These files were written by older versions that used backend reuse.
/// Transitional cleanup — can be removed once all users have upgraded.
fn cleanup_stale_endpoints(config_dir: &std::path::Path) {
    let instances_dir = config_dir.join("instances");
    if let Ok(entries) = std::fs::read_dir(&instances_dir) {
        for entry in entries.flatten() {
            let endpoints_file = entry.path().join("wave-endpoints.json");
            if endpoints_file.exists() {
                if let Err(e) = std::fs::remove_file(&endpoints_file) {
                    tracing::warn!("Failed to remove stale endpoints file {}: {}", endpoints_file.display(), e);
                } else {
                    tracing::info!("Removed stale endpoints file: {}", endpoints_file.display());
                }
            }
        }
    }
}

/// Handle backend event messages from agentmuxsrv-rs.
/// Called by `event_loop::run` when a `WAVESRV-EVENT:` line is received.
pub(crate) fn handle_backend_event(app: &tauri::AppHandle, event_data: &str) {
    tracing::debug!("Backend event: {}", event_data);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("agentmuxsrv-event", event_data.to_string());
    }
}
