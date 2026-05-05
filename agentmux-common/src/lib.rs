//! Shared utilities for AgentMux crates.

mod cli;
pub mod data_paths;
pub mod ipc;
pub mod layout_types;
pub mod runtime_mode;

pub use cli::make_cli_cmd;
pub use data_paths::DataPaths;
pub use layout_types::{FlexDirection, LayoutNode, LayoutNodeData, ResizeOp, SplitPosition};
pub use runtime_mode::RuntimeMode;
