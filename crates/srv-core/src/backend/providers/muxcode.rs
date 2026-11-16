// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

// Mux Code — AgentMux's first-party agentic coding CLI.
// Local GGUF inference via llama-server or cloud APIs (Anthropic,
// OpenAI, OpenAI-compat). Emits claude-compatible stream-json NDJSON
// (same `session_id` field, same event envelope), so ClaudeTranslator
// handles it without modification.  `--resume <id>` resumes a prior
// session.  npm: `@agentmuxai/muxcode`.
pub(super) static MUX_CODE: ProviderConfig = ProviderConfig {
    id: "muxcode",
    cli_command: "muxcode",
    controller_type: ControllerType::Subprocess,
    app_server: None,
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
    // The first version published to npm (0.1.0 never was, so the installer
    // 404ed). Every version pinned here must be published first.
    pinned_version: "0.8.0",
    base_url_env_var: None,
    supported_vendors: &["ollama", "anthropic", "openai"],
    // Confirmed by design intent: this provider's own doc comment above
    // states it "emits claude-compatible stream-json NDJSON... handled
    // without modification [by ClaudeTranslator]" — a deliberate
    // compatibility choice by the same team that owns both, not a guess.
    startup_instructions_filename: Some("CLAUDE.md"),
    // Read set = the write target; claude-compatible by design, per above.
    native_instruction_sources: &["CLAUDE.md"],
    native_instruction_dirs: &[],
};
