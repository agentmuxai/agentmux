// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// IPC bridge between frontend JavaScript and Rust backend.
//
// Phase 2 implementation: This module will provide a cefQuery-based message
// router that replaces Tauri's invoke()/emit() pattern.
//
// Architecture:
//   JS: window.cefQuery({ request, onSuccess, onFailure })
//   →  CEF MessageRouter (C++ layer)
//   →  Rust handler (this module)
//   →  Response back to JS callback
//
// For Phase 1 (POC), this module is a placeholder. The frontend connects
// directly to agentmuxsrv-rs via WebSocket for terminal RPC.

/// IPC command request from the frontend.
#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
pub struct IpcRequest {
    /// Command name (maps to Tauri command names).
    pub cmd: String,
    /// Command arguments as JSON.
    pub args: serde_json::Value,
}

/// IPC response back to the frontend.
#[derive(Debug, serde::Serialize)]
#[allow(dead_code)]
pub struct IpcResponse {
    /// Whether the command succeeded.
    pub success: bool,
    /// Result data (on success) or error message (on failure).
    pub data: serde_json::Value,
}

/// Handle an IPC request from the frontend.
///
/// Phase 2: This will be called by the CEF message router when a
/// cefQuery message arrives from JavaScript.
#[allow(dead_code)]
pub fn handle_ipc_request(request: &IpcRequest) -> IpcResponse {
    tracing::debug!("IPC request: cmd={} args={}", request.cmd, request.args);

    // Phase 2: Route to appropriate handler based on cmd name.
    // For now, return an error indicating the IPC bridge is not yet implemented.
    IpcResponse {
        success: false,
        data: serde_json::json!({
            "error": format!("IPC command '{}' not yet implemented (Phase 2)", request.cmd)
        }),
    }
}
