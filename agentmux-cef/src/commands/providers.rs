// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Provider management commands for the CEF host.
// Ported from src-tauri/src/commands/providers.rs and cli_installer.rs.
//
// Uses JSON file storage instead of tauri-plugin-store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

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
    pub auth_status: String,
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
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CliAuthStatus {
    pub logged_in: bool,
    pub auth_method: Option<String>,
    pub api_provider: Option<String>,
    pub email: Option<String>,
    pub subscription_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CliInstallResult {
    pub provider: String,
    pub cli_path: String,
    pub version: String,
    pub already_installed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodejsStatus {
    pub available: bool,
    pub version: Option<String>,
    pub npm_available: bool,
    pub npm_version: Option<String>,
    pub path: Option<String>,
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

// ---- File-based config storage (replaces tauri-plugin-store) ----

fn config_path() -> Result<std::path::PathBuf, String> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "Failed to get config dir".to_string())?
        .join("ai.agentmux.cef");
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;
    Ok(config_dir.join("provider-config.json"))
}

fn load_config() -> Result<ProviderConfig, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(ProviderConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read provider config: {e}"))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse provider config: {e}"))
}

fn save_config(config: &ProviderConfig) -> Result<(), String> {
    let path = config_path()?;
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize provider config: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write provider config: {e}"))
}

// ---- CLI detection helpers ----

fn detect_cli(name: &str) -> CliDetectionResult {
    let find_cmd = if cfg!(windows) { "where" } else { "which" };

    let mut find = std::process::Command::new(find_cmd);
    find.arg(name);
    #[cfg(windows)]
    find.creation_flags(0x08000000);

    let path = find
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().next().map(|s| s.trim().to_string())
            } else {
                None
            }
        });

    let version = if path.is_some() {
        let mut ver = std::process::Command::new(name);
        ver.arg("--version");
        #[cfg(windows)]
        ver.creation_flags(0x08000000);

        ver.output()
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

// ---- CLI installer helpers ----

const CLAUDE_VERSION: &str = "latest";
const CODEX_VERSION: &str = "0.107.0";
const GEMINI_VERSION: &str = "0.31.0";

fn get_provider_install_dir(provider: &str) -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let version = env!("CARGO_PKG_VERSION");
    Ok(home
        .join(".agentmux")
        .join("instances")
        .join(format!("v{}", version))
        .join("cli")
        .join(provider))
}

fn get_local_cli_bin_path(provider: &str) -> Result<std::path::PathBuf, String> {
    let install_dir = get_provider_install_dir(provider)?;
    let bin_name = match provider {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        _ => return Err(format!("Unknown provider: {provider}")),
    };

    if cfg!(windows) {
        Ok(install_dir
            .join("node_modules")
            .join(".bin")
            .join(format!("{bin_name}.cmd")))
    } else {
        Ok(install_dir.join("node_modules").join(".bin").join(bin_name))
    }
}

fn get_npm_package(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok("@anthropic-ai/claude-code"),
        "codex" => Ok("@openai/codex"),
        "gemini" => Ok("@google/gemini-cli"),
        _ => Err(format!("Unknown provider: {provider}")),
    }
}

fn get_pinned_version(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok(CLAUDE_VERSION),
        "codex" => Ok(CODEX_VERSION),
        "gemini" => Ok(GEMINI_VERSION),
        _ => Err(format!("No pinned version for provider: {provider}")),
    }
}

// ---- Command handlers ----

/// Detect installed CLI tools.
pub async fn detect_installed_clis() -> Result<serde_json::Value, String> {
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

    serde_json::to_value(&results).map_err(|e| format!("Serialize error: {e}"))
}

/// Get the persisted provider configuration.
pub fn get_provider_config() -> Result<serde_json::Value, String> {
    let config = load_config()?;
    serde_json::to_value(&config).map_err(|e| format!("Serialize error: {e}"))
}

/// Save the provider configuration.
pub fn save_provider_config(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let config: ProviderConfig = serde_json::from_value(
        args.get("config").cloned().unwrap_or(args.clone()),
    )
    .map_err(|e| format!("Failed to parse config: {e}"))?;

    tracing::info!(
        "Saving provider config: default={}, setup_complete={}",
        config.default_provider,
        config.setup_complete
    );
    save_config(&config)?;
    Ok(serde_json::Value::Null)
}

/// Get install info for a provider.
pub fn get_provider_install_info(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;

    let info = match provider {
        "claude" => ProviderInstallInfo {
            provider: "claude".to_string(),
            install_command: "npm install -g @anthropic-ai/claude-code".to_string(),
            docs_url: "https://docs.anthropic.com/claude-code".to_string(),
        },
        "gemini" => ProviderInstallInfo {
            provider: "gemini".to_string(),
            install_command: "npm install -g @google/gemini-cli".to_string(),
            docs_url: "https://ai.google.dev/gemini-cli".to_string(),
        },
        "codex" => ProviderInstallInfo {
            provider: "codex".to_string(),
            install_command: "npm install -g @openai/codex".to_string(),
            docs_url: "https://platform.openai.com/docs/codex".to_string(),
        },
        _ => return Err(format!("Unknown provider: {provider}")),
    };

    serde_json::to_value(&info).map_err(|e| format!("Serialize error: {e}"))
}

/// Store an auth token for a provider.
pub fn set_provider_auth(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing token".to_string())?;

    tracing::info!("Setting auth token for provider: {}", provider);
    let mut config = load_config()?;

    let settings = config
        .providers
        .entry(provider.to_string())
        .or_insert_with(|| ProviderSettings {
            cli_path: None,
            auth_token: None,
            auth_status: "none".to_string(),
            output_format: String::new(),
            extra_args: vec![],
        });

    settings.auth_token = Some(token.to_string());
    settings.auth_status = "authenticated".to_string();

    save_config(&config)?;
    Ok(serde_json::Value::Null)
}

/// Clear auth token for a provider.
pub fn clear_provider_auth(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;

    tracing::info!("Clearing auth token for provider: {}", provider);
    let mut config = load_config()?;

    if let Some(settings) = config.providers.get_mut(provider) {
        settings.auth_token = None;
        settings.auth_status = "none".to_string();
    }

    save_config(&config)?;
    Ok(serde_json::Value::Null)
}

/// Get auth status for a provider.
pub fn get_provider_auth_status(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;

    let config = load_config()?;
    let status = config
        .providers
        .get(provider)
        .map(|s| s.auth_status.clone())
        .unwrap_or_else(|| "none".to_string());

    let result = ProviderAuthStatus {
        provider: provider.to_string(),
        status,
        error: None,
    };
    serde_json::to_value(&result).map_err(|e| format!("Serialize error: {e}"))
}

/// Check CLI authentication status.
pub async fn check_cli_auth_status(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?
        .to_string();

    let cli_path = args
        .get("cli_path")
        .or_else(|| args.get("cliPath"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let cli_cmd = cli_path.unwrap_or_else(|| provider.clone());

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
    .map_err(|e| format!("Auth check task failed: {e}"))??;

    serde_json::to_value(&result).map_err(|e| format!("Serialize error: {e}"))
}

fn check_claude_auth(cli_cmd: &str) -> Result<CliAuthStatus, String> {
    let mut cmd = std::process::Command::new(cli_cmd);
    cmd.args(["auth", "status", "--json"]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let output = cmd
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

fn check_codex_auth(cli_cmd: &str) -> Result<CliAuthStatus, String> {
    let mut cmd = std::process::Command::new(cli_cmd);
    cmd.args(["login", "status"]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let output = cmd
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

fn check_gemini_auth(cli_cmd: &str) -> Result<CliAuthStatus, String> {
    let mut cmd = std::process::Command::new(cli_cmd);
    cmd.args(["auth", "status"]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let output = cmd
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

/// Get CLI path from isolated install directory.
pub fn get_cli_path(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;

    let local_path = get_local_cli_bin_path(provider)?;
    if local_path.exists() {
        tracing::info!("Found {} in isolated install: {}", provider, local_path.display());
        return Ok(serde_json::json!(local_path.to_string_lossy()));
    }

    tracing::info!("{} CLI not found in isolated install", provider);
    Ok(serde_json::Value::Null)
}

/// Install a provider CLI via npm.
pub async fn install_cli(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?
        .to_string();

    let local_path = get_local_cli_bin_path(&provider)?;
    if local_path.exists() {
        tracing::info!("CLI already installed for {}: {}", provider, local_path.display());
        let result = CliInstallResult {
            provider,
            cli_path: local_path.to_string_lossy().to_string(),
            version: "installed".to_string(),
            already_installed: true,
        };
        return serde_json::to_value(&result).map_err(|e| format!("Serialize error: {e}"));
    }

    let provider_clone = provider.clone();
    let result = tokio::task::spawn_blocking(move || {
        let npm_package = get_npm_package(&provider_clone)?;
        let pinned_version = get_pinned_version(&provider_clone)?;
        let install_dir = get_provider_install_dir(&provider_clone)?;

        let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };

        // Pre-flight: verify npm is available
        let mut check = std::process::Command::new(npm_cmd);
        check.arg("--version");
        #[cfg(windows)]
        check.creation_flags(0x08000000);
        match check.output() {
            Ok(output) if output.status.success() => {}
            _ => {
                return Err(
                    "NODEJS_NOT_FOUND: Node.js/npm is not installed.".to_string(),
                );
            }
        }

        std::fs::create_dir_all(&install_dir)
            .map_err(|e| format!("Failed to create install dir: {e}"))?;

        let package_spec = format!("{npm_package}@{pinned_version}");
        let mut cmd = std::process::Command::new(npm_cmd);
        cmd.args([
            "install",
            "--prefix",
            &install_dir.to_string_lossy(),
            &package_spec,
        ]);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to run npm install: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("npm install failed: {stderr}"));
        }

        let cli_path = get_local_cli_bin_path(&provider_clone)?;
        if !cli_path.exists() {
            return Err(format!(
                "Installation completed but CLI binary not found at {}",
                cli_path.display()
            ));
        }

        Ok(CliInstallResult {
            provider: provider_clone,
            cli_path: cli_path.to_string_lossy().to_string(),
            version: "installed".to_string(),
            already_installed: false,
        })
    })
    .await
    .map_err(|e| format!("Install task failed: {e}"))??;

    serde_json::to_value(&result).map_err(|e| format!("Serialize error: {e}"))
}

/// Check if Node.js and npm are available.
pub async fn check_nodejs_available() -> Result<serde_json::Value, String> {
    let result = tokio::task::spawn_blocking(|| {
        let node_cmd = if cfg!(windows) { "node.exe" } else { "node" };
        let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };

        let mut status = NodejsStatus {
            available: false,
            version: None,
            npm_available: false,
            npm_version: None,
            path: None,
        };

        let mut cmd = std::process::Command::new(node_cmd);
        cmd.arg("--version");
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                status.available = true;
                status.version = Some(
                    String::from_utf8_lossy(&output.stdout).trim().to_string(),
                );

                let which_cmd = if cfg!(windows) { "where" } else { "which" };
                let mut wcmd = std::process::Command::new(which_cmd);
                wcmd.arg(node_cmd);
                #[cfg(windows)]
                wcmd.creation_flags(0x08000000);
                if let Ok(path_out) = wcmd.output() {
                    if path_out.status.success() {
                        status.path = Some(
                            String::from_utf8_lossy(&path_out.stdout)
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string(),
                        );
                    }
                }
            }
        }

        let mut cmd = std::process::Command::new(npm_cmd);
        cmd.arg("--version");
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                status.npm_available = true;
                status.npm_version = Some(
                    String::from_utf8_lossy(&output.stdout).trim().to_string(),
                );
            }
        }

        status
    })
    .await
    .map_err(|e| format!("Failed to check Node.js: {e}"))?;

    serde_json::to_value(&result).map_err(|e| format!("Serialize error: {e}"))
}

/// Copy a file to a directory.
pub fn copy_file_to_dir(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source_path = args
        .get("source_path")
        .or_else(|| args.get("sourcePath"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing source_path".to_string())?;

    let target_dir = args
        .get("target_dir")
        .or_else(|| args.get("targetDir"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing target_dir".to_string())?;

    let source = std::path::Path::new(source_path);
    let target_dir_norm = normalize_path_for_platform(target_dir);
    let target_dir = std::path::Path::new(&target_dir_norm);

    if !source.exists() {
        return Err(format!("Source not found: {}", source.display()));
    }
    if !target_dir.exists() {
        return Err(format!("Target directory not found: {}", target_dir.display()));
    }
    if !target_dir.is_dir() {
        return Err(format!("Target path is not a directory: {}", target_dir.display()));
    }

    let name = source
        .file_name()
        .ok_or_else(|| "Invalid source path".to_string())?;

    let target = deconflict_path(target_dir, name)?;
    copy_recursive(source, &target)?;

    Ok(serde_json::json!(target.display().to_string()))
}

// ---- File operation helpers ----

fn normalize_path_for_platform(path: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(rest) = path.strip_prefix('/') {
            let mut chars = rest.chars();
            if let Some(drive) = chars.next() {
                if drive.is_ascii_alphabetic() {
                    let after_drive = chars.as_str();
                    if after_drive.is_empty() || after_drive.starts_with('/') {
                        let tail = after_drive.replace('/', "\\");
                        return format!("{}:{}", drive.to_ascii_uppercase(), tail);
                    }
                }
            }
        }
        path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    path.to_string()
}

fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    if src.is_file() {
        std::fs::copy(src, dst).map_err(|e| format!("Copy failed: {}", e))?;
    } else if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("Create dir failed: {}", e))?;
        for entry in std::fs::read_dir(src).map_err(|e| format!("Read dir failed: {}", e))? {
            let entry = entry.map_err(|e| format!("Dir entry error: {}", e))?;
            let name = entry.file_name();
            copy_recursive(&entry.path(), &dst.join(&name))?;
        }
    }
    Ok(())
}

fn deconflict_path(
    dir: &std::path::Path,
    name: &std::ffi::OsStr,
) -> Result<std::path::PathBuf, String> {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return Ok(candidate);
    }

    let name_str = name.to_string_lossy();
    let (stem, ext) = match name_str.rfind('.') {
        Some(dot) => (&name_str[..dot], &name_str[dot..]),
        None => (name_str.as_ref(), ""),
    };

    for n in 1..=99 {
        let new_name = format!("{stem}_{n}{ext}");
        let candidate = dir.join(&new_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not find a free filename for '{}' in '{}'",
        name_str,
        dir.display()
    ))
}
