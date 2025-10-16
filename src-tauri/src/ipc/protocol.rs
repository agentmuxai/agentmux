// IPC protocol definitions

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// IPC command sent from CLI to running GUI instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcCommand {
    pub command_type: String,  // "agents", "messages", "status", "logs"
    pub action: String,         // "list", "spawn", "stop", etc.
    pub args: HashMap<String, serde_json::Value>,
    pub caller_pid: Option<u32>,
}

/// IPC response sent from GUI back to CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub output: String,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

impl IpcResponse {
    pub fn success(output: String, data: Option<serde_json::Value>, duration_ms: u64) -> Self {
        Self {
            success: true,
            output,
            data,
            error: None,
            duration_ms,
        }
    }

    pub fn error(error: String, duration_ms: u64) -> Self {
        Self {
            success: false,
            output: error.clone(),
            data: None,
            error: Some(error),
            duration_ms,
        }
    }
}
