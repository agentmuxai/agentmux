// Tauri command modules
//
// This module organizes all Tauri commands into focused, single-responsibility modules
// instead of having everything in main.rs

pub mod types;
pub mod bus;
pub mod watchers;
pub mod messages;
pub mod agents;
pub mod claude;
pub mod logs;
pub mod cli;

// Re-export all commands for backward compatibility
pub use bus::*;
pub use watchers::*;
pub use messages::*;
pub use agents::*;
pub use claude::*;
pub use logs::*;
pub use cli::*;
pub use types::*;
