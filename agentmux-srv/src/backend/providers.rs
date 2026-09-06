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
    /// Sub-directory (relative to this provider's config/auth dir) that
    /// actually holds its native session/transcript history — e.g.
    /// Claude's `"projects"`, Codex's `"sessions"`, Gemini's `"history"`.
    /// `None` for providers with no simple directory-based history to
    /// isolate/link (ACP providers — openclaw/pi/copilot — expose history
    /// through the live protocol stream instead, not a fixed on-disk
    /// path). Per-provider, filesystem-verified table:
    /// `docs/specs/SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md` §2.1.
    /// Used by `identity_auth_dirs.rs` to redirect an isolated identity's
    /// history to the always-global
    /// `DataPaths::identity_history_dir` — reagentx P1 on PR #2605 caught
    /// that hardcoding `"projects"` there only worked for `claude`,
    /// silently leaving every other OAuth-class provider's history
    /// channel-local and unlinked.
    pub history_native_subdir: Option<&'static str>,
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
    // ── Model vendor (harness vs. model vendor decoupling) ──────────────────
    /// Environment variable this harness reads to redirect its model vendor
    /// backend (e.g. Claude Code's `ANTHROPIC_BASE_URL`, pointing it at a
    /// proxy/Bedrock/OpenRouter instead of Anthropic's default endpoint).
    /// `None` when a harness isn't confirmed to support redirection — the
    /// harness (CLI driving the session) and the model vendor (LLM backend
    /// actually serving responses) are independent dimensions; this is the
    /// declared capability side of that split. Set only where independently
    /// verified, not guessed. See docs/specs for the harness/vendor concept.
    pub base_url_env_var: Option<&'static str>,
    /// Intelligence model vendors this harness talks to, most-default-first
    /// (e.g. claude → `&["anthropic"]`, openclaw → `&["openai", "anthropic",
    /// "google"]` since it's model-agnostic). Purely descriptive/display
    /// data — drives the dual-icon vendor badge and the picker's default
    /// vendor inference (`resolveEffectiveVendor` on the frontend); does not
    /// gate anything at spawn time. The one thing that DOES gate spawn-time
    /// behavior is `base_url_env_var` above — a provider can be redirected
    /// to a custom endpoint independent of whether that endpoint's vendor
    /// is even listed here.
    pub supported_vendors: &'static [&'static str],
    /// Path (relative to the agent's working directory) this provider
    /// natively auto-discovers its startup instructions from — e.g.
    /// `"CLAUDE.md"`, `"AGENTS.md"`, `".pi/APPEND_SYSTEM.md"`. `None` when
    /// no native file-based convention is confirmed to exist (currently
    /// only `kimi`) — `build_config_files` skips writing the instructions
    /// file entirely in that case rather than writing inert content nobody
    /// reads. Set only where independently verified against the provider's
    /// own docs, not guessed — same discipline as `base_url_env_var` above.
    /// See docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    pub startup_instructions_filename: Option<&'static str>,
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

    /// Return the provider-specific continuation argv strategy stored in pane
    /// metadata. Codex uses an `exec resume` subcommand; existing providers
    /// append their configured flag.
    pub fn resume_strategy_str(&self) -> &'static str {
        if self.id == "codex" {
            "codex-exec"
        } else if self.resume_flag.is_some() {
            "flag"
        } else {
            "none"
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
    history_native_subdir: Some("projects"),
    auth_extra_env: &[],
    unset_env: &["CLAUDECODE"],
    npm_package: "@anthropic-ai/claude-code",
    // Keep in sync with frontend/app/view/agent/providers/index.ts `pinnedVersion`,
    // agentmux-cef/src/commands/providers.rs `CLAUDE_VERSION`, and
    // .github/workflows/container-image.yml `claude_version` default — enforced by
    // frontend/app/view/agent/providers/pin-consistency.test.ts.
    pinned_version: "2.1.263",
    // Documented Claude Code behavior: redirects the CLI at a non-Anthropic
    // (or proxied) backend — Bedrock, Vertex, OpenRouter, a custom proxy.
    base_url_env_var: Some("ANTHROPIC_BASE_URL"),
    supported_vendors: &["anthropic"],
    startup_instructions_filename: Some("CLAUDE.md"),
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
    history_native_subdir: Some("sessions"),
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@openai/codex",
    pinned_version: "0.153.4",
    base_url_env_var: None,
    supported_vendors: &["openai"],
    // Confirmed: SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md §10.2 —
    // "Codex's native project instruction discovery continues to load
    // user/repository AGENTS.md files normally."
    startup_instructions_filename: Some("AGENTS.md"),
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
    history_native_subdir: Some("history"),
    auth_extra_env: &[("GEMINI_FORCE_FILE_STORAGE", "true")],
    unset_env: &[],
    npm_package: "@google/gemini-cli",
    pinned_version: "0.58.0",
    base_url_env_var: None,
    supported_vendors: &["google"],
    // Confirmed: Gemini CLI docs — context files default to GEMINI.md;
    // AGENTS.md support exists but requires an explicit contextFileName
    // override, so GEMINI.md is the correct default-behavior target. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: Some("GEMINI.md"),
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
    // Not OAuth-class (provider_class() has no arm for "qwen") -- never
    // reaches ensure_history_link regardless. None documents "not yet
    // characterized" rather than asserting a specific layout.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@qwen-code/qwen-code",
    pinned_version: "0.23.0",
    // Note: qwen's default backend already routes through an OpenAI-compatible
    // endpoint (see comment above) as a fixed part of its auth setup — but
    // that's baked-in default routing, not a confirmed user-configurable
    // override mechanism, so this stays unset rather than guessed.
    base_url_env_var: None,
    supported_vendors: &["openrouter"],
    // Confirmed: Qwen Code settings docs — QWEN.md is a built-in default
    // contextFileName; AGENTS.md needs explicit config (open feature
    // requests QwenLM/qwen-code#2006, #504 ask for it to become default,
    // confirming it isn't yet). See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: Some("QWEN.md"),
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
    // API-key-class, not OAuth-class -- never reaches ensure_history_link.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "",
    pinned_version: "",
    base_url_env_var: None,
    supported_vendors: &["moonshot"],
    // Confirmed absence: docs/specs/KIMI_PROVIDER_INTEGRATION_SPEC.md
    // already researched this — Kimi has no auto-read markdown convention
    // ("does not appear to auto-read CLAUDE.md... skip KIMI.md
    // generation"). Re-verified 2026-08-24: Kimi's only file-based prompt
    // customization is --agent-file <yaml> with a system_prompt_path field
    // (a CLI flag this provider's launch_args above doesn't pass), and an
    // open, unshipped feature request (MoonshotAI/kimi-cli#1856) for a
    // project-level system_prompt.md override. Writing any markdown file
    // here would be inert output nobody reads — None, not a guessed
    // filename. build_config_files skips the instructions file entirely
    // for a provider with None here. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: None,
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
    // ACP-native: exposes history through the live protocol stream, not
    // a fixed on-disk directory -- nothing for ensure_history_link to
    // redirect. Per SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md §4.2.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "openclaw",
    pinned_version: "2026.9.2",
    base_url_env_var: None,
    supported_vendors: &["openai", "anthropic", "google"],
    // Confirmed convention, UNCONFIRMED path: docs.openclaw.ai/reference/AGENTS.default
    // — AGENTS.md is a required bootstrap file OpenClaw itself creates,
    // read from each agent's workspace at session start. Whether that
    // workspace maps 1:1 onto AgentMux's own working_directory for an
    // ACP-bridged `openclaw acp` session (vs. OpenClaw's own
    // Gateway-daemon-managed sandbox) is not independently verified here —
    // best-effort root-level AGENTS.md, flagged as a known gap. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2, §6.
    startup_instructions_filename: Some("AGENTS.md"),
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
    // Not OAuth-class -- never reaches ensure_history_link. Also
    // ACP-native (see the "openclaw" entry above) if that changes.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@mariozechner/pi-coding-agent",
    pinned_version: "0.73.1",
    base_url_env_var: None,
    supported_vendors: &["pi"],
    // Confirmed: npmjs.com/package/@mariozechner/pi-coding-agent docs —
    // .pi/SYSTEM.md REPLACES pi's default system prompt; .pi/APPEND_SYSTEM.md
    // APPENDS to it. AgentMux's Soul+AgentMD+Memory content is additive
    // background, not a full system-prompt replacement (pi's own default
    // prompt carries pi's own tool-usage instructions) — APPEND_SYSTEM.md
    // is the correct target, not SYSTEM.md. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: Some(".pi/APPEND_SYSTEM.md"),
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
    // Not OAuth-class -- never reaches ensure_history_link.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@agentmuxai/muxcode",
    pinned_version: "0.1.0",
    base_url_env_var: None,
    supported_vendors: &["ollama", "anthropic", "openai"],
    // Confirmed by design intent: this provider's own doc comment above
    // states it "emits claude-compatible stream-json NDJSON... handled
    // without modification [by ClaudeTranslator]" — a deliberate
    // compatibility choice by the same team that owns both, not a guess.
    startup_instructions_filename: Some("CLAUDE.md"),
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
    // ACP-native, same as "openclaw" above -- no fixed on-disk history
    // directory to redirect.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@github/copilot",
    pinned_version: "1.0.83",
    base_url_env_var: None,
    supported_vendors: &["github"],
    // Confirmed: GitHub Copilot CLI custom-instructions docs — supports
    // AGENTS.md (root, single-file, no subdirectory needed) alongside
    // .github/copilot-instructions.md, CLAUDE.md, GEMINI.md. AGENTS.md
    // chosen as the one canonical target since Copilot has no single
    // privileged default the way Gemini/Qwen do. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: Some("AGENTS.md"),
};

// Antigravity (AGY) — Google's agentic coding CLI harness. Emits the same
// stream-json NDJSON envelope as Gemini CLI (its sibling harness), so it
// reuses the gemini translator (styled_output_format "gemini-json") rather
// than a new one. No base_url_env_var — not independently verified to
// support a custom-endpoint override (same "unset unless confirmed"
// discipline as every other non-claude provider above).
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
    // Not OAuth-class -- never reaches ensure_history_link. Not covered
    // by SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md (added later);
    // None until independently verified rather than guessed.
    history_native_subdir: None,
    auth_extra_env: &[("ANTIGRAVITY_FORCE_FILE_STORAGE", "true")],
    unset_env: &[],
    npm_package: "@google/antigravity-cli",
    pinned_version: "1.0.0",
    base_url_env_var: None,
    supported_vendors: &["google"],
    // INFERRED, not independently doc-confirmed: Antigravity CLI's own
    // settings live at ~/.gemini/antigravity-cli/settings.json — same
    // ~/.gemini/ namespace root as Gemini CLI itself, consistent with this
    // provider's own doc comment above (shares Gemini CLI's NDJSON
    // schema). No explicit Antigravity docs page independently confirms
    // GEMINI.md context-file behavior. Flagged as a known gap. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2, §6.
    startup_instructions_filename: Some("GEMINI.md"),
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

/// True when `dir` resolves to `provider`'s literal ambient home directory
/// (e.g. `~/.claude` for Claude Code) rather than an AgentMux-isolated dir.
/// Used to block identity bindings/spawns from ever pointing a spawned
/// agent at the operator's own global CLI login — a real, currently-live
/// instance of exactly this was found in this repo's own data (an account's
/// `secret_ref.dir` set to the literal ambient path — see
/// `docs/status/STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md`
/// §8), which nothing in the codebase previously validated against. See
/// `docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`.
///
/// Canonicalizes both sides when possible (defeats `..`, symlink, and
/// Windows `\\?\`-prefix tricks — same approach `identity::cleanup`'s
/// containment check already uses); falls back to a normalized lexical
/// comparison when either path doesn't exist yet on disk (e.g. validating a
/// not-yet-materialized dir at account-creation time), so a not-yet-created
/// ambient path can't slip past the guard just by not existing at check
/// time.
pub fn is_provider_ambient_home_dir(provider: &ProviderConfig, dir: &str) -> bool {
    let ambient = crate::backend::base::get_home_dir().join(format!(".{}", provider.auth_dir_name));
    paths_resolve_to_same_dir(&ambient, dir)
}

/// Placeholder written by `seed_claude_md_placeholder_if_missing` — explains
/// itself so an operator who opens the file isn't confused by an unexplained
/// empty one.
const CLAUDE_MD_ISOLATION_PLACEHOLDER: &str = "<!--\n\
AgentMux: intentionally empty.\n\
\n\
This is an isolated Claude Code config directory (CLAUDE_CONFIG_DIR),\n\
separate from your personal ~/.claude/CLAUDE.md, so this agent never\n\
silently inherits your personal global instructions. To give every\n\
agent shared instructions, use Armory -> Memory -> Global instead --\n\
those compose into this agent's own project-level CLAUDE.md at launch,\n\
not this file. See SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md.\n\
-->\n";

/// Seeds an empty placeholder `CLAUDE.md` into an isolated Claude Code
/// `CLAUDE_CONFIG_DIR` the first time it's used, so Claude Code CLI's own
/// user-level `CLAUDE.md` discovery always finds *something* there instead
/// of silently falling through to the operator's real
/// `$HOME/.claude/CLAUDE.md`. `CLAUDE_CONFIG_DIR` relocates credential/
/// session/project storage but AgentMux never wrote a `CLAUDE.md` into the
/// relocated dir — verified live on a real machine: 18+ isolated `claude`
/// config dirs, none with a `CLAUDE.md`, and a session whose
/// `CLAUDE_CONFIG_DIR` was a genuinely separate (non-ambient) dir still
/// received the real host file's contents. See
/// `docs/specs/SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md`.
///
/// Scoped to the `claude` provider only (`auth_dir_name == "claude"`) —
/// this is a Claude Code CLI-specific fallback behavior; whether any other
/// provider's CLI has an equivalent gap is unverified. Never overwrites an
/// existing `CLAUDE.md` — Global Memory (or anything else) may have
/// legitimately placed real content at that path. Returns `Ok(true)` only
/// when it actually wrote the placeholder.
pub fn seed_claude_md_placeholder_if_missing(
    provider: &ProviderConfig,
    config_dir: &str,
) -> std::io::Result<bool> {
    if provider.auth_dir_name != "claude" {
        return Ok(false);
    }
    // Codex P2 on PR #2854: a blank config_dir would resolve
    // `Path::new("").join("CLAUDE.md")` relative to the server process's
    // own CWD (`create_dir_all("")` succeeds, silently no-op) instead of
    // erroring — writing an unrelated file while leaving the actual
    // (empty-string) CLAUDE_CONFIG_DIR still exposed to ambient fallback.
    // Reject before touching the filesystem so the caller's fail-closed
    // handling (both call sites now block spawn on any Err here) covers
    // this case too, rather than silently succeeding at nothing.
    if config_dir.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config_dir is empty",
        ));
    }
    let path = std::path::Path::new(config_dir).join("CLAUDE.md");
    if path.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(&path, CLAUDE_MD_ISOLATION_PLACEHOLDER)?;
    Ok(true)
}

/// Prepare an isolated provider auth/config directory for use as
/// `CLAUDE_CONFIG_DIR` (or a provider's equivalent): create it, then apply
/// every isolation guarantee that directory needs before a CLI is pointed
/// at it.
///
/// Exists so callers cannot obtain a usable auth dir WITHOUT its isolation
/// guarantees — the spawn paths call this instead of `create_dir_all` +
/// a separately-skippable seed step. Deleting the isolation from a spawn
/// path now means deleting the directory preparation too, which fails
/// loudly instead of silently reopening the leak. See
/// `docs/specs/SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` and
/// `docs/reports/REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md`
/// (the live three-arm experiment proving the leak is real and this closes
/// it).
pub fn prepare_provider_auth_dir(
    provider: &ProviderConfig,
    auth_dir: &str,
) -> std::io::Result<()> {
    if auth_dir.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "auth_dir is empty",
        ));
    }
    std::fs::create_dir_all(auth_dir)?;
    seed_claude_md_placeholder_if_missing(provider, auth_dir)?;
    Ok(())
}

/// The actual comparison, split out with the reference side pre-resolved
/// (not calling `get_home_dir()` internally) so it's directly testable
/// against a tempdir instead of the real `$HOME`/`%USERPROFILE%` — same
/// "inject the path, don't resolve it internally" pattern
/// `read_claude_global_config` already uses (`agent_handlers/memory.rs`).
fn paths_resolve_to_same_dir(reference: &std::path::Path, candidate: &str) -> bool {
    let candidate_path = std::path::Path::new(candidate);

    if let (Ok(canon_reference), Ok(canon_candidate)) =
        (std::fs::canonicalize(reference), std::fs::canonicalize(candidate_path))
    {
        return canon_reference == canon_candidate;
    }

    normalize_path_lexically(reference) == normalize_path_lexically(candidate_path)
}

/// Lexically normalizes a path with NO filesystem access (unlike
/// `canonicalize`, safe to call on a path that doesn't exist). Collapses
/// `.` and `..` components (codex P1, PR #2802: without this, a not-yet-
/// created dir like `$HOME/.claude/.` compared unequal to `$HOME/.claude`
/// under plain string comparison, bypassing the guard entirely on a fresh
/// machine — a `..` segment is normalized the same way for the same
/// reason). Never pops past a root/prefix/leading `..` — a leading `..`
/// with nothing to cancel stays literal, standard lexical-normalization
/// semantics.
fn normalize_path_lexically(p: &std::path::Path) -> String {
    use std::path::Component;

    let mut normalized: Vec<Component> = Vec::new();
    for component in p.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(normalized.last(), Some(Component::Normal(_))) {
                    normalized.pop();
                } else {
                    normalized.push(component);
                }
            }
            other => normalized.push(other),
        }
    }
    let joined: std::path::PathBuf = normalized.into_iter().collect();
    let as_str = joined.to_string_lossy().replace('\\', "/");
    let trimmed = as_str.trim_end_matches('/');

    // codex P2, PR #2802: case-fold only on platforms whose default
    // filesystem is case-insensitive (Windows, macOS). Unconditionally
    // lowercasing made a not-yet-created `$HOME/.CLAUDE` compare equal to
    // `$HOME/.claude` on Linux (case-sensitive ext4) — once both existed,
    // canonicalize would correctly tell them apart, so the guard's
    // verdict depended on creation order instead of being deterministic.
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// Whether `path` matches ANY registered provider's
/// `startup_instructions_filename` — used by the "click Launch" RPC write
/// path (`editor_handlers.rs`'s `WriteAgentConfig` handler), which receives
/// an already-built file list from the frontend with no accompanying
/// provider ID, so it can't otherwise tell "this is the startup
/// instructions file" from "this is some other config file" without either
/// re-deriving the mapping itself (drift risk) or checking membership here
/// (single source of truth). See
/// docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §3.
pub fn is_known_startup_instructions_filename(path: &str) -> bool {
    REGISTRY
        .values()
        .any(|p| p.startup_instructions_filename == Some(path))
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
    fn antigravity_is_subprocess_with_gemini_stream_json() {
        let p = get_provider("antigravity").unwrap();
        assert_eq!(p.controller_type, ControllerType::Subprocess);
        assert_eq!(p.controller_type_str(), "subprocess");
        assert_eq!(p.styled_output_format, "gemini-json");
        assert_eq!(p.cli_command, "agy");
        assert_eq!(p.session_id_field, "session_id");
        assert_eq!(p.resume_flag, Some("-r"));
        assert_eq!(p.npm_package, "@google/antigravity-cli");
        assert_eq!(p.supported_vendors, &["google"]);
        assert!(p.base_url_env_var.is_none());
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
    fn every_provider_declares_at_least_one_supported_vendor() {
        for id in ["claude", "codex", "gemini", "qwen", "kimi", "openclaw", "pi", "muxcode", "copilot", "antigravity"] {
            let p = get_provider(id).unwrap_or_else(|| panic!("provider '{id}' not registered"));
            assert!(
                !p.supported_vendors.is_empty(),
                "provider '{id}' declares no supported_vendors"
            );
        }
    }

    #[test]
    fn claude_default_vendor_is_anthropic() {
        assert_eq!(get_provider("claude").unwrap().supported_vendors, &["anthropic"]);
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
        assert_eq!(get_provider("antigravity_cli").unwrap().id, "antigravity");
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
    fn codex_uses_exec_resume_strategy() {
        let p = get_provider("codex").unwrap();
        assert!(p.resume_flag.is_none());
        assert_eq!(p.controller_type, ControllerType::Subprocess);
        assert_eq!(p.resume_strategy_str(), "codex-exec");
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

    // Harness vs. model vendor decoupling: only claude has a verified,
    // documented way to redirect its backend (ANTHROPIC_BASE_URL). Every
    // other provider is explicit None rather than a guessed env var name.
    #[test]
    fn claude_declares_base_url_env_var() {
        let p = get_provider("claude").unwrap();
        assert_eq!(p.base_url_env_var, Some("ANTHROPIC_BASE_URL"));
    }

    #[test]
    fn non_claude_providers_have_no_base_url_env_var() {
        for id in ["codex", "gemini", "qwen", "kimi", "openclaw", "pi", "copilot", "muxcode", "antigravity"] {
            let p = get_provider(id).unwrap();
            assert!(
                p.base_url_env_var.is_none(),
                "provider '{id}' should not declare base_url_env_var yet (unverified)"
            );
        }
    }

    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2's
    // per-provider table, pinned so a future edit can't silently drift from
    // the researched/cited values.
    #[test]
    fn startup_instructions_filename_matches_researched_table() {
        let expected: &[(&str, Option<&str>)] = &[
            ("claude", Some("CLAUDE.md")),
            ("codex", Some("AGENTS.md")),
            ("gemini", Some("GEMINI.md")),
            ("qwen", Some("QWEN.md")),
            ("copilot", Some("AGENTS.md")),
            ("openclaw", Some("AGENTS.md")),
            ("pi", Some(".pi/APPEND_SYSTEM.md")),
            ("antigravity", Some("GEMINI.md")),
            ("muxcode", Some("CLAUDE.md")),
            ("kimi", None),
        ];
        for (id, filename) in expected {
            let p = get_provider(id).unwrap_or_else(|| panic!("provider '{id}' not registered"));
            assert_eq!(
                p.startup_instructions_filename, *filename,
                "provider '{id}' startup_instructions_filename mismatch"
            );
        }
    }

    #[test]
    fn kimi_has_no_startup_instructions_filename() {
        let p = get_provider("kimi").unwrap();
        assert!(
            p.startup_instructions_filename.is_none(),
            "kimi has no confirmed native startup-instructions file — see SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2"
        );
    }

    // docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md.
    // `paths_resolve_to_same_dir` takes the reference side pre-resolved
    // (not `get_home_dir()` itself) specifically so these tests don't
    // depend on the real $HOME/%USERPROFILE% — see its own doc comment.
    mod ambient_home_dir_tests {
        use super::*;

        #[test]
        fn identical_existing_dirs_match_via_canonicalize() {
            let dir = tempfile::tempdir().unwrap();
            assert!(paths_resolve_to_same_dir(dir.path(), &dir.path().to_string_lossy()));
        }

        #[test]
        fn different_existing_dirs_do_not_match() {
            let a = tempfile::tempdir().unwrap();
            let b = tempfile::tempdir().unwrap();
            assert!(!paths_resolve_to_same_dir(a.path(), &b.path().to_string_lossy()));
        }

        #[test]
        fn nonexistent_paths_fall_back_to_lexical_comparison() {
            // Neither side exists on disk — canonicalize fails for both,
            // so this exercises the lexical fallback, not the
            // canonicalize branch. Confirms a not-yet-created ambient
            // path still gets caught (the whole point of the guard —
            // §7's "can't slip past by not existing at check time").
            let reference = std::path::Path::new("/tmp/agentmux-test-does-not-exist-ambient/.claude");
            assert!(paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-does-not-exist-ambient/.claude"));
            assert!(!paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-does-not-exist-ambient/.codex"));
        }

        #[test]
        fn lexical_fallback_ignores_trailing_slash() {
            let reference = std::path::Path::new("/tmp/agentmux-test-nonexistent/.claude");
            assert!(paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-nonexistent/.claude/"));
        }

        // codex P2, PR #2802: case-folding must match the current platform's
        // default filesystem case-sensitivity, not be unconditional —
        // asserts whichever behavior is CORRECT for whatever OS actually
        // runs this test, so it's meaningful on both windows-latest and
        // ubuntu-latest CI legs.
        #[test]
        fn lexical_fallback_case_folding_matches_platform_default() {
            let reference = std::path::Path::new("/tmp/agentmux-test-nonexistent/.claude");
            let matches = paths_resolve_to_same_dir(reference, "/TMP/agentmux-test-nonexistent/.CLAUDE");
            if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
                assert!(matches, "case-insensitive platforms must fold case in the lexical fallback");
            } else {
                assert!(!matches, "case-sensitive platforms must NOT fold case — a not-yet-created dir must not be conflated with a differently-cased one");
            }
        }

        // codex P1, PR #2802: without dot-segment collapsing, a binding
        // like `$HOME/.claude/.` compared unequal to `$HOME/.claude` under
        // plain string comparison whenever neither existed yet (the fresh-
        // machine case), bypassing the guard — Claude Code would then
        // create and use the literal ambient dir once launched.
        #[test]
        fn lexical_fallback_collapses_dot_and_dotdot_segments() {
            let reference = std::path::Path::new("/tmp/agentmux-test-nonexistent/.claude");
            assert!(paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-nonexistent/.claude/."));
            assert!(paths_resolve_to_same_dir(
                reference,
                "/tmp/agentmux-test-nonexistent/sibling/../.claude",
            ));
            // A genuinely different directory reached via `..` must still
            // correctly NOT match — this isn't "ignore everything after a
            // dot-segment," it's real lexical normalization.
            assert!(!paths_resolve_to_same_dir(
                reference,
                "/tmp/agentmux-test-nonexistent/.claude/../.codex",
            ));
        }

        #[test]
        fn is_provider_ambient_home_dir_matches_the_real_configured_provider_home_suffix() {
            // Not asserting against the real $HOME (see module doc comment) —
            // just confirming the public wrapper joins get_home_dir() with
            // ".{auth_dir_name}" as documented, by checking the suffix
            // shape rather than an exact path.
            let claude = get_provider("claude").unwrap();
            assert_eq!(claude.auth_dir_name, "claude");
            // A dir that can't possibly be the real ambient home (this
            // process's actual $HOME/.claude) must never match.
            assert!(!is_provider_ambient_home_dir(claude, "/tmp/agentmux-test-definitely-not-ambient"));
        }
    }

    mod claude_md_placeholder_tests {
        use super::*;

        #[test]
        fn writes_the_placeholder_when_claude_md_is_missing() {
            let dir = tempfile::tempdir().unwrap();
            let claude = get_provider("claude").unwrap();
            let wrote = seed_claude_md_placeholder_if_missing(claude, &dir.path().to_string_lossy()).unwrap();
            assert!(wrote);
            let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
            assert!(content.contains("AgentMux: intentionally empty"));
        }

        #[test]
        fn creates_the_config_dir_first_if_it_does_not_exist_yet() {
            let base = tempfile::tempdir().unwrap();
            let not_yet_created = base.path().join("nested").join("claude");
            let claude = get_provider("claude").unwrap();
            let wrote =
                seed_claude_md_placeholder_if_missing(claude, &not_yet_created.to_string_lossy()).unwrap();
            assert!(wrote);
            assert!(not_yet_created.join("CLAUDE.md").exists());
        }

        #[test]
        fn never_overwrites_an_existing_claude_md() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("CLAUDE.md"), "real user content, do not touch").unwrap();
            let claude = get_provider("claude").unwrap();
            let wrote = seed_claude_md_placeholder_if_missing(claude, &dir.path().to_string_lossy()).unwrap();
            assert!(!wrote);
            let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
            assert_eq!(content, "real user content, do not touch");
        }

        #[test]
        fn no_ops_for_a_non_claude_provider_even_with_no_claude_md() {
            let dir = tempfile::tempdir().unwrap();
            let codex = get_provider("codex").unwrap();
            assert_eq!(codex.auth_dir_name, "codex");
            let wrote = seed_claude_md_placeholder_if_missing(codex, &dir.path().to_string_lossy()).unwrap();
            assert!(!wrote);
            assert!(!dir.path().join("CLAUDE.md").exists());
        }

        // Codex P2 on PR #2854: an empty config_dir would otherwise resolve
        // relative to the server process's own CWD instead of erroring.
        #[test]
        fn rejects_an_empty_config_dir() {
            let claude = get_provider("claude").unwrap();
            let err = seed_claude_md_placeholder_if_missing(claude, "").unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
            assert!(!std::path::Path::new("CLAUDE.md").exists(), "must not write into CWD");
        }

        #[test]
        fn rejects_a_whitespace_only_config_dir() {
            let claude = get_provider("claude").unwrap();
            let err = seed_claude_md_placeholder_if_missing(claude, "   ").unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    mod prepare_provider_auth_dir_tests {
        use super::*;

        // The default spawn path (agent_open.rs) previously did
        // `create_dir_all` and the isolation seed as two separate,
        // independently-deletable statements, with no test covering the
        // seed call at all — removing that one line reopened the leak
        // silently. Fusing them means a caller cannot get a usable auth
        // dir without its isolation guarantees.
        #[test]
        fn creates_the_dir_and_seeds_the_claude_placeholder_together() {
            let base = tempfile::tempdir().unwrap();
            let dir = base.path().join("not").join("yet").join("there");
            let claude = get_provider("claude").unwrap();
            prepare_provider_auth_dir(claude, &dir.to_string_lossy()).unwrap();
            assert!(dir.is_dir(), "auth dir must be created");
            let content = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
            assert!(content.contains("AgentMux: intentionally empty"));
        }

        #[test]
        fn creates_the_dir_without_a_claude_md_for_a_non_claude_provider() {
            let base = tempfile::tempdir().unwrap();
            let dir = base.path().join("codexhome");
            let codex = get_provider("codex").unwrap();
            prepare_provider_auth_dir(codex, &dir.to_string_lossy()).unwrap();
            assert!(dir.is_dir(), "auth dir must still be created");
            assert!(!dir.join("CLAUDE.md").exists(), "must not seed a foreign provider's dir");
        }

        #[test]
        fn is_idempotent_and_never_clobbers_existing_content() {
            let base = tempfile::tempdir().unwrap();
            let dir = base.path().join("iso");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("CLAUDE.md"), "real user content").unwrap();
            let claude = get_provider("claude").unwrap();
            prepare_provider_auth_dir(claude, &dir.to_string_lossy()).unwrap();
            prepare_provider_auth_dir(claude, &dir.to_string_lossy()).unwrap();
            assert_eq!(
                std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
                "real user content",
            );
        }

        #[test]
        fn rejects_an_empty_auth_dir_instead_of_touching_the_cwd() {
            let claude = get_provider("claude").unwrap();
            let err = prepare_provider_auth_dir(claude, "   ").unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        }
    }
}
