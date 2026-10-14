// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// Crush (Charm): no ACP in this version and no JSON output, so it runs in a
// terminal pane with its own UI (the terminal tier). Installed from Charm's
// own npm package, never bundled: its license (FSL-1.1-MIT) limits competing
// commercial redistribution. Checked against 0.98.1 on 2026-10-14
// (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.8).
pub(super) static CRUSH: ProviderConfig = ProviderConfig {
    id: "crush",
    cli_command: "crush",
    controller_type: ControllerType::Terminal,
    app_server: None,
    launch_args: &[],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "",
    styled_output_format: "raw",
    // Crush keeps sign-ins and keys in its global data dir.
    auth_config_dir_env_var: "CRUSH_GLOBAL_DATA",
    auth_dir_name: "crush",
    history_native_subdir: None,
    // No usage metrics, and the pinned provider list rather than a download.
    auth_extra_env: &[
        ("CRUSH_DISABLE_METRICS", "1"),
        ("DO_NOT_TRACK", "1"),
        ("CRUSH_DISABLE_PROVIDER_AUTO_UPDATE", "1"),
    ],
    unset_env: &[],
    npm_package: "@charmland/crush",
    pinned_version: "0.98.1",
    base_url_env_var: None,
    supported_vendors: &["crush"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md", "CRUSH.md", "CLAUDE.md"],
    native_instruction_dirs: &[],
};
