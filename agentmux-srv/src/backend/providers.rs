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
    /// Sub-directory name for this provider's auth/config dir, e.g. `"claude"`.
    /// Resolved under `shared/providers/<name>/` (the default, account-wide and
    /// instance-independent) and `shared/identities/<bundle>/<name>/` (per-identity)
    /// — see `DataPaths::provider_auth_dir` and `identity_dir`.
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
    // ── Harness & Vendor Decoupling ──────────────────────────────────────────
    /// Harness CLI execution engine name (e.g. `"claude"`, `"agy"`, `"codex"`).
    pub harness_engine: &'static str,
    /// Supported intelligence model vendor names (e.g. `&["anthropic", "openrouter"]`).
    pub supported_vendors: &'static [&'static str],
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
    cli_command: "claude",
    // Persistent (bidirectional stream-json) + the Agent SDK CONTROL PROTOCOL is
    // the only way AskUserQuestion (and interactive tool-permission) works headless.
    // The CLI auto-rejects AskUserQuestion with `Error: Answer questions?` in any
    // mode UNLESS the driver speaks the control protocol: launch with
    // `--permission-prompt-tool stdio` (+ a non-bypass `--permission-mode`), then
    // answer the CLI's `can_use_tool` control_request with a control_response
    // carrying `updatedInput.answers`. See SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md
    // (§2 captured the exact wire bytes against bundled CLI v2.1.178).
    //
    // CRITICAL: `--dangerously-skip-permissions` DISABLES that routing (it bypasses
    // canUseTool), so it must NOT be in persistent_launch_args. The persistent
    // controller's ControlChannel auto-allows ordinary tools to preserve today's
    // yolo UX; only AskUserQuestion is surfaced to the user.
    controller_type: ControllerType::Persistent,
    launch_args: &[
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--dangerously-skip-permissions",
        // Moves per-machine sections (cwd, env info, memory paths, git status)
        // out of the system prompt into the first user message, so the system
        // prompt stays byte-identical across agents/machines and the 1hr
        // prompt cache (docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md) isn't
        // invalidated by per-instance dynamic content. No-op if the default
        // system prompt is overridden — not guaranteed here, since
        // `agent.provider_flags` (app_api/agent_open.rs) is a free-form
        // user-configurable string appended after these args; a user-set
        // `--system-prompt` there would silently no-op this flag. Harmless
        // either way (that's the flag's own documented ignore-if-overridden
        // behavior), just not something this hardcoded default can prevent.
        // reagent P2 on PR #1964.
        "--exclude-dynamic-system-prompt-sections",
    ],
    persistent_launch_args: Some(&[
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--exclude-dynamic-system-prompt-sections",
        // Enable the control protocol so the sidecar can answer can_use_tool /
        // AskUserQuestion. Replaces --dangerously-skip-permissions (which bypasses it).
        "--permission-prompt-tool",
        "stdio",
        "--permission-mode",
        "default",
    ]),
    resume_flag: Some("--resume"),
    session_id_field: "session_id",
    styled_output_format: "claude-stream-json",
    auth_config_dir_env_var: "CLAUDE_CONFIG_DIR",
    auth_dir_name: "claude",
    auth_extra_env: &[],
    unset_env: &["CLAUDECODE"],
    npm_package: "@anthropic-ai/claude-code",
    // Keep in sync with frontend/app/view/agent/providers/index.ts `pinnedVersion`,
    // agentmux-cef/src/commands/providers.rs `CLAUDE_VERSION`, and
    // .github/workflows/container-image.yml `claude_version` default — enforced by
    // frontend/app/view/agent/providers/pin-consistency.test.ts.
    pinned_version: "2.1.198",
    harness_engine: "claude",
    supported_vendors: &["anthropic", "openrouter", "custom"],
};

static CODEX: ProviderConfig = ProviderConfig {
    id: "codex",
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
    harness_engine: "codex",
    supported_vendors: &["openai", "custom"],
};

static GEMINI: ProviderConfig = ProviderConfig {
    id: "gemini",
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
    harness_engine: "gemini",
    supported_vendors: &["google", "custom"],
};

// Qwen Code — Alibaba's open-source coding agent, a fork of Gemini CLI.
// Same stream-json headless surface, so it reuses the Gemini translator.
// Backend is OpenAI-compatible (OPENAI_BASE_URL=https://openrouter.ai/api/v1
// + OPENAI_API_KEY/OPENAI_MODEL), so it runs any OpenRouter model.
static QWEN: ProviderConfig = ProviderConfig {
    id: "qwen",
    cli_command: "qwen",
    controller_type: ControllerType::Subprocess,
    // -p: non-interactive; --output-format stream-json: NDJSON events;
    // --yolo: auto-approve all tools. Mirrors GEMINI (its upstream).
    launch_args: &["--output-format", "stream-json", "--yolo", "-p", ""],
    persistent_launch_args: None,
    // Docs mention --resume <id>/--continue but it's unconfirmed for the
    // headless stream-json path; multi-turn re-runs like Codex/Kimi (None).
    resume_flag: None,
    session_id_field: "session_id",
    // Gemini-CLI fork → same stream-json schema; reuse the gemini translator.
    styled_output_format: "gemini-json",
    // QWEN_HOME relocates the config/credentials dir (default ~/.qwen),
    // the Qwen analogue of GEMINI_CLI_HOME — gives per-agent auth isolation.
    auth_config_dir_env_var: "QWEN_HOME",
    auth_dir_name: "qwen",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@qwen-code/qwen-code",
    pinned_version: "0.19.2",
    harness_engine: "gemini",
    supported_vendors: &["openrouter", "custom"],
};

static KIMI: ProviderConfig = ProviderConfig {
    id: "kimi",
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
    harness_engine: "kimi",
    supported_vendors: &["moonshot", "custom"],
};

static OPENCLAW: ProviderConfig = ProviderConfig {
    id: "openclaw",
    // `openclaw acp` runs OpenClaw's ACP bridge — speaks ACP over stdio
    // for IDE/tool clients (us) and forwards turns to the local
    // OpenClaw Gateway over WebSocket. The Gateway is OpenClaw's own
    // daemon (`openclaw gateway`) and MUST be running before this
    // bridge can establish a session — surfaced to the user as an
    // onboarding requirement in SPEC_OPENCLAW_AGENT_2026_05_17.md §6β.
    //
    // The previous scaffold pointed at `acpx` / `@openclaw/acpx`,
    // which is not a real package. The canonical binary is `openclaw`
    // (npm: `openclaw`) and the ACP subcommand is `openclaw acp`.
    // Verified against docs.openclaw.ai/cli/acp + GitHub README.
    cli_command: "openclaw",
    controller_type: ControllerType::Acp,
    launch_args: &["acp"],
    persistent_launch_args: None,
    // ACP handles sessions natively — no resume flag or session ID parsing needed
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    auth_config_dir_env_var: "OPENCLAW_HOME",
    auth_dir_name: "openclaw",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "openclaw",
    pinned_version: "2026.6.10",
    harness_engine: "openclaw",
    supported_vendors: &["openai", "anthropic", "google", "pi", "custom"],
};

static PI: ProviderConfig = ProviderConfig {
    id: "pi",
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
    pinned_version: "0.73.1",
    harness_engine: "pi",
    supported_vendors: &["pi", "custom"],
};

// Mux Code — AgentMux's first-party agentic coding CLI.
// Local GGUF inference via llama-server or cloud APIs (Anthropic,
// OpenAI, OpenAI-compat). Emits claude-compatible stream-json NDJSON
// (same `session_id` field, same event envelope), so ClaudeTranslator
// handles it without modification.  `--resume <id>` resumes a prior
// session.  npm: `@agentmuxai/muxcode`.
static MUX_CODE: ProviderConfig = ProviderConfig {
    id: "muxcode",
    cli_command: "muxcode",
    controller_type: ControllerType::Subprocess,
    // muxcode emits NDJSON unconditionally; no --output-format flag exists.
    // The `run` subcommand is explicit even though it is Commander's default,
    // so the invocation is unambiguous: `muxcode run -p "<prompt>"`.
    launch_args: &["run", "-p"],
    // muxcode takes a single prompt and exits; persistent mode not supported.
    persistent_launch_args: None,
    resume_flag: Some("--resume"),
    session_id_field: "session_id",
    styled_output_format: "claude-stream-json",
    auth_config_dir_env_var: "MUXCODE_CONFIG_DIR",
    auth_dir_name: "muxcode",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@agentmuxai/muxcode",
    pinned_version: "0.1.0",
    harness_engine: "muxcode",
    supported_vendors: &["ollama", "anthropic", "openai", "custom"],
};

// GitHub Copilot CLI — Microsoft's coding agent. Runs in ACP mode via
// `--acp` so the existing ACP controller drives it. Non-interactive
// `-p`/`--prompt` doesn't accept stdin prompts (github/copilot-cli#96,
// #1046), hence ACP.
static COPILOT: ProviderConfig = ProviderConfig {
    id: "copilot",
    cli_command: "copilot",
    controller_type: ControllerType::Acp,
    launch_args: &["--acp"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    auth_config_dir_env_var: "COPILOT_HOME",
    auth_dir_name: "copilot",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@github/copilot",
    pinned_version: "1.0.65",
    harness_engine: "copilot",
    supported_vendors: &["github", "custom"],
};

// Antigravity (AGY) — Google DeepMind's agentic AI coding CLI harness.
// High-throughput subprocess execution with Gemini 3.6 Flash / 2.5 Pro models,
// native skill discovery, MCP integration, and subagent support.
static ANTIGRAVITY: ProviderConfig = ProviderConfig {
    id: "antigravity",
    cli_command: "agy",
    controller_type: ControllerType::Subprocess,
    launch_args: &["--output-format", "stream-json", "--yolo", "-p", ""],
    persistent_launch_args: None,
    resume_flag: Some("-r"),
    session_id_field: "session_id",
    styled_output_format: "gemini-json",
    auth_config_dir_env_var: "ANTIGRAVITY_CONFIG_DIR",
    auth_dir_name: "antigravity",
    auth_extra_env: &[("ANTIGRAVITY_FORCE_FILE_STORAGE", "true")],
    unset_env: &[],
    npm_package: "@google/antigravity-cli",
    pinned_version: "1.0.0",
    harness_engine: "agy",
    supported_vendors: &["google", "custom"],
};

// ─── Static registry ─────────────────────────────────────────────────────────

static REGISTRY: LazyLock<HashMap<&'static str, &'static ProviderConfig>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert(CLAUDE.id, &CLAUDE);
    m.insert(CODEX.id, &CODEX);
    m.insert(GEMINI.id, &GEMINI);
    m.insert(QWEN.id, &QWEN);
    m.insert(KIMI.id, &KIMI);
    m.insert(OPENCLAW.id, &OPENCLAW);
    m.insert(PI.id, &PI);
    m.insert(COPILOT.id, &COPILOT);
    m.insert(MUX_CODE.id, &MUX_CODE);
    m.insert(ANTIGRAVITY.id, &ANTIGRAVITY);
    m
});

// Aliases for provider IDs from older databases or alternate naming.
static ALIASES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("claude-code", "claude");
    m.insert("claude_code", "claude");
    m.insert("codex-cli", "codex");
    m.insert("gemini-cli", "gemini");
    m.insert("qwen-code", "qwen");
    m.insert("qwen3-coder", "qwen");
    m.insert("kimi-cli", "kimi");
    m.insert("kimi_code", "kimi");
    m.insert("openclaw-cli", "openclaw");
    m.insert("open-claw", "openclaw");
    m.insert("copilot-cli", "copilot");
    m.insert("github-copilot", "copilot");
    m.insert("copilot_cli", "copilot");
    m.insert("mux-code", "muxcode");
    m.insert("mux_code", "muxcode");
    m.insert("agy", "antigravity");
    m.insert("antigravity-cli", "antigravity");
    m.insert("antigravity_cli", "antigravity");
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
        assert!(get_provider("qwen").is_some());
        assert!(get_provider("muxcode").is_some());
        assert!(get_provider("antigravity").is_some());
    }

    #[test]
    fn mux_code_is_subprocess_with_claude_stream_json() {
        let p = get_provider("muxcode").unwrap();
        assert_eq!(p.controller_type, ControllerType::Subprocess);
        assert_eq!(p.controller_type_str(), "subprocess");
        assert_eq!(p.styled_output_format, "claude-stream-json");
        assert_eq!(p.cli_command, "muxcode");
        assert_eq!(p.session_id_field, "session_id");
        assert_eq!(p.resume_flag, Some("--resume"));
        assert_eq!(p.npm_package, "@agentmuxai/muxcode");
        assert_eq!(p.launch_args, &["run", "-p"]);
        assert!(p.persistent_launch_args.is_none());
    }

    #[test]
    fn aliases_resolve() {
        assert_eq!(get_provider("claude-code").unwrap().id, "claude");
        assert_eq!(get_provider("claude_code").unwrap().id, "claude");
        assert_eq!(get_provider("codex-cli").unwrap().id, "codex");
        assert_eq!(get_provider("gemini-cli").unwrap().id, "gemini");
        assert_eq!(get_provider("qwen-code").unwrap().id, "qwen");
        assert_eq!(get_provider("qwen3-coder").unwrap().id, "qwen");
        assert_eq!(get_provider("kimi-cli").unwrap().id, "kimi");
        assert_eq!(get_provider("openclaw-cli").unwrap().id, "openclaw");
        assert_eq!(get_provider("agy").unwrap().id, "antigravity");
        assert_eq!(get_provider("antigravity-cli").unwrap().id, "antigravity");
    }

    #[test]
    fn antigravity_is_subprocess_controller() {
        let p = get_provider("antigravity").unwrap();
        assert_eq!(p.controller_type, ControllerType::Subprocess);
        assert_eq!(p.controller_type_str(), "subprocess");
        assert_eq!(p.styled_output_format, "gemini-json");
        assert_eq!(p.cli_command, "agy");
        assert_eq!(p.session_id_field, "session_id");
        assert_eq!(p.resume_flag, Some("-r"));
        assert_eq!(p.npm_package, "@google/antigravity-cli");
    }

    #[test]
    fn unknown_returns_none() {
        assert!(get_provider("unknown-provider").is_none());
    }

    #[test]
    fn claude_persistent_with_persistent_args_present() {
        let p = get_provider("claude").unwrap();
        // Claude runs on the persistent controller so AskUserQuestion can block
        // on a tool_use and consume a tool_result over live stdin (see the
        // controller_type comment on `static CLAUDE`).
        assert!(p.persistent_launch_args.is_some());
        assert_eq!(p.controller_type, ControllerType::Persistent);
        assert_eq!(p.controller_type_str(), "persistent");
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
