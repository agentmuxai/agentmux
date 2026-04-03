// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! User input: modal dialogs for interactive user prompts.
//! Port of Go's pkg/userinput/userinput.go.
//!
//! Provides request/response types for:
//! - Text input prompts
//! - Confirmation dialogs
//! - Checkbox-bearing dialogs
//!
//! The actual display is handled by the frontend (Tauri webview);
//! this module defines the wire format and a registry for pending requests.


use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::time;

// ---- Request/Response types ----

/// Request for user input, sent to the frontend for display.
/// Matches Go's `userinput.UserInputRequest` JSON tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInputRequest {
    /// Unique request ID.
    #[serde(rename = "requestid")]
    pub request_id: String,

    /// Prompt text to display.
    #[serde(rename = "querytext")]
    pub query_text: String,

    /// Expected response type: "text", "confirm".
    #[serde(rename = "responsetype")]
    pub response_type: String,

    /// Dialog title.
    pub title: String,

    /// Whether to render title as markdown.
    #[serde(default)]
    pub markdown: bool,

    /// Timeout in milliseconds (0 = no timeout).
    #[serde(rename = "timeoutms", default)]
    pub timeout_ms: i64,

    /// Optional checkbox label.
    #[serde(rename = "checkboxmsg", default, skip_serializing_if = "String::is_empty")]
    pub checkbox_msg: String,

    /// Whether the input text is public (can be logged).
    #[serde(rename = "publictext", default)]
    pub public_text: bool,

    /// Custom "OK" button label.
    #[serde(rename = "oklabel", default, skip_serializing_if = "String::is_empty")]
    pub ok_label: String,

    /// Custom "Cancel" button label.
    #[serde(rename = "cancellabel", default, skip_serializing_if = "String::is_empty")]
    pub cancel_label: String,
}

/// Response from the frontend after user interaction.
/// Matches Go's `userinput.UserInputResponse` JSON tags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserInputResponse {
    /// Response type.
    #[serde(rename = "type", default)]
    pub response_type: String,

    /// Request ID this responds to.
    #[serde(rename = "requestid")]
    pub request_id: String,

    /// Text input value (for text responses).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,

    /// Confirmation result (for confirm responses).
    #[serde(default)]
    pub confirm: bool,

    /// Error message (if the dialog was cancelled or errored).
    #[serde(rename = "errormsg", default, skip_serializing_if = "String::is_empty")]
    pub error_msg: String,

    /// Checkbox state.
    #[serde(rename = "checkboxstat", default)]
    pub checkbox_stat: bool,
}

impl UserInputResponse {
    /// Check if the response indicates an error.
    pub fn is_error(&self) -> bool {
        !self.error_msg.is_empty()
    }

    /// Check if the response is a confirmation.
    pub fn is_confirmed(&self) -> bool {
        self.confirm && self.error_msg.is_empty()
    }
}

// ---- Response type constants ----

pub const RESPONSE_TYPE_TEXT: &str = "text";
pub const RESPONSE_TYPE_CONFIRM: &str = "confirm";

// ---- Default timeout ----

/// Default timeout for user input prompts (30 seconds).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout used for SSH passphrase/password prompts (60 seconds).
pub const SSH_PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

// ---- Input handler registry ----

/// Registry for pending user input requests.
/// Routes responses from the frontend back to waiting async tasks.
pub struct UserInputHandler {
    channels: Mutex<HashMap<String, oneshot::Sender<UserInputResponse>>>,
}

impl UserInputHandler {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new user input request.
    /// Returns the request ID and a receiver for the response.
    pub fn register(&self, request_id: &str) -> oneshot::Receiver<UserInputResponse> {
        let (tx, rx) = oneshot::channel();
        let mut channels = self.channels.lock().unwrap();
        channels.insert(request_id.to_string(), tx);
        rx
    }

    /// Deliver a response from the frontend.
    /// Returns error if no pending request with the given ID.
    pub fn deliver(&self, response: UserInputResponse) -> Result<(), String> {
        let mut channels = self.channels.lock().unwrap();
        let tx = channels
            .remove(&response.request_id)
            .ok_or_else(|| format!("no pending request: {}", response.request_id))?;
        tx.send(response)
            .map_err(|_| "receiver dropped".to_string())
    }

    /// Cancel a pending request (removes it from the registry).
    pub fn cancel(&self, request_id: &str) {
        let mut channels = self.channels.lock().unwrap();
        channels.remove(request_id);
    }

    /// Check if there's a pending request with the given ID.
    pub fn has_pending(&self, request_id: &str) -> bool {
        let channels = self.channels.lock().unwrap();
        channels.contains_key(request_id)
    }

    /// Get count of pending requests.
    pub fn pending_count(&self) -> usize {
        let channels = self.channels.lock().unwrap();
        channels.len()
    }
}

impl Default for UserInputHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait for a user input response with timeout.
pub async fn wait_for_response(
    rx: oneshot::Receiver<UserInputResponse>,
    timeout: Duration,
) -> Result<UserInputResponse, String> {
    match time::timeout(timeout, rx).await {
        Ok(Ok(response)) => {
            if response.is_error() {
                Err(response.error_msg)
            } else {
                Ok(response)
            }
        }
        Ok(Err(_)) => Err("user input handler was cancelled".to_string()),
        Err(_) => Err("timed out waiting for user input".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_input_request_serde() {
        let req = UserInputRequest {
            request_id: "req-1".to_string(),
            query_text: "Enter password:".to_string(),
            response_type: RESPONSE_TYPE_TEXT.to_string(),
            title: "SSH Authentication".to_string(),
            markdown: true,
            timeout_ms: 60000,
            checkbox_msg: String::new(),
            public_text: false,
            ok_label: String::new(),
            cancel_label: String::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"requestid\":\"req-1\""));
        assert!(json.contains("\"querytext\":\"Enter password:\""));
        assert!(json.contains("\"responsetype\":\"text\""));
        assert!(json.contains("\"timeoutms\":60000"));
        // Empty fields should be omitted
        assert!(!json.contains("\"checkboxmsg\""));
        assert!(!json.contains("\"oklabel\""));

        let parsed: UserInputRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.request_id, "req-1");
        assert!(parsed.markdown);
    }

    #[test]
    fn test_user_input_response_serde() {
        let resp = UserInputResponse {
            response_type: RESPONSE_TYPE_TEXT.to_string(),
            request_id: "req-1".to_string(),
            text: "my_password".to_string(),
            confirm: false,
            error_msg: String::new(),
            checkbox_stat: false,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"requestid\":\"req-1\""));
        assert!(json.contains("\"text\":\"my_password\""));
        assert!(!json.contains("\"errormsg\"")); // empty, omitted

        let parsed: UserInputResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, "my_password");
    }

    #[test]
    fn test_response_is_error() {
        let resp = UserInputResponse {
            error_msg: "cancelled".to_string(),
            ..Default::default()
        };
        assert!(resp.is_error());
        assert!(!resp.is_confirmed());
    }

    #[test]
    fn test_response_is_confirmed() {
        let resp = UserInputResponse {
            confirm: true,
            ..Default::default()
        };
        assert!(resp.is_confirmed());
        assert!(!resp.is_error());
    }

    #[test]
    fn test_handler_register_deliver() {
        let handler = UserInputHandler::new();
        let mut rx = handler.register("req-1");
        assert!(handler.has_pending("req-1"));
        assert_eq!(handler.pending_count(), 1);

        let response = UserInputResponse {
            request_id: "req-1".to_string(),
            text: "hello".to_string(),
            ..Default::default()
        };
        handler.deliver(response).unwrap();
        assert!(!handler.has_pending("req-1"));

        let received = rx.try_recv().unwrap();
        assert_eq!(received.text, "hello");
    }

    #[test]
    fn test_handler_deliver_no_pending() {
        let handler = UserInputHandler::new();
        let response = UserInputResponse {
            request_id: "nonexistent".to_string(),
            ..Default::default()
        };
        assert!(handler.deliver(response).is_err());
    }

    #[test]
    fn test_handler_cancel() {
        let handler = UserInputHandler::new();
        let _rx = handler.register("req-1");
        assert!(handler.has_pending("req-1"));
        handler.cancel("req-1");
        assert!(!handler.has_pending("req-1"));
    }

    #[tokio::test]
    async fn test_wait_for_response_success() {
        let handler = UserInputHandler::new();
        let rx = handler.register("req-1");

        // Deliver response in background
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            handler
                .deliver(UserInputResponse {
                    request_id: "req-1".to_string(),
                    text: "password123".to_string(),
                    ..Default::default()
                })
                .unwrap();
        });

        let result = wait_for_response(rx, Duration::from_secs(5)).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text, "password123");
    }

    #[tokio::test]
    async fn test_wait_for_response_error() {
        let handler = UserInputHandler::new();
        let rx = handler.register("req-1");

        tokio::spawn(async move {
            handler
                .deliver(UserInputResponse {
                    request_id: "req-1".to_string(),
                    error_msg: "user cancelled".to_string(),
                    ..Default::default()
                })
                .unwrap();
        });

        let result = wait_for_response(rx, Duration::from_secs(5)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("user cancelled"));
    }

    #[tokio::test]
    async fn test_wait_for_response_timeout() {
        let (_tx, rx) = oneshot::channel::<UserInputResponse>();
        let result = wait_for_response(rx, Duration::from_millis(10)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));
    }

    #[test]
    fn test_confirm_request_serde() {
        let req = UserInputRequest {
            request_id: "req-2".to_string(),
            query_text: "Add host key?".to_string(),
            response_type: RESPONSE_TYPE_CONFIRM.to_string(),
            title: "SSH Host Key".to_string(),
            markdown: true,
            timeout_ms: 30000,
            checkbox_msg: "Don't ask again".to_string(),
            public_text: true,
            ok_label: "Accept".to_string(),
            cancel_label: "Reject".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"checkboxmsg\":\"Don't ask again\""));
        assert!(json.contains("\"oklabel\":\"Accept\""));
        assert!(json.contains("\"cancellabel\":\"Reject\""));
    }
}
