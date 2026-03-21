#[cfg(unix)]
use libc;
use std::path::PathBuf;
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

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
fn ensure_versioned_sidecar(app: &tauri::AppHandle, sidecar_name: &str) -> Result<PathBuf, String> {
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

/// Spawn the agentmuxsrv-rs Rust backend as a Tauri sidecar.
///
/// The backend prints a line to stderr when ready:
///   WAVESRV-ESTART ws:<addr> web:<addr> version:<ver> buildtime:<time>
///
/// We parse that line to get the WebSocket and HTTP endpoints,
/// then the frontend connects to them directly.
pub async fn spawn_backend(app: &tauri::AppHandle) -> Result<BackendSpawnResult, String> {
    tracing::info!("spawn_backend() called");

    // Use app_local_data_dir for database storage (AppData\Local on Windows)
    // Use app_config_dir for configuration (AppData\Roaming on Windows)
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

    // Kill any orphaned backend processes from previous versions
    #[cfg(unix)]
    cleanup_stale_backends(current_version);

    // Clean up any stale endpoints files from previous versions that used the reuse mechanism
    cleanup_stale_endpoints(&config_dir);

    // Ensure base directories exist
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data dir: {}", e))?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    // Pre-create version-namespaced instance directory structure
    let version_data_instance_dir = data_dir.join("instances").join(&version_instance_id).join("db");
    std::fs::create_dir_all(&version_data_instance_dir)
        .map_err(|e| format!("Failed to create version instance data dir: {}", e))?;

    let version_config_instance_dir = config_dir.join("instances").join(&version_instance_id);
    std::fs::create_dir_all(&version_config_instance_dir)
        .map_err(|e| format!("Failed to create version instance config dir: {}", e))?;

    // Get auth key from app state
    let auth_key = app.state::<crate::state::AppState>().auth_key.lock().unwrap().clone();
    let key_preview = auth_key.chars().take(8).collect::<String>();
    tracing::info!("Spawning agentmuxsrv-rs with auth key: {}", key_preview);

    let shell = app.shell();

    let backend_name = "agentmuxsrv-rs";

    // Try to find backend in portable mode first (bin/ subdir next to exe),
    // or in dev mode (agentmuxsrv-rs.exe without triple suffix in the same dir as the debug binary,
    // placed there by the sync:dev:binaries task).
    let portable_path = std::env::current_exe().ok().and_then(|exe_path| {
        let exe_dir = exe_path.parent()?;
        // Portable release: bin/{name}.x64.exe next to app exe
        let portable_binary = exe_dir.join("bin").join(format!("{}.x64.exe", backend_name));
        if portable_binary.exists() {
            tracing::info!("Using portable {} at: {:?}", backend_name, portable_binary);
            return Some(portable_binary);
        }
        // Dev mode (tauri dev / cargo run): sync:dev:binaries copies the binary as
        // {name}.exe (no triple) into target/debug/ alongside the host binary.
        let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
        let dev_binary = exe_dir.join(format!("{}{}", backend_name, exe_suffix));
        if dev_binary.exists() {
            tracing::info!("Using dev-mode {} at: {:?}", backend_name, dev_binary);
            return Some(dev_binary);
        }
        None
    });

    let sidecar_cmd = if let Some(portable_exe) = portable_path {
        // Portable mode: run executable from bin/ subdir next to the app exe.
        // This path is already version-isolated because each portable ZIP has its own dir.
        shell.command(portable_exe)
    } else {
        // Installer / dev mode: copy sidecar to the version-isolated app local data dir
        // and run from there.  This avoids the Windows file-lock problem where tauri-build
        // tries to hash `src-tauri/binaries/agentmuxsrv-rs-{triple}.exe` while a previous
        // version is still running and holding a read lock on that exact file.
        let versioned_path = ensure_versioned_sidecar(app, backend_name)?;
        shell.command(versioned_path)
    };

    // Resolve AGENTMUX_APP_PATH and deploy wsh binary for the backend.
    let app_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();

    // Deploy bundled wsh to bin/ with the name the backend expects
    let bin_dir = app_path.join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin_dir) {
        tracing::warn!("Failed to create bin dir for wsh: {}", e);
    } else {
        let bundled_wsh = app_path.join("wsh");
        if bundled_wsh.exists() {
            let version = env!("CARGO_PKG_VERSION");
            let (goos, goarch) = if cfg!(target_os = "macos") {
                ("darwin", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
            } else if cfg!(target_os = "linux") {
                ("linux", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
            } else {
                ("windows", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
            };
            let wsh_name = format!("wsh-{}-{}.{}", version, goos, goarch);
            let dest = bin_dir.join(&wsh_name);
            if !dest.exists() {
                if let Err(e) = std::fs::copy(&bundled_wsh, &dest) {
                    tracing::warn!("Failed to copy wsh to {}: {}", dest.display(), e);
                } else {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
                    }
                    tracing::info!("Deployed wsh to: {}", dest.display());
                }
            }
        } else {
            tracing::warn!("Bundled wsh not found at: {}", bundled_wsh.display());
        }
    }

    let app_path_str = app_path.to_string_lossy().to_string();
    tracing::info!("Setting AGENTMUX_APP_PATH to: {}", app_path_str);

    // Version-specific data/config directories to isolate SQLite databases per version
    let version_data_home = data_dir.join("instances").join(&version_instance_id);
    let version_config_home = config_dir.join("instances").join(&version_instance_id);

    let (mut rx, child) = sidecar_cmd
        .args([
            "--wavedata",
            &version_data_home.to_string_lossy(),
            "--instance",
            &version_instance_id,
        ])
        .env("AGENTMUX_AUTH_KEY", &auth_key)
        .env("AGENTMUX_CONFIG_HOME", version_config_home.to_string_lossy().to_string())
        .env("AGENTMUX_DATA_HOME", version_data_home.to_string_lossy().to_string())
        .env("AGENTMUX_SETTINGS_DIR", config_dir.to_string_lossy().to_string())
        .env("AGENTMUX_APP_PATH", &app_path_str)
        .env("AGENTMUX_DEV", if cfg!(debug_assertions) { "1" } else { "" })
        .env("WCLOUD_ENDPOINT", "https://api.agentmux.ai/central")
        .env("WCLOUD_WS_ENDPOINT", "wss://wsapi.agentmux.ai/")
        .spawn()
        .map_err(|e| format!("Failed to spawn agentmuxsrv-rs: {}", e))?;

    // Get the child PID before storing the handle
    let child_pid = child.pid();

    // Store child handle, PID, and start time
    {
        let state = app.state::<crate::state::AppState>();
        let mut sidecar = state.sidecar_child.lock().unwrap();
        *sidecar = Some(child);
        *state.backend_pid.lock().unwrap() = Some(child_pid);
        *state.backend_started_at.lock().unwrap() = Some(chrono::Utc::now().to_rfc3339());
    }

    // Windows: Create Job Object and assign the backend process to it.
    // This ensures the backend is killed by the kernel if the frontend crashes.
    #[cfg(target_os = "windows")]
    {
        match create_job_object_for_child(child_pid) {
            Ok(job_handle) => {
                tracing::info!("Created Job Object for backend (pid={}), KILL_ON_JOB_CLOSE active", child_pid);
                let state = app.state::<crate::state::AppState>();
                let mut handle = state.job_handle.lock().unwrap();
                *handle = Some(crate::state::JobHandle::new(job_handle));
            }
            Err(e) => {
                tracing::error!("Failed to create Job Object for backend: {}. Backend may orphan on crash.", e);
                // Non-fatal — graceful shutdown via child.kill() still works
            }
        }
    }

    // Wait for WAVESRV-ESTART line from stderr
    let (tx, mut endpoint_rx) = tokio::sync::mpsc::channel::<(String, String, String, String)>(1);
    let app_handle = app.clone();

    tokio::spawn(async move {
        use tauri_plugin_shell::process::CommandEvent;

        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(line) => {
                    let line = String::from_utf8_lossy(&line);
                    for l in line.lines() {
                        if l.starts_with("WAVESRV-ESTART") {
                            let parts: Vec<&str> = l.split_whitespace().collect();
                            let ws = parts
                                .iter()
                                .find_map(|p| p.strip_prefix("ws:"))
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            let web = parts
                                .iter()
                                .find_map(|p| p.strip_prefix("web:"))
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            let version = parts
                                .iter()
                                .find_map(|p| p.strip_prefix("version:"))
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            let instance_id = parts
                                .iter()
                                .find_map(|p| p.strip_prefix("instance:"))
                                .map(|s| s.to_string())
                                .unwrap_or_default();

                            tracing::info!("Backend started: ws={}, web={}, version={}, instance={}", ws, web, version, instance_id);
                            let _ = tx.send((ws, web, version, instance_id)).await;
                        } else if let Some(event_data) = l.strip_prefix("WAVESRV-EVENT:") {
                            handle_backend_event(&app_handle, event_data);
                        } else {
                            tracing::info!("[agentmuxsrv-rs] {}", l);
                        }
                    }
                }
                CommandEvent::Stdout(line) => {
                    let line = String::from_utf8_lossy(&line);
                    tracing::info!("[agentmuxsrv-rs stdout] {}", line.trim());
                }
                CommandEvent::Error(err) => {
                    tracing::error!("[agentmuxsrv-rs error] {}", err);
                }
                CommandEvent::Terminated(status) => {
                    tracing::warn!("[agentmuxsrv-rs] terminated with status: {:?}", status);
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.emit("backend-terminated", serde_json::json!({
                            "code": status.code,
                            "signal": status.signal,
                        }));
                    }
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for endpoints with timeout
    let timeout = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        endpoint_rx.recv(),
    )
    .await
    .map_err(|_| "Timeout waiting for agentmuxsrv-rs to start (30s)".to_string())?
    .ok_or_else(|| "agentmuxsrv-rs channel closed before sending endpoints".to_string())?;

    let result = BackendSpawnResult {
        ws_endpoint: timeout.0,
        web_endpoint: timeout.1,
        version: timeout.2,
        instance_id: timeout.3,
        auth_key: auth_key.clone(),
    };

    let key_preview = result.auth_key.chars().take(8).collect::<String>();
    tracing::info!("Backend successfully started: ws={}, web={}, version={}, instance={}, auth_key={}...",
        result.ws_endpoint, result.web_endpoint, result.version, result.instance_id, key_preview);

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
        // Never kill our own process
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
                    pid,
                    e
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
                pid,
                instance_version
            );
            continue;
        }

        tracing::info!(
            "cleanup_stale_backends: PID {} is stale (instance={}, current={}), terminating",
            pid,
            instance_version,
            current_instance
        );

        // Step 5: Send SIGTERM
        let pid_i32 = pid as i32;
        let term_result = unsafe { libc::kill(pid_i32, libc::SIGTERM) };
        if term_result != 0 {
            let err = std::io::Error::last_os_error();
            tracing::warn!(
                "cleanup_stale_backends: SIGTERM failed for PID {}: {}",
                pid,
                err
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
            tracing::info!(
                "cleanup_stale_backends: PID {} exited after SIGTERM",
                pid
            );
        } else {
            tracing::warn!(
                "cleanup_stale_backends: PID {} did not exit after SIGTERM, sending SIGKILL",
                pid
            );
            unsafe {
                libc::kill(pid_i32, libc::SIGKILL);
            }
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
fn handle_backend_event(app: &tauri::AppHandle, event_data: &str) {
    tracing::debug!("Backend event: {}", event_data);

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("agentmuxsrv-event", event_data.to_string());
    }
}
