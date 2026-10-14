// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// Cline CLI: `cline --acp` speaks ACP over stdio. It refuses `session/new`
// until signed in ("Authentication required") and has no status command.
// Checked against 3.0.71 on 2026-10-14 (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.3).
pub(super) static CLINE: ProviderConfig = ProviderConfig {
    id: "cline",
    cli_command: "cline",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["--acp"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // Moves Cline's whole tree (settings, sessions, logs).
    auth_config_dir_env_var: "CLINE_DIR",
    auth_dir_name: "cline",
    history_native_subdir: None,
    // Run from a node_modules install, Cline otherwise updates itself with
    // a global `npm update` at startup.
    auth_extra_env: &[("CLINE_NO_AUTO_UPDATE", "1")],
    unset_env: &[],
    npm_package: "cline",
    pinned_version: "3.0.71",
    base_url_env_var: None,
    supported_vendors: &["cline"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
