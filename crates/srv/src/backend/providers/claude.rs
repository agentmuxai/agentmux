// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

pub(super) static CLAUDE: ProviderConfig = ProviderConfig {
    id: "claude",
    cli_command: "claude",
    // Persistent (bidirectional stream-json) + the Agent SDK CONTROL PROTOCOL is
    // the only way AskUserQuestion (and interactive tool-permission) works headless.
    // The CLI auto-rejects AskUserQuestion with `Error: Answer questions?` in any
    // mode UNLESS the driver speaks the control protocol: launch with
    // `--permission-prompt-tool stdio` (+ a non-bypass `--permission-mode`), then
    // answer the CLI's `can_use_tool` control_request with a control_response
    // carrying `updatedInput.answers`. See SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md
    // (§2 captured the exact wire bytes against bundled CLI v2.1.178).
    //
    // CRITICAL: `--dangerously-skip-permissions` DISABLES that routing (it bypasses
    // canUseTool), so it must NOT be in persistent_launch_args. The persistent
    // controller's ControlChannel auto-allows ordinary tools to preserve today's
    // yolo UX; only AskUserQuestion is surfaced to the user.
    controller_type: ControllerType::Persistent,
    app_server: None,
    launch_args: &[
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--dangerously-skip-permissions",
        // Moves per-machine sections (cwd, env info, memory paths, git status)
        // out of the system prompt into the first user message, so the system
        // prompt stays byte-identical across agents/machines and the 1hr
        // prompt cache (docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md) isn't
        // invalidated by per-instance dynamic content. No-op if the default
        // system prompt is overridden — not guaranteed here, since
        // `agent.provider_flags` (app_api/agent_open.rs) is a free-form
        // user-configurable string appended after these args; a user-set
        // `--system-prompt` there would silently no-op this flag. Harmless
        // either way (that's the flag's own documented ignore-if-overridden
        // behavior), just not something this hardcoded default can prevent.
        // reagent P2 on PR #1964.
        "--exclude-dynamic-system-prompt-sections",
    ],
    persistent_launch_args: Some(&[
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--exclude-dynamic-system-prompt-sections",
        // Enable the control protocol so the sidecar can answer can_use_tool /
        // AskUserQuestion. Replaces --dangerously-skip-permissions (which bypasses it).
        "--permission-prompt-tool",
        "stdio",
        "--permission-mode",
        "default",
    ]),
    resume_flag: Some("--resume"),
    session_id_field: "session_id",
    styled_output_format: "claude-stream-json",
    auth_config_dir_env_var: "CLAUDE_CONFIG_DIR",
    auth_dir_name: "claude",
    history_native_subdir: Some("projects"),
    auth_extra_env: &[],
    unset_env: &["CLAUDECODE"],
    npm_package: "@anthropic-ai/claude-code",
    // Keep in sync with the frontend catalog's `pinnedVersion`, enforced by
    // frontend/app/view/agent/providers/pin-consistency.test.ts. Host installs and
    // the container agents' first-start install both use this version.
    pinned_version: "2.1.288",
    // Documented Claude Code behavior: redirects the CLI at a non-Anthropic
    // (or proxied) backend — Bedrock, Vertex, OpenRouter, a custom proxy.
    base_url_env_var: Some("ANTHROPIC_BASE_URL"),
    supported_vendors: &["anthropic"],
    startup_instructions_filename: Some("CLAUDE.md"),
    // Read set = the write target. Claude Code auto-discovers CLAUDE.md;
    // `.claude/AGENTMUX_MEMORY.md` is reached only through an @import line
    // AgentMux itself adds (SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md),
    // so it is not a native source. Nested/parent CLAUDE.md discovery is
    // real but out of scope here: this field is working-directory-relative.
    native_instruction_sources: &["CLAUDE.md"],
    native_instruction_dirs: &[],
};
