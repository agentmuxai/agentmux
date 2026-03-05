// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Provider management commands: CLI detection, config CRUD, auth storage.
//! Stores config in app_config_dir()/provider-config.json via tauri-plugin-store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri_plugin_store::StoreExt;

// ---- Types ----

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CliDetectionResult {
    pub provider: String,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderConfig {
    pub default_provider: String,
    pub providers: HashMap<String, ProviderSettings>,
    pub setup_complete: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderSettings {
    pub cli_path: Option<String>,
    pub auth_token: Option<String>,
    pub auth_status: String, // "none" | "authenticated" | "expired"
    pub output_format: String,
    pub extra_args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderInstallInfo {
    pub provider: String,
    pub install_command: String,
    pub docs_url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub status: String, // "none" | "authenticated" | "expired"
    pub error: Option<String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            default_provider: String::new(),
            providers: HashMap::new(),
            setup_complete: false,
        }
    }
}

// ---- Store helpers ----

const STORE_FILENAME: &str = "provider-config.json";
const STORE_KEY: &str = "provider_config";

fn load_config(app: &tauri::AppHandle) -> Result<ProviderConfig, String> {
    let store = app
        .store(STORE_FILENAME)
        .map_err(|e| format!("Failed to open store: {e}"))?;
    match store.get(STORE_KEY) {
        Some(val) => serde_json::from_value(val.clone())
            .map_err(|e| format!("Failed to deserialize provider config: {e}")),
        None => Ok(ProviderConfig::default()),
    }
}

fn save_config_to_store(app: &tauri::AppHandle, config: &ProviderConfig) -> Result<(), String> {
    let store = app
        .store(STORE_FILENAME)
        .map_err(|e| format!("Failed to open store: {e}"))?;
    let val = serde_json::to_value(config)
        .map_err(|e| format!("Failed to serialize provider config: {e}"))?;
    store.set(STORE_KEY, val);
    store.save().map_err(|e| format!("Failed to save store: {e}"))?;
    Ok(())
}

// ---- CLI detection helpers ----

/// Run `where` (Windows) or `which` (Unix) to find a CLI binary.
fn detect_cli(name: &str) -> CliDetectionResult {
    let find_cmd = if cfg!(windows) { "where" } else { "which" };

    let path = std::process::Command::new(find_cmd)
        .arg(name)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // `where` on Windows may return multiple lines; take the first.
                stdout.lines().next().map(|s| s.trim().to_string())
            } else {
                None
            }
        });

    let version = if path.is_some() {
        std::process::Command::new(name)
            .arg("--version")
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    Some(stdout.lines().next().unwrap_or("").trim().to_string())
                } else {
                    None
                }
            })
    } else {
        None
    };

    CliDetectionResult {
        provider: name.to_string(),
        installed: path.is_some(),
        path,
        version,
    }
}

// ---- Tauri commands ----

/// Detect installed CLI tools (claude, gemini, codex).
#[tauri::command]
pub async fn detect_installed_clis() -> Result<Vec<CliDetectionResult>, String> {
    // Run detection in a blocking thread to avoid blocking the async runtime
    let results = tokio::task::spawn_blocking(|| {
        vec![
            detect_cli("claude"),
            detect_cli("gemini"),
            detect_cli("codex"),
        ]
    })
    .await
    .map_err(|e| format!("Detection task failed: {e}"))?;

    tracing::info!(
        "CLI detection: {}",
        results
            .iter()
            .map(|r| format!("{}={}", r.provider, r.installed))
            .collect::<Vec<_>>()
            .join(", ")
    );

    Ok(results)
}

/// Get the persisted provider configuration.
#[tauri::command]
pub async fn get_provider_config(app: tauri::AppHandle) -> Result<ProviderConfig, String> {
    load_config(&app)
}

/// Save the provider configuration.
#[tauri::command]
pub async fn save_provider_config(
    app: tauri::AppHandle,
    config: ProviderConfig,
) -> Result<(), String> {
    tracing::info!(
        "Saving provider config: default={}, setup_complete={}",
        config.default_provider,
        config.setup_complete
    );
    save_config_to_store(&app, &config)
}

/// Get install info for a provider.
#[tauri::command]
pub async fn get_provider_install_info(provider: String) -> Result<ProviderInstallInfo, String> {
    match provider.as_str() {
        "claude" => Ok(ProviderInstallInfo {
            provider: "claude".to_string(),
            install_command: "npm install -g @anthropic-ai/claude-code".to_string(),
            docs_url: "https://docs.anthropic.com/claude-code".to_string(),
        }),
        "gemini" => Ok(ProviderInstallInfo {
            provider: "gemini".to_string(),
            install_command: "npm install -g @anthropic-ai/gemini-cli".to_string(),
            docs_url: "https://ai.google.dev/gemini-cli".to_string(),
        }),
        "codex" => Ok(ProviderInstallInfo {
            provider: "codex".to_string(),
            install_command: "npm install -g @openai/codex".to_string(),
            docs_url: "https://platform.openai.com/docs/codex".to_string(),
        }),
        _ => Err(format!("Unknown provider: {provider}")),
    }
}

/// Store an auth token for a provider.
#[tauri::command]
pub async fn set_provider_auth(
    app: tauri::AppHandle,
    provider: String,
    token: String,
) -> Result<(), String> {
    tracing::info!("Setting auth token for provider: {}", provider);
    let mut config = load_config(&app)?;

    let settings = config
        .providers
        .entry(provider.clone())
        .or_insert_with(|| ProviderSettings {
            cli_path: None,
            auth_token: None,
            auth_status: "none".to_string(),
            output_format: String::new(),
            extra_args: vec![],
        });

    settings.auth_token = Some(token);
    settings.auth_status = "authenticated".to_string();

    save_config_to_store(&app, &config)
}

/// Clear auth token for a provider.
#[tauri::command]
pub async fn clear_provider_auth(
    app: tauri::AppHandle,
    provider: String,
) -> Result<(), String> {
    tracing::info!("Clearing auth token for provider: {}", provider);
    let mut config = load_config(&app)?;

    if let Some(settings) = config.providers.get_mut(&provider) {
        settings.auth_token = None;
        settings.auth_status = "none".to_string();
    }

    save_config_to_store(&app, &config)
}

/// Get auth status for a provider.
#[tauri::command]
pub async fn get_provider_auth_status(
    app: tauri::AppHandle,
    provider: String,
) -> Result<ProviderAuthStatus, String> {
    let config = load_config(&app)?;

    let status = config
        .providers
        .get(&provider)
        .map(|s| s.auth_status.clone())
        .unwrap_or_else(|| "none".to_string());

    Ok(ProviderAuthStatus {
        provider,
        status,
        error: None,
    })
}

// ---- CLI auth status (runs the CLI's own auth check) ----

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CliAuthStatus {
    pub logged_in: bool,
    pub auth_method: Option<String>,
    pub api_provider: Option<String>,
    pub email: Option<String>,
    pub subscription_type: Option<String>,
}

/// Check CLI authentication status by running the provider's auth check command.
///
/// Accepts an optional `cli_path` for locally-installed CLIs (from ~/.agentmux/cli/).
/// Falls back to the provider name on system PATH if no cli_path is given.
///
/// Provider-specific parsing:
/// - Claude: `<cli> auth status --json` → parse JSON (camelCase fields)
/// - Codex: `<cli> login status` → check exit code
/// - Gemini: `<cli> auth status` → check exit code
#[tauri::command]
pub async fn check_cli_auth_status(
    provider: String,
    cli_path: Option<String>,
) -> Result<CliAuthStatus, String> {
    let cli_cmd = cli_path.unwrap_or_else(|| {
        match provider.as_str() {
            "claude" => "claude".to_string(),
            "gemini" => "gemini".to_string(),
            "codex" => "codex".to_string(),
            _ => provider.clone(),
        }
    });

    let provider_clone = provider.clone();
    let result = tokio::task::spawn_blocking(move || {
        match provider_clone.as_str() {
            "claude" => check_claude_auth(&cli_cmd),
            "codex" => check_codex_auth(&cli_cmd),
            "gemini" => check_gemini_auth(&cli_cmd),
            _ => Err(format!("Unknown provider: {provider_clone}")),
        }
    })
    .await
    .map_err(|e| format!("Auth check task failed: {e}"))?;

    result
}

/// Claude: `<cli> auth status --json` → parse JSON
fn check_claude_auth(cli_cmd: &str) -> Result<CliAuthStatus, String> {
    let output = std::process::Command::new(cli_cmd)
        .args(["auth", "status", "--json"])
        .output()
        .map_err(|e| format!("Failed to run `{cli_cmd} auth status`: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();

    if trimmed.is_empty() {
        return Ok(CliAuthStatus {
            logged_in: false,
            auth_method: None,
            api_provider: None,
            email: None,
            subscription_type: None,
        });
    }

    let json: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| format!("Failed to parse auth status JSON: {e}"))?;

    Ok(CliAuthStatus {
        logged_in: json.get("loggedIn").and_then(|v| v.as_bool()).unwrap_or(false),
        auth_method: json.get("authMethod").and_then(|v| v.as_str()).map(|s| s.to_string()),
        api_provider: json.get("apiProvider").and_then(|v| v.as_str()).map(|s| s.to_string()),
        email: json.get("email").and_then(|v| v.as_str()).map(|s| s.to_string()),
        subscription_type: json.get("subscriptionType").and_then(|v| v.as_str()).map(|s| s.to_string()),
    })
}

/// Codex: `<cli> login status` → exit code 0 means logged in
fn check_codex_auth(cli_cmd: &str) -> Result<CliAuthStatus, String> {
    let output = std::process::Command::new(cli_cmd)
        .args(["login", "status"])
        .output()
        .map_err(|e| format!("Failed to run `{cli_cmd} login status`: {e}"))?;

    Ok(CliAuthStatus {
        logged_in: output.status.success(),
        auth_method: if output.status.success() { Some("oauth".to_string()) } else { None },
        api_provider: None,
        email: None,
        subscription_type: None,
    })
}

/// Gemini: `<cli> auth status` → exit code 0 means logged in
fn check_gemini_auth(cli_cmd: &str) -> Result<CliAuthStatus, String> {
    let output = std::process::Command::new(cli_cmd)
        .args(["auth", "status"])
        .output()
        .map_err(|e| format!("Failed to run `{cli_cmd} auth status`: {e}"))?;

    Ok(CliAuthStatus {
        logged_in: output.status.success(),
        auth_method: if output.status.success() { Some("oauth".to_string()) } else { None },
        api_provider: None,
        email: None,
        subscription_type: None,
    })
}
