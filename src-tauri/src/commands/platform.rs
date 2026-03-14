use std::io::Read;

use tauri::Manager;

use crate::state::AppState;

const SETTINGS_TEMPLATE: &str = include_str!("../../../settings-template.jsonc");

/// Get the current OS platform name.
/// Replaces: ipcMain.on("get-platform") in emain/platform.ts
#[tauri::command]
pub fn get_platform() -> String {
    match std::env::consts::OS {
        "macos" => "darwin".to_string(),
        "windows" => "win32".to_string(),
        other => other.to_string(),
    }
}

/// Get the current user's username.
/// Replaces: ipcMain.on("get-user-name") in emain/platform.ts
#[tauri::command]
pub fn get_user_name() -> String {
    whoami::username()
}

/// Get the system hostname.
/// Replaces: ipcMain.on("get-host-name") in emain/platform.ts
#[tauri::command]
pub fn get_host_name() -> String {
    whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string())
}

/// Check if running in development mode.
/// Replaces: ipcMain.on("get-is-dev") in emain/platform.ts
#[tauri::command]
pub fn get_is_dev() -> bool {
    cfg!(debug_assertions)
}

/// Get the app data directory path.
/// Replaces: ipcMain.on("get-data-dir") in emain/platform.ts
#[tauri::command]
pub fn get_data_dir(app: tauri::AppHandle) -> Result<String, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to get data dir: {}", e))
}

/// Get the app config directory path.
/// Replaces: ipcMain.on("get-config-dir") in emain/platform.ts
#[tauri::command]
pub fn get_config_dir(app: tauri::AppHandle) -> Result<String, String> {
    app.path()
        .app_config_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to get config dir: {}", e))
}

/// Ensure settings.json exists in the config directory with the latest template.
/// Reads any existing user settings and merges them into the fresh template,
/// so the file always has the full commented reference plus user overrides.
/// Returns the absolute path to settings.json.
#[tauri::command]
pub fn ensure_settings_file(app: tauri::AppHandle) -> Result<String, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Failed to get config dir: {}", e))?;

    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    let settings_path = config_dir.join("settings.json");

    // Read existing user values (strips JSONC comments, parses JSON)
    let existing = read_settings_jsonc(&settings_path);

    // Merge user values into fresh template
    let merged = merge_into_template(SETTINGS_TEMPLATE, &existing);
    std::fs::write(&settings_path, &merged)
        .map_err(|e| format!("Failed to write settings.json: {}", e))?;

    Ok(settings_path.to_string_lossy().to_string())
}

/// Read a JSONC settings file, stripping comments and trailing commas.
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

/// Remove trailing commas before `}` or `]` in JSON text.
fn strip_trailing_commas(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_string = false;
    let mut last_comma_pos: Option<usize> = None;

    for ch in input.chars() {
        if in_string {
            result.push(ch);
            if ch == '"' {
                // Check if this quote is escaped (count preceding backslashes)
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

/// Merge user settings into a JSONC template string.
/// Commented template lines matching user keys get uncommented with the user value.
/// Unknown keys are appended in a "User Overrides" section before the closing `}`.
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

/// Extract the settings key from a commented-out template line.
/// Matches lines like: `    // "some:key":   value,`
fn extract_commented_setting_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("//")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Open a file in the best available code editor.
/// On macOS/Linux: probe CLI editors on PATH, then .app bundles, then OS default.
/// On Windows: use OS default directly (avoids cmd shell flash).
#[tauri::command]
pub fn open_in_editor(path: String) -> Result<(), String> {
    // Windows: use OS default directly via shell execute (no visible cmd window)
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
        return Ok(());
    }

    // macOS/Linux: try CLI editors on PATH first
    #[cfg(not(target_os = "windows"))]
    {
        let cli_editors = ["code", "cursor", "zed", "subl", "atom"];
        for editor in &cli_editors {
            if std::process::Command::new(editor).arg(&path).spawn().is_ok() {
                return Ok(());
            }
        }
    }

    // macOS: try .app bundles (handles editors not on PATH)
    #[cfg(target_os = "macos")]
    {
        let app_bins = [
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
            "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
            "/Applications/Zed.app/Contents/MacOS/zed",
            "/Applications/Sublime Text.app/Contents/SharedSupport/bin/subl",
        ];
        for bin in &app_bins {
            if std::path::Path::new(bin).exists() {
                if std::process::Command::new(bin).arg(&path).spawn().is_ok() {
                    return Ok(());
                }
            }
        }
    }

    // OS default fallback (macOS/Linux only — Windows already returned above)
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&path).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Get an environment variable value.
/// Replaces: ipcMain.on("get-env") in emain/emain.ts
#[tauri::command]
pub fn get_env(key: String) -> Option<String> {
    std::env::var(&key).ok()
}

/// Get details for the About modal.
/// Replaces: ipcMain.on("get-about-modal-details") in emain/emain.ts
#[tauri::command]
pub fn get_about_modal_details(app: tauri::AppHandle) -> serde_json::Value {
    let version = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    // Get backend version from state if available
    let state = app.state::<AppState>();
    let endpoints = state.backend_endpoints.lock().unwrap();

    serde_json::json!({
        "version": version,
        "buildTime": env!("CARGO_PKG_VERSION"),
        "platform": get_platform(),
        "arch": std::env::consts::ARCH,
        "backendEndpoints": {
            "ws": endpoints.ws_endpoint,
            "web": endpoints.web_endpoint,
        }
    })
}

/// Get the documentation site URL.
/// Replaces: ipcMain.on("get-docsite-url") in emain/docsite.ts
#[tauri::command]
pub fn get_docsite_url(app: tauri::AppHandle) -> String {
    let state = app.state::<AppState>();
    let endpoints = state.backend_endpoints.lock().unwrap();

    if !endpoints.web_endpoint.is_empty() {
        // Try embedded docsite first
        format!("http://{}/docsite/", endpoints.web_endpoint)
    } else {
        // Fallback to public docs
        "https://docs.agentmux.ai".to_string()
    }
}
