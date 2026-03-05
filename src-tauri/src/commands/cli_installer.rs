// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Local CLI installer — detects provider CLIs on system PATH or installs them locally.
//!
//! Detection priority:
//! 1. System PATH (`where`/`which`)
//! 2. Local install dir (`~/.agentmux/cli/<provider>/`)
//! 3. If neither found → install
//!
//! Install strategy per provider:
//! - Claude: native installer (curl/PowerShell — npm is deprecated)
//! - Codex: npm install --prefix (pinned version)
//! - Gemini: npm install --prefix (pinned version)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Pinned versions for local installs
const CODEX_VERSION: &str = "0.107.0";
const GEMINI_VERSION: &str = "0.31.0";

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

/// Check if a CLI is available on the system PATH using `where` (Windows) or `which` (Unix).
/// Returns the full path if found.
fn detect_on_system_path(bin_name: &str) -> Option<String> {
    let find_cmd = if cfg!(windows) { "where" } else { "which" };

    let mut cmd = std::process::Command::new(find_cmd);
    cmd.arg(bin_name);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let output = cmd.output().ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().next().map(|s| s.trim().to_string())
    } else {
        None
    }
}

/// Get the CLI binary name for a provider.
fn get_bin_name(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok("claude"),
        "codex" => Ok("codex"),
        "gemini" => Ok("gemini"),
        _ => Err(format!("Unknown provider: {provider}")),
    }
}

/// Get the pinned version for npm-installed providers.
fn get_pinned_version(provider: &str) -> Result<&'static str, String> {
    match provider {
        "codex" => Ok(CODEX_VERSION),
        "gemini" => Ok(GEMINI_VERSION),
        _ => Err(format!("No pinned version for provider: {provider}")),
    }
}

/// Install Claude Code using the native installer.
///
/// - Windows: `irm https://claude.ai/install.ps1 | iex`
/// - Unix: `curl -fsSL https://claude.ai/install.sh | bash`
fn install_claude_native() -> Result<String, String> {
    tracing::info!("Installing Claude Code via native installer...");

    let output = {
        #[cfg(windows)]
        {
            let mut cmd = std::process::Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "irm https://claude.ai/install.ps1 | iex",
            ]);
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            cmd.output()
                .map_err(|e| format!("Failed to run Claude installer: {e}"))?
        }
        #[cfg(not(windows))]
        {
            std::process::Command::new("bash")
                .args(["-c", "curl -fsSL https://claude.ai/install.sh | bash"])
                .output()
                .map_err(|e| format!("Failed to run Claude installer: {e}"))?
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Claude installer failed: {stderr}"));
    }

    // After install, find it on PATH
    if let Some(path) = detect_on_system_path("claude") {
        Ok(path)
    } else {
        // Common install locations to check
        let candidates = if cfg!(windows) {
            vec![
                dirs::home_dir()
                    .map(|h| h.join(".claude").join("local").join("claude.exe")),
                dirs::home_dir()
                    .map(|h| h.join("AppData").join("Local").join("Programs").join("claude-code").join("claude.exe")),
            ]
        } else {
            vec![
                Some(PathBuf::from("/usr/local/bin/claude")),
                dirs::home_dir().map(|h| h.join(".local").join("bin").join("claude")),
                dirs::home_dir().map(|h| h.join(".claude").join("local").join("claude")),
            ]
        };

        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }

        Err("Claude installed but binary not found on PATH. You may need to restart your terminal.".to_string())
    }
}

/// Install a provider CLI via npm into the local AgentMux directory.
/// Hides the console window on Windows.
fn install_via_npm(provider: &str) -> Result<String, String> {
    let npm_package = get_npm_package(provider)?;
    let pinned_version = get_pinned_version(provider)?;
    let install_dir = get_provider_install_dir(provider)?;

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

/// Get the CLI path for a provider, checking system PATH first, then local install.
///
/// Returns the full path to the binary, or null if not found anywhere.
#[tauri::command]
pub async fn get_cli_path(provider: String) -> Result<Option<String>, String> {
    let bin_name = get_bin_name(&provider)?;

    // 1. Check system PATH first
    if let Some(system_path) = detect_on_system_path(bin_name) {
        tracing::info!(
            "Found {} on system PATH: {}",
            provider,
            system_path
        );
        return Ok(Some(system_path));
    }

    // 2. Check local AgentMux install dir
    let local_path = get_local_cli_bin_path(&provider)?;
    if local_path.exists() {
        tracing::info!(
            "Found {} in local install: {}",
            provider,
            local_path.display()
        );
        return Ok(Some(local_path.to_string_lossy().to_string()));
    }

    // 3. Not found anywhere
    tracing::info!("{} CLI not found on PATH or in local install", provider);
    Ok(None)
}

/// Install a provider CLI locally.
///
/// - Claude: uses native installer (curl/PowerShell)
/// - Codex/Gemini: uses npm with pinned version
///
/// After install, verifies the binary works by running `--version`.
#[tauri::command]
pub async fn install_cli(provider: String) -> Result<CliInstallResult, String> {
    // Check if already available (PATH or local)
    let bin_name = get_bin_name(&provider)?;
    if let Some(existing_path) = detect_on_system_path(bin_name) {
        tracing::info!("CLI already on system PATH for {}: {}", provider, existing_path);
        return Ok(CliInstallResult {
            provider,
            cli_path: existing_path,
            version: "system".to_string(),
            already_installed: true,
        });
    }

    let local_path = get_local_cli_bin_path(&provider)?;
    if local_path.exists() {
        tracing::info!("CLI already installed locally for {}: {}", provider, local_path.display());
        return Ok(CliInstallResult {
            provider,
            cli_path: local_path.to_string_lossy().to_string(),
            version: "installed".to_string(),
            already_installed: true,
        });
    }

    // Install
    let provider_clone = provider.clone();
    let result = tokio::task::spawn_blocking(move || {
        let cli_path = match provider_clone.as_str() {
            "claude" => install_claude_native(),
            "codex" | "gemini" => install_via_npm(&provider_clone),
            _ => Err(format!("Unknown provider: {provider_clone}")),
        }?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_provider_install_dir_claude() {
        let dir = get_provider_install_dir("claude").unwrap();
        assert!(
            dir.ends_with(".agentmux/cli/claude")
                || dir.ends_with(".agentmux\\cli\\claude")
        );
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
    fn test_get_bin_name() {
        assert_eq!(get_bin_name("claude").unwrap(), "claude");
        assert_eq!(get_bin_name("codex").unwrap(), "codex");
        assert_eq!(get_bin_name("gemini").unwrap(), "gemini");
        assert!(get_bin_name("unknown").is_err());
    }

    #[test]
    fn test_get_pinned_version() {
        assert_eq!(get_pinned_version("codex").unwrap(), CODEX_VERSION);
        assert_eq!(get_pinned_version("gemini").unwrap(), GEMINI_VERSION);
        assert!(get_pinned_version("claude").is_err()); // Claude uses native installer
    }
}
