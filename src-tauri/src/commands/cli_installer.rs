// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Local CLI installer — manages version-isolated provider CLIs.
//!
//! Each AgentMux version gets its own CLI directory:
//!   ~/.agentmux/instances/v<version>/cli/<provider>/
//!
//! Detection checks the version-isolated directory only (not system PATH).
//! All providers use npm install --prefix for consistency.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Pinned versions for local installs
const CLAUDE_VERSION: &str = "latest";
const CODEX_VERSION: &str = "0.107.0";
const GEMINI_VERSION: &str = "0.31.0";

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

/// Returns the base install directory: ~/.agentmux/instances/v<version>/cli/<provider>/
fn get_provider_install_dir(provider: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let version = env!("CARGO_PKG_VERSION");
    Ok(home
        .join(".agentmux")
        .join("instances")
        .join(format!("v{}", version))
        .join("cli")
        .join(provider))
}

/// Returns the expected CLI binary path inside the npm prefix.
fn get_local_cli_bin_path(provider: &str) -> Result<PathBuf, String> {
    let install_dir = get_provider_install_dir(provider)?;
    let bin_name = match provider {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        _ => return Err(format!("Unknown provider: {provider}")),
    };

    if cfg!(windows) {
        // npm on Windows puts .cmd shims in node_modules/.bin/
        Ok(install_dir
            .join("node_modules")
            .join(".bin")
            .join(format!("{bin_name}.cmd")))
    } else {
        Ok(install_dir.join("node_modules").join(".bin").join(bin_name))
    }
}

/// Get the npm package name for a provider.
fn get_npm_package(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok("@anthropic-ai/claude-code"),
        "codex" => Ok("@openai/codex"),
        "gemini" => Ok("@google/gemini-cli"),
        _ => Err(format!("Unknown provider: {provider}")),
    }
}

/// Get the pinned version for npm-installed providers.
fn get_pinned_version(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok(CLAUDE_VERSION),
        "codex" => Ok(CODEX_VERSION),
        "gemini" => Ok(GEMINI_VERSION),
        _ => Err(format!("No pinned version for provider: {provider}")),
    }
}

/// Install a provider CLI via npm into the version-isolated directory.
/// Hides the console window on Windows.
fn install_via_npm(provider: &str) -> Result<String, String> {
    let npm_package = get_npm_package(provider)?;
    let pinned_version = get_pinned_version(provider)?;
    let install_dir = get_provider_install_dir(provider)?;

    // Pre-flight: verify npm is available before attempting install
    let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let mut check = std::process::Command::new(npm_cmd);
    check.arg("--version");
    #[cfg(windows)]
    check.creation_flags(0x08000000);
    match check.output() {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout);
            tracing::info!("npm {} available for install", ver.trim());
        }
        _ => {
            return Err(
                "NODEJS_NOT_FOUND: Node.js/npm is not installed. \
                 Codex and Gemini CLIs require Node.js to install. \
                 Install Node.js from https://nodejs.org/ (LTS recommended)."
                    .to_string(),
            );
        }
    }

    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Failed to create install dir: {e}"))?;

    tracing::info!(
        "Installing {}@{} for {} into {}",
        npm_package,
        pinned_version,
        provider,
        install_dir.display()
    );

    let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let package_spec = format!("{npm_package}@{pinned_version}");

    let mut cmd = std::process::Command::new(npm_cmd);
    cmd.args([
        "install",
        "--prefix",
        &install_dir.to_string_lossy(),
        &package_spec,
    ]);

    // Hide console window on Windows
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run npm install: {e}. Is npm installed?"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("npm install failed: {stderr}"));
    }

    let cli_path = get_local_cli_bin_path(provider)?;
    if !cli_path.exists() {
        return Err(format!(
            "Installation completed but CLI binary not found at {}",
            cli_path.display()
        ));
    }

    Ok(cli_path.to_string_lossy().to_string())
}

/// Verify a CLI binary works by running `<binary> --version`.
fn verify_cli(cli_path: &str) -> Result<String, String> {
    let mut cmd = std::process::Command::new(cli_path);
    cmd.arg("--version");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let output = cmd.output()
        .map_err(|e| format!("Failed to verify CLI at {cli_path}: {e}"))?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string();
        Ok(version)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "CLI at {cli_path} returned error: {}",
            stderr.trim()
        ))
    }
}

/// Get the CLI path for a provider from the version-isolated directory.
///
/// Returns the full path to the binary, or null if not installed.
/// Does NOT check system PATH — isolation means we always use our own copy.
#[tauri::command]
pub async fn get_cli_path(provider: String) -> Result<Option<String>, String> {
    let local_path = get_local_cli_bin_path(&provider)?;
    if local_path.exists() {
        tracing::info!(
            "Found {} in isolated install: {}",
            provider,
            local_path.display()
        );
        return Ok(Some(local_path.to_string_lossy().to_string()));
    }

    tracing::info!("{} CLI not found in isolated install", provider);
    Ok(None)
}

/// Install a provider CLI into the version-isolated directory via npm.
///
/// All providers use npm install --prefix for consistency.
/// After install, verifies the binary works by running `--version`.
#[tauri::command]
pub async fn install_cli(provider: String) -> Result<CliInstallResult, String> {
    // Check if already installed in isolated dir
    let local_path = get_local_cli_bin_path(&provider)?;
    if local_path.exists() {
        tracing::info!("CLI already installed for {}: {}", provider, local_path.display());
        return Ok(CliInstallResult {
            provider,
            cli_path: local_path.to_string_lossy().to_string(),
            version: "installed".to_string(),
            already_installed: true,
        });
    }

    // Install via npm for all providers
    let provider_clone = provider.clone();
    let result = tokio::task::spawn_blocking(move || {
        let cli_path = install_via_npm(&provider_clone)?;

        // Verify the install worked
        match verify_cli(&cli_path) {
            Ok(version) => {
                tracing::info!("Verified {} CLI: {} ({})", provider_clone, cli_path, version);
                Ok(CliInstallResult {
                    provider: provider_clone,
                    cli_path,
                    version,
                    already_installed: false,
                })
            }
            Err(e) => {
                tracing::warn!("CLI installed but verification failed for {}: {}", provider_clone, e);
                // Still return the path — verification failure might be a minor issue
                Ok(CliInstallResult {
                    provider: provider_clone,
                    cli_path,
                    version: "unverified".to_string(),
                    already_installed: false,
                })
            }
        }
    })
    .await
    .map_err(|e| format!("Install task failed: {e}"))?;

    result
}

/// Check if Node.js and npm are available on the system.
#[tauri::command]
pub async fn check_nodejs_available() -> Result<NodejsStatus, String> {
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

        // Check node
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

        // Check npm
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

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_provider_install_dir_claude() {
        let dir = get_provider_install_dir("claude").unwrap();
        let dir_str = dir.to_string_lossy();
        assert!(dir_str.contains("instances"));
        assert!(dir_str.contains("cli"));
        assert!(
            dir_str.ends_with("cli/claude") || dir_str.ends_with("cli\\claude")
        );
    }

    #[test]
    fn test_get_provider_install_dir_codex() {
        let dir = get_provider_install_dir("codex").unwrap();
        let dir_str = dir.to_string_lossy();
        assert!(dir_str.contains("instances"));
        assert!(dir_str.ends_with("cli/codex") || dir_str.ends_with("cli\\codex"));
    }

    #[test]
    fn test_get_provider_install_dir_gemini() {
        let dir = get_provider_install_dir("gemini").unwrap();
        let dir_str = dir.to_string_lossy();
        assert!(dir_str.contains("instances"));
        assert!(dir_str.ends_with("cli/gemini") || dir_str.ends_with("cli\\gemini"));
    }

    #[test]
    fn test_get_local_cli_bin_path_known_providers() {
        for provider in &["claude", "codex", "gemini"] {
            let path = get_local_cli_bin_path(provider).unwrap();
            let path_str = path.to_string_lossy();
            assert!(path_str.contains("node_modules"));
            assert!(path_str.contains(".bin"));
            assert!(path_str.contains(provider));
        }
    }

    #[test]
    fn test_get_local_cli_bin_path_unknown_provider() {
        let result = get_local_cli_bin_path("unknown");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown provider"));
    }

    #[test]
    fn test_get_npm_package_claude() {
        assert_eq!(
            get_npm_package("claude").unwrap(),
            "@anthropic-ai/claude-code"
        );
    }

    #[test]
    fn test_get_npm_package_codex() {
        assert_eq!(get_npm_package("codex").unwrap(), "@openai/codex");
    }

    #[test]
    fn test_get_npm_package_gemini() {
        assert_eq!(
            get_npm_package("gemini").unwrap(),
            "@google/gemini-cli"
        );
    }

    #[test]
    fn test_get_npm_package_unknown() {
        let result = get_npm_package("unknown");
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_bin_path_windows_extension() {
        let path = get_local_cli_bin_path("claude").unwrap();
        let path_str = path.to_string_lossy();
        if cfg!(windows) {
            assert!(path_str.ends_with(".cmd"));
        } else {
            assert!(!path_str.ends_with(".cmd"));
        }
    }

    #[test]
    fn test_install_dirs_are_isolated() {
        let claude_dir = get_provider_install_dir("claude").unwrap();
        let codex_dir = get_provider_install_dir("codex").unwrap();
        let gemini_dir = get_provider_install_dir("gemini").unwrap();

        assert_ne!(claude_dir, codex_dir);
        assert_ne!(claude_dir, gemini_dir);
        assert_ne!(codex_dir, gemini_dir);

        assert_eq!(claude_dir.parent(), codex_dir.parent());
        assert_eq!(codex_dir.parent(), gemini_dir.parent());
    }

    #[test]
    fn test_get_pinned_version() {
        assert_eq!(get_pinned_version("claude").unwrap(), CLAUDE_VERSION);
        assert_eq!(get_pinned_version("codex").unwrap(), CODEX_VERSION);
        assert_eq!(get_pinned_version("gemini").unwrap(), GEMINI_VERSION);
    }
}
