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

/// Spawn the agentmux-srv backend sidecar and wait for it to signal
/// readiness via a `WAVESRV-ESTART` line on stderr (30s timeout).
pub async fn spawn_backend(state: &Arc<AppState>) -> Result<BackendSpawnResult, String> {
    tracing::info!("spawn_backend() called");

    // 1. Resolve directories — per-version top-level dir (matches Tauri pattern)
    //    Tauri uses: ai.agentmux.app.v0-31-100/
    //    CEF uses:   ai.agentmux.cef.v0-32-111/
    //    Dev uses:   ai.agentmux.cef.dev/
    let current_version = env!("CARGO_PKG_VERSION");
    let version_instance_id = format!("v{}", current_version);

    let is_dev = cfg!(debug_assertions);
    let dir_name = if is_dev {
        "ai.agentmux.cef.dev".to_string()
    } else {
        let version_slug = current_version.replace('.', "-");
        format!("ai.agentmux.cef.v{}", version_slug)
    };

    let data_dir = dirs::data_dir()
        .ok_or_else(|| "Failed to get data dir".to_string())?
        .join(&dir_name);
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "Failed to get config dir".to_string())?
        .join(&dir_name);

    tracing::info!("Using config_dir: {}", config_dir.display());
    tracing::info!("Using data_dir: {}", data_dir.display());

    // 2. Ensure directory tree (flat — no instances/ subdirectory)
    std::fs::create_dir_all(data_dir.join("db"))
        .map_err(|e| format!("Failed to create data dir: {}", e))?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    // Store version-specific paths in AppState for frontend IPC commands
    *state.version_data_dir.lock().unwrap() = Some(data_dir.to_string_lossy().to_string());
    *state.version_config_dir.lock().unwrap() = Some(config_dir.to_string_lossy().to_string());

    // 3. Resolve the backend binary path
    let backend_name = "agentmux-srv";
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
        "Spawning agentmux-srv with auth key: {}...",
        &auth_key[..8.min(auth_key.len())]
    );

    let mut cmd = std::process::Command::new(&backend_path);
    cmd.args([
        "--wavedata",
        &data_dir.to_string_lossy(),
        "--instance",
        &version_instance_id,
    ])
    .env("AGENTMUX_AUTH_KEY", &auth_key)
    .env(
        "AGENTMUX_CONFIG_HOME",
        config_dir.to_string_lossy().to_string(),
    )
    .env(
        "AGENTMUX_DATA_HOME",
        data_dir.to_string_lossy().to_string(),
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
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn agentmux-srv: {}", e))?;

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
                    Ok(l) => tracing::info!("[agentmux-srv stdout] {}", l),
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
                        tracing::info!("[agentmux-srv] {}", l);
                    }
                }
                Err(_) => break,
            }
        }

        // Process exited — emit backend-terminated
        let pid = state_for_monitor.backend_pid.lock().unwrap().unwrap_or(0);
        if estart_received {
            tracing::error!(
                "[agentmux-srv] RUNTIME CRASH — pid={}",
                pid
            );
        } else {
            tracing::error!(
                "[agentmux-srv] STARTUP CRASH — terminated before ready (pid={})",
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
        .map_err(|_| "Timeout waiting for agentmux-srv to start (30s)".to_string())?
        .ok_or_else(|| "agentmux-srv channel closed before sending endpoints".to_string())?;

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
/// Looks for versioned agentmux-srv-{version}{suffix} in the same dir as the CEF host,
/// or unversioned agentmux-srv{suffix} for dev mode (cargo output).
/// Hard fails with a directory listing if not found — no fallbacks.
fn resolve_backend_binary(
    backend_name: &str,
    exe_suffix: &str,
) -> Result<std::path::PathBuf, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe: {}", e))?;
    let exe_dir = exe_path.parent().unwrap();
    let version = env!("CARGO_PKG_VERSION");

    // Portable layout: agentmux-srv-{version}.exe in the same dir as CEF host
    let versioned = exe_dir.join(format!("{}-{}{}", backend_name, version, exe_suffix));
    if versioned.exists() {
        tracing::info!("Using {} at: {:?}", backend_name, versioned);
        return Ok(versioned);
    }

    // Dev mode: agentmux-srv(.exe) adjacent to the host binary (cargo build output)
    let dev_binary = exe_dir.join(format!("{}{}", backend_name, exe_suffix));
    if dev_binary.exists() {
        tracing::info!("Using dev-mode {} at: {:?}", backend_name, dev_binary);
        return Ok(dev_binary);
    }

    // Hard fail — list directory contents to aid diagnosis
    let mut listing = String::new();
    if let Ok(entries) = std::fs::read_dir(exe_dir) {
        for entry in entries.flatten() {
            listing.push_str(&format!("\n  {}", entry.file_name().to_string_lossy()));
        }
    }
    Err(format!(
        "FATAL: Backend binary not found.\n  Expected: {:?}\n  Dev mode: {:?}\n  Contents of {:?}:{}",
        versioned, dev_binary, exe_dir, listing
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

/// Deploy the bundled agentmux-wsh binary.
fn deploy_wsh(app_path: &std::path::Path) {
    let bin_dir = app_path.join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin_dir) {
        tracing::warn!("Failed to create bin dir for agentmux-wsh: {}", e);
        return;
    }

    let version = env!("CARGO_PKG_VERSION");
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };

    // Versioned source name in portable runtime/
    let wsh_src_name = format!("agentmux-wsh-{}{}", version, exe_suffix);
    let bundled_wsh = app_path.join(&wsh_src_name);

    // Dev-mode fallback: cargo outputs unversioned agentmux-wsh adjacent to the CEF binary
    let dev_wsh = app_path.join(format!("agentmux-wsh{}", exe_suffix));

    let bundled_wsh = if bundled_wsh.exists() {
        bundled_wsh
    } else if dev_wsh.exists() {
        tracing::debug!("deploy_wsh: using dev-mode unversioned binary: {}", dev_wsh.display());
        dev_wsh
    } else {
        tracing::debug!(
            "deploy_wsh: agentmux-wsh not found (versioned: {}, dev: {}) — shell integration unavailable",
            bundled_wsh.display(),
            dev_wsh.display()
        );
        return;
    };

    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "windows"
    };

    let dest_name = format!("agentmux-wsh-{}-{}.{}{}", version, platform, arch, exe_suffix);
    let dest = bin_dir.join(&dest_name);

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

    tracing::info!("Deployed agentmux-wsh to: {}", dest.display());
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
