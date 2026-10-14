// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

pub(super) static KIMI: ProviderConfig = ProviderConfig {
    id: "kimi",
    cli_command: "kimi",
    controller_type: ControllerType::Subprocess,
    app_server: None,
    // No `-p ""` placeholder: kimi 1.37 rejects an empty --prompt ("Prompt
    // cannot be empty") before reading stdin. Without -p, --print reads the
    // prompt from stdin, which is how the turn delivers it.
    launch_args: &[
        "--print",
        "--output-format",
        "stream-json",
        "--yolo",
    ],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "session_id",
    styled_output_format: "kimi-stream-json",
    auth_config_dir_env_var: "KIMI_SHARE_DIR",
    auth_dir_name: "kimi",
    // API-key-class, not OAuth-class -- never reaches ensure_history_link.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "",
    pinned_version: "",
    base_url_env_var: None,
    supported_vendors: &["moonshot"],
    // Confirmed absence: docs/specs/KIMI_PROVIDER_INTEGRATION_SPEC.md
    // already researched this — Kimi has no auto-read markdown convention
    // ("does not appear to auto-read CLAUDE.md... skip KIMI.md
    // generation"). Re-verified 2026-08-24: Kimi's only file-based prompt
    // customization is --agent-file <yaml> with a system_prompt_path field
    // (a CLI flag this provider's launch_args above doesn't pass), and an
    // open, unshipped feature request (MoonshotAI/kimi-cli#1856) for a
    // project-level system_prompt.md override. Writing any markdown file
    // here would be inert output nobody reads — None, not a guessed
    // filename. build_config_files skips the instructions file entirely
    // for a provider with None here. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: None,
    // No confirmed native file convention at all — hence the None above.
    native_instruction_sources: &[],
    native_instruction_dirs: &[],
};
