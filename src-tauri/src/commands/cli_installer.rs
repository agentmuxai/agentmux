// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Local CLI installer — installs provider CLIs into ~/.agentmux/cli/<provider>/
//! using system `npm`. CLIs are pinned to specific versions and isolated from the user's PATH.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CliInstallResult {
    pub provider: String,
    pub cli_path: String,
    pub version: String,
    pub already_installed: bool,
}

/// Returns the base install directory: ~/.agentmux/cli/<provider>/
fn get_provider_install_dir(provider: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    Ok(home.join(".agentmux").join("cli").join(provider))
}

/// Returns the expected CLI binary path inside the npm prefix.
fn get_cli_bin_path(provider: &str) -> Result<PathBuf, String> {
    let install_dir = get_provider_install_dir(provider)?;
    let bin_name = match provider {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        _ => return Err(format!("Unknown provider: {provider}")),
    };

    if cfg!(windows) {
        // npm on Windows puts .cmd shims in node_modules/.bin/
        Ok(install_dir.join("node_modules").join(".bin").join(format!("{bin_name}.cmd")))
    } else {
        Ok(install_dir.join("node_modules").join(".bin").join(bin_name))
    }
}

/// Get the npm package name for a provider.
fn get_npm_package(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok("@anthropic-ai/claude-code"),
        "codex" => Ok("@openai/codex"),
        "gemini" => Ok("@anthropic-ai/gemini-cli"),
        _ => Err(format!("Unknown provider: {provider}")),
    }
}

/// Install a provider CLI locally using npm.
///
/// Runs `npm install --prefix ~/.agentmux/cli/<provider> <package>@latest`
/// and returns the path to the installed binary.
#[tauri::command]
pub async fn install_cli(provider: String) -> Result<CliInstallResult, String> {
    let npm_package = get_npm_package(&provider)?;
    let install_dir = get_provider_install_dir(&provider)?;
    let cli_path = get_cli_bin_path(&provider)?;

    // Check if already installed
    if cli_path.exists() {
        tracing::info!("CLI already installed for {}: {}", provider, cli_path.display());
        return Ok(CliInstallResult {
            provider: provider.clone(),
            cli_path: cli_path.to_string_lossy().to_string(),
            version: "installed".to_string(),
            already_installed: true,
        });
    }

    let provider_clone = provider.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Create install directory
        std::fs::create_dir_all(&install_dir)
            .map_err(|e| format!("Failed to create install dir: {e}"))?;

        tracing::info!(
            "Installing {} for {} into {}",
            npm_package, provider_clone, install_dir.display()
        );

        // Find npm
        let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };

        let output = std::process::Command::new(npm_cmd)
            .args([
                "install",
                "--prefix",
                &install_dir.to_string_lossy(),
                &format!("{npm_package}@latest"),
            ])
            .output()
            .map_err(|e| format!("Failed to run npm install: {e}. Is npm installed?"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("npm install failed: {stderr}"));
        }

        let cli_path = get_cli_bin_path(&provider_clone)?;
        if !cli_path.exists() {
            return Err(format!(
                "Installation completed but CLI binary not found at {}",
                cli_path.display()
            ));
        }

        Ok(CliInstallResult {
            provider: provider_clone,
            cli_path: cli_path.to_string_lossy().to_string(),
            version: "latest".to_string(),
            already_installed: false,
        })
    })
    .await
    .map_err(|e| format!("Install task failed: {e}"))?;

    result
}

/// Get the CLI path for a provider if already installed.
///
/// Returns the full path to the binary, or null if not installed.
#[tauri::command]
pub async fn get_cli_path(provider: String) -> Result<Option<String>, String> {
    let cli_path = get_cli_bin_path(&provider)?;

    if cli_path.exists() {
        Ok(Some(cli_path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}
