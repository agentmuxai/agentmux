// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{ProviderConfig, ControllerType};

pub(super) static OPENCLAW: ProviderConfig = ProviderConfig {
    id: "openclaw",
    // `openclaw acp` runs OpenClaw's ACP bridge — speaks ACP over stdio
    // for IDE/tool clients (us) and forwards turns to the local
    // OpenClaw Gateway over WebSocket. The Gateway is OpenClaw's own
    // daemon (`openclaw gateway`) and MUST be running before this
    // bridge can establish a session — surfaced to the user as an
    // onboarding requirement in SPEC_OPENCLAW_AGENT_2026_05_17.md §6β.
    //
    // The previous scaffold pointed at `acpx` / `@openclaw/acpx`,
    // which is not a real package. The canonical binary is `openclaw`
    // (npm: `openclaw`) and the ACP subcommand is `openclaw acp`.
    // Verified against docs.openclaw.ai/cli/acp + GitHub README.
    cli_command: "openclaw",
    controller_type: ControllerType::Acp,
    app_server: None,
    launch_args: &["acp"],
    persistent_launch_args: None,
    // ACP handles sessions natively — no resume flag or session ID parsing needed
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    auth_config_dir_env_var: "OPENCLAW_HOME",
    auth_dir_name: "openclaw",
    // ACP-native: exposes history through the live protocol stream, not
    // a fixed on-disk directory -- nothing for ensure_history_link to
    // redirect. Per SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md §4.2.
    history_native_subdir: None,
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "openclaw",
    pinned_version: "2026.9.8",
    base_url_env_var: None,
    supported_vendors: &["openai", "anthropic", "google"],
    // Confirmed convention, UNCONFIRMED path: docs.openclaw.ai/reference/AGENTS.default
    // — AGENTS.md is a required bootstrap file OpenClaw itself creates,
    // read from each agent's workspace at session start. Whether that
    // workspace maps 1:1 onto AgentMux's own working_directory for an
    // ACP-bridged `openclaw acp` session (vs. OpenClaw's own
    // Gateway-daemon-managed sandbox) is not independently verified here —
    // best-effort root-level AGENTS.md, flagged as a known gap. See
    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2, §6.
    startup_instructions_filename: Some("AGENTS.md"),
    // Read set = the write target, with the same UNCONFIRMED-path caveat
    // recorded above.
    native_instruction_sources: &["AGENTS.md"],
    native_instruction_dirs: &[],
};
