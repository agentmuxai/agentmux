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

/// Phase B.1: when the launcher has already spawned srv (env var
/// `AGENTMUX_BACKEND_WS` is set), populate `state` from the env vars
/// the launcher passed and return Ok — no need to spawn srv ourselves.
///
/// Returns `Ok(BackendSpawnResult)` mirroring the shape of
/// `spawn_backend` so the caller in main.rs is symmetric. Returns
/// `Err` only if env vars are present but malformed; if they're
/// absent the caller should fall through to `spawn_backend` (the
/// `task dev` path where the launcher isn't in the loop).
pub fn use_launcher_endpoints(
    state: &Arc<AppState>,
) -> Option<Result<BackendSpawnResult, String>> {
    // The env var being absent means "host is running standalone";
    // fall through to spawn_backend. The env var being PRESENT but
    // empty means the launcher set it (we're in launcher-managed
    // mode) but with a malformed value — likely an ESTART parse
    // mismatch in the launcher. Treat that as Err, NOT as
    // "fall back to spawn_backend" — the latter would spawn a
    // SECOND srv against the same data dir while the launcher's
    // already-running srv keeps going, corrupting state. Better to
    // fail fast and surface the launcher bug. (codex P2 @
    // sidecar.rs:35, PR #571 round-4.)
    let ws = std::env::var("AGENTMUX_BACKEND_WS").ok()?;
    if ws.is_empty() {
        return Some(Err(
            "launcher set AGENTMUX_BACKEND_WS but it was empty — \
             refusing to fall back to spawn_backend (would create a \
             duplicate srv against the same data dir). This is a \
             launcher bug; check the AGENTMUXSRV-ESTART parse path in \
             agentmux-launcher/src/srv_spawner.rs::parse_estart."
                .to_string(),
        ));
    }
    // From here on we KNOW the launcher provided env; missing fields
    // mean a launcher bug, so emit Err rather than fall through to
    // the dev-mode spawn (which would fight the launcher's srv).
    let try_get = |key: &str| -> Result<String, String> {
        std::env::var(key).map_err(|_| format!("launcher set AGENTMUX_BACKEND_WS but not {}", key))
    };
    let result = (|| -> Result<BackendSpawnResult, String> {
        let web = try_get("AGENTMUX_BACKEND_WEB")?;
        let pid_str = try_get("AGENTMUX_BACKEND_PID")?;
        let pid: u32 = pid_str
            .parse()
            .map_err(|_| format!("AGENTMUX_BACKEND_PID not a u32: {}", pid_str))?;
        let auth_key = try_get("AGENTMUX_AUTH_KEY")?;
        let instance_id = try_get("AGENTMUX_INSTANCE_ID")?;
        let data_dir = try_get("AGENTMUX_DATA_DIR")?;
        let config_dir = try_get("AGENTMUX_CONFIG_DIR")?;
        let user_home_dir = try_get("AGENTMUX_USER_HOME_DIR")?;

        // Populate AppState in the same shape `spawn_backend` would.
        // Notably we do NOT take ownership of a Child handle (launcher
        // owns it) and we do NOT create a Job Object (launcher's J0
        // already covers srv via assignment, not inheritance).
        *state.auth_key.lock() = auth_key;
        *state.backend_pid.lock() = Some(pid);
        *state.backend_started_at.lock() = Some(chrono::Utc::now().to_rfc3339());
        *state.version_data_dir.lock() = Some(data_dir);
        *state.version_config_dir.lock() = Some(config_dir);
        *state.user_home_dir.lock() = Some(user_home_dir);

        let version = env!("CARGO_PKG_VERSION").to_string();
        Ok(BackendSpawnResult {
            ws_endpoint: ws,
            web_endpoint: web,
            version,
            instance_id,
        })
    })();
    Some(result)
}

/// Spawn the agentmux-srv backend sidecar and wait for it to signal
/// readiness via a `AGENTMUXSRV-ESTART` line on stderr (30s timeout).
pub async fn spawn_backend(state: &Arc<AppState>) -> Result<BackendSpawnResult, String> {
    tracing::info!("spawn_backend() called");

    // 1. Resolve directories from the launcher-injected env vars.
    //
    // Pre-PR-#695, the host re-derived its own portable + dev mode and
    // computed paths independently as a fallback for the no-launcher
    // case (legacy `task dev`). After unification all paths flow from
    // the launcher; this function is now only reached via the
    // `restart_backend` RPC, where the launcher's env vars are still
    // in scope, so `DataPaths::from_env` always succeeds. If they're
    // somehow absent, refuse to spawn — re-deriving silently would
    // restore the old desync risk.
    let paths = agentmux_common::DataPaths::from_env().ok_or_else(|| {
        "spawn_backend: launcher env vars (AGENTMUX_DATA_DIR etc.) absent — \
         host was started without the launcher, which is no longer supported"
            .to_string()
    })?;
    let current_version = env!("CARGO_PKG_VERSION");
    let version_instance_id = format!("v{}", current_version);
    let data_dir = paths.data_dir.clone();
    let config_dir = paths.config_dir.clone();

    tracing::info!(
        runtime_mode = ?paths.mode.to_env_string(),
        "Using data_dir: {}",
        data_dir.display()
    );
    tracing::info!("Using config_dir: {}", config_dir.display());

    // 2. Ensure directory tree (idempotent; launcher already did this
    // at startup but we rerun in case the dir got nuked between launch
    // and restart).
    paths
        .ensure_dirs()
        .map_err(|e| format!("Failed to ensure data dirs: {}", e))?;

    // Store version-specific paths in AppState for frontend IPC commands.
    *state.version_data_dir.lock() = Some(data_dir.to_string_lossy().to_string());
    *state.version_config_dir.lock() = Some(config_dir.to_string_lossy().to_string());
    // Frontend "user home" → maps to per-version agents_dir under the
    // unified layout. (Account-wide stuff goes in shared_dir; agents
    // stay version-keyed for now per spec §3.1 phase-1 scope.)
    *state.user_home_dir.lock() = Some(paths.agents_dir.to_string_lossy().to_string());

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

    // wsh has been retired — see specs/SPEC_RETIRE_WSH_2026_04_12.md.
    // No binary to deploy anymore.

    // Spawn the process
    let auth_key = state.auth_key.lock().clone();
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
    // Canonical AGENTMUX_* env vars from `DataPaths::to_env_vars()`.
    // Replaces the pre-unification AGENTMUX_DATA_HOME / CONFIG_HOME /
    // SETTINGS_DIR / DEV set, which is no longer set by anyone in the
    // chain (launcher → host, host → srv).
    .envs(paths.to_env_vars())
    .env("AGENTMUX_APP_PATH", &app_path_str)
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
    *state.backend_pid.lock() = Some(child_pid);
    *state.backend_started_at.lock() = Some(chrono::Utc::now().to_rfc3339());

    // (Phase B.1: removed host-owned Job Object J1 on srv. The
    // launcher's J0 covers srv via direct AssignProcessToJobObject.
    //
    // Why not "defense-in-depth": the spec originally suggested
    // keeping host's J1 as a backstop, but on inspection it would
    // ACTIVELY defeat B.1's goal. KILL_ON_JOB_CLOSE on J1 fires
    // when the host process exits — taking srv with it. After B.1,
    // we want srv to survive host crashes (so the launcher can
    // restart the host without losing srv state). The two reapers
    // are mutually exclusive given that goal: launcher's J0 wants
    // srv to live; host's J1 wants srv to die. We picked J0.
    //
    // The `task dev` fallback path (host runs without launcher)
    // still loses kernel-level reaping for srv on host crash.
    // That's an accepted regression for dev-mode-only — production
    // (portable / installed) is unaffected. Phase 7 may add a
    // separate Job Object for the dev path.)

    // 9. Parse stderr for ESTART (30s timeout)
    let stderr = child.stderr.take().expect("Failed to get stderr");

    // Take ownership of stdout for logging
    let stdout = child.stdout.take();

    // Store the child handle
    *state.sidecar_child.lock() = Some(child);

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
                    if l.starts_with("AGENTMUXSRV-ESTART") {
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
                    } else if let Some(event_data) = l.strip_prefix("AGENTMUXSRV-EVENT:") {
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
        let pid = state_for_monitor.backend_pid.lock().unwrap_or(0);
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
///
/// The CEF host lives in `runtime/` alongside the backend binary in portable builds,
/// so `exe_dir` IS the runtime directory. Search order:
///   1. Same dir as CEF host: {name}-{version}-{os}.{arch}.exe (versioned portable)
///   2. Same dir as CEF host: {name}.exe (dev mode — cargo build output)
///   3. Workspace dist/bin/: {name}-{version}-{os}.{arch}.exe
///   4. Workspace dist/bin/: {name}.exe (plain fallback)
fn resolve_backend_binary(
    backend_name: &str,
    exe_suffix: &str,
) -> Result<std::path::PathBuf, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe: {}", e))?;
    let exe_dir = exe_path.parent().unwrap();
    let version = env!("CARGO_PKG_VERSION");

    tracing::info!("resolve_backend_binary: exe_dir={:?}, version={}", exe_dir, version);

    let (os_name, arch) = if cfg!(target_os = "macos") {
        ("darwin", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    } else if cfg!(target_os = "linux") {
        ("linux", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    } else {
        ("windows", if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" })
    };

    // 1. Versioned binary in same directory as CEF host (portable layout)
    //    e.g. runtime/agentmux-srv-0.33.37-windows.x64.exe
    let versioned = exe_dir.join(format!(
        "{}-{}-{}.{}{}", backend_name, version, os_name, arch, exe_suffix
    ));
    if versioned.exists() {
        tracing::info!("Using versioned {} at: {:?}", backend_name, versioned);
        return Ok(versioned);
    }

    // 2. Plain binary in same directory (dev mode — cargo build output)
    //    e.g. target/release/agentmux-srv.exe
    let plain = exe_dir.join(format!("{}{}", backend_name, exe_suffix));
    if plain.exists() {
        tracing::info!("Using dev-mode {} at: {:?}", backend_name, plain);
        return Ok(plain);
    }

    // 3. Workspace dist/bin/ (for `task dev` / `task cef:run`)
    let dist_bin = exe_dir.parent()
        .and_then(|p| p.parent())
        .map(|ws| ws.join("dist").join("bin"));

    if let Some(ref dist_bin) = dist_bin {
        let dist_versioned = dist_bin.join(format!(
            "{}-{}-{}.{}{}", backend_name, version, os_name, arch, exe_suffix
        ));
        if dist_versioned.exists() {
            tracing::info!("Using dist {} at: {:?}", backend_name, dist_versioned);
            return Ok(dist_versioned);
        }

        let dist_plain = dist_bin.join(format!("{}{}", backend_name, exe_suffix));
        if dist_plain.exists() {
            tracing::info!("Using dist {} at: {:?}", backend_name, dist_plain);
            return Ok(dist_plain);
        }
    }

    // Diagnostic: list exe_dir contents
    let dir_listing = std::fs::read_dir(exe_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.contains("agentmux") || n.contains("srv"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|_| "unreadable".to_string());

    let dist_info = dist_bin
        .map(|d| format!("dist/bin: {:?}", d))
        .unwrap_or_else(|| "dist/bin: N/A (no workspace root)".to_string());

    Err(format!(
        "Backend binary '{}' not found (version {}).\n\
         exe_dir: {:?}\n\
         Searched:\n\
         \x20 1. {:?} (versioned, same dir)\n\
         \x20 2. {:?} (plain, dev mode)\n\
         \x20 3. {}\n\
         Relevant files in exe_dir: [{}]",
        backend_name, version, exe_dir, versioned, plain, dist_info, dir_listing
    ))
}

/// Parse the key=value fields out of a `AGENTMUXSRV-ESTART` line.
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

// Phase B.1: removed `create_job_object_for_child`. Host no longer
// owns a Job Object; launcher's J0 (in agentmux-launcher/src/main.rs)
// covers srv via direct AssignProcessToJobObject. The same windows-sys
// FFI pattern lives in the launcher now.
