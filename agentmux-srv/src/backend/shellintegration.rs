// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shell integration script deployment and shell startup configuration.
//!
//! Embeds shell integration scripts (bash, zsh, pwsh, fish) and deploys them to
//! `~/.agentmux/shell/<type>/` on first use or when the version changes.
//! The shell controller uses these scripts to install prompt hooks that send
//! OSC 16162;E commands carrying `AGENTMUX_AGENT_ID`, enabling per-pane title
//! and color to work.

use std::path::Path;

// ─── Embedded scripts ────────────────────────────────────────────────────────

const BASH_SCRIPT: &str = include_str!("shellintegration/bash.sh");
const ZSH_SCRIPT: &str = include_str!("shellintegration/zsh.sh");
const PWSH_SCRIPT: &str = include_str!("shellintegration/pwsh.ps1");
const FISH_SCRIPT: &str = include_str!("shellintegration/fish.fish");
/// Shared muxlog core (Node). Deployed once at `<shell>/muxlog.mjs`; every
/// shell's `muxlog` function delegates to it. One tested implementation does log
/// discovery + NDJSON rendering + filtering for all shells.
const MUXLOG_JS: &str = include_str!("shellintegration/muxlog.mjs");
/// Shared muxspect core (Node) — muxlog's live-state sibling. Deployed once
/// at `<shell>/muxspect.mjs`; every shell's `muxspect` function delegates to
/// it. See docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md.
const MUXSPECT_JS: &str = include_str!("shellintegration/muxspect.mjs");

/// Deployment marker: `<package version>-<content hash>`, NOT the bare
/// package version alone (codex P2 on PR #2380). Local/dev builds routinely
/// iterate on these embedded scripts WITHOUT a version bump (this repo's own
/// convention — see CLAUDE.md's "Build versioning — local builds are
/// *labeled*, not *versioned*"), so a version-only marker left `.version`
/// already matching on every same-version rebuild/restart, silently
/// skipping deployment of a newly-added or newly-edited script (muxspect.mjs
/// specifically, added in the same PR that added the startup call site —
/// the marker never invalidated to pick it up). Hashing the actual embedded
/// content means ANY change to ANY script forces a redeploy regardless of
/// whether the package version moved, while an unchanged rebuild still
/// skips the (cheap, but not free) disk writes.
fn version_marker() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    BASH_SCRIPT.hash(&mut hasher);
    ZSH_SCRIPT.hash(&mut hasher);
    PWSH_SCRIPT.hash(&mut hasher);
    FISH_SCRIPT.hash(&mut hasher);
    MUXLOG_JS.hash(&mut hasher);
    MUXSPECT_JS.hash(&mut hasher);
    format!("{}-{:x}", env!("CARGO_PKG_VERSION"), hasher.finish())
}

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
    let marker = version_marker();

    // Check if already up-to-date
    if let Ok(existing) = std::fs::read_to_string(&version_file) {
        if existing.trim() == marker {
            return;
        }
    }

    tracing::info!("Deploying shell integration scripts ({})", marker);

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

    // Deploy the shared muxlog core next to the per-shell dirs. Each rcfile
    // resolves it relative to its own location and delegates to `node muxlog.mjs`.
    let muxlog_path = shell_base.join("muxlog.mjs");
    if let Err(e) = std::fs::write(&muxlog_path, MUXLOG_JS) {
        tracing::warn!("shell integration: failed to write {}: {}", muxlog_path.display(), e);
        all_ok = false;
    }

    // Deploy the shared muxspect core the same way, right next to muxlog.mjs.
    let muxspect_path = shell_base.join("muxspect.mjs");
    if let Err(e) = std::fs::write(&muxspect_path, MUXSPECT_JS) {
        tracing::warn!("shell integration: failed to write {}: {}", muxspect_path.display(), e);
        all_ok = false;
    }

    // Write version marker only if all scripts deployed successfully
    if all_ok {
        let _ = std::fs::write(&version_file, &marker);
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

// wsh has been retired — see docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md.
// The `AGENTMUX` env var is now a plain "1" sentinel, not a path.

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

    /// `muxspect.mjs` deploys next to `muxlog.mjs` — codified as a test since
    /// it's easy to add a new embedded script and forget the deploy line
    /// (see docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md).
    #[test]
    fn deploy_scripts_writes_muxlog_and_muxspect_side_by_side() {
        let tmp = std::env::temp_dir().join(format!(
            "agentmux-shellintegration-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        deploy_scripts(&tmp);

        let shell_base = tmp.join("shell");
        let muxlog = shell_base.join("muxlog.mjs");
        let muxspect = shell_base.join("muxspect.mjs");
        assert!(muxlog.exists(), "muxlog.mjs should be deployed");
        assert!(muxspect.exists(), "muxspect.mjs should be deployed alongside it");
        assert_eq!(std::fs::read_to_string(&muxspect).unwrap(), MUXSPECT_JS);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// codex P2 on PR #2380: a version-only marker left an EXISTING profile
    /// (same CARGO_PKG_VERSION, older content) permanently skipping
    /// deployment of a newly-added script — exactly what an existing
    /// `~/.agentmux` from a prior same-version dev build would have. Content
    /// hashing must force a redeploy in that case, not just on a genuinely
    /// fresh profile (the only case the test above covers).
    #[test]
    fn deploy_scripts_redeploys_when_marker_predates_a_content_change() {
        let tmp = std::env::temp_dir().join(format!(
            "agentmux-shellintegration-test-stale-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let shell_base = tmp.join("shell");
        std::fs::create_dir_all(&shell_base).unwrap();

        // Simulate a profile deployed by an older build of shellintegration.rs
        // that only ever wrote the bare package version as its marker (i.e.
        // BEFORE this fix existed) — the exact "same version, no muxspect.mjs
        // yet" scenario codex's finding describes.
        std::fs::write(shell_base.join(".version"), env!("CARGO_PKG_VERSION")).unwrap();
        assert!(!shell_base.join("muxspect.mjs").exists(), "precondition: not deployed yet");

        deploy_scripts(&tmp);

        assert!(
            shell_base.join("muxspect.mjs").exists(),
            "a stale version-only marker must not suppress deployment of a script that was never written"
        );
        assert_eq!(
            std::fs::read_to_string(shell_base.join(".version")).unwrap(),
            version_marker(),
            "marker must be updated to the new content-hashed form"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
