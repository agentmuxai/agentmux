// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Codex `exec --json` → `AgentEvent` translator — the answer text only.
//!
//! Scoped to what a tool-less, one-shot turn produces (`/btw`,
//! `server/agent_handlers/side_question.rs`): assistant text, the end of
//! the turn, and failures. Tool items (`command_execution`,
//! `mcp_tool_call`, …) are dropped rather than translated; the full
//! renderer is the frontend's `providers/codex-translator.ts`, which this
//! follows for the frames it does handle:
//!
//! - `item.started` / `item.updated` / `item.completed` with
//!   `item.type = "agent_message"` (text in `item.text`) or
//!   `item.type = "message"`, `role = "assistant"` (text in
//!   `output_text` content blocks). Each carries the item's text SO FAR,
//!   not a delta, so only the new suffix is emitted as `AssistantText`.
//! - `turn.completed` — `Done`, whose `response` is every message's text.
//! - `turn.failed` / `error` — `Error`. An `error` whose message contains
//!   `Reconnecting...` is a transient retry notice, not a failure.

use std::collections::HashMap;

use serde_json::Value;

use super::super::types::AgentEvent;

#[derive(Default)]
pub struct CodexTranslator {
    /// item id → text already emitted for it.
    emitted: HashMap<String, String>,
    /// Item ids in first-seen order, for assembling `Done.response`.
    order: Vec<String>,
    terminal: bool,
}

impl CodexTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    fn item_text(item: &Value) -> Option<String> {
        match item.get("type").and_then(Value::as_str)? {
            "agent_message" => Some(item.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
            "message" if item.get("role").and_then(Value::as_str) == Some("assistant") => Some(
                item.get("content")
                    .and_then(Value::as_array)
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter(|b| b.get("type").and_then(Value::as_str) == Some("output_text"))
                            .filter_map(|b| b.get("text").and_then(Value::as_str))
                            .collect::<String>()
                    })
                    .unwrap_or_default(),
            ),
            _ => None,
        }
    }

    fn translate_item(&mut self, item: &Value) -> Vec<AgentEvent> {
        let Some(text) = Self::item_text(item) else {
            return Vec::new();
        };
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("message:anonymous")
            .to_string();
        if !self.emitted.contains_key(&id) {
            self.order.push(id.clone());
        }
        let prev = self.emitted.entry(id).or_default();
        match text.strip_prefix(prev.as_str()) {
            Some("") => Vec::new(),
            Some(suffix) => {
                let delta = suffix.to_string();
                *prev = text;
                vec![AgentEvent::AssistantText { delta }]
            }
            // A snapshot that doesn't extend what was already shown (a
            // rewrite) can't be a delta; `Done.response` carries the final
            // text, which the overlay swaps in.
            None => {
                *prev = text;
                Vec::new()
            }
        }
    }

    fn error_message(frame: &Value, fallback: &str) -> String {
        let err = frame.get("error").unwrap_or(frame);
        err.get("message")
            .and_then(Value::as_str)
            .or_else(|| err.as_str())
            .unwrap_or(fallback)
            .to_string()
    }

    fn finish(&mut self, event: AgentEvent) -> Vec<AgentEvent> {
        if self.terminal {
            return Vec::new();
        }
        self.terminal = true;
        vec![event]
    }
}

impl super::Translator for CodexTranslator {
    fn translate(&mut self, frame: Value) -> Vec<AgentEvent> {
        match frame.get("type").and_then(Value::as_str).unwrap_or("") {
            "item.started" | "item.updated" | "item.completed" => match frame.get("item") {
                Some(item) => self.translate_item(item),
                None => Vec::new(),
            },
            "turn.completed" => {
                let response = self
                    .order
                    .iter()
                    .filter_map(|id| self.emitted.get(id))
                    .filter(|t| !t.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n\n");
                self.finish(AgentEvent::Done {
                    response,
                    transcript: Vec::new(),
                })
            }
            "turn.failed" => {
                let message = Self::error_message(&frame, "Codex turn failed");
                self.finish(AgentEvent::Error { message })
            }
            "error" => {
                let message = Self::error_message(&frame, "Codex error");
                if message.contains("Reconnecting...") {
                    return Vec::new();
                }
                self.finish(AgentEvent::Error { message })
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

    fn texts(events: &[AgentEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::AssistantText { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn snapshots_become_deltas_and_the_turn_end_is_done() {
        let mut t = CodexTranslator::new();
        let mut out = Vec::new();
        for frame in [
            json!({"type": "thread.started", "thread_id": "th-1"}),
            json!({"type": "turn.started"}),
            json!({"type": "item.started", "item": {"id": "m1", "type": "agent_message", "text": "Hel"}}),
            json!({"type": "item.updated", "item": {"id": "m1", "type": "agent_message", "text": "Hello"}}),
            json!({"type": "item.completed", "item": {"id": "m1", "type": "agent_message", "text": "Hello."}}),
            json!({"type": "turn.completed", "usage": {"input_tokens": 3, "output_tokens": 2}}),
        ] {
            out.extend(t.translate(frame));
        }
        assert_eq!(texts(&out), ["Hel", "lo", "."]);
        match out.last() {
            Some(AgentEvent::Done { response, .. }) => assert_eq!(response, "Hello."),
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn assistant_message_items_carry_output_text() {
        let mut t = CodexTranslator::new();
        let out = t.translate(json!({"type": "item.completed", "item": {
            "id": "m1", "type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": "Hi"}, {"type": "other", "text": "x"}]
        }}));
        assert_eq!(texts(&out), ["Hi"]);
    }

    #[test]
    fn tool_items_and_user_messages_are_dropped() {
        let mut t = CodexTranslator::new();
        assert!(t
            .translate(json!({"type": "item.started", "item": {"id": "c1", "type": "command_execution", "command": "ls"}}))
            .is_empty());
        assert!(t
            .translate(json!({"type": "item.completed", "item": {"id": "u1", "type": "message", "role": "user", "content": []}}))
            .is_empty());
    }

    #[test]
    fn a_failed_turn_is_one_error_and_reconnects_are_not_failures() {
        let mut t = CodexTranslator::new();
        assert!(t.translate(json!({"type": "error", "message": "Reconnecting... 1/5"})).is_empty());
        let out = t.translate(json!({"type": "turn.failed", "error": {"message": "quota exceeded"}}));
        match out.as_slice() {
            [AgentEvent::Error { message }] => assert_eq!(message, "quota exceeded"),
            other => panic!("expected one Error, got {other:?}"),
        }
        assert!(t.translate(json!({"type": "error", "message": "late"})).is_empty(), "only one terminal event");
    }
}
