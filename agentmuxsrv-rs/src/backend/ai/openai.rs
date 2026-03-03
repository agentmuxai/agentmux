// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! OpenAI AI backend: compatible with OpenAI, Perplexity, and local APIs.
//! Port of Go's pkg/aiusechat/openai/openai-backend.go.
//!
//! Streaming protocol:
//! - POST to /v1/responses with `stream: true`
//! - Response is SSE with events: response.created, response.in_progress,
//!   response.output_text.delta, response.completed, etc.
//!
//! Note: This module provides the type definitions and backend structure.
//! Actual HTTP streaming requires the `reqwest` crate which will be added
//! when the AI feature is fully wired into the Tauri app.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{AIBackend, AIOptsType, AIStreamEvent, AIStreamRequest, StopReason};

// ---- OpenAI API constants ----

pub const OPENAI_API_URL: &str = "https://api.openai.com/v1/responses";

// ---- OpenAI message types ----

/// OpenAI API message format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: Vec<OpenAIContentPart>,
}

/// Content part within an OpenAI message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OpenAIContentPart {
    #[serde(rename = "input_text")]
    InputText { text: String },

    #[serde(rename = "output_text")]
    OutputText { text: String },

    #[serde(rename = "input_image")]
    InputImage {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        image_url: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        file_data: String,
    },

    #[serde(rename = "input_file")]
    InputFile {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        filename: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        file_data: String,
    },
}

/// OpenAI function call input (tool use).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIFunctionCall {
    #[serde(rename = "type")]
    pub call_type: String,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
}

/// OpenAI function call output (tool result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIFunctionCallOutput {
    #[serde(rename = "type")]
    pub output_type: String,
    pub call_id: String,
    pub output: serde_json::Value,
}

/// OpenAI API request body.
#[derive(Debug, Clone, Serialize)]
pub struct OpenAIRequest {
    pub model: String,
    pub input: Vec<serde_json::Value>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// OpenAI usage information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAIUsage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
}

// ---- SSE event types from OpenAI API ----

pub const EVENT_RESPONSE_CREATED: &str = "response.created";
pub const EVENT_RESPONSE_IN_PROGRESS: &str = "response.in_progress";
pub const EVENT_OUTPUT_ITEM_ADDED: &str = "response.output_item.added";
pub const EVENT_OUTPUT_ITEM_DONE: &str = "response.output_item.done";
pub const EVENT_CONTENT_PART_ADDED: &str = "response.content_part.added";
pub const EVENT_CONTENT_PART_DONE: &str = "response.content_part.done";
pub const EVENT_OUTPUT_TEXT_DELTA: &str = "response.output_text.delta";
pub const EVENT_FUNCTION_CALL_ARGS_DELTA: &str = "response.function_call_arguments.delta";
pub const EVENT_FUNCTION_CALL_ARGS_DONE: &str = "response.function_call_arguments.done";
pub const EVENT_REASONING_SUMMARY_ADDED: &str = "response.reasoning_summary_part.added";
pub const EVENT_REASONING_SUMMARY_DONE: &str = "response.reasoning_summary_part.done";
pub const EVENT_WEB_SEARCH_COMPLETED: &str = "response.web_search_call.completed";
pub const EVENT_RESPONSE_COMPLETED: &str = "response.completed";

// ---- OpenAI backend ----

/// OpenAI-compatible AI backend implementation.
/// Works with OpenAI, Perplexity, and local LLM APIs.
pub struct OpenAIBackend {
    opts: AIOptsType,
}

impl OpenAIBackend {
    pub fn new(opts: AIOptsType) -> Self {
        Self { opts }
    }

    /// Build the API URL (supports custom base URL).
    pub fn api_url(&self) -> String {
        if self.opts.baseurl.is_empty() {
            OPENAI_API_URL.to_string()
        } else {
            format!("{}/v1/responses", self.opts.baseurl.trim_end_matches('/'))
        }
    }

    /// Build the request body from a stream request.
    pub fn build_request(&self, request: &AIStreamRequest) -> OpenAIRequest {
        let mut instructions = None;
        let mut input_messages = Vec::new();

        for msg in &request.prompt {
            if msg.role == "system" {
                instructions = Some(msg.content.clone());
            } else {
                input_messages.push(serde_json::json!({
                    "role": msg.role,
                    "content": msg.content
                }));
            }
        }

        OpenAIRequest {
            model: self.opts.effective_model().to_string(),
            input: input_messages,
            stream: true,
            max_output_tokens: Some(self.opts.effective_max_tokens()),
            tools: None,
            instructions,
        }
    }
}

impl AIBackend for OpenAIBackend {
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
                        "OpenAI backend: HTTP streaming not yet implemented (requires reqwest)"
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
    fn test_openai_backend_new() {
        let opts = AIOptsType {
            apitype: "openai".to_string(),
            apitoken: "sk-test".to_string(),
            model: "gpt-5-mini".to_string(),
            ..Default::default()
        };
        let backend = OpenAIBackend::new(opts);
        assert_eq!(backend.api_url(), OPENAI_API_URL);
    }

    #[test]
    fn test_openai_custom_url() {
        let opts = AIOptsType {
            baseurl: "http://localhost:11434".to_string(),
            ..Default::default()
        };
        let backend = OpenAIBackend::new(opts);
        assert_eq!(backend.api_url(), "http://localhost:11434/v1/responses");
    }

    #[test]
    fn test_build_request() {
        let opts = AIOptsType {
            model: "gpt-5".to_string(),
            maxtokens: 8192,
            ..Default::default()
        };
        let backend = OpenAIBackend::new(opts);

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
        assert_eq!(api_req.model, "gpt-5");
        assert_eq!(api_req.max_output_tokens, Some(8192));
        assert_eq!(api_req.instructions.unwrap(), "You are helpful.");
        assert_eq!(api_req.input.len(), 1);
        assert!(api_req.stream);
    }

    #[test]
    fn test_openai_content_part_text() {
        let part = OpenAIContentPart::InputText {
            text: "Hello".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"input_text\""));
        assert!(json.contains("\"text\":\"Hello\""));
    }

    #[test]
    fn test_openai_function_call() {
        let call = OpenAIFunctionCall {
            call_type: "function_call".to_string(),
            call_id: "call-1".to_string(),
            name: "read_file".to_string(),
            arguments: "{\"path\":\"/tmp/test\"}".to_string(),
            status: String::new(),
        };
        let json = serde_json::to_string(&call).unwrap();
        assert!(json.contains("\"call_id\":\"call-1\""));
        assert!(json.contains("\"name\":\"read_file\""));
    }

    #[test]
    fn test_openai_function_call_output() {
        let output = OpenAIFunctionCallOutput {
            output_type: "function_call_output".to_string(),
            call_id: "call-1".to_string(),
            output: serde_json::json!("file contents here"),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"call_id\":\"call-1\""));
    }

    #[test]
    fn test_openai_request_serde() {
        let req = OpenAIRequest {
            model: "gpt-5-mini".to_string(),
            input: vec![serde_json::json!({"role": "user", "content": "Hi"})],
            stream: true,
            max_output_tokens: Some(4096),
            tools: None,
            instructions: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"model\":\"gpt-5-mini\""));
        assert!(json.contains("\"stream\":true"));
        assert!(!json.contains("\"tools\"")); // None, omitted
        assert!(!json.contains("\"instructions\"")); // None, omitted
    }

    #[test]
    fn test_sse_event_constants() {
        assert_eq!(EVENT_RESPONSE_CREATED, "response.created");
        assert_eq!(EVENT_OUTPUT_TEXT_DELTA, "response.output_text.delta");
        assert_eq!(EVENT_RESPONSE_COMPLETED, "response.completed");
    }

    #[tokio::test]
    async fn test_stream_completion_not_implemented() {
        let opts = AIOptsType {
            apitype: "openai".to_string(),
            apitoken: "sk-test".to_string(),
            ..Default::default()
        };
        let backend = OpenAIBackend::new(opts.clone());
        let (tx, mut rx) = mpsc::channel(10);

        let request = AIStreamRequest {
            clientid: "test".to_string(),
            opts,
            prompt: vec![],
        };

        let result = backend.stream_completion(request, tx).await;
        assert!(result.is_ok());

        let event = rx.recv().await.unwrap();
        match event {
            AIStreamEvent::Error { message } => {
                assert!(message.contains("not yet implemented"));
            }
            _ => panic!("expected error event"),
        }
    }
}
