// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType, AppServerConfig, AppServerKind};

pub(super) static CODEX: ProviderConfig = ProviderConfig {
    id: "codex",
    cli_command: "codex",
    controller_type: ControllerType::Subprocess,
    app_server: Some(AppServerConfig {
        kind: AppServerKind::Codex,
        launch_args: &["app-server", "--listen", "stdio://"],
        fallback_controller: ControllerType::Subprocess,
        fallback_output_format: "codex-json",
    }),
    // exec subcommand runs non-interactively; --json emits NDJSON events; - reads prompt from stdin
    launch_args: &[
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
        "-",
    ],
    // Codex resume requires a subcommand change (exec resume <id>), not a simple flag.
    // Multi-turn is handled by re-running exec; None disables automatic --resume append.
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "thread_id",
    styled_output_format: "codex-json",
    auth_config_dir_env_var: "CODEX_HOME",
    auth_dir_name: "codex",
    history_native_subdir: Some("sessions"),
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@openai/codex",
    pinned_version: "0.154.0",
    base_url_env_var: None,
    supported_vendors: &["openai"],
    // Confirmed: SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md §10.2 —
    // "Codex's native project instruction discovery continues to load
    // user/repository AGENTS.md files normally."
    startup_instructions_filename: Some("AGENTS.md"),
    // Read set = the write target, per the same citation above: Codex loads
    // repository AGENTS.md natively.
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
