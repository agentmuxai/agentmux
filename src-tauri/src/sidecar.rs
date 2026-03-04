use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

/// State returned after successfully spawning the backend.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackendSpawnResult {
    pub ws_endpoint: String,
    pub web_endpoint: String,
    pub auth_key: String,
    pub instance_id: String,  // version-namespaced, e.g. "v0.31.23"
    pub version: String,      // Backend version (e.g., "0.27.12")
    pub is_reused: bool,      // true if reusing an existing backend (not spawned by this process)
}

/// Spawn the agentmuxsrv-rs Rust backend as a Tauri sidecar.
///
/// The backend prints a line to stderr when ready:
///   WAVESRV-ESTART ws:<addr> web:<addr> version:<ver> buildtime:<time>
///
/// We parse that line to get the WebSocket and HTTP endpoints,
/// then the frontend connects to them directly.
pub async fn spawn_backend(app: &tauri::AppHandle) -> Result<BackendSpawnResult, String> {
    tracing::info!("🚀 spawn_backend() called");

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

    // Check for existing backend with matching version (O(1) lookup by version)
    let current_version = env!("CARGO_PKG_VERSION");
    let version_instance_id = format!("v{}", current_version);

    {
        let instance_dir = config_dir.join("instances").join(&version_instance_id);
        let endpoints_file = instance_dir.join("wave-endpoints.json");

        if endpoints_file.exists() {
            tracing::info!("Checking for existing backend at: {}", endpoints_file.display());

            if let Ok(contents) = std::fs::read_to_string(&endpoints_file) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                    let backend_version = json["version"].as_str().unwrap_or("");

                    if backend_version == current_version {
                        let existing = BackendSpawnResult {
                            ws_endpoint: json["ws_endpoint"].as_str().unwrap_or_default().to_string(),
                            web_endpoint: json["web_endpoint"].as_str().unwrap_or_default().to_string(),
                            auth_key: json["auth_key"].as_str().unwrap_or_default().to_string(),
                            instance_id: json["instance_id"].as_str().unwrap_or_default().to_string(),
                            version: backend_version.to_string(),
                            is_reused: true,
                        };

                        // Test if the existing backend is still responsive
                        let test_url = if existing.web_endpoint.starts_with("http") {
                            existing.web_endpoint.clone()
                        } else {
                            format!("http://{}", existing.web_endpoint)
                        };

                        tracing::info!("Testing connection to existing backend at: {}", test_url);
                        match reqwest::get(&test_url).await {
                            Ok(resp) => {
                                if resp.status().is_success() || resp.status().is_client_error() {
                                    tracing::info!(
                                        "Reusing existing backend v{} (instance: {})",
                                        existing.version,
                                        existing.instance_id
                                    );

                                    // Reuse the auth key from the existing backend
                                    let state = app.state::<crate::state::AppState>();
                                    let mut auth_key_guard = state.auth_key.lock().unwrap();
                                    *auth_key_guard = existing.auth_key.clone();

                                    return Ok(existing);
                                } else {
                                    tracing::warn!("Backend returned unexpected status: {}", resp.status());
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to connect to existing backend: {}", e);
                            }
                        }

                        // Backend not responsive - remove stale file
                        tracing::warn!("Backend not responsive, removing stale endpoints file");
                        let _ = std::fs::remove_file(&endpoints_file);
                    } else {
                        tracing::warn!(
                            "Backend version mismatch in endpoints file: backend={}, frontend={}. Removing stale file.",
                            backend_version,
                            current_version
                        );
                        let _ = std::fs::remove_file(&endpoints_file);
                    }
                }
            }
        }
    }

    tracing::info!("No existing backend found, spawning new one");

    // Ensure base directories exist
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data dir: {}", e))?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    // Pre-create version-namespaced instance directory structure
    // Backend needs these to exist before it starts
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

    // Try to find backend in portable mode first (same dir as exe)
    let portable_path = std::env::current_exe().ok().and_then(|exe_path| {
        let exe_dir = exe_path.parent()?;
        let portable_binary = exe_dir.join(format!("{}.x64.exe", backend_name));
        if portable_binary.exists() {
            tracing::info!("Using portable {} at: {:?}", backend_name, portable_binary);
            Some(portable_binary)
        } else {
            None
        }
    });

    let sidecar_cmd = if let Some(portable_exe) = portable_path {
        // Portable mode: run executable from same directory
        shell.command(portable_exe)
    } else {
        // Installer mode: use bundled sidecar
        shell
            .sidecar(backend_name)
            .map_err(|e| format!("Failed to find {} sidecar: {}", backend_name, e))?
    };

    // Resolve AGENTMUX_APP_PATH and deploy wsh binary for the Go backend.
    // Tauri bundles wsh as a sidecar at Contents/MacOS/wsh (stripped platform suffix).
    // The Go backend expects: AGENTMUX_APP_PATH/bin/wsh-VERSION-OS.ARCH
    let app_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();

    // Deploy bundled wsh to bin/ with the name the Go backend expects
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
                    // Ensure executable permission on Unix
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
        .env("AGENTMUX_APP_PATH", &app_path_str)
        .env("AGENTMUX_DEV", if cfg!(debug_assertions) { "1" } else { "" })
        .env("WCLOUD_ENDPOINT", "https://api.agentmux.ai/central")
        .env("WCLOUD_WS_ENDPOINT", "wss://wsapi.agentmux.ai/")
        .spawn()
        .map_err(|e| format!("Failed to spawn agentmuxsrv-rs: {}", e))?;

    // Store child handle for graceful shutdown
    {
        let state = app.state::<crate::state::AppState>();
        let mut sidecar = state.sidecar_child.lock().unwrap();
        *sidecar = Some(child);
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
                    // Emit quit event to frontend
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
        is_reused: false,
    };

    let key_preview = result.auth_key.chars().take(8).collect::<String>();
    tracing::info!("Backend successfully started with endpoints: ws={}, web={}, version={}, instance={}, auth_key={}...",
        result.ws_endpoint, result.web_endpoint, result.version, result.instance_id, key_preview);

    // Compute nested instance directory inside base config dir
    let instance_dir = config_dir.join("instances").join(&result.instance_id);

    // Ensure instance directory exists
    if let Err(e) = std::fs::create_dir_all(&instance_dir) {
        tracing::error!("Failed to create instance config dir: {}", e);
        return Err(format!("Failed to create instance config dir: {}", e));
    }

    let endpoints_file = instance_dir.join("wave-endpoints.json");
    tracing::info!("Saving endpoints to: {}", endpoints_file.display());

    // Save endpoints with additional metadata
    let endpoints_json = serde_json::json!({
        "version": result.version,
        "ws_endpoint": result.ws_endpoint,
        "web_endpoint": result.web_endpoint,
        "auth_key": result.auth_key,
        "instance_id": result.instance_id,
        "pid": std::process::id(),
        "started_at": chrono::Utc::now().to_rfc3339(),
    });

    match serde_json::to_string_pretty(&endpoints_json) {
        Ok(json) => {
            tracing::info!("Serialized endpoints: {}", json);
            match std::fs::write(&endpoints_file, &json) {
                Ok(_) => {
                    tracing::info!("✅ Successfully saved endpoints to {} for multi-window reuse", endpoints_file.display());
                    // Verify the file was written
                    if endpoints_file.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&endpoints_file) {
                            tracing::info!("Verified file contents: {}", contents);
                        }
                    } else {
                        tracing::error!("❌ File doesn't exist after write!");
                    }
                }
                Err(e) => {
                    tracing::error!("❌ Failed to write endpoints file: {}", e);
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to serialize endpoints: {}", e);
        }
    }

    Ok(result)
}

/// Handle backend event messages from agentmuxsrv-rs.
/// These are forwarded to the frontend via Tauri events.
fn handle_backend_event(app: &tauri::AppHandle, event_data: &str) {
    tracing::debug!("Backend event: {}", event_data);

    // Forward raw event to frontend
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("agentmuxsrv-event", event_data.to_string());
    }
}
