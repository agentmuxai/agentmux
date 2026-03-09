use tauri::Manager;

use crate::state::AppState;

const DEFAULT_SETTINGS_TEMPLATE: &str = r#"// AgentMux Settings
// Save this file to apply changes immediately.
// Uncomment a line to override its default value.
//
// Docs: https://docs.agentmux.ai/settings
{
    // -- Terminal --
    // "term:fontsize":            12,
    // "term:fontfamily":          "JetBrains Mono",
    // "term:theme":               "default-dark",
    // "term:scrollback":          1000,
    // "term:copyonselect":        true,
    // "term:transparency":        0.5,
    // "term:localshellpath":      "/bin/bash",
    // "term:localshellopts":      [],
    // "term:disablewebgl":        false,
    // "term:allowbracketedpaste": true,
    // "term:shiftenternewline":   false,

    // -- AI --
    // "ai:preset":     "",
    // "ai:apitype":    "anthropic",
    // "ai:baseurl":    "",
    // "ai:apitoken":   "",
    // "ai:model":      "claude-sonnet-4-6",
    // "ai:maxtokens":  4096,
    // "ai:timeoutms":  60000,
    // "ai:fontsize":   14,
    // "ai:fixedfontsize": 14,

    // -- Editor --
    // "editor:fontsize":          14,
    // "editor:minimapenabled":    false,
    // "editor:stickyscrollenabled": false,
    // "editor:wordwrap":          true,

    // -- Window --
    // "window:transparent":       false,
    // "window:blur":              false,
    // "window:opacity":           1.0,
    // "window:bgcolor":           "",
    // "window:zoom":              1.0,
    // "window:tilegapsize":       3,
    // "window:showmenubar":       false,
    // "window:nativetitlebar":    false,
    // "window:confirmclose":      false,
    // "window:savelastwindow":    true,
    // "window:dimensions":        "",
    // "window:reducedmotion":     false,
    // "window:magnifiedblockopacity": 0.6,
    // "window:magnifiedblocksize":    0.9,
    // "window:maxtabcachesize":   10,
    // "window:disablehardwareacceleration": false,

    // -- App --
    // "app:globalhotkey":         "",
    // "app:defaultnewblock":      "",
    // "app:showoverlayblocknums": false,

    // -- Shell Environment --
    // "cmd:env":                  {},

    // -- Auto Update --
    // "autoupdate:enabled":       true,
    // "autoupdate:installonquit": true,
    // "autoupdate:channel":       "latest",

    // -- Telemetry --
    // "telemetry:enabled":        true,
    // "telemetry:interval":       1.0,

    // -- Connections --
    // "conn:wshenabled":          true,
    // "conn:askbeforewshinstall": true,

    // -- Other --
    // "widget:showhelp":          true,
    // "widget:icononly":           false,
    // "blockheader:showblockids": false,
    // "markdown:fontsize":        14,
    // "preview:showhiddenfiles":  false,
    // "tab:preset":               ""
}
"#;

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

/// Ensure settings.json exists in the config directory, creating the directory
/// and a default file if needed. Returns the absolute path to settings.json.
#[tauri::command]
pub fn ensure_settings_file(app: tauri::AppHandle) -> Result<String, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Failed to get config dir: {}", e))?;

    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    let settings_path = config_dir.join("settings.json");
    if !settings_path.exists() {
        std::fs::write(&settings_path, DEFAULT_SETTINGS_TEMPLATE)
            .map_err(|e| format!("Failed to create settings.json: {}", e))?;
    }

    Ok(settings_path.to_string_lossy().to_string())
}

/// Open a file in the best available code editor.
/// Priority: known CLI editors on PATH → macOS .app bundles → OS default.
#[tauri::command]
pub fn open_in_editor(path: String) -> Result<(), String> {
    // 1. CLI editors on PATH
    let cli_editors = ["code", "cursor", "zed", "subl", "atom"];
    for editor in &cli_editors {
        if std::process::Command::new(editor).arg(&path).spawn().is_ok() {
            return Ok(());
        }
    }

    // 2. macOS .app bundles (handles editors not on PATH)
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

    // 3. OS default fallback
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(&path).spawn().map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd").args(["/C", "start", "", &path]).spawn().map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;

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
