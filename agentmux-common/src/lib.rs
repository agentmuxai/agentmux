//! Shared utilities for AgentMux crates.

mod cli;
pub mod ipc;
pub mod layout_types;

pub use cli::make_cli_cmd;
pub use layout_types::{FlexDirection, LayoutNode, LayoutNodeData, ResizeOp, SplitPosition};
