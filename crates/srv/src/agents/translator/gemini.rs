// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Gemini CLI `--output-format stream-json` → `AgentEvent` translator —
//! the answer text only.
//!
//! Scoped to what a tool-less, one-shot turn produces (`/btw`,
//! `server/agent_handlers/side_question.rs`). The full renderer is the
//! frontend's `providers/gemini-translator.ts`, which this follows for the
//! frames it handles:
//!
//! - `message` with `role = "assistant"` — `content` is an incremental
//!   chunk (not accumulated text), emitted as `AssistantText`.
//! - `result` — `Done` when `status` is `"success"`, else `Error` with the
//!   frame's error message.
//!
//! `init`, user-prompt echoes, `tool_use` / `tool_result` and anything
//! unrecognised are dropped.

use serde_json::Value;

use super::super::types::AgentEvent;

#[derive(Default)]
pub struct GeminiTranslator {
    response: String,
    terminal: bool,
}

impl GeminiTranslator {
    pub fn new() -> Self {
        Self::default()
    }
}

impl super::Translator for GeminiTranslator {
    fn translate(&mut self, frame: Value) -> Vec<AgentEvent> {
        if self.terminal {
            return Vec::new();
        }
        match frame.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" if frame.get("role").and_then(Value::as_str) == Some("assistant") => {
                let chunk = frame.get("content").and_then(Value::as_str).unwrap_or("");
                if chunk.is_empty() {
                    return Vec::new();
                }
                self.response.push_str(chunk);
                vec![AgentEvent::AssistantText {
                    delta: chunk.to_string(),
                }]
            }
            "result" => {
                self.terminal = true;
                let status = frame.get("status").and_then(Value::as_str).unwrap_or("success");
                if status == "success" {
                    return vec![AgentEvent::Done {
                        response: std::mem::take(&mut self.response),
                        transcript: Vec::new(),
                    }];
                }
                let err = frame.get("error");
                let message = err
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .or_else(|| err.and_then(Value::as_str))
                    .or_else(|| frame.get("message").and_then(Value::as_str))
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("gemini turn {status}"));
                vec![AgentEvent::Error { message }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Translator;
    use super::*;
    use serde_json::json;

    #[test]
    fn assistant_chunks_stream_and_a_successful_result_is_done() {
        let mut t = GeminiTranslator::new();
        let mut out = Vec::new();
        for frame in [
            json!({"type": "init", "session_id": "s1", "model": "gemini"}),
            json!({"type": "message", "role": "user", "content": "why?"}),
            json!({"type": "message", "role": "assistant", "content": "Because ", "delta": true}),
            json!({"type": "message", "role": "assistant", "content": "reasons.", "delta": true}),
            json!({"type": "result", "status": "success", "stats": {}}),
        ] {
            out.extend(t.translate(frame));
        }
        let deltas: Vec<_> = out
            .iter()
            .filter_map(|e| match e {
                AgentEvent::AssistantText { delta } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, ["Because ", "reasons."]);
        match out.last() {
            Some(AgentEvent::Done { response, .. }) => assert_eq!(response, "Because reasons."),
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_result_is_an_error_with_its_message() {
        let mut t = GeminiTranslator::new();
        let out = t.translate(json!({"type": "result", "status": "error", "error": {"type": "unknown", "message": "[API Error: quota]"}}));
        match out.as_slice() {
            [AgentEvent::Error { message }] => assert_eq!(message, "[API Error: quota]"),
            other => panic!("expected one Error, got {other:?}"),
        }
        assert!(t.translate(json!({"type": "message", "role": "assistant", "content": "late"})).is_empty());
    }

    #[test]
    fn tool_frames_are_dropped() {
        let mut t = GeminiTranslator::new();
        assert!(t.translate(json!({"type": "tool_use", "tool_name": "read_file", "tool_id": "t1"})).is_empty());
        assert!(t.translate(json!({"type": "tool_result", "tool_id": "t1", "status": "success"})).is_empty());
    }
}
