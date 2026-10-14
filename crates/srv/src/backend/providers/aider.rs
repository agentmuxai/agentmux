// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// Aider: no ACP, MCP or JSON output, so it runs in a terminal pane with its own
// UI (the terminal tier). Not on npm: the user installs it with Aider's own
// installer (uv, Python 3.12), and srv finds it on PATH or in
// `known_install_paths`. Checked against 0.86.2 on 2026-10-14
// (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.9).
pub(super) static AIDER: ProviderConfig = ProviderConfig {
    id: "aider",
    cli_command: "aider",
    controller_type: ControllerType::Terminal,
    app_server: None,
    // Aider reads no startup file by itself, so the one AgentMux writes is
    // passed to it. No analytics, update checks, release notes, or edits to
    // the repo's .gitignore.
    launch_args: &[
        "--read",
        "AGENTS.md",
        "--analytics-disable",
        "--no-check-update",
        "--no-show-release-notes",
        "--no-gitignore",
    ],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "",
    styled_output_format: "raw",
    // Aider keeps its state in ~/.aider and only the user profile moves it:
    // no variable of its own to isolate. Keys come from the user's env.
    auth_config_dir_env_var: "",
    auth_dir_name: "aider",
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "",
    pinned_version: "0.86.2",
    base_url_env_var: None,
    supported_vendors: &["aider"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
