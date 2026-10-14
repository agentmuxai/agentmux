// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType, InstructionDirScan};

// GitHub Copilot CLI — Microsoft's coding agent. Runs in ACP mode via
// `--acp` so the existing ACP controller drives it. Non-interactive
// `-p`/`--prompt` doesn't accept stdin prompts (github/copilot-cli#96,
// #1046), hence ACP.
pub(super) static COPILOT: ProviderConfig = ProviderConfig {
    id: "copilot",
    cli_command: "copilot",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["--acp"],
    persistent_launch_args: None,
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    auth_config_dir_env_var: "COPILOT_HOME",
    auth_dir_name: "copilot",
    // ACP-native, same as openclaw (providers/openclaw.rs) -- no fixed on-disk history
    // directory to redirect.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@github/copilot",
    pinned_version: "1.0.91",
    base_url_env_var: None,
    supported_vendors: &["github"],
    // Confirmed: GitHub Copilot CLI custom-instructions docs — supports
    // AGENTS.md (root, single-file, no subdirectory needed) alongside
    // .github/copilot-instructions.md, CLAUDE.md, GEMINI.md. AGENTS.md
    // chosen as the one canonical target since Copilot has no single
    // privileged default the way Gemini/Qwen do. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    startup_instructions_filename: Some("AGENTS.md"),
    // FOUR sources, all four named in the citation above: Copilot reads
    // .github/copilot-instructions.md, CLAUDE.md and GEMINI.md alongside the
    // AGENTS.md chosen as the write target. This is the case that proved the
    // write target alone is the wrong question (Codex, PR #3144).
    native_instruction_sources: &[
        "AGENTS.md",
        ".github/copilot-instructions.md",
        "CLAUDE.md",
        "GEMINI.md",
    ],
    // Path-scoped instructions: .github/instructions/**/*.instructions.md,
    // claim 3 of the Copilot verification doc. Recursive, per the `**`.
    // Not in the list above because it is a pattern, not a path.
    //
    // Deliberately NOT represented: $HOME/.copilot/copilot-instructions.md
    // (claim 4) and $COPILOT_CUSTOM_INSTRUCTIONS_DIRS (claim 6). Both are
    // real, and both live outside the agent working directory, which is
    // what these two fields are relative to. Tracking them is a bigger
    // question — they are machine-global, not project state, so they do
    // not travel with a bundle — and is out of scope here rather than
    // forgotten.
    native_instruction_dirs: &[InstructionDirScan {
        dir: ".github/instructions",
        suffix: ".instructions.md",
        recursive: true,
    }],
};
