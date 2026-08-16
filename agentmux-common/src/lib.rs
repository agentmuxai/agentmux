// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared utilities for AgentMux crates.

pub mod api_types;
mod cli;
pub mod data_paths;
pub mod errors;
pub mod ipc;
pub mod jekt_sign;
pub mod layout_types;
pub mod pagefile;
pub mod runtime_mode;
pub mod toolchain_path;

pub use cli::{make_cli_cmd, resolve_cli_spawn_target};
pub use data_paths::{
    ensure_history_link, isolated_auth_enabled, isolated_auth_reason, DataPaths, IsolatedAuthReason,
};
pub use errors::{AgentMuxError, AmxCode};
pub use layout_types::{
    FlexDirection, LayoutClientSlices, LayoutNode, LayoutNodeData, ResizeOp, SplitPosition,
};
pub use runtime_mode::{is_dev_build_exe, is_dev_self, RuntimeMode};
pub use toolchain_path::{
    enrich_current_process_path, looks_like_launchd_default, resolve_login_path, EnrichedPath,
    PathSource,
};

/// Crate-wide test-only lock for env-var-touching tests. Both
/// `runtime_mode::tests` and `data_paths::tests` mutate process-global
/// env vars (`AGENTMUX_RUNTIME_MODE`, `AGENTMUX_HOME_OVERRIDE`, etc.);
/// without a lock that's shared ACROSS modules, cargo's parallel test
/// runner can race a setter in one module against a reader in the
/// other. A per-module `static Mutex<()>` was insufficient — they
/// need to share the same Mutex instance.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
