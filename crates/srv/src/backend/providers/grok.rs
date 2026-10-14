// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// Grok Build (xAI): `grok agent stdio` speaks ACP over stdio. Not on npm: the
// user installs it with xAI's installer (the install hint), and srv finds it
// on PATH or in `known_install_paths`. Checked against 1.0.52 on 2026-10-14
// (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.5).
pub(super) static GROK: ProviderConfig = ProviderConfig {
    id: "grok",
    cli_command: "grok",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["agent", "stdio"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // Moves ~/.grok: config, sign-in, sessions, trusted folders.
    auth_config_dir_env_var: "GROK_HOME",
    auth_dir_name: "grok",
    history_native_subdir: None,
    // No self-update (the pin holds) and no telemetry. Grok also imports
    // Claude Code's and Cursor's rules, skills, MCP servers and hooks by
    // default, which would pull an AgentMux agent's Claude setup into a
    // Grok one.
    auth_extra_env: &[
        ("GROK_DISABLE_AUTOUPDATER", "1"),
        ("GROK_TELEMETRY_ENABLED", "false"),
        ("GROK_CLAUDE_SKILLS_ENABLED", "0"),
        ("GROK_CLAUDE_RULES_ENABLED", "0"),
        ("GROK_CLAUDE_AGENTS_ENABLED", "0"),
        ("GROK_CLAUDE_MCPS_ENABLED", "0"),
        ("GROK_CLAUDE_HOOKS_ENABLED", "0"),
        ("GROK_CLAUDE_SESSIONS_ENABLED", "0"),
        ("GROK_CURSOR_SKILLS_ENABLED", "0"),
        ("GROK_CURSOR_RULES_ENABLED", "0"),
        ("GROK_CURSOR_AGENTS_ENABLED", "0"),
        ("GROK_CURSOR_MCPS_ENABLED", "0"),
        ("GROK_CURSOR_HOOKS_ENABLED", "0"),
        ("GROK_CURSOR_SESSIONS_ENABLED", "0"),
    ],
    unset_env: &[],
    npm_package: "",
    pinned_version: "1.0.52",
    base_url_env_var: None,
    supported_vendors: &["xai"],
    // Read only in a trusted folder: srv trusts the agent's own
    // (blockcontroller/acp_prelaunch.rs).
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
