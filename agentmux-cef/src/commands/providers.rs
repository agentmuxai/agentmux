// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Provider management commands for the CEF host.
// Ported from src-tauri/src/commands/providers.rs and cli_installer.rs.
//
// Uses JSON file storage instead of tauri-plugin-store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::state::AppState;

/// Helper to extract the version-specific config dir from AppState.
fn get_config_dir(state: &Arc<AppState>) -> Result<String, String> {
    state
        .version_config_dir
        .lock()
        .clone()
        .ok_or_else(|| "Config dir not initialized yet".to_string())
}

// ---- Types ----

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

fn config_path(config_dir: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::path::PathBuf::from(config_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;
    Ok(dir.join("provider-config.json"))
}

fn load_config(config_dir: &str) -> Result<ProviderConfig, String> {
    let path = config_path(config_dir)?;
    if !path.exists() {
        return Ok(ProviderConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read provider config: {e}"))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse provider config: {e}"))
}

fn save_config(config_dir: &str, config: &ProviderConfig) -> Result<(), String> {
    let path = config_path(config_dir)?;
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize provider config: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write provider config: {e}"))
}

// ---- CLI detection helpers ----

// ---- CLI installer helpers ----

// ---- Command handlers ----

/// Store an auth token for a provider.
pub async fn set_provider_auth(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing token".to_string())?;

    // For Claude (and any CLI-based provider), deliver the auth code directly
    // to the running login process via its stdin. The CLI prints the
    // OAuth URL, then waits for the user to paste the device code on stdin.
    // The stdin is either a piped tokio::process::ChildStdin (typical) or a
    // portable_pty master writer (for TTY-required providers like OpenClaw)
    // — `CliLoginStdin::write_line` dispatches on the variant.
    let maybe_stdin = state.cli_login_stdin.lock().take();
    if let Some(mut child_stdin) = maybe_stdin {
        tracing::info!(provider = %provider, "set_provider_auth: delivering code to CLI stdin");
        if let Err(e) = child_stdin.write_line(&token).await {
            tracing::warn!(error = %e, "set_provider_auth: failed to write to CLI stdin");
            return Err(format!("Failed to deliver auth code to CLI: {e}"));
        }
        // Don't put stdin back — it's single-use (one code per login flow).
        return Ok(serde_json::Value::Null);
    }

    // Fallback for providers that use AgentMux's own config-file auth
    // (non-CLI providers, or when no login process is running).
    tracing::info!("Setting auth token for provider: {}", provider);
    let cfg_dir = get_config_dir(state)?;
    let mut config = load_config(&cfg_dir)?;

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

    save_config(&cfg_dir, &config)?;
    Ok(serde_json::Value::Null)
}

// `seed_provider_auth_from_global` REMOVED 2026-08-31.
//
// It read the user's PERSONAL `~/.claude/.credentials.json` (or
// `$CLAUDE_CONFIG_DIR`) and copied it verbatim — refresh token included — into
// an agent's isolated auth dir. The `INV-R` containment guard it carried only
// ever constrained the DESTINATION (a dir pointing at `~/.claude` was rejected
// so the seed could never WRITE into the user's own env); nothing constrained
// the SOURCE, which was unconditionally the user's personal login.
//
// That made it a per-channel-isolation bypass, and in direct tension with
// `SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md` ("agents never
// use ~/.claude"): that spec blocks BINDING an account whose dir *is*
// `~/.claude`, while this copied the credential OUT of `~/.claude` into a
// compliant dir — same end state, passing the check, because the check tested a
// path rather than the credential's provenance. See
// `docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md` #3.
//
// Terminal login now points every provider's config-dir env var AT its isolated
// dir (as codex/gemini/openclaw/copilot already did), so the login writes there
// directly and no copy-back is needed. Do not reintroduce a source-side global
// read here.

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
