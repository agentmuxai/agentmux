// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

// Pi runs through `pi-acp`, an ACP adapter that drives `pi --mode rpc`: pi
// itself speaks no ACP (`pi --json` was never an option, so no Pi launch ever
// started). Both packages install together (`companion_npm_packages`), and
// `pi-acp` is told which `pi` to run (`PI_ACP_PI_COMMAND`, acp.rs).
// SPEC_PI_HARNESS_VIA_PI_ACP_2026_10_06.md.
pub(super) static PI: ProviderConfig = ProviderConfig {
    id: "pi",
    cli_command: "pi-acp",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &[],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // pi's own config/auth dir variable; `PI_HOME` was never read.
    auth_config_dir_env_var: "PI_CODING_AGENT_DIR",
    auth_dir_name: "pi",
    // Not OAuth-class -- never reaches ensure_history_link. Also
    // ACP-native (see providers/openclaw.rs) if that changes.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "pi-acp",
    pinned_version: "0.0.34",
    base_url_env_var: None,
    supported_vendors: &["pi"],
    // AGENTS.md, a context file pi always loads. pi 1.0.4 treats
    // .pi/APPEND_SYSTEM.md (the earlier target) and .pi/SYSTEM.md as
    // trust-protected, and RPC mode (pi-acp) skips them for a project with no
    // saved trust decision, so the instructions written there were never read.
    // SPEC_PI_HARNESS_VIA_PI_ACP_2026_10_06.md.
    startup_instructions_filename: Some("AGENTS.md"),
    // pi reads AGENTS.md or CLAUDE.md as context, and .pi/SYSTEM.md (which
    // replaces its default prompt) and .pi/APPEND_SYSTEM.md in a trusted
    // project. A repo-provided SYSTEM.md is exactly the kind of instruction
    // file Phase 3 exists to surface.
    native_instruction_sources: &["AGENTS.md", "CLAUDE.md", ".pi/APPEND_SYSTEM.md", ".pi/SYSTEM.md"],
    native_instruction_dirs: &[],
};
