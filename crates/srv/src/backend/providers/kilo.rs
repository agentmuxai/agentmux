// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// Kilo Code CLI, an OpenCode fork: `kilo acp` speaks ACP over stdio and runs
// signed out on Kilo Gateway's free models. Checked against 7.8.8 on
// 2026-10-14 (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.2).
pub(super) static KILO: ProviderConfig = ProviderConfig {
    id: "kilo",
    cli_command: "kilo",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["acp"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // Kilo keeps auth.json and its session database under
    // XDG_DATA_HOME/kilo; KILO_CONFIG_DIR only adds a config directory.
    auth_config_dir_env_var: "XDG_DATA_HOME",
    auth_dir_name: "kilo",
    history_native_subdir: None,
    // The pin holds (no self-update), and the user's own ~/.claude/CLAUDE.md
    // stays out of a Kilo agent.
    auth_extra_env: &[("KILO_DISABLE_AUTOUPDATE", "1"), ("KILO_DISABLE_CLAUDE_CODE_PROMPT", "1")],
    unset_env: &[],
    npm_package: "@kilocode/cli",
    pinned_version: "7.8.8",
    base_url_env_var: None,
    supported_vendors: &["kilo"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
