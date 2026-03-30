// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Platform info commands for the CEF host.
// Ported from src-tauri/src/commands/platform.rs without Tauri dependencies.

use std::io::Read;
use std::sync::Arc;

use crate::state::AppState;

const SETTINGS_TEMPLATE: &str = include_str!("../../../settings-template.jsonc");

/// Get the current OS platform name.
pub fn get_platform() -> serde_json::Value {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    serde_json::json!(platform)
}

/// Get the current user's username.
pub fn get_user_name() -> serde_json::Value {
    serde_json::json!(whoami::username())
}

/// Get the system hostname.
pub fn get_host_name() -> serde_json::Value {
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    serde_json::json!(hostname)
}

/// Check if running in development mode.
pub fn get_is_dev() -> serde_json::Value {
    serde_json::json!(cfg!(debug_assertions))
}

/// Get the app data directory path (version-specific).
pub fn get_data_dir(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let dir = state.version_data_dir.lock().unwrap();
    match dir.as_ref() {
        Some(d) => Ok(serde_json::json!(d)),
        None => Err("Data dir not initialized yet".to_string()),
    }
}

/// Get the app config directory path (version-specific).
pub fn get_config_dir(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let dir = state.version_config_dir.lock().unwrap();
    match dir.as_ref() {
        Some(d) => Ok(serde_json::json!(d)),
        None => Err("Config dir not initialized yet".to_string()),
    }
}

/// Ensure a provider auth directory exists and return its absolute path.
/// Auth dirs are version-isolated under the version-specific config dir.
pub fn ensure_auth_dir(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let provider_id = args
        .get("provider_id")
        .or_else(|| args.get("providerId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider_id".to_string())?;

    // Reject path traversal attempts in provider_id
    if provider_id.contains('/')
        || provider_id.contains('\\')
        || provider_id.contains("..")
        || provider_id.is_empty()
    {
        return Err(format!(
            "Invalid provider_id '{}': must not contain path separators or '..'",
            provider_id
        ));
    }

    let config_dir = state.version_config_dir.lock().unwrap();
    let config_dir = config_dir
        .as_ref()
        .ok_or_else(|| "Config dir not initialized yet".to_string())?;

    let auth_dir = std::path::PathBuf::from(config_dir)
        .join("auth")
        .join(provider_id);
    std::fs::create_dir_all(&auth_dir)
        .map_err(|e| format!("Failed to create auth dir for {}: {}", provider_id, e))?;

    Ok(serde_json::json!(auth_dir.to_string_lossy()))
}

/// Get an environment variable value.
pub fn get_env(args: &serde_json::Value) -> serde_json::Value {
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match std::env::var(key) {
        Ok(val) => serde_json::json!(val),
        Err(_) => serde_json::Value::Null,
    }
}

/// Get details for the About modal.
pub fn get_about_modal_details(state: &Arc<AppState>) -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION");
    let endpoints = state.backend_endpoints.lock().unwrap();

    serde_json::json!({
        "version": version,
        "buildTime": version,
        "platform": match std::env::consts::OS {
            "macos" => "darwin",
            "windows" => "win32",
            other => other,
        },
        "arch": std::env::consts::ARCH,
        "backendEndpoints": {
            "ws": endpoints.ws_endpoint,
            "web": endpoints.web_endpoint,
        }
    })
}

/// Get comprehensive host info for the hostname popover.
pub fn get_host_info(state: &Arc<AppState>) -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION");
    let endpoints = state.backend_endpoints.lock().unwrap();
    let ipc_port = *state.ipc_port.lock().unwrap();
    let data_dir = state.version_data_dir.lock().unwrap().clone().unwrap_or_default();
    let pid = std::process::id();

    // Resolve primary local IP
    let local_ip = local_ip_address().unwrap_or_else(|| "127.0.0.1".to_string());

    let os_info = format!("{} {}",
        match std::env::consts::OS {
            "windows" => "Windows",
            "macos" => "macOS",
            "linux" => "Linux",
            other => other,
        },
        std::env::consts::ARCH
    );

    serde_json::json!({
        "hostname": whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()),
        "os": os_info,
        "localIp": local_ip,
        "instanceId": format!("v{}", version),
        "version": version,
        "dataDir": data_dir,
        "hostType": "CEF 146",
        "pid": pid,
        "ports": {
            "ipc": format!("127.0.0.1:{}", ipc_port),
            "web": endpoints.web_endpoint,
            "ws": endpoints.ws_endpoint,
            "devtools": "127.0.0.1:9222",
        }
    })
}

/// Get the primary non-loopback IPv4 address.
fn local_ip_address() -> Option<String> {
    // Connect a UDP socket to an external address to determine the local IP
    // (doesn't actually send data — just resolves the route)
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

/// Get the documentation site URL.
pub fn get_docsite_url(state: &Arc<AppState>) -> serde_json::Value {
    let endpoints = state.backend_endpoints.lock().unwrap();
    if !endpoints.web_endpoint.is_empty() {
        serde_json::json!(format!("http://{}/docsite/", endpoints.web_endpoint))
    } else {
        serde_json::json!("https://docs.agentmux.ai")
    }
}

/// Open a file in the best available code editor.
pub fn open_in_editor(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing path".to_string())?;

    #[cfg(target_os = "windows")]
    {
        // Use explorer.exe directly instead of cmd /C start to avoid shell injection.
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
        return Ok(serde_json::Value::Null);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let cli_editors = ["code", "cursor", "zed", "subl", "atom"];
        for editor in &cli_editors {
            if std::process::Command::new(editor).arg(path).spawn().is_ok() {
                return Ok(serde_json::Value::Null);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }

    #[allow(unreachable_code)]
    Ok(serde_json::Value::Null)
}

/// Ensure settings.json exists in the config directory with the latest template.
pub fn ensure_settings_file(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let config_dir_str = state
        .version_config_dir
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Config dir not initialized yet".to_string())?;
    let config_dir = std::path::PathBuf::from(&config_dir_str);

    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    let settings_path = config_dir.join("settings.json");

    // Read existing user values (strips JSONC comments, parses JSON)
    let existing = read_settings_jsonc(&settings_path);

    // Merge user values into fresh template
    let merged = merge_into_template(SETTINGS_TEMPLATE, &existing);
    std::fs::write(&settings_path, &merged)
        .map_err(|e| format!("Failed to write settings.json: {}", e))?;

    Ok(serde_json::json!(settings_path.to_string_lossy()))
}

/// Spawn a CLI auth login flow.
pub async fn run_cli_login(
    state: Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let cli_path = args
        .get("cli_path")
        .or_else(|| args.get("cliPath"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing cli_path".to_string())?
        .to_string();

    let login_args: Vec<String> = args
        .get("login_args")
        .or_else(|| args.get("loginArgs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let auth_env: std::collections::HashMap<String, String> = args
        .get("auth_env")
        .or_else(|| args.get("authEnv"))
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let mut cmd = tokio::process::Command::new(&cli_path);
    cmd.args(&login_args)
        .envs(&auth_env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn {cli_path}: {e}"))?;

    tracing::info!(cli = %cli_path, "run_cli_login: spawned, browser should open");

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut stored = state.cli_login_cancel.lock().unwrap();
        *stored = Some(cancel_tx);
    }

    tokio::spawn(async move {
        tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) => tracing::info!(
                        exit_code = ?status.code(),
                        "run_cli_login: child exited"
                    ),
                    Err(e) => tracing::warn!(
                        error = %e,
                        "run_cli_login: child wait error"
                    ),
                }
            }
            _ = cancel_rx => {
                tracing::info!("run_cli_login: cancel signal received, killing child");
                let _ = child.kill().await;
            }
        }
    });

    Ok(serde_json::Value::Null)
}

/// Kill the in-progress CLI login process.
pub fn cancel_cli_login(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let sender = {
        let mut stored = state.cli_login_cancel.lock().unwrap();
        stored.take()
    };
    if let Some(tx) = sender {
        let _ = tx.send(());
        tracing::info!("cancel_cli_login: cancel signal sent");
    }
    Ok(serde_json::Value::Null)
}

// --- Settings helpers (ported from src-tauri/src/commands/platform.rs) ---

fn read_settings_jsonc(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    if !path.exists() {
        return serde_json::Map::new();
    }
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let stripped = json_comments::StripComments::new(content.as_bytes());
            let mut json_bytes = Vec::new();
            std::io::BufReader::new(stripped)
                .read_to_end(&mut json_bytes)
                .unwrap_or_default();
            let json_str = strip_trailing_commas(&String::from_utf8_lossy(&json_bytes));
            match serde_json::from_str::<serde_json::Value>(&json_str) {
                Ok(serde_json::Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            }
        }
        Err(_) => serde_json::Map::new(),
    }
}

fn strip_trailing_commas(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_string = false;
    let mut last_comma_pos: Option<usize> = None;

    for ch in input.chars() {
        if in_string {
            result.push(ch);
            if ch == '"' {
                let backslashes = result[..result.len() - 1]
                    .chars()
                    .rev()
                    .take_while(|&c| c == '\\')
                    .count();
                if backslashes % 2 == 0 {
                    in_string = false;
                }
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                last_comma_pos = None;
                result.push(ch);
            }
            ',' => {
                last_comma_pos = Some(result.len());
                result.push(ch);
            }
            '}' | ']' => {
                if let Some(pos) = last_comma_pos {
                    result.replace_range(pos..pos + 1, " ");
                }
                last_comma_pos = None;
                result.push(ch);
            }
            _ if ch.is_whitespace() => {
                result.push(ch);
            }
            _ => {
                last_comma_pos = None;
                result.push(ch);
            }
        }
    }
    result
}

fn merge_into_template(
    template: &str,
    user_settings: &serde_json::Map<String, serde_json::Value>,
) -> String {
    if user_settings.is_empty() {
        return template.to_string();
    }

    let mut remaining: std::collections::HashMap<&str, &serde_json::Value> =
        user_settings.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut lines: Vec<String> = Vec::new();

    for line in template.lines() {
        if let Some(key) = extract_commented_setting_key(line) {
            if let Some(value) = remaining.remove(key) {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                let val_str = serde_json::to_string(value).unwrap_or_default();
                lines.push(format!("{}\"{}\": {},", indent, key, val_str));
                continue;
            }
        }
        lines.push(line.to_string());
    }

    if !remaining.is_empty() {
        if let Some(brace_pos) = lines.iter().rposition(|l| l.trim() == "}") {
            let mut extra: Vec<String> = Vec::new();
            extra.push(String::new());
            extra.push("    // -- User Overrides --".to_string());
            let mut sorted_keys: Vec<&&str> = remaining.keys().collect();
            sorted_keys.sort();
            for key in sorted_keys {
                let value = remaining[*key];
                let val_str = serde_json::to_string(value).unwrap_or_default();
                extra.push(format!("    \"{}\": {},", key, val_str));
            }
            for (i, line) in extra.into_iter().enumerate() {
                lines.insert(brace_pos + i, line);
            }
        }
    }

    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Open a URL in the system's default browser.
pub fn open_external(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing url".to_string())?;

    // Only allow safe URL schemes
    if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("devtools://") {
        return Err(format!("Refusing to open URL with unsupported scheme: {}", url));
    }

    #[cfg(target_os = "windows")]
    {
        // Use explorer.exe instead of cmd /C start to avoid command injection
        // (cmd.exe interprets & and | in URLs as command separators)
        let _ = std::process::Command::new("explorer")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }

    Ok(serde_json::Value::Null)
}

fn extract_commented_setting_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("//")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}
