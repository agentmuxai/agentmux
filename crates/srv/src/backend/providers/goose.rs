// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// goose: `goose acp` speaks ACP over stdio. Not on npm: the user installs the
// GitHub release (the install hint), and srv finds it on PATH or in
// `known_install_paths`. Checked against 1.54.0 on 2026-10-14
// (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.4).
pub(super) static GOOSE: ProviderConfig = ProviderConfig {
    id: "goose",
    cli_command: "goose",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["acp"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // Moves goose's config, data and state (an absolute path).
    auth_config_dir_env_var: "GOOSE_PATH_ROOT",
    auth_dir_name: "goose",
    history_native_subdir: None,
    // Secrets in a file under GOOSE_PATH_ROOT, not the OS keyring, whose
    // prompt can block an agent nobody is watching. Telemetry off.
    auth_extra_env: &[("GOOSE_DISABLE_KEYRING", "1"), ("GOOSE_TELEMETRY_OFF", "1")],
    unset_env: &[],
    npm_package: "",
    pinned_version: "1.54.0",
    base_url_env_var: None,
    supported_vendors: &["goose"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
