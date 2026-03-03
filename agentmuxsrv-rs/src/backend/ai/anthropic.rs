// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Anthropic AI backend: Claude API with SSE streaming.
//! Port of Go's pkg/aiusechat/anthropic/anthropic-backend.go.
//!
//! Streaming protocol:
//! - POST to /v1/messages with `stream: true`
//! - Response is SSE with events: message_start, content_block_start,
//!   content_block_delta, content_block_stop, message_delta, message_stop
//! - Supports text, thinking, and tool_use content blocks
//!
//! Note: This module provides the type definitions and backend structure.
//! Actual HTTP streaming requires the `reqwest` crate which will be added
//! when the AI feature is fully wired into the Tauri app.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{AIBackend, AIOptsType, AIStreamEvent, AIStreamRequest, StopReason};

// ---- Anthropic API constants ----

pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";

// ---- Anthropic message types ----

/// Anthropic API message format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
}

/// Content block within an Anthropic message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
    },

    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        signature: String,
    },

    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        is_error: bool,
        content: serde_json::Value,
    },

    #[serde(rename = "image")]
    Image {
        source: AnthropicSource,
    },
}

/// Source for image/file content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

/// Anthropic API request body.
#[derive(Debug, Clone, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: i32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<AnthropicThinking>,
}

/// Anthropic thinking/reasoning configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicThinking {
    #[serde(rename = "type")]
    pub thinking_type: String,
    pub budget_tokens: i32,
}

// ---- SSE event types from Anthropic API ----

/// Anthropic SSE event types.
pub const EVENT_MESSAGE_START: &str = "message_start";
pub const EVENT_CONTENT_BLOCK_START: &str = "content_block_start";
pub const EVENT_CONTENT_BLOCK_DELTA: &str = "content_block_delta";
pub const EVENT_CONTENT_BLOCK_STOP: &str = "content_block_stop";
pub const EVENT_MESSAGE_DELTA: &str = "message_delta";
pub const EVENT_MESSAGE_STOP: &str = "message_stop";
pub const EVENT_PING: &str = "ping";
pub const EVENT_ERROR: &str = "error";

// ---- Anthropic backend ----

/// Anthropic AI backend implementation.
pub struct AnthropicBackend {
    opts: AIOptsType,
}

impl AnthropicBackend {
    pub fn new(opts: AIOptsType) -> Self {
        Self { opts }
    }

    /// Build the API URL (supports custom base URL).
    pub fn api_url(&self) -> String {
        if self.opts.baseurl.is_empty() {
            ANTHROPIC_API_URL.to_string()
        } else {
            format!("{}/v1/messages", self.opts.baseurl.trim_end_matches('/'))
        }
    }

    /// Build the request body from a stream request.
    pub fn build_request(&self, request: &AIStreamRequest) -> AnthropicRequest {
        let mut system_prompt = None;
        let mut messages = Vec::new();

        for msg in &request.prompt {
            if msg.role == "system" {
                system_prompt = Some(msg.content.clone());
            } else {
                messages.push(AnthropicMessage {
                    role: msg.role.clone(),
                    content: vec![AnthropicContentBlock::Text {
                        text: msg.content.clone(),
                    }],
                });
            }
        }

        AnthropicRequest {
            model: self.opts.effective_model().to_string(),
            max_tokens: self.opts.effective_max_tokens(),
            messages,
            system: system_prompt,
            stream: true,
            tools: None,
            thinking: None,
        }
    }
}

impl AIBackend for AnthropicBackend {
    fn stream_completion(
        &self,
        request: AIStreamRequest,
        event_tx: mpsc::Sender<AIStreamEvent>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<StopReason, String>> + Send + '_>>
    {
        Box::pin(async move {
            // Build the request
            let _api_request = self.build_request(&request);
            let _api_url = self.api_url();

            // Note: Actual HTTP streaming will be implemented when reqwest is added.
            let _ = event_tx
                .send(AIStreamEvent::Error {
                    message:
                        "Anthropic backend: HTTP streaming not yet implemented (requires reqwest)"
                            .to_string(),
                })
                .await;

            Ok(StopReason::error("HTTP streaming not yet implemented"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_backend_new() {
        let opts = AIOptsType {
            apitype: "anthropic".to_string(),
            apitoken: "sk-ant-test".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            ..Default::default()
        };
        let backend = AnthropicBackend::new(opts);
        assert_eq!(backend.api_url(), ANTHROPIC_API_URL);
    }

    #[test]
    fn test_anthropic_custom_url() {
        let opts = AIOptsType {
            baseurl: "http://localhost:8080".to_string(),
            ..Default::default()
        };
        let backend = AnthropicBackend::new(opts);
        assert_eq!(backend.api_url(), "http://localhost:8080/v1/messages");
    }

    #[test]
    fn test_build_request() {
        let opts = AIOptsType {
            model: "claude-sonnet-4-5".to_string(),
            maxtokens: 2048,
            ..Default::default()
        };
        let backend = AnthropicBackend::new(opts);

        let request = AIStreamRequest {
            clientid: "client-1".to_string(),
            opts: AIOptsType::default(),
            prompt: vec![
                super::super::PromptMessage {
                    role: "system".to_string(),
                    content: "You are helpful.".to_string(),
                    name: String::new(),
                },
                super::super::PromptMessage {
                    role: "user".to_string(),
                    content: "Hello!".to_string(),
                    name: String::new(),
                },
            ],
        };

        let api_req = backend.build_request(&request);
        assert_eq!(api_req.model, "claude-sonnet-4-5");
        assert_eq!(api_req.max_tokens, 2048);
        assert_eq!(api_req.system.unwrap(), "You are helpful.");
        assert_eq!(api_req.messages.len(), 1);
        assert_eq!(api_req.messages[0].role, "user");
        assert!(api_req.stream);
    }

    #[test]
    fn test_anthropic_content_block_text() {
        let block = AnthropicContentBlock::Text {
            text: "Hello world".to_string(),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello world\""));
    }

    #[test]
    fn test_anthropic_content_block_tool_use() {
        let block = AnthropicContentBlock::ToolUse {
            id: "tool-1".to_string(),
            name: "read_file".to_string(),
            input: serde_json::json!({"path": "/tmp/test"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"tool_use\""));
        assert!(json.contains("\"name\":\"read_file\""));
    }

    #[test]
    fn test_anthropic_content_block_tool_result() {
        let block = AnthropicContentBlock::ToolResult {
            tool_use_id: "tool-1".to_string(),
            is_error: false,
            content: serde_json::json!("file contents here"),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"tool_result\""));
        assert!(json.contains("\"tool_use_id\":\"tool-1\""));
    }

    #[test]
    fn test_anthropic_thinking_block() {
        let block = AnthropicContentBlock::Thinking {
            thinking: "Let me analyze...".to_string(),
            signature: String::new(),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"thinking\""));
        assert!(!json.contains("\"signature\"")); // empty, omitted
    }

    #[test]
    fn test_anthropic_request_serde() {
        let req = AnthropicRequest {
            model: "claude-sonnet-4-5".to_string(),
            max_tokens: 4096,
            messages: vec![],
            system: Some("Be helpful".to_string()),
            stream: true,
            tools: None,
            thinking: Some(AnthropicThinking {
                thinking_type: "enabled".to_string(),
                budget_tokens: 1024,
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"model\":\"claude-sonnet-4-5\""));
        assert!(json.contains("\"stream\":true"));
        assert!(json.contains("\"budget_tokens\":1024"));
        assert!(!json.contains("\"tools\"")); // None, omitted
    }

    #[test]
    fn test_sse_event_constants() {
        assert_eq!(EVENT_MESSAGE_START, "message_start");
        assert_eq!(EVENT_CONTENT_BLOCK_DELTA, "content_block_delta");
        assert_eq!(EVENT_MESSAGE_STOP, "message_stop");
    }

    #[tokio::test]
    async fn test_stream_completion_not_implemented() {
        let opts = AIOptsType {
            apitype: "anthropic".to_string(),
            apitoken: "sk-test".to_string(),
            ..Default::default()
        };
        let backend = AnthropicBackend::new(opts.clone());
        let (tx, mut rx) = mpsc::channel(10);

        let request = AIStreamRequest {
            clientid: "test".to_string(),
            opts,
            prompt: vec![],
        };

        let result = backend.stream_completion(request, tx).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().kind, "error");

        // Should have received an error event
        let event = rx.recv().await.unwrap();
        match event {
            AIStreamEvent::Error { message } => {
                assert!(message.contains("not yet implemented"));
            }
            _ => panic!("expected error event"),
        }
    }
}
