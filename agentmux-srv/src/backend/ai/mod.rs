// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AI integration layer: multi-provider chat backend with streaming responses.
//! Port of Go's pkg/waveai/waveai.go + pkg/aiusechat/usechat.go.

#![allow(dead_code)]
//!
//! Architecture:
//! - `AIBackend` trait abstracts provider-specific APIs
//! - Streaming via tokio channels (`mpsc::Sender<AIStreamEvent>`)
//! - Tool execution with approval flow
//! - In-memory chat store for message history

pub mod anthropic;
pub mod chatstore;
pub mod openai;
pub mod tools;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;

// ---- API type constants (match Go) ----

pub const API_TYPE_ANTHROPIC: &str = "anthropic";
pub const API_TYPE_OPENAI: &str = "openai";
pub const API_TYPE_GOOGLE: &str = "google";
pub const API_TYPE_PERPLEXITY: &str = "perplexity";

/// Default API type when none specified.
pub const DEFAULT_API_TYPE: &str = API_TYPE_OPENAI;

// ---- Default model constants ----

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-5";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-5-mini";
pub const PREMIUM_OPENAI_MODEL: &str = "gpt-5";
pub const DEFAULT_MAX_TOKENS: i32 = 4096;

// ---- Thinking level constants ----

pub const THINKING_LEVEL_LOW: &str = "low";
pub const THINKING_LEVEL_MEDIUM: &str = "medium";
pub const THINKING_LEVEL_HIGH: &str = "high";
pub const ANTHROPIC_THINKING_BUDGET: i32 = 1024;

// ---- Stop reason kinds (match Go's StopReasonKind) ----

pub const STOP_KIND_DONE: &str = "done";
pub const STOP_KIND_TOOL_USE: &str = "tool_use";
pub const STOP_KIND_MAX_TOKENS: &str = "max_tokens";
pub const STOP_KIND_CONTENT_FILTER: &str = "content_filter";
pub const STOP_KIND_CANCELED: &str = "canceled";
pub const STOP_KIND_ERROR: &str = "error";
pub const STOP_KIND_PAUSE_TURN: &str = "pause_turn";
pub const STOP_KIND_PREMIUM_RATE_LIMIT: &str = "premium_rate_limit";
pub const STOP_KIND_RATE_LIMIT: &str = "rate_limit";

// ---- Default endpoint ----

pub const DEFAULT_AI_ENDPOINT: &str = "https://cfapi.agentmux.ai/api/waveai";

// ---- Configuration types ----

/// AI backend options (matches Go's WaveAIOptsType).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AIOptsType {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub apitype: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub apitoken: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub orgid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub apiversion: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub baseurl: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxyurl: String,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub maxtokens: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub timeoutms: i32,
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

impl AIOptsType {
    /// Get the effective API type, defaulting to OpenAI.
    pub fn effective_api_type(&self) -> &str {
        if self.apitype.is_empty() {
            DEFAULT_API_TYPE
        } else {
            &self.apitype
        }
    }

    /// Get the effective model for the given API type.
    pub fn effective_model(&self) -> &str {
        if !self.model.is_empty() {
            return &self.model;
        }
        match self.effective_api_type() {
            API_TYPE_ANTHROPIC => DEFAULT_ANTHROPIC_MODEL,
            _ => DEFAULT_OPENAI_MODEL,
        }
    }

    /// Get the effective max tokens.
    pub fn effective_max_tokens(&self) -> i32 {
        if self.maxtokens > 0 {
            self.maxtokens
        } else {
            DEFAULT_MAX_TOKENS
        }
    }

    /// Check if this is a cloud (proxy) request.
    pub fn is_cloud_request(&self) -> bool {
        self.baseurl.is_empty() && self.apitoken.is_empty()
    }
}

// ---- Prompt message type ----

/// A single message in the AI prompt (matches Go's WaveAIPromptMessageType).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

// ---- Stream request type ----

/// Request to start a streaming AI completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIStreamRequest {
    #[serde(default)]
    pub clientid: String,
    pub opts: AIOptsType,
    pub prompt: Vec<PromptMessage>,
}

// ---- Streaming event types (sent to UI via SSE) ----

/// Events emitted during AI streaming (matches Go's SSE handler events).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AIStreamEvent {
    /// Stream started.
    #[serde(rename = "start")]
    Start { messageid: String },

    /// New reasoning step started.
    #[serde(rename = "start-step")]
    StartStep,

    /// Text content started.
    #[serde(rename = "text-start")]
    TextStart { id: String },

    /// Incremental text delta.
    #[serde(rename = "text-delta")]
    TextDelta { id: String, delta: String },

    /// Text content ended.
    #[serde(rename = "text-end")]
    TextEnd { id: String },

    /// Reasoning/thinking started.
    #[serde(rename = "reasoning-start")]
    ReasoningStart { id: String },

    /// Reasoning/thinking delta.
    #[serde(rename = "reasoning-delta")]
    ReasoningDelta { id: String, delta: String },

    /// Reasoning/thinking ended.
    #[serde(rename = "reasoning-end")]
    ReasoningEnd { id: String },

    /// Tool call input started.
    #[serde(rename = "tool-input-start")]
    ToolInputStart { callid: String, toolname: String },

    /// Tool call input delta.
    #[serde(rename = "tool-input-delta")]
    ToolInputDelta { callid: String, delta: String },

    /// Tool call input fully available.
    #[serde(rename = "tool-input-available")]
    ToolInputAvailable {
        callid: String,
        toolname: String,
        input: serde_json::Value,
    },

    /// Tool use metadata for UI display.
    #[serde(rename = "data-tooluse")]
    DataToolUse { data: ToolUseData },

    /// Step finished.
    #[serde(rename = "finish-step")]
    FinishStep,

    /// Stream finished.
    #[serde(rename = "finish")]
    Finish {
        reason: String,
        metadata: Option<serde_json::Value>,
    },

    /// Error occurred.
    #[serde(rename = "error")]
    Error { message: String },
}

/// Tool use metadata displayed in UI (matches Go's UIMessageDataToolUse).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolUseData {
    pub toolcallid: String,
    pub toolname: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tooldesc: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub errormessage: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub approval: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blockid: String,
}

// ---- AI message types (stored in chat history) ----

/// A chat message with multipart content (matches Go's AIMessage).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIMessage {
    pub messageid: String,
    pub parts: Vec<AIMessagePart>,
}

/// A single part of an AI message (text, file, image).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AIMessagePart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filename: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mimetype: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub size: usize,
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

// ---- Stop reason ----

/// Why the AI stream stopped (matches Go's WaveStopReason).
#[derive(Debug, Clone)]
pub struct StopReason {
    pub kind: String,
    pub raw_reason: String,
    pub tool_calls: Vec<tools::WaveToolCall>,
}

impl StopReason {
    pub fn done() -> Self {
        Self {
            kind: STOP_KIND_DONE.to_string(),
            raw_reason: String::new(),
            tool_calls: Vec::new(),
        }
    }

    pub fn error(msg: &str) -> Self {
        Self {
            kind: STOP_KIND_ERROR.to_string(),
            raw_reason: msg.to_string(),
            tool_calls: Vec::new(),
        }
    }

    pub fn tool_use(calls: Vec<tools::WaveToolCall>) -> Self {
        Self {
            kind: STOP_KIND_TOOL_USE.to_string(),
            raw_reason: String::new(),
            tool_calls: calls,
        }
    }

    pub fn is_tool_use(&self) -> bool {
        self.kind == STOP_KIND_TOOL_USE
    }
}

// ---- AI usage metrics ----

/// Token usage tracking (matches Go's AIUsage).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AIUsage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub apitype: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
}

/// Comprehensive AI metrics (matches Go's AIMetrics).
#[derive(Debug, Clone, Default)]
pub struct AIMetrics {
    pub usage: AIUsage,
    pub request_count: i32,
    pub tool_use_count: i32,
    pub tool_use_error_count: i32,
    pub tool_detail: HashMap<String, i32>,
    pub had_error: bool,
    pub first_byte_latency_ms: i64,
    pub request_duration_ms: i64,
}

// ---- Rate limit info ----

/// Rate limit information from the API (matches Go's RateLimitInfo).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RateLimitInfo {
    #[serde(default)]
    pub req: i32,
    #[serde(default)]
    pub reqlimit: i32,
    #[serde(default)]
    pub preq: i32,
    #[serde(default)]
    pub preqlimit: i32,
    #[serde(default)]
    pub resetepoch: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unknown: bool,
}

// ---- AIBackend trait ----

/// Trait for AI provider backends.
/// Each provider (Anthropic, OpenAI, etc.) implements this to handle streaming completions.
///
/// Uses boxed futures instead of async fn to be dyn-compatible.
pub trait AIBackend: Send + Sync {
    /// Stream a completion response. Events are sent to the provided channel.
    /// The implementation should close the sender when done.
    fn stream_completion(
        &self,
        request: AIStreamRequest,
        event_tx: mpsc::Sender<AIStreamEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<StopReason, String>> + Send + '_>>;
}

// ---- Backend selection ----

/// Select the appropriate backend based on configuration.
pub fn select_backend(opts: &AIOptsType) -> Box<dyn AIBackend> {
    if opts.is_cloud_request() {
        // Cloud backend — not yet implemented, fall back to direct API
        match opts.effective_api_type() {
            API_TYPE_ANTHROPIC => Box::new(anthropic::AnthropicBackend::new(opts.clone())),
            _ => Box::new(openai::OpenAIBackend::new(opts.clone())),
        }
    } else {
        match opts.effective_api_type() {
            API_TYPE_ANTHROPIC => Box::new(anthropic::AnthropicBackend::new(opts.clone())),
            API_TYPE_OPENAI | API_TYPE_PERPLEXITY => {
                Box::new(openai::OpenAIBackend::new(opts.clone()))
            }
            _ => Box::new(openai::OpenAIBackend::new(opts.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_opts_defaults() {
        let opts = AIOptsType::default();
        assert_eq!(opts.effective_api_type(), DEFAULT_API_TYPE);
        assert_eq!(opts.effective_model(), DEFAULT_OPENAI_MODEL);
        assert_eq!(opts.effective_max_tokens(), DEFAULT_MAX_TOKENS);
        assert!(opts.is_cloud_request());
    }

    #[test]
    fn test_ai_opts_anthropic() {
        let opts = AIOptsType {
            apitype: API_TYPE_ANTHROPIC.to_string(),
            apitoken: "sk-test".to_string(),
            ..Default::default()
        };
        assert_eq!(opts.effective_api_type(), API_TYPE_ANTHROPIC);
        assert_eq!(opts.effective_model(), DEFAULT_ANTHROPIC_MODEL);
        assert!(!opts.is_cloud_request());
    }

    #[test]
    fn test_ai_opts_custom_model() {
        let opts = AIOptsType {
            model: "custom-model-v1".to_string(),
            ..Default::default()
        };
        assert_eq!(opts.effective_model(), "custom-model-v1");
    }

    #[test]
    fn test_ai_opts_serde() {
        let opts = AIOptsType {
            model: "gpt-5".to_string(),
            apitype: "openai".to_string(),
            apitoken: "sk-123".to_string(),
            maxtokens: 8192,
            ..Default::default()
        };
        let json = serde_json::to_string(&opts).unwrap();
        assert!(json.contains("\"model\":\"gpt-5\""));
        assert!(json.contains("\"maxtokens\":8192"));
        // Empty fields should be omitted
        assert!(!json.contains("\"orgid\""));
        assert!(!json.contains("\"proxyurl\""));

        let parsed: AIOptsType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, "gpt-5");
        assert_eq!(parsed.maxtokens, 8192);
    }

    #[test]
    fn test_prompt_message_serde() {
        let msg = PromptMessage {
            role: "user".to_string(),
            content: "Hello, AI!".to_string(),
            name: String::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(!json.contains("\"name\"")); // empty should be omitted

        let parsed: PromptMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, "user");
        assert_eq!(parsed.content, "Hello, AI!");
    }

    #[test]
    fn test_ai_stream_event_serde() {
        let event = AIStreamEvent::TextDelta {
            id: "text-1".to_string(),
            delta: "Hello".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"text-delta\""));
        assert!(json.contains("\"delta\":\"Hello\""));

        let error = AIStreamEvent::Error {
            message: "rate limit".to_string(),
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"type\":\"error\""));
    }

    #[test]
    fn test_stop_reason() {
        let done = StopReason::done();
        assert_eq!(done.kind, STOP_KIND_DONE);
        assert!(!done.is_tool_use());

        let tool = StopReason::tool_use(vec![]);
        assert!(tool.is_tool_use());

        let err = StopReason::error("timeout");
        assert_eq!(err.kind, STOP_KIND_ERROR);
        assert_eq!(err.raw_reason, "timeout");
    }

    #[test]
    fn test_ai_message_serde() {
        let msg = AIMessage {
            messageid: "msg-123".to_string(),
            parts: vec![AIMessagePart {
                part_type: "text".to_string(),
                text: "Hello world".to_string(),
                ..Default::default()
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"messageid\":\"msg-123\""));
        assert!(json.contains("\"type\":\"text\""));
    }

    #[test]
    fn test_tool_use_data_serde() {
        let data = ToolUseData {
            toolcallid: "call-1".to_string(),
            toolname: "read_file".to_string(),
            status: "completed".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("\"toolname\":\"read_file\""));
        assert!(!json.contains("\"errormessage\"")); // empty, should be omitted
    }

    #[test]
    fn test_rate_limit_info() {
        let info = RateLimitInfo {
            req: 50,
            reqlimit: 100,
            preq: 5,
            preqlimit: 10,
            resetepoch: 1700000000,
            unknown: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"req\":50"));
        assert!(!json.contains("\"unknown\"")); // false, should be omitted
    }

    #[test]
    fn test_ai_usage_default() {
        let usage = AIUsage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
    }

    #[test]
    fn test_ai_metrics_default() {
        let metrics = AIMetrics::default();
        assert_eq!(metrics.request_count, 0);
        assert!(!metrics.had_error);
        assert!(metrics.tool_detail.is_empty());
    }

    #[test]
    fn test_constants() {
        assert_eq!(API_TYPE_ANTHROPIC, "anthropic");
        assert_eq!(API_TYPE_OPENAI, "openai");
        assert_eq!(STOP_KIND_DONE, "done");
        assert_eq!(STOP_KIND_TOOL_USE, "tool_use");
        assert_eq!(DEFAULT_MAX_TOKENS, 4096);
    }

    #[test]
    fn test_select_backend_anthropic() {
        let opts = AIOptsType {
            apitype: API_TYPE_ANTHROPIC.to_string(),
            apitoken: "sk-test".to_string(),
            ..Default::default()
        };
        let _backend = select_backend(&opts);
        // Just verify it doesn't panic and returns a valid backend
    }

    #[test]
    fn test_select_backend_openai() {
        let opts = AIOptsType {
            apitype: API_TYPE_OPENAI.to_string(),
            apitoken: "sk-test".to_string(),
            ..Default::default()
        };
        let _backend = select_backend(&opts);
    }
}
