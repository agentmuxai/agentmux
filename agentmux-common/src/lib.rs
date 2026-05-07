//! Shared utilities for AgentMux crates.

mod cli;
pub mod data_paths;
pub mod ipc;
pub mod layout_types;
pub mod runtime_mode;

pub use cli::make_cli_cmd;
pub use data_paths::DataPaths;
pub use layout_types::{FlexDirection, LayoutNode, LayoutNodeData, ResizeOp, SplitPosition};
pub use runtime_mode::{is_dev_build_exe, RuntimeMode};

/// Crate-wide test-only lock for env-var-touching tests. Both
/// `runtime_mode::tests` and `data_paths::tests` mutate process-global
/// env vars (`AGENTMUX_RUNTIME_MODE`, `AGENTMUX_HOME_OVERRIDE`, etc.);
/// without a lock that's shared ACROSS modules, cargo's parallel test
/// runner can race a setter in one module against a reader in the
/// other. A per-module `static Mutex<()>` was insufficient — they
/// need to share the same Mutex instance.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
