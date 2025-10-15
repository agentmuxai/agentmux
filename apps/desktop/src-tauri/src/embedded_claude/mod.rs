// Embedded Claude CLI - Modularized architecture
//
// This module manages embedded Claude CLI instances with WebSocket streaming
// and message file watching for agent communication.

pub mod types;
pub mod utils;
pub mod instance;
pub mod process;
pub mod websocket;
pub mod messages;

// Re-exports for backward compatibility
pub use types::{AgentMessage, AgentIdentity, MessagePayload, Tx, PeerMap, WebSocketSink};
pub use instance::{ClaudeInstance, ClaudeInstancesState};
pub use utils::find_available_port;
