// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

pub(super) static GEMINI: ProviderConfig = ProviderConfig {
    id: "gemini",
    cli_command: "gemini",
    controller_type: ControllerType::Subprocess,
    app_server: None,
    // --output-format stream-json: NDJSON events; --yolo: auto-approve all tools;
    // -p "": enable headless/non-interactive mode (prompt comes from stdin)
    launch_args: &["--output-format", "stream-json", "--yolo", "-p", ""],
    persistent_launch_args: None,
    resume_flag: Some("-r"),
    session_id_field: "session_id",
    styled_output_format: "gemini-json",
    auth_config_dir_env_var: "GEMINI_CLI_HOME",
    auth_dir_name: "gemini",
    history_native_subdir: Some("history"),
    auth_extra_env: &[("GEMINI_FORCE_FILE_STORAGE", "true")],
    unset_env: &[],
    npm_package: "@google/gemini-cli",
    pinned_version: "0.62.0",
    base_url_env_var: None,
    supported_vendors: &["google"],
    // Confirmed: Gemini CLI docs — context files default to GEMINI.md;
    // AGENTS.md support exists but requires an explicit contextFileName
    // override, so GEMINI.md is the correct default-behavior target. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: Some("GEMINI.md"),
    // Read set = the write target. AGENTS.md is supported only behind an
    // explicit contextFileName override, so it is not read by default.
    native_instruction_sources: &["GEMINI.md"],
    native_instruction_dirs: &[],
};
