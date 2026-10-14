// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

// Qwen Code — Alibaba's open-source coding agent, a fork of Gemini CLI.
// Same stream-json headless surface, so it reuses the Gemini translator.
// Backend is OpenAI-compatible (OPENAI_BASE_URL=https://openrouter.ai/api/v1
// + OPENAI_API_KEY/OPENAI_MODEL), so it runs any OpenRouter model.
pub(super) static QWEN: ProviderConfig = ProviderConfig {
    id: "qwen",
    cli_command: "qwen",
    controller_type: ControllerType::Subprocess,
    app_server: None,
    // -p: non-interactive; --output-format stream-json: NDJSON events;
    // --include-partial-messages: Claude-style stream_event deltas, which is
    // where the Claude translator reads reply text; --yolo: auto-approve all
    // tools.
    launch_args: &["--output-format", "stream-json", "--include-partial-messages", "--yolo", "-p", ""],
    persistent_launch_args: None,
    // Docs mention --resume <id>/--continue but it's unconfirmed for the
    // headless stream-json path; multi-turn re-runs like Codex/Kimi (None).
    resume_flag: None,
    session_id_field: "session_id",
    // A Gemini-CLI fork, but its stream-json is Claude Code's shape
    // (system/assistant/user/result frames; verified on 0.24.0), not Gemini's,
    // so it is read by the Claude translator under its own name. Its usage
    // counts cached tokens inside input_tokens, unlike Claude's, so the context
    // meter does not read it (main-agent-usage.ts `readsMainAgentUsage`).
    styled_output_format: "qwen-stream-json",
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
    pinned_version: "0.24.7",
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
    // Read set = the write target, for the same reason as Gemini: AGENTS.md
    // needs explicit config (QwenLM/qwen-code#2006, #504).
    native_instruction_sources: &["QWEN.md"],
    native_instruction_dirs: &[],
};
