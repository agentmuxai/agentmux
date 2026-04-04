// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shell integration script deployment and shell startup configuration.
//!
//! Embeds shell integration scripts (bash, zsh, pwsh, fish) and deploys them to
//! `~/.agentmux/shell/<type>/` on first use or when the version changes.
//! The shell controller uses these scripts to install prompt hooks that send
//! OSC 16162;E commands carrying `AGENTMUX_AGENT_ID`, enabling per-pane title
//! and color to work.

use std::path::{Path, PathBuf};

// ─── Embedded scripts ────────────────────────────────────────────────────────

const BASH_SCRIPT: &str = include_str!("shellintegration/bash.sh");
const ZSH_SCRIPT: &str = include_str!("shellintegration/zsh.sh");
const PWSH_SCRIPT: &str = include_str!("shellintegration/pwsh.ps1");
const FISH_SCRIPT: &str = include_str!("shellintegration/fish.fish");
const VERSION_MARKER: &str = env!("CARGO_PKG_VERSION");

// ─── Shell type ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShellType {
    Bash,
    Zsh,
    Pwsh,
    Fish,
    Unknown,
}

/// Detect shell type from the shell binary path.
pub fn detect_shell_type(shell_path: &str) -> ShellType {
    let name = Path::new(shell_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    match name.as_str() {
        "pwsh" | "powershell" => ShellType::Pwsh,
        "bash" => ShellType::Bash,
        "zsh" => ShellType::Zsh,
        "fish" => ShellType::Fish,
        _ => ShellType::Unknown,
    }
}

// ─── Deploy ──────────────────────────────────────────────────────────────────

/// Deploy shell integration scripts to `<wave_data_dir>/shell/<type>/`.
/// Skips deployment if the version marker is already current.
/// Errors are logged but not fatal — a missing script just means no integration.
pub fn deploy_scripts(wave_data_dir: &Path) {
    let shell_base = wave_data_dir.join("shell");
    let version_file = shell_base.join(".version");

    // Check if already up-to-date
    if let Ok(existing) = std::fs::read_to_string(&version_file) {
        if existing.trim() == VERSION_MARKER {
            return;
        }
    }

    tracing::info!("Deploying shell integration scripts (v{})", VERSION_MARKER);

    let deploys: &[(&str, &str, &str)] = &[
        ("bash", ".bashrc", BASH_SCRIPT),
        ("zsh", ".zshrc", ZSH_SCRIPT),
        ("pwsh", "wavepwsh.ps1", PWSH_SCRIPT),
        ("fish", "wave.fish", FISH_SCRIPT),
    ];

    let mut all_ok = true;
    for (dir_name, file_name, content) in deploys {
        let dir = shell_base.join(dir_name);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("shell integration: failed to create {}: {}", dir.display(), e);
            all_ok = false;
            continue;
        }
        let path = dir.join(file_name);
        if let Err(e) = std::fs::write(&path, content) {
            tracing::warn!("shell integration: failed to write {}: {}", path.display(), e);
            all_ok = false;
        }
    }

    // Write version marker only if all scripts deployed successfully
    if all_ok {
        let _ = std::fs::write(&version_file, VERSION_MARKER);
    }
}

// ─── Startup configuration ───────────────────────────────────────────────────

/// Shell startup configuration: extra args and env vars to inject.
pub struct ShellStartup {
    /// Extra args to append to the shell command.
    pub extra_args: Vec<String>,
    /// Environment variables to set in the PTY.
    pub env_vars: Vec<(String, String)>,
}

/// Get the startup configuration for launching an interactive shell with
/// AgentMux integration. Returns `None` for unknown shell types.
pub fn get_shell_startup(
    shell_type: ShellType,
    wave_data_dir: &Path,
) -> Option<ShellStartup> {
    match shell_type {
        ShellType::Bash => {
            let rcfile = wave_data_dir.join("shell").join("bash").join(".bashrc");
            Some(ShellStartup {
                extra_args: vec![
                    "--rcfile".to_string(),
                    rcfile.to_string_lossy().into_owned(),
                ],
                env_vars: vec![],
            })
        }
        ShellType::Zsh => {
            let zdotdir = wave_data_dir.join("shell").join("zsh");
            Some(ShellStartup {
                extra_args: vec![],
                env_vars: vec![
                    ("ZDOTDIR".to_string(), zdotdir.to_string_lossy().into_owned()),
                    // Preserve original ZDOTDIR so the integration script can source ~/.zshrc
                    ("AGENTMUX_ZDOTDIR".to_string(), zdotdir.to_string_lossy().into_owned()),
                ],
            })
        }
        ShellType::Pwsh => {
            let script = wave_data_dir
                .join("shell")
                .join("pwsh")
                .join("wavepwsh.ps1");
            Some(ShellStartup {
                extra_args: vec![
                    "-ExecutionPolicy".to_string(),
                    "Bypass".to_string(),
                    "-NoExit".to_string(),
                    "-File".to_string(),
                    script.to_string_lossy().into_owned(),
                ],
                env_vars: vec![],
            })
        }
        ShellType::Fish => {
            let script = wave_data_dir
                .join("shell")
                .join("fish")
                .join("wave.fish");
            Some(ShellStartup {
                extra_args: vec![
                    "-C".to_string(),
                    format!("source {}", shell_quote(&script.to_string_lossy())),
                ],
                env_vars: vec![],
            })
        }
        ShellType::Unknown => None,
    }
}

/// Find the wsh binary deployed alongside the current executable.
/// Returns the path if found, or None.
pub fn find_wsh_binary() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let version = env!("CARGO_PKG_VERSION");

    // Try versioned name (matches package-portable.ps1 naming)
    let versioned = if cfg!(windows) {
        exe_dir.join(format!("wsh-{}-windows.x64.exe", version))
    } else if cfg!(target_os = "macos") {
        exe_dir.join(format!("wsh-{}-darwin.arm64", version))
    } else {
        exe_dir.join(format!("wsh-{}-linux.amd64", version))
    };
    if versioned.exists() {
        return Some(versioned);
    }

    // Fallback: plain wsh / wsh.exe
    let plain = if cfg!(windows) {
        exe_dir.join("wsh.exe")
    } else {
        exe_dir.join("wsh")
    };
    if plain.exists() {
        return Some(plain);
    }

    None
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Single-quote a path for POSIX shell usage.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_shell_type() {
        assert_eq!(detect_shell_type("bash"), ShellType::Bash);
        assert_eq!(detect_shell_type("/bin/bash"), ShellType::Bash);
        assert_eq!(detect_shell_type("zsh"), ShellType::Zsh);
        assert_eq!(detect_shell_type("/usr/bin/zsh"), ShellType::Zsh);
        assert_eq!(detect_shell_type("pwsh"), ShellType::Pwsh);
        assert_eq!(detect_shell_type("powershell"), ShellType::Pwsh);
        assert_eq!(detect_shell_type("fish"), ShellType::Fish);
        assert_eq!(detect_shell_type("cmd.exe"), ShellType::Unknown);
        assert_eq!(detect_shell_type("cmd"), ShellType::Unknown);
    }

    #[test]
    fn test_bash_startup_args() {
        let dir = Path::new("/home/user/.agentmux");
        let startup = get_shell_startup(ShellType::Bash, dir).unwrap();
        assert_eq!(startup.extra_args[0], "--rcfile");
        assert!(startup.extra_args[1].contains("bash"));
        assert!(startup.extra_args[1].ends_with(".bashrc"));
    }

    #[test]
    fn test_pwsh_startup_args() {
        let dir = Path::new("/home/user/.agentmux");
        let startup = get_shell_startup(ShellType::Pwsh, dir).unwrap();
        assert!(startup.extra_args.contains(&"-NoExit".to_string()));
        assert!(startup.extra_args.contains(&"-File".to_string()));
    }

    #[test]
    fn test_zsh_uses_zdotdir() {
        let dir = Path::new("/home/user/.agentmux");
        let startup = get_shell_startup(ShellType::Zsh, dir).unwrap();
        assert!(startup.extra_args.is_empty());
        assert!(startup.env_vars.iter().any(|(k, _)| k == "ZDOTDIR"));
    }

    #[test]
    fn test_unknown_shell_returns_none() {
        let dir = Path::new("/tmp");
        assert!(get_shell_startup(ShellType::Unknown, dir).is_none());
    }
}
