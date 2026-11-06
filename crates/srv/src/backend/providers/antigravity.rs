// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

// Antigravity (AGY) — Google's agentic coding CLI harness. Its own
// stream-json schema, prompt as an argument (`prompt_arg_flag`), and flags,
// all checked against agy 1.1.11: SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md.
// No base_url_env_var: a custom endpoint is not verified.
pub(super) static ANTIGRAVITY: ProviderConfig = ProviderConfig {
    id: "antigravity",
    cli_command: "agy",
    controller_type: ControllerType::Subprocess,
    app_server: None,
    launch_args: &["--output-format", "stream-json", "--dangerously-skip-permissions"],
    persistent_launch_args: None,
    resume_flag: Some("--conversation"),
    session_id_field: "conversation_id",
    styled_output_format: "agy-stream-json",
    auth_config_dir_env_var: "ANTIGRAVITY_CONFIG_DIR",
    auth_dir_name: "antigravity",
    // Not OAuth-class -- never reaches ensure_history_link. Not covered
    // by SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md (added later);
    // None until independently verified rather than guessed.
    history_native_subdir: None,
    // agy reads neither ANTIGRAVITY_FORCE_FILE_STORAGE nor
    // ANTIGRAVITY_CONFIG_DIR (neither string is in agy.exe); it signs in once
    // per machine. The frontend's `authType: "cli-managed"` binds no account.
    // The pinned copy must not replace itself with a newer build.
    auth_extra_env: &[("AGY_CLI_DISABLE_AUTO_UPDATE", "true")],
    unset_env: &[],
    // Not on npm: `agy` is a native binary. AgentMux downloads the pinned build
    // itself (`cli_download::SPECS`; Google's server lists only the latest, so
    // the build current on 2026-11-06 is the pin), and a copy the user
    // installed is still found on PATH or in `known_install_paths`. The old
    // `@google/antigravity-cli` pin never existed, so every install of it failed.
    npm_package: "",
    pinned_version: "1.5.2",
    base_url_env_var: None,
    supported_vendors: &["google"],
    // INFERRED, not independently doc-confirmed: Antigravity CLI's own
    // settings live at ~/.gemini/antigravity-cli/settings.json — the same
    // ~/.gemini/ namespace root as Gemini CLI itself (its output schema is
    // its own, though). No explicit Antigravity docs page independently confirms
    // GEMINI.md context-file behavior. Flagged as a known gap. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2, §6.
    startup_instructions_filename: Some("GEMINI.md"),
    // Read set = the write target.
    native_instruction_sources: &["GEMINI.md"],
    native_instruction_dirs: &[],
};
