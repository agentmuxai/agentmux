// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// Muse Code (Meta): no ACP, but a typed protocol of its own, MSP (`muse
// serve`), which the `msp` controller drives in an agent pane. Closed source,
// installed with Meta's installer: srv runs the native binary beside the
// launcher (`known_install_paths`), which also skips the launcher's hourly
// self-update. Checked against 1.4.7-R6101.1 on 2026-10-14
// (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.7).
pub(super) static MUSE: ProviderConfig = ProviderConfig {
    id: "muse",
    cli_command: "muse",
    controller_type: ControllerType::Msp,
    app_server: None,
    // Project rules (the agent's AGENTS.md) load only in a trusted workspace:
    // this trusts the agent's own folder.
    launch_args: &["serve", "--trust-workspace"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "msp",
    // Muse keeps muse/auth.json and its settings under XDG_CONFIG_HOME.
    auth_config_dir_env_var: "XDG_CONFIG_HOME",
    auth_dir_name: "muse",
    history_native_subdir: None,
    auth_extra_env: &[("MUSE_NO_AUTO_UPDATE", "1"), ("TBH_DISABLE_TELEMETRY", "1")],
    unset_env: &[],
    npm_package: "",
    pinned_version: "1.4.7-R6101.1",
    base_url_env_var: None,
    supported_vendors: &["meta"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md", "CLAUDE.md"],
    native_instruction_dirs: &[],
};
