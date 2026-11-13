// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Provider management commands for the CEF host.
// Ported from src-tauri/src/commands/providers.rs and its cli_installer module.
//
// Uses JSON file storage instead of tauri-plugin-store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::commands::cli_login::CliLoginStdin;
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

/// Puts a PTY login's writer back after a code was written to it, so the CLI's
/// input side stays open until the child exits. Only while that login is still
/// the current one (`owner` is the login generation the writer was taken under)
/// and running, and nothing holds the slot: a stale writer left behind would
/// make the next `set_provider_auth` write to a dead PTY instead of falling
/// through to the config-file path, and a superseded login's writer put over a
/// newer login's empty slot would make that login's own restore drop its writer.
/// The login's reaper clears `active` before it clears the slot, so this can't
/// refill the slot after the reap.
fn restore_pty_writer(
    slot: &parking_lot::Mutex<Option<CliLoginStdin>>,
    active: &std::sync::atomic::AtomicBool,
    generation: &std::sync::atomic::AtomicU64,
    owner: u64,
    stdin: CliLoginStdin,
) {
    if !matches!(stdin, CliLoginStdin::Pty(_)) {
        return;
    }
    let mut slot = slot.lock();
    let current = generation.load(std::sync::atomic::Ordering::SeqCst) == owner;
    if current && slot.is_none() && active.load(std::sync::atomic::Ordering::SeqCst) {
        *slot = Some(stdin);
    }
}

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
    let (maybe_stdin, owner) = {
        let mut slot = state.cli_login_stdin.lock();
        (slot.take(), state.cli_login_generation.load(std::sync::atomic::Ordering::SeqCst))
    };
    if let Some(mut child_stdin) = maybe_stdin {
        tracing::info!(provider = %provider, "set_provider_auth: delivering code to CLI stdin");
        if let Err(e) = child_stdin.write_line(&token).await {
            tracing::warn!(error = %e, "set_provider_auth: failed to write to CLI stdin");
            return Err(format!("Failed to deliver auth code to CLI: {e}"));
        }
        // A pipe is single-use: one code per login, and closing it is how the
        // CLI learns there is no more input. A PTY writer must outlive the
        // write: dropping it closes ConPTY's input side, which the CLI reads
        // as a hangup, so it exits (129) before it can exchange a valid code.
        // Put it back; the login's reaper drops it when the child exits.
        restore_pty_writer(&state.cli_login_stdin, &state.cli_login_active, &state.cli_login_generation, owner, child_stdin);
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
    let name = source
        .file_name()
        .ok_or_else(|| "Invalid source path".to_string())?
        .to_string_lossy()
        .into_owned();
    let copy = agentmux_common::copy_into_dir::CopyControl::default();
    let target = agentmux_common::copy_into_dir::copy_into_dir(source, target_dir, &name, &copy)
        .map_err(|e| format!("Copy failed: {e}"))?;

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

#[cfg(test)]
mod login_writer_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    /// A PTY writer that records its bytes and whether it was dropped.
    struct Probe {
        written: Arc<parking_lot::Mutex<Vec<u8>>>,
        dropped: Arc<AtomicBool>,
    }

    impl std::io::Write for Probe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.lock().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn probe() -> (CliLoginStdin, Arc<parking_lot::Mutex<Vec<u8>>>, Arc<AtomicBool>) {
        let written = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let w = Probe { written: written.clone(), dropped: dropped.clone() };
        (CliLoginStdin::Pty(Box::new(w)), written, dropped)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_delivered_code_does_not_hang_up_the_login_child() {
        let (mut stdin, written, dropped) = probe();
        stdin.write_line("the-code").await.unwrap();
        let slot = parking_lot::Mutex::new(None);
        let active = AtomicBool::new(true);

        restore_pty_writer(&slot, &active, &AtomicU64::new(3), 3, stdin);

        assert_eq!(written.lock().as_slice(), b"the-code\n");
        assert!(!dropped.load(Ordering::SeqCst), "the writer must outlive the code delivery");
        assert!(slot.lock().is_some(), "the writer is back in the slot for the reaper");
        *slot.lock() = None;
        assert!(dropped.load(Ordering::SeqCst), "the reaper's clear drops it");
    }

    #[test]
    fn a_writer_of_a_finished_login_is_not_kept() {
        let (stdin, _written, dropped) = probe();
        let slot = parking_lot::Mutex::new(None);
        restore_pty_writer(&slot, &AtomicBool::new(false), &AtomicU64::new(1), 1, stdin);
        assert!(slot.lock().is_none());
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn a_superseded_logins_writer_is_not_put_into_the_newer_logins_empty_slot() {
        // The code was written under login generation 4; a newer login (5) has started and has not stored its writer yet.
        let (old, _written, old_dropped) = probe();
        let slot = parking_lot::Mutex::new(None);
        restore_pty_writer(&slot, &AtomicBool::new(true), &AtomicU64::new(5), 4, old);
        assert!(slot.lock().is_none(), "the slot stays free for the newer login's own writer");
        assert!(old_dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn a_newer_logins_writer_is_not_replaced() {
        let (newer, _w, newer_dropped) = probe();
        let slot = parking_lot::Mutex::new(Some(newer));
        let (old, _w2, old_dropped) = probe();
        restore_pty_writer(&slot, &AtomicBool::new(true), &AtomicU64::new(1), 1, old);
        assert!(old_dropped.load(Ordering::SeqCst));
        assert!(!newer_dropped.load(Ordering::SeqCst));
        assert!(slot.lock().is_some());
    }
}
