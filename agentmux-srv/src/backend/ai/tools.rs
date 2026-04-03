// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AI tool definitions and execution framework.
//! Port of Go's pkg/aiusechat/uctypes/usechat-types.go (tool types)

#![allow(dead_code)]
//! and pkg/aiusechat/tools*.go (tool definitions and execution).
//!
//! Tools are functions that AI models can call during conversations.
//! Each tool has a JSON Schema for input validation, a callback for execution,
//! and an optional approval requirement.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::time;

// ---- Tool approval constants ----

pub const APPROVAL_NEEDS_APPROVAL: &str = "needs-approval";
pub const APPROVAL_USER_APPROVED: &str = "user-approved";
pub const APPROVAL_USER_DENIED: &str = "user-denied";
pub const APPROVAL_TIMEOUT: &str = "timeout";
pub const APPROVAL_AUTO_APPROVED: &str = "auto-approved";

// ---- Tool use status constants ----

pub const TOOL_USE_STATUS_PENDING: &str = "pending";
pub const TOOL_USE_STATUS_ERROR: &str = "error";
pub const TOOL_USE_STATUS_COMPLETED: &str = "completed";

// ---- Approval timeouts ----

/// Initial timeout for user to approve a tool call (10 seconds).
pub const INITIAL_APPROVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Extension when keepalive received (10 seconds).
pub const KEEPALIVE_EXTENSION: Duration = Duration::from_secs(10);

// ---- Tool call types ----

/// A tool call from the AI model (matches Go's WaveToolCall).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveToolCall {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
}

/// Result of executing a tool (matches Go's AIToolResult).
#[derive(Debug, Clone)]
pub struct AIToolResult {
    pub text: String,
    pub error_text: String,
    pub is_error: bool,
}

impl AIToolResult {
    pub fn success(text: String) -> Self {
        Self {
            text,
            error_text: String::new(),
            is_error: false,
        }
    }

    pub fn error(msg: String) -> Self {
        Self {
            text: String::new(),
            error_text: msg,
            is_error: true,
        }
    }
}

// ---- Tool definition ----

/// Type alias for async tool callback.
pub type ToolCallback = Box<
    dyn Fn(serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Type alias for approval check function.
pub type ApprovalFn = Box<dyn Fn(&serde_json::Value) -> String + Send + Sync>;

/// Definition of a tool that AI can invoke.
pub struct ToolDefinition {
    /// Unique tool name (used in API calls).
    pub name: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Description shown to the AI model.
    pub description: String,
    /// Short description for UI.
    pub short_description: String,
    /// Telemetry log name (e.g., "term:getscrollback").
    pub tool_log_name: String,
    /// JSON Schema for tool input parameters.
    pub input_schema: serde_json::Value,
    /// Whether the schema is strict (no extra properties).
    pub strict: bool,
    /// Callback that executes the tool. Returns result text or error.
    pub callback: Option<ToolCallback>,
    /// Function that returns approval status for given input.
    /// Returns "" for auto-approved, APPROVAL_NEEDS_APPROVAL for manual approval.
    pub approval_fn: Option<ApprovalFn>,
}

impl ToolDefinition {
    /// Create a simple tool that is auto-approved.
    pub fn new_auto_approved(
        name: &str,
        description: &str,
        schema: serde_json::Value,
        callback: ToolCallback,
    ) -> Self {
        Self {
            name: name.to_string(),
            display_name: name.to_string(),
            description: description.to_string(),
            short_description: String::new(),
            tool_log_name: format!("gen:{name}"),
            input_schema: schema,
            strict: false,
            callback: Some(callback),
            approval_fn: None,
        }
    }

    /// Create a tool that requires user approval.
    pub fn new_with_approval(
        name: &str,
        description: &str,
        schema: serde_json::Value,
        callback: ToolCallback,
        approval_fn: ApprovalFn,
    ) -> Self {
        Self {
            name: name.to_string(),
            display_name: name.to_string(),
            description: description.to_string(),
            short_description: String::new(),
            tool_log_name: format!("gen:{name}"),
            input_schema: schema,
            strict: false,
            callback: Some(callback),
            approval_fn: Some(approval_fn),
        }
    }

    /// Check if this tool needs approval for the given input.
    pub fn needs_approval(&self, input: &serde_json::Value) -> bool {
        match &self.approval_fn {
            Some(f) => f(input) == APPROVAL_NEEDS_APPROVAL,
            None => false, // Auto-approved by default
        }
    }

    /// Convert to the JSON representation sent to the AI model.
    pub fn to_api_format(&self) -> serde_json::Value {
        let mut tool = serde_json::json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.input_schema,
        });
        if self.strict {
            tool["strict"] = serde_json::Value::Bool(true);
        }
        tool
    }
}

// ---- Tool approval registry ----

/// State for a pending approval request.
struct ApprovalRequest {
    approval: String,
    done: bool,
    /// Watch sender — notifies all receivers when approval resolves.
    done_tx: watch::Sender<String>,
}

/// Global registry for pending tool approval requests.
pub struct ApprovalRegistry {
    requests: Mutex<HashMap<String, ApprovalRequest>>,
}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new approval request for a tool call.
    /// Returns a receiver that will be notified when approval resolves.
    /// Multiple receivers can be created from this via `wait_for_approval()`.
    pub fn register(&self, tool_call_id: &str) -> watch::Receiver<String> {
        let (tx, rx) = watch::channel(String::new());
        let mut requests = self.requests.lock().unwrap();
        requests.insert(
            tool_call_id.to_string(),
            ApprovalRequest {
                approval: String::new(),
                done: false,
                done_tx: tx,
            },
        );
        rx
    }

    /// Update approval status for a tool call.
    /// If `keep_alive` is true and approval is empty, extends the timeout.
    pub fn update(&self, tool_call_id: &str, approval: &str) -> Result<(), String> {
        let mut requests = self.requests.lock().unwrap();
        let request = requests
            .get_mut(tool_call_id)
            .ok_or_else(|| format!("no pending approval for {tool_call_id}"))?;

        if request.done {
            return Err(format!("approval for {tool_call_id} already resolved"));
        }

        if !approval.is_empty() {
            request.approval = approval.to_string();
            request.done = true;
            let _ = request.done_tx.send(approval.to_string());
        }

        Ok(())
    }

    /// Wait for approval with timeout.
    /// Returns the approval status or APPROVAL_TIMEOUT.
    /// Safe to call concurrently — subscribes to the existing watch channel
    /// without replacing the sender, so all waiters are notified.
    pub async fn wait_for_approval(&self, tool_call_id: &str) -> String {
        let mut rx = {
            let requests = self.requests.lock().unwrap();
            match requests.get(tool_call_id) {
                Some(req) if req.done => return req.approval.clone(),
                Some(req) => req.done_tx.subscribe(),
                None => return APPROVAL_TIMEOUT.to_string(),
            }
        };

        // Wait for a non-empty value (approval resolved) or timeout
        let result = time::timeout(INITIAL_APPROVAL_TIMEOUT, async {
            loop {
                if rx.changed().await.is_err() {
                    // Sender dropped without resolving
                    break APPROVAL_TIMEOUT.to_string();
                }
                let val = rx.borrow_and_update().clone();
                if !val.is_empty() {
                    break val;
                }
            }
        })
        .await;

        match result {
            Ok(approval) => approval,
            Err(_) => {
                // Timeout — mark as timed out
                let mut requests = self.requests.lock().unwrap();
                if let Some(req) = requests.get_mut(tool_call_id) {
                    if !req.done {
                        req.approval = APPROVAL_TIMEOUT.to_string();
                        req.done = true;
                        let _ = req.done_tx.send(APPROVAL_TIMEOUT.to_string());
                    }
                    req.approval.clone()
                } else {
                    APPROVAL_TIMEOUT.to_string()
                }
            }
        }
    }

    /// Get current approval status without blocking.
    pub fn current_status(&self, tool_call_id: &str) -> Option<String> {
        let requests = self.requests.lock().unwrap();
        requests.get(tool_call_id).map(|r| r.approval.clone())
    }

    /// Clean up a resolved approval request.
    pub fn cleanup(&self, tool_call_id: &str) {
        self.requests.lock().unwrap().remove(tool_call_id);
    }
}

impl Default for ApprovalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Tool execution ----

/// Execute a tool call against a set of tool definitions.
pub async fn resolve_tool_call(
    tool_call: &WaveToolCall,
    tools: &[Arc<ToolDefinition>],
) -> AIToolResult {
    // Find the tool definition
    let tool = match tools.iter().find(|t| t.name == tool_call.name) {
        Some(t) => t,
        None => {
            return AIToolResult::error(format!("unknown tool: {}", tool_call.name));
        }
    };

    // Get the callback
    let callback = match &tool.callback {
        Some(cb) => cb,
        None => {
            return AIToolResult::error(format!("tool {} has no callback", tool_call.name));
        }
    };

    // Execute the callback
    let input = tool_call
        .input
        .clone()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    match callback(input).await {
        Ok(text) => AIToolResult::success(text),
        Err(e) => AIToolResult::error(e),
    }
}

// ---- Built-in tool definitions ----

/// Create a "read_file" tool definition.
pub fn read_file_tool() -> ToolDefinition {
    ToolDefinition::new_auto_approved(
        "read_file",
        "Read the contents of a text file. Returns the file content as text.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to read"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        Box::new(|_input| {
            Box::pin(async {
                // Stub: actual implementation will use filesystem
                Err("read_file not yet implemented".to_string())
            })
        }),
    )
}

/// Create a "read_dir" tool definition.
pub fn read_dir_tool() -> ToolDefinition {
    ToolDefinition::new_auto_approved(
        "read_dir",
        "List the contents of a directory. Returns file and subdirectory names.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory path to list"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        Box::new(|_input| {
            Box::pin(async {
                // Stub: actual implementation will use filesystem
                Err("read_dir not yet implemented".to_string())
            })
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wave_tool_call_serde() {
        let call = WaveToolCall {
            id: "call-1".to_string(),
            name: "read_file".to_string(),
            input: Some(serde_json::json!({"path": "/tmp/test.txt"})),
        };
        let json = serde_json::to_string(&call).unwrap();
        assert!(json.contains("\"name\":\"read_file\""));

        let parsed: WaveToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "call-1");
        assert_eq!(parsed.name, "read_file");
    }

    #[test]
    fn test_tool_result_success() {
        let result = AIToolResult::success("file contents".to_string());
        assert!(!result.is_error);
        assert_eq!(result.text, "file contents");
        assert!(result.error_text.is_empty());
    }

    #[test]
    fn test_tool_result_error() {
        let result = AIToolResult::error("file not found".to_string());
        assert!(result.is_error);
        assert_eq!(result.error_text, "file not found");
        assert!(result.text.is_empty());
    }

    #[test]
    fn test_tool_definition_auto_approved() {
        let tool = ToolDefinition::new_auto_approved(
            "test_tool",
            "A test tool",
            serde_json::json!({"type": "object"}),
            Box::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        );
        assert_eq!(tool.name, "test_tool");
        assert!(!tool.needs_approval(&serde_json::Value::Null));
    }

    #[test]
    fn test_tool_definition_with_approval() {
        let tool = ToolDefinition::new_with_approval(
            "dangerous_tool",
            "Does something dangerous",
            serde_json::json!({"type": "object"}),
            Box::new(|_| Box::pin(async { Ok("ok".to_string()) })),
            Box::new(|_| APPROVAL_NEEDS_APPROVAL.to_string()),
        );
        assert!(tool.needs_approval(&serde_json::Value::Null));
    }

    #[test]
    fn test_tool_to_api_format() {
        let tool = ToolDefinition::new_auto_approved(
            "read_file",
            "Read a file",
            serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
            Box::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        );

        let api = tool.to_api_format();
        assert_eq!(api["name"], "read_file");
        assert_eq!(api["description"], "Read a file");
        assert!(api["input_schema"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_approval_constants() {
        assert_eq!(APPROVAL_NEEDS_APPROVAL, "needs-approval");
        assert_eq!(APPROVAL_USER_APPROVED, "user-approved");
        assert_eq!(APPROVAL_USER_DENIED, "user-denied");
        assert_eq!(APPROVAL_TIMEOUT, "timeout");
        assert_eq!(APPROVAL_AUTO_APPROVED, "auto-approved");
    }

    #[test]
    fn test_tool_use_status_constants() {
        assert_eq!(TOOL_USE_STATUS_PENDING, "pending");
        assert_eq!(TOOL_USE_STATUS_ERROR, "error");
        assert_eq!(TOOL_USE_STATUS_COMPLETED, "completed");
    }

    #[test]
    fn test_approval_registry_basic() {
        let registry = ApprovalRegistry::new();

        let _rx = registry.register("call-1");
        assert_eq!(registry.current_status("call-1"), Some(String::new()));

        registry
            .update("call-1", APPROVAL_USER_APPROVED)
            .unwrap();
        assert_eq!(
            registry.current_status("call-1"),
            Some(APPROVAL_USER_APPROVED.to_string())
        );
    }

    #[test]
    fn test_approval_registry_update_nonexistent() {
        let registry = ApprovalRegistry::new();
        let result = registry.update("nonexistent", APPROVAL_USER_APPROVED);
        assert!(result.is_err());
    }

    #[test]
    fn test_approval_registry_cleanup() {
        let registry = ApprovalRegistry::new();
        let _rx = registry.register("call-1");
        registry.cleanup("call-1");
        assert!(registry.current_status("call-1").is_none());
    }

    #[tokio::test]
    async fn test_approval_registry_immediate_resolve() {
        let registry = Arc::new(ApprovalRegistry::new());
        let _rx = registry.register("call-1");

        // Approve immediately
        registry
            .update("call-1", APPROVAL_USER_APPROVED)
            .unwrap();

        let result = registry.wait_for_approval("call-1").await;
        assert_eq!(result, APPROVAL_USER_APPROVED);
    }

    #[tokio::test]
    async fn test_resolve_tool_call_unknown() {
        let call = WaveToolCall {
            id: "call-1".to_string(),
            name: "unknown_tool".to_string(),
            input: None,
        };
        let tools: Vec<Arc<ToolDefinition>> = vec![];
        let result = resolve_tool_call(&call, &tools).await;
        assert!(result.is_error);
        assert!(result.error_text.contains("unknown tool"));
    }

    #[tokio::test]
    async fn test_resolve_tool_call_success() {
        let tool = Arc::new(ToolDefinition::new_auto_approved(
            "echo",
            "Echo input",
            serde_json::json!({"type": "object"}),
            Box::new(|input| {
                Box::pin(async move {
                    let text = input
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("no input");
                    Ok(format!("echo: {text}"))
                })
            }),
        ));

        let call = WaveToolCall {
            id: "call-1".to_string(),
            name: "echo".to_string(),
            input: Some(serde_json::json!({"text": "hello"})),
        };
        let result = resolve_tool_call(&call, &[tool]).await;
        assert!(!result.is_error);
        assert_eq!(result.text, "echo: hello");
    }

    #[tokio::test]
    async fn test_resolve_tool_call_error() {
        let tool = Arc::new(ToolDefinition::new_auto_approved(
            "failing",
            "Always fails",
            serde_json::json!({"type": "object"}),
            Box::new(|_| Box::pin(async { Err("intentional error".to_string()) })),
        ));

        let call = WaveToolCall {
            id: "call-1".to_string(),
            name: "failing".to_string(),
            input: None,
        };
        let result = resolve_tool_call(&call, &[tool]).await;
        assert!(result.is_error);
        assert_eq!(result.error_text, "intentional error");
    }

    #[test]
    fn test_builtin_read_file_tool() {
        let tool = read_file_tool();
        assert_eq!(tool.name, "read_file");
        assert!(!tool.needs_approval(&serde_json::Value::Null));
        let api = tool.to_api_format();
        assert!(api["input_schema"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_builtin_read_dir_tool() {
        let tool = read_dir_tool();
        assert_eq!(tool.name, "read_dir");
        let api = tool.to_api_format();
        assert!(api["input_schema"]["properties"]["path"].is_object());
    }
}
