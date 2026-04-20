// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Static provider registry — Rust equivalent of
//! `frontend/app/view/agent/providers/index.ts`.
//!
//! All string data is `&'static str` / `&'static [&'static str]` so lookups
//! are zero-allocation.  The registry is initialised once via `LazyLock` and
//! then read-only for the lifetime of the process.

use std::collections::HashMap;
use std::sync::LazyLock;

// ─── Controller type ─────────────────────────────────────────────────────────

/// How the provider process is managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerType {
    /// A single long-running process; input is streamed to stdin.
    Persistent,
    /// A fresh subprocess is spawned for every turn; prior sessions are
    /// resumed via `resume_flag`.
    Subprocess,
    /// Agent Client Protocol (ACP): JSON-RPC 2.0 over stdio.
    /// Sessions are managed by the protocol — no resume flags needed.
    Acp,
}

// ─── ProviderConfig ──────────────────────────────────────────────────────────

/// All configuration needed to launch, authenticate, and stream output from
/// a provider CLI.
#[derive(Debug)]
pub struct ProviderConfig {
    /// Canonical provider identifier (e.g. `"claude"`).
    pub id: &'static str,
    /// Human-readable name shown in the UI.
    pub display_name: &'static str,
    /// Executable name on PATH (e.g. `"claude"`).
    pub cli_command: &'static str,
    /// Whether the provider keeps a persistent subprocess or spawns per turn.
    pub controller_type: ControllerType,
    /// Complete CLI args for a single-turn (subprocess) invocation.
    /// The user prompt is written to the process's stdin.
    pub launch_args: &'static [&'static str],
    /// CLI args for persistent (long-running) mode.
    /// `None` when `controller_type` is `Subprocess`.
    pub persistent_launch_args: Option<&'static [&'static str]>,
    /// Flag passed to resume a prior session, e.g. `"--resume"`.
    /// `None` when the provider does not support simple-flag resume.
    pub resume_flag: Option<&'static str>,
    /// JSON field name in the CLI's init event that carries the session /
    /// thread ID, e.g. `"session_id"` or `"thread_id"`.
    pub session_id_field: &'static str,
    /// Output format produced by the CLI in styled / streaming mode.
    pub styled_output_format: &'static str,
    // ── Auth isolation ───────────────────────────────────────────────────────
    /// Environment variable that redirects the provider's config / auth
    /// directory, e.g. `"CLAUDE_CONFIG_DIR"`.
    pub auth_config_dir_env_var: &'static str,
    /// Sub-directory name under `{dataDir}/auth/`, e.g. `"claude"`.
    pub auth_dir_name: &'static str,
    /// Extra environment variables required for auth isolation.
    /// Each entry is a `(key, value)` pair.
    pub auth_extra_env: &'static [(&'static str, &'static str)],
    /// Environment variables that must be *unset* before launching the CLI
    /// (guards against nested-session issues, etc.).
    pub unset_env: &'static [&'static str],
    // ── npm install ──────────────────────────────────────────────────────────
    /// npm package name used for local installation, e.g.
    /// `"@anthropic-ai/claude-code"`.
    pub npm_package: &'static str,
    /// Version string passed to `npm install`, e.g. `"latest"` or `"0.116.0"`.
    pub pinned_version: &'static str,
    // ── Misc ─────────────────────────────────────────────────────────────────
    /// Icon identifier used by the frontend.
    pub icon: &'static str,
    /// URL of the provider's documentation.
    pub docs_url: &'static str,
}

impl ProviderConfig {
    /// Return the controller type as the string used in block metadata.
    pub fn controller_type_str(&self) -> &'static str {
        match self.controller_type {
            ControllerType::Persistent => "persistent",
            ControllerType::Subprocess => "subprocess",
            ControllerType::Acp => "acp",
        }
    }
}

// ─── Provider definitions ────────────────────────────────────────────────────

static CLAUDE: ProviderConfig = ProviderConfig {
    id: "claude",
    display_name: "Claude Code",
    cli_command: "claude",
    controller_type: ControllerType::Subprocess,
    launch_args: &[
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--dangerously-skip-permissions",
    ],
    persistent_launch_args: Some(&[
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--dangerously-skip-permissions",
    ]),
    resume_flag: Some("--resume"),
    session_id_field: "session_id",
    styled_output_format: "claude-stream-json",
    auth_config_dir_env_var: "CLAUDE_CONFIG_DIR",
    auth_dir_name: "claude",
    auth_extra_env: &[],
    unset_env: &["CLAUDECODE"],
    npm_package: "@anthropic-ai/claude-code",
    pinned_version: "latest",
    icon: "sparkles",
    docs_url: "https://docs.anthropic.com/claude-code",
};

static CODEX: ProviderConfig = ProviderConfig {
    id: "codex",
    display_name: "Codex CLI",
    cli_command: "codex",
    controller_type: ControllerType::Subprocess,
    // exec subcommand runs non-interactively; --json emits NDJSON events; - reads prompt from stdin
    launch_args: &[
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
        "-",
    ],
    // Codex resume requires a subcommand change (exec resume <id>), not a simple flag.
    // Multi-turn is handled by re-running exec; None disables automatic --resume append.
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "thread_id",
    styled_output_format: "codex-json",
    auth_config_dir_env_var: "CODEX_HOME",
    auth_dir_name: "codex",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@openai/codex",
    pinned_version: "0.116.0",
    icon: "robot",
    docs_url: "https://platform.openai.com/docs/codex",
};

static GEMINI: ProviderConfig = ProviderConfig {
    id: "gemini",
    display_name: "Gemini CLI",
    cli_command: "gemini",
    controller_type: ControllerType::Subprocess,
    // --output-format stream-json: NDJSON events; --yolo: auto-approve all tools;
    // -p "": enable headless/non-interactive mode (prompt comes from stdin)
    launch_args: &["--output-format", "stream-json", "--yolo", "-p", ""],
    persistent_launch_args: None,
    resume_flag: Some("-r"),
    session_id_field: "session_id",
    styled_output_format: "gemini-json",
    auth_config_dir_env_var: "GEMINI_CLI_HOME",
    auth_dir_name: "gemini",
    auth_extra_env: &[("GEMINI_FORCE_FILE_STORAGE", "true")],
    unset_env: &[],
    npm_package: "@google/gemini-cli",
    pinned_version: "0.32.1",
    icon: "diamond",
    docs_url: "https://ai.google.dev/gemini-cli",
};

static KIMI: ProviderConfig = ProviderConfig {
    id: "kimi",
    display_name: "Kimi Code CLI",
    cli_command: "kimi",
    controller_type: ControllerType::Subprocess,
    launch_args: &[
        "--print",
        "--output-format",
        "stream-json",
        "--yolo",
        "-p",
        "",
    ],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "session_id",
    styled_output_format: "kimi-stream-json",
    auth_config_dir_env_var: "KIMI_SHARE_DIR",
    auth_dir_name: "kimi",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "",
    pinned_version: "",
    icon: "moon",
    docs_url: "https://moonshotai.github.io/kimi-cli/",
};

static OPENCLAW: ProviderConfig = ProviderConfig {
    id: "openclaw",
    display_name: "OpenClaw",
    cli_command: "acpx",
    controller_type: ControllerType::Acp,
    launch_args: &["--agent", "openclaw"],
    persistent_launch_args: None,
    // ACP handles sessions natively — no resume flag or session ID parsing needed
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    auth_config_dir_env_var: "OPENCLAW_HOME",
    auth_dir_name: "openclaw",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@openclaw/acpx",
    pinned_version: "latest",
    icon: "lobster",
    docs_url: "https://docs.openclaw.ai",
};

static PI: ProviderConfig = ProviderConfig {
    id: "pi",
    display_name: "Pi",
    cli_command: "pi",
    controller_type: ControllerType::Acp,
    launch_args: &["--json"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    auth_config_dir_env_var: "PI_HOME",
    auth_dir_name: "pi",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@mariozechner/pi-coding-agent",
    pinned_version: "latest",
    icon: "terminal",
    docs_url: "https://github.com/badlogic/pi-mono",
};

// ─── Static registry ─────────────────────────────────────────────────────────

static REGISTRY: LazyLock<HashMap<&'static str, &'static ProviderConfig>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert(CLAUDE.id, &CLAUDE);
    m.insert(CODEX.id, &CODEX);
    m.insert(GEMINI.id, &GEMINI);
    m.insert(KIMI.id, &KIMI);
    m.insert(OPENCLAW.id, &OPENCLAW);
    m.insert(PI.id, &PI);
    m
});

// Aliases for provider IDs from older databases or alternate naming.
static ALIASES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("claude-code", "claude");
    m.insert("claude_code", "claude");
    m.insert("codex-cli", "codex");
    m.insert("gemini-cli", "gemini");
    m.insert("kimi-cli", "kimi");
    m.insert("kimi_code", "kimi");
    m.insert("openclaw-cli", "openclaw");
    m.insert("open-claw", "openclaw");
    m
});

// ─── Public API ──────────────────────────────────────────────────────────────

/// Resolve a provider alias to its canonical ID.
///
/// Returns `id` unchanged if it is not a known alias.
pub fn resolve_provider_alias(id: &str) -> &'static str {
    ALIASES.get(id).copied().unwrap_or_else(|| {
        // If the id itself is a canonical key return the interned key, otherwise
        // return a best-effort static ref. The caller should treat the return
        // value as a lookup key only.
        REGISTRY
            .get_key_value(id)
            .map(|(k, _)| *k)
            .unwrap_or("") // unknown — get_provider will return None
    })
}

/// Look up a provider by canonical ID or alias.
///
/// Returns `None` when the ID (and any resolved alias) does not match a known
/// provider.
pub fn get_provider(id: &str) -> Option<&'static ProviderConfig> {
    // Direct lookup first.
    if let Some(p) = REGISTRY.get(id) {
        return Some(p);
    }
    // Fall back to alias resolution.
    let canonical = ALIASES.get(id).copied()?;
    REGISTRY.get(canonical).copied()
}

/// Return an iterator over all registered providers in insertion order.
pub fn get_provider_list() -> impl Iterator<Item = &'static ProviderConfig> {
    // Stable canonical order matches the TypeScript PROVIDERS object order.
    static ORDER: &[&str] = &["claude", "codex", "gemini", "kimi", "openclaw", "pi"];
    ORDER.iter().filter_map(|id| REGISTRY.get(*id).copied())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ids_resolve() {
        assert!(get_provider("claude").is_some());
        assert!(get_provider("codex").is_some());
        assert!(get_provider("gemini").is_some());
        assert!(get_provider("kimi").is_some());
        assert!(get_provider("openclaw").is_some());
    }

    #[test]
    fn aliases_resolve() {
        assert_eq!(get_provider("claude-code").unwrap().id, "claude");
        assert_eq!(get_provider("claude_code").unwrap().id, "claude");
        assert_eq!(get_provider("codex-cli").unwrap().id, "codex");
        assert_eq!(get_provider("gemini-cli").unwrap().id, "gemini");
        assert_eq!(get_provider("kimi-cli").unwrap().id, "kimi");
        assert_eq!(get_provider("openclaw-cli").unwrap().id, "openclaw");
    }

    #[test]
    fn unknown_returns_none() {
        assert!(get_provider("unknown-provider").is_none());
    }

    #[test]
    fn provider_list_has_six_entries() {
        assert_eq!(get_provider_list().count(), 6);
    }

    #[test]
    fn claude_subprocess_with_persistent_args_present() {
        let p = get_provider("claude").unwrap();
        assert!(p.persistent_launch_args.is_some());
        assert_eq!(p.controller_type, ControllerType::Subprocess);
    }

    #[test]
    fn codex_resume_flag_is_none() {
        let p = get_provider("codex").unwrap();
        assert!(p.resume_flag.is_none());
        assert_eq!(p.controller_type, ControllerType::Subprocess);
    }

    #[test]
    fn gemini_auth_extra_env() {
        let p = get_provider("gemini").unwrap();
        assert!(p
            .auth_extra_env
            .iter()
            .any(|(k, v)| *k == "GEMINI_FORCE_FILE_STORAGE" && *v == "true"));
    }

    #[test]
    fn kimi_is_subprocess_controller() {
        let p = get_provider("kimi").unwrap();
        assert_eq!(p.controller_type, ControllerType::Subprocess);
        assert_eq!(p.controller_type_str(), "subprocess");
        assert_eq!(p.styled_output_format, "kimi-stream-json");
        assert_eq!(p.cli_command, "kimi");
        assert!(p.npm_package.is_empty());
    }

    #[test]
    fn openclaw_is_acp_controller() {
        let p = get_provider("openclaw").unwrap();
        assert_eq!(p.controller_type, ControllerType::Acp);
        assert_eq!(p.controller_type_str(), "acp");
        assert_eq!(p.styled_output_format, "acp");
        assert!(p.resume_flag.is_none());
    }

    #[test]
    fn pi_is_acp_controller() {
        let p = get_provider("pi").unwrap();
        assert_eq!(p.controller_type, ControllerType::Acp);
        assert_eq!(p.controller_type_str(), "acp");
        assert_eq!(p.styled_output_format, "acp");
        assert_eq!(p.cli_command, "pi");
        assert_eq!(p.npm_package, "@mariozechner/pi-coding-agent");
    }
}
