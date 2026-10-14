// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// OpenCode (V1 line): `opencode acp` speaks ACP over stdio and runs signed out
// on OpenCode Zen's free models. Checked against 1.19.0 on 2026-10-14
// (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.1).
pub(super) static OPENCODE: ProviderConfig = ProviderConfig {
    id: "opencode",
    cli_command: "opencode",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["acp"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    // ACP, read with the turn's token usage (the frontend's translator-factory.ts).
    styled_output_format: "opencode-acp",
    // OpenCode keeps auth.json and its session database under
    // XDG_DATA_HOME/opencode; OPENCODE_CONFIG_DIR doesn't move them.
    auth_config_dir_env_var: "XDG_DATA_HOME",
    auth_dir_name: "opencode",
    history_native_subdir: None,
    // The pin holds (no self-update), and the user's own ~/.claude/CLAUDE.md
    // stays out of an OpenCode agent.
    auth_extra_env: &[("OPENCODE_DISABLE_AUTOUPDATE", "1"), ("OPENCODE_DISABLE_CLAUDE_CODE_PROMPT", "1")],
    unset_env: &[],
    npm_package: "opencode-ai",
    pinned_version: "1.19.0",
    base_url_env_var: None,
    supported_vendors: &["opencode"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
