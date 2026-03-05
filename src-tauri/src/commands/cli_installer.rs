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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_provider_install_dir_claude() {
        let dir = get_provider_install_dir("claude").unwrap();
        assert!(dir.ends_with(".agentmux/cli/claude") || dir.ends_with(".agentmux\\cli\\claude"));
    }

    #[test]
    fn test_get_provider_install_dir_codex() {
        let dir = get_provider_install_dir("codex").unwrap();
        assert!(dir.ends_with("cli/codex") || dir.ends_with("cli\\codex"));
    }

    #[test]
    fn test_get_provider_install_dir_gemini() {
        let dir = get_provider_install_dir("gemini").unwrap();
        assert!(dir.ends_with("cli/gemini") || dir.ends_with("cli\\gemini"));
    }

    #[test]
    fn test_get_cli_bin_path_known_providers() {
        for provider in &["claude", "codex", "gemini"] {
            let path = get_cli_bin_path(provider).unwrap();
            let path_str = path.to_string_lossy();
            assert!(path_str.contains("node_modules"));
            assert!(path_str.contains(".bin"));
            assert!(path_str.contains(provider));
        }
    }

    #[test]
    fn test_get_cli_bin_path_unknown_provider() {
        let result = get_cli_bin_path("unknown");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown provider"));
    }

    #[test]
    fn test_get_npm_package_claude() {
        assert_eq!(get_npm_package("claude").unwrap(), "@anthropic-ai/claude-code");
    }

    #[test]
    fn test_get_npm_package_codex() {
        assert_eq!(get_npm_package("codex").unwrap(), "@openai/codex");
    }

    #[test]
    fn test_get_npm_package_gemini() {
        assert_eq!(get_npm_package("gemini").unwrap(), "@anthropic-ai/gemini-cli");
    }

    #[test]
    fn test_get_npm_package_unknown() {
        let result = get_npm_package("unknown");
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_bin_path_windows_extension() {
        let path = get_cli_bin_path("claude").unwrap();
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

        // Each provider has its own directory
        assert_ne!(claude_dir, codex_dir);
        assert_ne!(claude_dir, gemini_dir);
        assert_ne!(codex_dir, gemini_dir);

        // All share the same parent
        assert_eq!(claude_dir.parent(), codex_dir.parent());
        assert_eq!(codex_dir.parent(), gemini_dir.parent());
    }
}
