// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sidecar spawning and management for the CEF host.
// Ported from src-tauri/src/sidecar/ using std::process instead of tauri-plugin-shell.

use std::io::BufRead;
use std::sync::Arc;

use crate::state::AppState;

/// State returned after successfully spawning the backend.
#[derive(Clone, Debug)]
pub struct BackendSpawnResult {
    pub ws_endpoint: String,
    pub web_endpoint: String,
    pub version: String,
    pub instance_id: String,
}

/// Spawn the agentmuxsrv-rs backend sidecar and wait for it to signal
/// readiness via a `WAVESRV-ESTART` line on stderr (30s timeout).
pub async fn spawn_backend(state: &Arc<AppState>) -> Result<BackendSpawnResult, String> {
    tracing::info!("spawn_backend() called");

    // 1. Resolve directories
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "Failed to get data dir".to_string())?
        .join("ai.agentmux.cef");
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "Failed to get config dir".to_string())?
        .join("ai.agentmux.cef");

    tracing::info!("Using config_dir: {}", config_dir.display());
    tracing::info!("Using data_dir: {}", data_dir.display());

    let current_version = env!("CARGO_PKG_VERSION");
    let version_instance_id = format!("v{}", current_version);

    // 2. Ensure directory tree
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

    // 3. Resolve the backend binary path
    let backend_name = "agentmuxsrv-rs";
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };

    let backend_path = resolve_backend_binary(backend_name, exe_suffix)?;
    tracing::info!("Using backend binary: {}", backend_path.display());

    // 4. Resolve AGENTMUX_APP_PATH
    let app_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();

    let app_path_str = app_path.to_string_lossy().to_string();

    // 5. Deploy wsh binary
    deploy_wsh(&app_path);

    // 6. Spawn the process
    let auth_key = state.auth_key.lock().unwrap().clone();
    tracing::info!(
        "Spawning agentmuxsrv-rs with auth key: {}...",
        &auth_key[..8.min(auth_key.len())]
    );

    let mut cmd = std::process::Command::new(&backend_path);
    cmd.args([
        "--wavedata",
        &version_data_home.to_string_lossy(),
        "--instance",
        &version_instance_id,
    ])
    .env("AGENTMUX_AUTH_KEY", &auth_key)
    .env(
        "AGENTMUX_CONFIG_HOME",
        version_config_home.to_string_lossy().to_string(),
    )
    .env(
        "AGENTMUX_DATA_HOME",
        version_data_home.to_string_lossy().to_string(),
    )
    .env(
        "AGENTMUX_SETTINGS_DIR",
        config_dir.to_string_lossy().to_string(),
    )
    .env("AGENTMUX_APP_PATH", &app_path_str)
    .env(
        "AGENTMUX_DEV",
        if cfg!(debug_assertions) { "1" } else { "" },
    )
    .env("WCLOUD_ENDPOINT", "https://api.agentmux.ai/central")
    .env("WCLOUD_WS_ENDPOINT", "wss://wsapi.agentmux.ai/")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn agentmuxsrv-rs: {}", e))?;

    let child_pid = child.id();
    tracing::info!("Backend spawned with PID: {}", child_pid);

    // 7. Store PID and start time
    *state.backend_pid.lock().unwrap() = Some(child_pid);
    *state.backend_started_at.lock().unwrap() = Some(chrono::Utc::now().to_rfc3339());

    // 8. Windows: Job Object (KILL_ON_JOB_CLOSE)
    #[cfg(target_os = "windows")]
    {
        match create_job_object_for_child(child_pid) {
            Ok(job_handle) => {
                tracing::info!(
                    "Created Job Object for backend (pid={}), KILL_ON_JOB_CLOSE active",
                    child_pid
                );
                *state.job_handle.lock().unwrap() =
                    Some(crate::state::JobHandle::new(job_handle));
            }
            Err(e) => {
                tracing::error!(
                    "Failed to create Job Object for backend: {}. Backend may orphan on crash.",
                    e
                );
            }
        }
    }

    // 9. Parse stderr for ESTART (30s timeout)
    let stderr = child.stderr.take().expect("Failed to get stderr");

    // Take ownership of stdout for logging
    let stdout = child.stdout.take();

    // Store the child handle
    *state.sidecar_child.lock().unwrap() = Some(child);

    // Spawn stdout reader
    if let Some(stdout) = stdout {
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => tracing::info!("[agentmuxsrv-rs stdout] {}", l),
                    Err(_) => break,
                }
            }
        });
    }

    // Parse ESTART from stderr
    let (tx, mut rx) = tokio::sync::mpsc::channel::<BackendSpawnResult>(1);
    let state_for_monitor = state.clone();

    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stderr);
        let mut estart_received = false;
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if l.starts_with("WAVESRV-ESTART") {
                        let result = parse_estart(&l);
                        tracing::info!(
                            "Backend started: ws={} web={} version={} instance={}",
                            result.ws_endpoint,
                            result.web_endpoint,
                            result.version,
                            result.instance_id
                        );
                        estart_received = true;
                        let _ = tx.blocking_send(result);
                    } else if let Some(event_data) = l.strip_prefix("WAVESRV-EVENT:") {
                        tracing::debug!("Backend event: {}", event_data);
                        // Forward events to the frontend
                        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(event_data)
                        {
                            crate::events::emit_event_from_state(
                                &state_for_monitor,
                                "agentmuxsrv-event",
                                &payload,
                            );
                        } else {
                            crate::events::emit_event_from_state(
                                &state_for_monitor,
                                "agentmuxsrv-event",
                                &serde_json::json!(event_data),
                            );
                        }
                    } else {
                        tracing::info!("[agentmuxsrv-rs] {}", l);
                    }
                }
                Err(_) => break,
            }
        }

        // Process exited — emit backend-terminated
        let pid = state_for_monitor.backend_pid.lock().unwrap().unwrap_or(0);
        if estart_received {
            tracing::error!(
                "[agentmuxsrv-rs] RUNTIME CRASH — pid={}",
                pid
            );
        } else {
            tracing::error!(
                "[agentmuxsrv-rs] STARTUP CRASH — terminated before ready (pid={})",
                pid
            );
        }

        let payload = serde_json::json!({
            "pid": pid,
        });
        crate::events::emit_event_from_state(
            &state_for_monitor,
            "backend-terminated",
            &payload,
        );
    });

    // Wait for ESTART with 30s timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
        .await
        .map_err(|_| "Timeout waiting for agentmuxsrv-rs to start (30s)".to_string())?
        .ok_or_else(|| "agentmuxsrv-rs channel closed before sending endpoints".to_string())?;

    tracing::info!(
        "Backend successfully started: ws={} web={} version={} instance={}",
        result.ws_endpoint,
        result.web_endpoint,
        result.version,
        result.instance_id
    );

    Ok(result)
}

/// Resolve the backend binary path.
/// Checks: portable path (bin/agentmuxsrv-rs.x64.exe), dev mode (adjacent to exe),
/// and dist/bin/ paths.
fn resolve_backend_binary(
    backend_name: &str,
    exe_suffix: &str,
) -> Result<std::path::PathBuf, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe: {}", e))?;
    let exe_dir = exe_path.parent().unwrap();

    // Portable release: bin/{name}.x64.exe next to the app exe
    let portable_binary = exe_dir
        .join("bin")
        .join(format!("{}.x64{}", backend_name, exe_suffix));
    if portable_binary.exists() {
        tracing::info!(
            "Using portable {} at: {:?}",
            backend_name,
            portable_binary
        );
        return Ok(portable_binary);
    }

    // Dev mode: {name}(.exe) adjacent to the host binary
    let dev_binary = exe_dir.join(format!("{}{}", backend_name, exe_suffix));
    if dev_binary.exists() {
        tracing::info!("Using dev-mode {} at: {:?}", backend_name, dev_binary);
        return Ok(dev_binary);
    }

    // Check dist/bin/ in the workspace
    let dist_binary = exe_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|workspace| {
            workspace
                .join("dist")
                .join("bin")
                .join(format!("{}{}", backend_name, exe_suffix))
        });
    if let Some(ref dist) = dist_binary {
        if dist.exists() {
            tracing::info!("Using dist {} at: {:?}", backend_name, dist);
            return Ok(dist.clone());
        }
    }

    Err(format!(
        "Backend binary '{}' not found. Searched:\n  - {:?}\n  - {:?}\n  - {:?}",
        backend_name, portable_binary, dev_binary, dist_binary
    ))
}

/// Parse the key=value fields out of a `WAVESRV-ESTART` line.
fn parse_estart(line: &str) -> BackendSpawnResult {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let get = |prefix: &str| -> String {
        parts
            .iter()
            .find_map(|p| p.strip_prefix(prefix))
            .unwrap_or_default()
            .to_string()
    };
    BackendSpawnResult {
        ws_endpoint: get("ws:"),
        web_endpoint: get("web:"),
        version: get("version:"),
        instance_id: get("instance:"),
    }
}

/// Deploy the bundled wsh binary.
fn deploy_wsh(app_path: &std::path::Path) {
    let bin_dir = app_path.join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin_dir) {
        tracing::warn!("Failed to create bin dir for wsh: {}", e);
        return;
    }

    let wsh_src_name = if cfg!(windows) { "wsh.exe" } else { "wsh" };
    let bundled_wsh = app_path.join(wsh_src_name);
    if !bundled_wsh.exists() {
        // Not an error in dev mode — wsh may not be available
        tracing::debug!("Bundled wsh not found at: {}", bundled_wsh.display());
        return;
    }

    let version = env!("CARGO_PKG_VERSION");
    let (goos, goarch) = if cfg!(target_os = "macos") {
        (
            "darwin",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            },
        )
    } else if cfg!(target_os = "linux") {
        (
            "linux",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            },
        )
    } else {
        (
            "windows",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            },
        )
    };

    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    let wsh_name = format!("wsh-{}-{}.{}{}", version, goos, goarch, exe_suffix);
    let dest = bin_dir.join(&wsh_name);

    if dest.exists() {
        return; // already deployed
    }

    if let Err(e) = std::fs::copy(&bundled_wsh, &dest) {
        tracing::warn!("Failed to copy wsh to {}: {}", dest.display(), e);
        return;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    tracing::info!("Deployed wsh to: {}", dest.display());
}

/// Create a Windows Job Object and assign the child process to it.
#[cfg(target_os = "windows")]
fn create_job_object_for_child(pid: u32) -> Result<*mut std::ffi::c_void, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::*;

    // CreateJobObjectW is not exported by windows-sys 0.59's JobObjects feature,
    // so we link to kernel32.dll directly.
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(
            lpjobattributes: *const std::ffi::c_void,
            lpname: *const u16,
        ) -> *mut std::ffi::c_void;
    }

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
