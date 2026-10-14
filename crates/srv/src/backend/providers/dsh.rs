// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ControllerType, ProviderConfig};

// DeepSeek Harness (`dsh`): `dsh --profile acp` speaks ACP over stdio. A
// prerelease upstream (breaking changes promised), shown as a preview. It
// runs signed out until a prompt needs DEEPSEEK_API_KEY. Checked against
// 0.2.0-rc.2 on 2026-10-14 (SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.6).
pub(super) static DSH: ProviderConfig = ProviderConfig {
    id: "dsh",
    cli_command: "dsh",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["--profile", "acp"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // Moves ~/.dsh: profiles, sessions, credentials. The session-log upload
    // is turned off by a patch in it (blockcontroller/acp_prelaunch.rs).
    auth_config_dir_env_var: "DSH_HOME",
    auth_dir_name: "dsh",
    history_native_subdir: None,
    auth_extra_env: &[("DSH_TELEMETRY_DISABLED", "1")],
    unset_env: &[],
    npm_package: "@deepseek-ai/dsh",
    pinned_version: "0.2.0-rc.2",
    base_url_env_var: None,
    supported_vendors: &["deepseek"],
    startup_instructions_filename: Some("AGENTS.md"),
    native_instruction_sources: &["AGENTS.md", "CLAUDE.md"],
    native_instruction_dirs: &[],
};
