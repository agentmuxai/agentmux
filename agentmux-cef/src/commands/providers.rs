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
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::state::AppState;

/// Helper to extract the version-specific config dir from AppState.
fn get_config_dir(state: &Arc<AppState>) -> Result<String, String> {
    state
        .version_config_dir
        .lock()
        .clone()
        .ok_or_else(|| "Config dir not initialized yet".to_string())
}

/// Helper to extract the version-specific data dir from AppState.
fn get_data_dir(state: &Arc<AppState>) -> Result<String, String> {
    state
        .version_data_dir
        .lock()
        .clone()
        .ok_or_else(|| "Data dir not initialized yet".to_string())
}

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

// INFORMATIONAL ONLY — for display in setup/toolchain UI.
// INV-X (SPEC_PROVIDER_ISOLATION): the path returned here must NEVER be used as
// an agent run target. Agents run ONLY the AgentMux-installed versioned binary
// under ~/.agentmux/.../cli/<provider>/. If the UI needs to detect a usable CLI,
// it must call the srv `ResolveCli` RPC instead (which installs if absent).
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

// Pinned CLI versions — update when a new version is validated.
// INV-X (SPEC_PROVIDER_ISOLATION): agents MUST run the AgentMux-installed,
// version-pinned binary; "latest" is never acceptable here because it bypasses
// the repeatable-install guarantee and could pull in a breaking CLI version.
// Keep in sync with agentmux-srv/src/backend/providers.rs `pinned_version`,
// frontend/app/view/agent/providers/index.ts `pinnedVersion`, and
// .github/workflows/container-image.yml `claude_version` default — enforced by
// frontend/app/view/agent/providers/pin-consistency.test.ts.
const CLAUDE_VERSION: &str = "2.1.247";
const CODEX_VERSION: &str = "0.116.0";
const GEMINI_VERSION: &str = "0.32.1";

fn get_provider_install_dir(data_dir: &str, provider: &str) -> Result<std::path::PathBuf, String> {
    Ok(std::path::PathBuf::from(data_dir)
        .join("cli")
        .join(provider))
}

fn get_local_cli_bin_path(data_dir: &str, provider: &str) -> Result<std::path::PathBuf, String> {
    let install_dir = get_provider_install_dir(data_dir, provider)?;
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
pub fn get_provider_config(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let config = load_config(&get_config_dir(state)?)?;
    serde_json::to_value(&config).map_err(|e| format!("Serialize error: {e}"))
}

/// Save the provider configuration.
pub fn save_provider_config(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let config: ProviderConfig = serde_json::from_value(
        args.get("config").cloned().unwrap_or(args.clone()),
    )
    .map_err(|e| format!("Failed to parse config: {e}"))?;

    tracing::info!(
        "Saving provider config: default={}, setup_complete={}",
        config.default_provider,
        config.setup_complete
    );
    save_config(&get_config_dir(state)?, &config)?;
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

/// Seed an agent's ISOLATED provider auth dir from the user's GLOBAL CLI login.
///
/// The reliable recovery when the host can't drive a provider's OAuth TUI
/// (Claude Code v2.1.x opens its own browser + localhost callback and never
/// prints a scrapeable URL — see `SPEC_HOST_CLI_LOGIN_CAPTURE` §5.5): if the
/// user already has a valid GLOBAL Claude login, copy it verbatim into the
/// agent's isolated dir, which the spawned CLI reads via `CLAUDE_CONFIG_DIR`.
/// The copy keeps its `refreshToken`, so the isolated session keeps refreshing.
///
/// - GLOBAL source: `$CLAUDE_CONFIG_DIR/.credentials.json` when the user set
///   that in their own shell env (the host inherits it), else
///   `~/.claude/.credentials.json` (Anthropic's documented default). This is the
///   user's real login — NOT the agent's isolated dir, which is the destination.
/// - ISOLATED destination: `~/.agentmux/shared/providers/<provider>/.credentials.json`
///   (`state.user_home_dir` + `shared/providers/<provider>`), matching
///   `ensure_auth_dir`.
///
/// Validity is gated HERE (the frontend can't read the global dir): we only
/// seed when `claudeAiOauth.expiresAt` is in the future — mirrors
/// `agentmux-srv` `identity::resolver::probe_oauth_status`. Returns
/// `{ seeded, status, expiresAt }` so the UI can explain a no-op without ever
/// seeing token material — `status`: `seeded` | `missing` | `expired`.
pub fn seed_provider_auth_from_global(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .or_else(|| args.get("provider_id"))
        .or_else(|| args.get("providerId"))
        .and_then(|v| v.as_str())
        .unwrap_or("claude");

    // Only Claude has both a documented global location and a credential shape
    // we can validate. Reject others explicitly rather than copy blind.
    if provider != "claude" {
        return Err(format!(
            "seed-from-global is only supported for 'claude' (got '{provider}')"
        ));
    }

    // GLOBAL source — the user's own login. Honour a user-level
    // CLAUDE_CONFIG_DIR (the host inherits it) else the documented `~/.claude`.
    let global_dir = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
        .ok_or_else(|| "Could not resolve the global Claude config dir".to_string())?;
    let global_cred = global_dir.join(".credentials.json");

    let contents = match std::fs::read_to_string(&global_cred) {
        Ok(s) => s,
        Err(_) => {
            tracing::info!(
                target: "login_pty",
                path = %global_cred.to_string_lossy(),
                "seed_provider_auth_from_global: no global credential to seed"
            );
            return Ok(serde_json::json!({ "seeded": false, "status": "missing" }));
        }
    };

    // Validate non-expired before seeding (claude shape: claudeAiOauth.expiresAt, ms).
    let json: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|_| "Global Claude credential is not valid JSON".to_string())?;
    let expires_at_ms = json
        .get("claudeAiOauth")
        .and_then(|o| o.get("expiresAt"))
        .and_then(|v| v.as_i64())
        .or_else(|| json.get("expiresAt").and_then(|v| v.as_i64()));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Some(exp) = expires_at_ms {
        if exp <= now_ms {
            tracing::info!(
                target: "login_pty",
                expires_at = exp,
                "seed_provider_auth_from_global: global credential expired — not seeding"
            );
            return Ok(
                serde_json::json!({ "seeded": false, "status": "expired", "expiresAt": exp }),
            );
        }
    }

    // ISOLATED destination (SPEC_PROVIDER_ISOLATION §4.5). Prefer the agent's
    // RESOLVED config dir (passed as `config_dir` from its `cmd:env`), so a
    // per-identity/bundle agent is seeded into the dir it actually reads — but
    // ONLY when that dir is under the AgentMux home (`~/.agentmux`). A stale
    // frozen dir pointing at the user's own `~/.claude` is REJECTED → fall back
    // to the shared default, so the seed can NEVER write into the user's
    // personal env (INV-R). Default agents resolve to the shared dir, which
    // matches their post-migration binding.
    let home = state
        .user_home_dir
        .lock()
        .clone()
        .ok_or_else(|| "User home dir not initialized yet".to_string())?;
    let home_path = std::path::PathBuf::from(&home);
    let shared_default = home_path.join("shared").join("providers").join(provider);
    let requested = args
        .get("config_dir")
        .or_else(|| args.get("configDir"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from);
    let dest_dir = match requested {
        // Accept the agent's resolved dir only if it's inside `~/.agentmux`
        // (covers `shared/providers/*` and `shared/identities/*` bundle dirs).
        Some(d) if d.starts_with(&home_path) => d,
        // Anything else (incl. the user's `~/.claude`) → shared default.
        _ => shared_default,
    };
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create isolated auth dir: {e}"))?;
    let dest_cred = dest_dir.join(".credentials.json");

    // Write verbatim via temp + rename so a concurrent reader (the agent's CLI)
    // never sees a half-written credential.
    let tmp = dest_dir.join(".credentials.json.seed-tmp");
    std::fs::write(&tmp, contents.as_bytes())
        .map_err(|e| format!("Failed to write seeded credential: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &dest_cred)
        .map_err(|e| format!("Failed to finalize seeded credential: {e}"))?;

    tracing::info!(
        target: "login_pty",
        provider,
        dest = %dest_cred.to_string_lossy(),
        "seed_provider_auth_from_global: seeded isolated dir from valid global login"
    );
    Ok(serde_json::json!({ "seeded": true, "status": "seeded", "expiresAt": expires_at_ms }))
}

/// Clear auth token for a provider.
pub fn clear_provider_auth(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;

    tracing::info!("Clearing auth token for provider: {}", provider);
    let cfg_dir = get_config_dir(state)?;
    let mut config = load_config(&cfg_dir)?;

    if let Some(settings) = config.providers.get_mut(provider) {
        settings.auth_token = None;
        settings.auth_status = "none".to_string();
    }

    save_config(&cfg_dir, &config)?;
    Ok(serde_json::Value::Null)
}

/// Get auth status for a provider.
pub fn get_provider_auth_status(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;

    let config = load_config(&get_config_dir(state)?)?;
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
pub fn get_cli_path(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?;

    let local_path = get_local_cli_bin_path(&get_data_dir(state)?, provider)?;
    if local_path.exists() {
        tracing::info!("Found {} in isolated install: {}", provider, local_path.display());
        return Ok(serde_json::json!(local_path.to_string_lossy()));
    }

    tracing::info!("{} CLI not found in isolated install", provider);
    Ok(serde_json::Value::Null)
}

/// Install a provider CLI via npm.
pub async fn install_cli(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing provider".to_string())?
        .to_string();

    let data_dir = get_data_dir(state)?;
    let local_path = get_local_cli_bin_path(&data_dir, &provider)?;
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
    let data_dir_clone = data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let npm_package = get_npm_package(&provider_clone)?;
        let pinned_version = get_pinned_version(&provider_clone)?;
        let install_dir = get_provider_install_dir(&data_dir_clone, &provider_clone)?;

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

        let cli_path = get_local_cli_bin_path(&data_dir_clone, &provider_clone)?;
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
