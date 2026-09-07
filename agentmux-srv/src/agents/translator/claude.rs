// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Claude Code stream-json → `AgentEvent` translator.
//!
//! Mirrors the frontend reference implementation at
//! `frontend/app/view/agent/providers/claude-translator.ts` so the
//! drone Agent block can consume the same stream without
//! round-tripping through the renderer. Handled frames:
//!
//! - `stream_event.content_block_start.content_block.type=text` —
//!   starts a text block; subsequent `text_delta`s belong to it.
//! - `stream_event.content_block_start.content_block.type=tool_use` —
//!   starts an in-flight tool_use; subsequent `input_json_delta`s
//!   accumulate; `content_block_stop` emits the finalized
//!   `AgentEvent::ToolUse`.
//! - `stream_event.content_block_delta.delta.type=text_delta` —
//!   `AgentEvent::AssistantText` (also accumulated into the final
//!   response).
//! - `stream_event.content_block_delta.delta.type=input_json_delta` —
//!   appends to the pending tool_use's partial_json.
//! - `stream_event.content_block_stop` — flushes the pending
//!   tool_use.
//! - `user.message.content[].type=tool_result` — emits
//!   `AgentEvent::ToolResult`.
//! - `stream_event.message_start` with `message.role=user` — same
//!   `tool_result` extraction as above (Anthropic sometimes carries
//!   tool results here instead of a top-level `user` frame; mirrors
//!   TS's `handleMessageStart`).
//! - `result.cost_usd` + `result.usage` — emits `AgentEvent::Cost`.
//!   Then, if `result.is_error` is true, emits `AgentEvent::Error`
//!   (mirrors TS's `error_result`); otherwise emits `AgentEvent::Done`
//!   whose `response` is the explicit `result.result` field if
//!   present, otherwise the accumulated text from streamed
//!   text_deltas.
//! - `system.subtype=compact_boundary` — emits
//!   `AgentEvent::CompactionBoundary` with the exact trigger/token/
//!   duration data from `compactMetadata`. Other `system` subtypes
//!   are still discarded. See
//!   `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
//!   §4.1 — this used to be silently dropped as an unhandled
//!   `system` frame.
//!
//! Unknown frame types and malformed shapes produce an empty `Vec`
//! rather than panicking — the runner falls back to whatever the
//! parallel raw-byte WPS path published, so an unfamiliar frame
//! degrades gracefully.
//!
//! `thinking_delta`, `message_delta` / `message_stop` are discarded
//! (the agent pane filters them too). `rate_limit_event` (TS:
//! `provider_waiting`) is also currently discarded — surfacing it
//! here would need a new `AgentEvent` variant, and `types.rs`
//! explicitly reserves that decision rather than shipping it
//! casually; left as a deliberate follow-up, not an oversight.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::super::types::{AgentEvent, AgentTurn, CompactionTrigger, TokenCounts};
use super::Translator;

#[derive(Debug, Default)]
pub struct ClaudeTranslator {
    /// In-flight tool_use between `content_block_start` (type=tool_use)
    /// and `content_block_stop`. Cleared on stop.
    pending_tool: Option<PendingToolUse>,
    /// Map from `tool_use_id` to its tool name, populated when a
    /// tool_use lands so a later `tool_result` can carry the name
    /// for renderers. Phase 1.5 doesn't surface tool name on the
    /// `ToolResult` event (the id is enough for downstream matching),
    /// but the map is kept for future use.
    #[allow(dead_code)]
    tool_names: HashMap<String, String>,
    /// Streamed text accumulated since the last assistant turn, used
    /// as the `Done.response` if the `result` frame has no explicit
    /// `result` text field.
    accumulated_response: String,
    /// Per-turn transcript built up as `assistant` / `user` frames land.
    transcript: Vec<AgentTurn>,
}

#[derive(Debug)]
struct PendingToolUse {
    id: String,
    name: String,
    partial_json: String,
}

impl ClaudeTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset all in-flight state. Intended for re-use of the same
    /// translator across distinct runs (rare; usually each run gets
    /// a fresh translator).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

impl Translator for ClaudeTranslator {
    fn translate(&mut self, frame: Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let Some(frame_type) = frame.get("type").and_then(|v| v.as_str()) else {
            return out;
        };
        match frame_type {
            "stream_event" => handle_stream_event(self, &frame, &mut out),
            "user" => handle_user_message(self, &frame, &mut out),
            "assistant" => handle_assistant_message(self, &frame, &mut out),
            "result" => handle_result(self, &frame, &mut out),
            "system" => handle_system_message(self, &frame, &mut out),
            _ => {}
        }
        out
    }
}

fn handle_stream_event(t: &mut ClaudeTranslator, frame: &Value, out: &mut Vec<AgentEvent>) {
    let Some(event) = frame.get("event") else {
        return;
    };
    let Some(ev_type) = event.get("type").and_then(|v| v.as_str()) else {
        return;
    };
    match ev_type {
        "content_block_start" => {
            let Some(block) = event.get("content_block") else {
                return;
            };
            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                t.tool_names.insert(id.clone(), name.clone());
                t.pending_tool = Some(PendingToolUse {
                    id,
                    name,
                    partial_json: String::new(),
                });
            }
        }
        "content_block_delta" => {
            let Some(delta) = event.get("delta") else {
                return;
            };
            match delta.get("type").and_then(|v| v.as_str()) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                        t.accumulated_response.push_str(text);
                        out.push(AgentEvent::AssistantText {
                            delta: text.to_string(),
                        });
                    }
                }
                Some("input_json_delta") => {
                    if let (Some(pending), Some(partial)) =
                        (&mut t.pending_tool, delta.get("partial_json").and_then(|v| v.as_str()))
                    {
                        pending.partial_json.push_str(partial);
                    }
                }
                _ => {} // thinking_delta and other variants — discard
            }
        }
        "content_block_stop" => {
            if let Some(pending) = t.pending_tool.take() {
                // Anthropic's stream starts every tool_use with an
                // implicit `input: {}` and only sends input_json_delta
                // chunks when there ARE arguments. Treat an empty
                // partial_json as the canonical no-arg object instead
                // of falling through to the parse-failure path, which
                // would emit Value::String("") and break downstream
                // tool runners expecting an object. Codex P2 on PR #833.
                let input: Value = if pending.partial_json.is_empty() {
                    Value::Object(Map::new())
                } else {
                    // Non-empty: parse, fall back to the raw string
                    // on failure so a malformed stream surfaces
                    // SOMETHING rather than silently dropping.
                    serde_json::from_str(&pending.partial_json)
                        .unwrap_or(Value::String(pending.partial_json))
                };
                out.push(AgentEvent::ToolUse {
                    tool_use_id: pending.id,
                    tool: pending.name,
                    input,
                });
            }
        }
        // Anthropic sometimes carries tool_result blocks on the
        // message_start frame instead of (or in addition to) a
        // top-level `user` frame. Mirrors TS's `handleMessageStart`.
        "message_start" => {
            if let Some(content) = event
                .get("message")
                .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                extract_tool_results(t, content, out);
            }
        }
        _ => {} // message_delta, message_stop — discard
    }
}

fn handle_user_message(t: &mut ClaudeTranslator, frame: &Value, out: &mut Vec<AgentEvent>) {
    let Some(content) = frame
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };
    extract_tool_results(t, content, out);
}

/// Shared by `handle_user_message` (top-level `user` frames) and the
/// `message_start` case in `handle_stream_event` — both carry
/// `tool_result` blocks in the same `content` array shape.
fn extract_tool_results(t: &mut ClaudeTranslator, content: &[Value], out: &mut Vec<AgentEvent>) {
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        let tool_use_id = block
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // `content` on a tool_result may be a string or an array of
        // content parts — preserve whichever shape arrived.
        let output = block.get("content").cloned().unwrap_or(Value::Null);
        let is_error = block
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push(AgentEvent::ToolResult {
            tool_use_id: tool_use_id.clone(),
            output: output.clone(),
            is_error,
        });
        // Also record the tool_result in the transcript so Done.transcript
        // is the full ordered turn list (assistant turns + tool_result
        // turns), not just the assistant side. Codex P2 on PR #833.
        t.transcript.push(AgentTurn {
            role: "tool_result".to_string(),
            content: serde_json::json!({
                "tool_use_id": tool_use_id,
                "content": output,
                "is_error": is_error,
            }),
            timestamp_ms: now_ms(),
        });
    }
}

fn handle_assistant_message(t: &mut ClaudeTranslator, frame: &Value, _out: &mut Vec<AgentEvent>) {
    // Skip partial snapshots — when Claude is launched with
    // --include-partial-messages it emits top-level `assistant`
    // frames with `partial: true` for each streaming delta before
    // the final consolidated turn. Recording every snapshot would
    // produce duplicate transcript entries per turn. Frontend
    // translator does the same skip
    // (frontend/app/view/agent/providers/claude-translator.ts).
    // Reagent P1 + codex P2 on PR #833.
    if frame.get("partial").and_then(|v| v.as_bool()) == Some(true) {
        return;
    }
    // Record the turn into the transcript — used as part of `Done`.
    // Don't emit per-block events here; the streaming path already
    // emitted them via stream_event deltas.
    let Some(message) = frame.get("message") else {
        return;
    };
    let content = message.get("content").cloned().unwrap_or(Value::Null);
    t.transcript.push(AgentTurn {
        role: "assistant".to_string(),
        content,
        timestamp_ms: now_ms(),
    });
}

fn handle_result(t: &mut ClaudeTranslator, frame: &Value, out: &mut Vec<AgentEvent>) {
    let cost_usd = frame
        .get("cost_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let tokens = parse_usage(frame.get("usage"));
    out.push(AgentEvent::Cost { cost_usd, tokens });

    // A terminal error is mutually exclusive with Done — mirrors TS's
    // `error_result` (claude-translator.ts) and gives `AgentEvent::Error`
    // (types.rs) its first real producer. The runner's own raw-frame
    // `is_error_result_frame` check (failure.rs) independently fails the
    // run either way; this only affects what the live event stream shows.
    if frame.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
        let api_error_status = frame.get("api_error_status").and_then(|v| v.as_i64());
        let message = frame
            .get("result")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| match api_error_status {
                Some(code) if code > 0 => format!("API error {code}"),
                _ => "Agent encountered an error".to_string(),
            });
        t.accumulated_response.clear();
        t.transcript.clear();
        out.push(AgentEvent::Error { message });
        return;
    }

    let response = frame
        .get("result")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::mem::take(&mut t.accumulated_response));
    let transcript = std::mem::take(&mut t.transcript);
    out.push(AgentEvent::Done {
        response,
        transcript,
    });
}

/// `type: "system"` frames cover several subtypes; today we only act
/// on `compact_boundary` (see
/// `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
/// §4.1). Any other subtype — or a `compact_boundary` frame missing
/// `compactMetadata` / carrying malformed fields — produces no event
/// rather than a bad one, matching this file's existing philosophy
/// (see the module doc comment).
fn handle_system_message(_t: &mut ClaudeTranslator, frame: &Value, out: &mut Vec<AgentEvent>) {
    if frame.get("subtype").and_then(|v| v.as_str()) != Some("compact_boundary") {
        return;
    }
    let Some(meta) = frame.get("compactMetadata") else {
        return;
    };
    let Some(trigger) = meta.get("trigger").and_then(|v| v.as_str()).and_then(|s| match s {
        "auto" => Some(CompactionTrigger::Auto),
        "manual" => Some(CompactionTrigger::Manual),
        _ => None, // unrecognized trigger string — don't guess, skip the event
    }) else {
        return;
    };
    let Some(pre_tokens) = meta.get("preTokens").and_then(|v| v.as_u64()) else {
        return;
    };
    let Some(post_tokens) = meta.get("postTokens").and_then(|v| v.as_u64()) else {
        return;
    };
    let Some(cumulative_dropped_tokens) =
        meta.get("cumulativeDroppedTokens").and_then(|v| v.as_u64())
    else {
        return;
    };
    let Some(duration_ms) = meta.get("durationMs").and_then(|v| v.as_u64()) else {
        return;
    };
    out.push(AgentEvent::CompactionBoundary {
        trigger,
        pre_tokens,
        post_tokens,
        cumulative_dropped_tokens,
        duration_ms,
    });
}

pub(crate) fn parse_usage(usage: Option<&Value>) -> TokenCounts {
    let Some(usage) = usage.and_then(|v| v.as_object()) else {
        return TokenCounts::default();
    };
    let take = |m: &Map<String, Value>, k: &str| -> u64 {
        m.get(k).and_then(|v| v.as_u64()).unwrap_or(0)
    };
    TokenCounts {
        input: take(usage, "input_tokens"),
        output: take(usage, "output_tokens"),
        cache_creation: take(usage, "cache_creation_input_tokens"),
        cache_read: take(usage, "cache_read_input_tokens"),
    }
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text_delta(text: &str) -> Value {
        json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": { "type": "text_delta", "text": text }
            }
        })
    }

    fn tool_use_start(id: &str, name: &str) -> Value {
        json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "content_block": { "type": "tool_use", "id": id, "name": name }
            }
        })
    }

    fn input_json_delta(partial: &str) -> Value {
        json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "delta": { "type": "input_json_delta", "partial_json": partial }
            }
        })
    }

    fn content_block_stop() -> Value {
        json!({
            "type": "stream_event",
            "event": { "type": "content_block_stop" }
        })
    }

    #[test]
    fn text_delta_emits_assistant_text() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(text_delta("hello"));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::AssistantText { delta } => assert_eq!(delta, "hello"),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn streaming_text_accumulates_for_done_response() {
        // No explicit `result.result` — `Done.response` should be the
        // concatenation of streamed text_deltas.
        let mut t = ClaudeTranslator::new();
        t.translate(text_delta("Hello "));
        t.translate(text_delta("world"));
        let events = t.translate(json!({ "type": "result", "cost_usd": 0.01 }));
        // Expect Cost + Done
        assert_eq!(events.len(), 2);
        match &events[1] {
            AgentEvent::Done { response, .. } => assert_eq!(response, "Hello world"),
            other => panic!("expected Done with accumulated text, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_emits_only_on_content_block_stop() {
        let mut t = ClaudeTranslator::new();
        // Start doesn't emit yet.
        assert!(t.translate(tool_use_start("t1", "Bash")).is_empty());
        // Delta accumulates, doesn't emit.
        assert!(t.translate(input_json_delta(r#"{"command":"#)).is_empty());
        assert!(t.translate(input_json_delta(r#""ls"}"#)).is_empty());
        // Stop flushes.
        let events = t.translate(content_block_stop());
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolUse {
                tool_use_id,
                tool,
                input,
            } => {
                assert_eq!(tool_use_id, "t1");
                assert_eq!(tool, "Bash");
                assert_eq!(input, &json!({ "command": "ls" }));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn no_arg_tool_emits_empty_object() {
        // No input_json_delta between start and stop — codex P2 on
        // PR #833. The fallback path used to emit Value::String(""),
        // which broke downstream tool runners. Now emits {}.
        let mut t = ClaudeTranslator::new();
        t.translate(tool_use_start("t1", "Echo"));
        let events = t.translate(content_block_stop());
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolUse { input, .. } => {
                assert_eq!(input, &json!({}));
            }
            other => panic!("expected ToolUse with empty object input, got {other:?}"),
        }
    }

    #[test]
    fn malformed_tool_input_falls_back_to_raw_string() {
        let mut t = ClaudeTranslator::new();
        t.translate(tool_use_start("t1", "Edit"));
        t.translate(input_json_delta(r#"{"this is bad json"#));
        let events = t.translate(content_block_stop());
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolUse { input, .. } => {
                // Falls back to the partial string so the
                // downstream consumer sees SOMETHING.
                assert_eq!(input, &json!(r#"{"this is bad json"#));
            }
            other => panic!("expected ToolUse fallback, got {other:?}"),
        }
    }

    #[test]
    fn user_tool_result_emits_event() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "t3",
                    "content": "command output here",
                    "is_error": false
                }]
            }
        }));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolResult {
                tool_use_id,
                output,
                is_error,
            } => {
                assert_eq!(tool_use_id, "t3");
                assert_eq!(output, &json!("command output here"));
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_is_error_propagates() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "t4",
                    "content": "permission denied",
                    "is_error": true
                }]
            }
        }));
        match &events[0] {
            AgentEvent::ToolResult { is_error, .. } => assert!(is_error),
            other => panic!("expected ToolResult with is_error, got {other:?}"),
        }
    }

    #[test]
    fn result_emits_cost_then_done_with_explicit_response() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "result",
            "cost_usd": 0.0123,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 200
            },
            "result": "final answer text"
        }));
        assert_eq!(events.len(), 2);
        match &events[0] {
            AgentEvent::Cost { cost_usd, tokens } => {
                assert_eq!(*cost_usd, 0.0123);
                assert_eq!(tokens.input, 100);
                assert_eq!(tokens.output, 50);
                assert_eq!(tokens.cache_creation, 0);
                assert_eq!(tokens.cache_read, 200);
            }
            other => panic!("expected Cost first, got {other:?}"),
        }
        match &events[1] {
            AgentEvent::Done { response, .. } => {
                assert_eq!(response, "final answer text");
            }
            other => panic!("expected Done second, got {other:?}"),
        }
    }

    #[test]
    fn assistant_message_added_to_transcript() {
        let mut t = ClaudeTranslator::new();
        // Assistant frame produces NO events directly (the streaming
        // path emits the per-block events); it contributes to the
        // transcript captured at Done.
        let events = t.translate(json!({
            "type": "assistant",
            "message": {
                "content": [{ "type": "text", "text": "hello" }]
            }
        }));
        assert!(events.is_empty());
        // The transcript shows up on Done.
        let done = t.translate(json!({ "type": "result", "cost_usd": 0.0 }));
        match &done[1] {
            AgentEvent::Done { transcript, .. } => {
                assert_eq!(transcript.len(), 1);
                assert_eq!(transcript[0].role, "assistant");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn unknown_frame_returns_empty() {
        let mut t = ClaudeTranslator::new();
        assert!(t.translate(json!({ "type": "system" })).is_empty());
        assert!(t.translate(json!({ "type": "stream_event", "event": { "type": "message_stop" } })).is_empty());
        assert!(t.translate(json!({ "type": "stream_event", "event": { "type": "message_delta" } })).is_empty());
    }

    // ── system / compact_boundary (SPEC_COMPACTION_DETECTION_AND_HANDLING) ──

    fn compact_boundary_frame(trigger: &str) -> Value {
        // Real example frame captured from a live session (see the spec
        // doc §2) — includes the extra fields (`preCompactDiscoveredTools`,
        // `preservedSegment`) that the translator ignores.
        json!({
            "type": "system",
            "subtype": "compact_boundary",
            "content": "Conversation compacted",
            "level": "info",
            "compactMetadata": {
                "trigger": trigger,
                "preTokens": 783_887,
                "postTokens": 11_775,
                "cumulativeDroppedTokens": 772_112,
                "durationMs": 231_606,
                "preCompactDiscoveredTools": ["Bash", "Edit"],
                "preservedSegment": {
                    "headUuid": "h1",
                    "anchorUuid": "a1",
                    "tailUuid": "t1"
                }
            },
            "timestamp": "2026-07-21T17:55:35.500Z"
        })
    }

    #[test]
    fn compact_boundary_manual_emits_compaction_boundary() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(compact_boundary_frame("manual"));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::CompactionBoundary {
                trigger,
                pre_tokens,
                post_tokens,
                cumulative_dropped_tokens,
                duration_ms,
            } => {
                assert_eq!(*trigger, CompactionTrigger::Manual);
                assert_eq!(*pre_tokens, 783_887);
                assert_eq!(*post_tokens, 11_775);
                assert_eq!(*cumulative_dropped_tokens, 772_112);
                assert_eq!(*duration_ms, 231_606);
            }
            other => panic!("expected CompactionBoundary, got {other:?}"),
        }
    }

    #[test]
    fn compact_boundary_auto_trigger_maps_correctly() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(compact_boundary_frame("auto"));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::CompactionBoundary { trigger, .. } => {
                assert_eq!(*trigger, CompactionTrigger::Auto);
            }
            other => panic!("expected CompactionBoundary, got {other:?}"),
        }
    }

    #[test]
    fn system_frame_with_unrelated_subtype_returns_empty() {
        // Only `compact_boundary` is handled specifically — every other
        // `system` subtype still falls through to no-op.
        let mut t = ClaudeTranslator::new();
        assert!(t
            .translate(json!({ "type": "system", "subtype": "other_thing" }))
            .is_empty());
    }

    #[test]
    fn compact_boundary_missing_metadata_returns_empty() {
        let mut t = ClaudeTranslator::new();
        assert!(t
            .translate(json!({ "type": "system", "subtype": "compact_boundary" }))
            .is_empty());
    }

    #[test]
    fn compact_boundary_malformed_field_returns_empty_not_panic() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "system",
            "subtype": "compact_boundary",
            "compactMetadata": {
                "trigger": "manual",
                "preTokens": "not-a-number",
                "postTokens": 11_775,
                "cumulativeDroppedTokens": 772_112,
                "durationMs": 231_606
            }
        }));
        assert!(events.is_empty());
    }

    #[test]
    fn compact_boundary_unrecognized_trigger_returns_empty() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(compact_boundary_frame("something_new"));
        assert!(events.is_empty());
    }

    #[test]
    fn malformed_or_missing_type_returns_empty() {
        let mut t = ClaudeTranslator::new();
        assert!(t.translate(json!({})).is_empty());
        assert!(t.translate(json!(null)).is_empty());
        assert!(t.translate(json!("not an object")).is_empty());
        assert!(t.translate(json!(42)).is_empty());
    }

    #[test]
    fn skips_partial_assistant_snapshots() {
        // When Claude is launched with --include-partial-messages it
        // streams partial assistant frames with `partial: true` before
        // the final consolidated turn. Recording each would duplicate
        // transcript entries. Reagent P1 on PR #833.
        let mut t = ClaudeTranslator::new();
        // Three partial snapshots — all should be ignored.
        t.translate(json!({
            "type": "assistant",
            "partial": true,
            "message": { "content": [{ "type": "text", "text": "h" }] }
        }));
        t.translate(json!({
            "type": "assistant",
            "partial": true,
            "message": { "content": [{ "type": "text", "text": "he" }] }
        }));
        t.translate(json!({
            "type": "assistant",
            "partial": true,
            "message": { "content": [{ "type": "text", "text": "hello" }] }
        }));
        // Now the consolidated turn (partial: false / absent).
        t.translate(json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": "hello" }] }
        }));
        let done = t.translate(json!({ "type": "result", "cost_usd": 0.0 }));
        match &done[1] {
            AgentEvent::Done { transcript, .. } => {
                assert_eq!(
                    transcript.len(),
                    1,
                    "only the consolidated turn should land; got {transcript:?}"
                );
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_recorded_in_transcript() {
        // Codex P2 on PR #833: Done.transcript must be the full
        // ordered turn list (assistant + tool_result), not just the
        // assistant side. Audit/replay needs the whole conversation.
        let mut t = ClaudeTranslator::new();
        // Assistant turn 1.
        t.translate(json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "t1",
                    "name": "Bash",
                    "input": { "command": "ls" }
                }]
            }
        }));
        // Tool result.
        t.translate(json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "t1",
                    "content": "output",
                    "is_error": false
                }]
            }
        }));
        // Final assistant turn.
        t.translate(json!({
            "type": "assistant",
            "message": {
                "content": [{ "type": "text", "text": "done" }]
            }
        }));
        let done = t.translate(json!({ "type": "result", "cost_usd": 0.0 }));
        match &done[1] {
            AgentEvent::Done { transcript, .. } => {
                assert_eq!(transcript.len(), 3, "got {transcript:?}");
                assert_eq!(transcript[0].role, "assistant");
                assert_eq!(transcript[1].role, "tool_result");
                assert_eq!(transcript[2].role, "assistant");
                // Verify the tool_result turn carries the structured
                // content payload — not just the bare output.
                let tr = &transcript[1].content;
                assert_eq!(tr["tool_use_id"], json!("t1"));
                assert_eq!(tr["content"], json!("output"));
                assert_eq!(tr["is_error"], json!(false));
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn reset_clears_in_flight_state() {
        let mut t = ClaudeTranslator::new();
        t.translate(tool_use_start("t5", "Edit"));
        t.translate(input_json_delta(r#"{"x":1}"#));
        t.translate(text_delta("partial response"));

        t.reset();

        // After reset, a content_block_stop should not emit because
        // the pending_tool was cleared.
        assert!(t.translate(content_block_stop()).is_empty());
        // And the accumulated response is gone.
        let done = t.translate(json!({ "type": "result", "cost_usd": 0.0 }));
        match &done[1] {
            AgentEvent::Done { response, .. } => assert_eq!(response, ""),
            other => panic!("expected empty Done response, got {other:?}"),
        }
    }

    // ── Bug fix regression tests (audit REPORT_REPO_HEALTH_AUDIT_2026_07_20 §1.1) ──

    #[test]
    fn result_is_error_emits_error_not_done() {
        let mut t = ClaudeTranslator::new();
        t.translate(text_delta("partial output before the failure"));
        let events = t.translate(json!({
            "type": "result",
            "cost_usd": 0.002,
            "is_error": true,
            "api_error_status": 429,
        }));
        assert_eq!(events.len(), 2, "expected Cost + Error, got {events:?}");
        assert!(matches!(events[0], AgentEvent::Cost { .. }));
        match &events[1] {
            AgentEvent::Error { message } => assert_eq!(message, "API error 429"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn result_is_error_prefers_explicit_result_text_over_status_code() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "result",
            "cost_usd": 0.0,
            "is_error": true,
            "result": "network error: connection reset",
        }));
        match &events[1] {
            AgentEvent::Error { message } => {
                assert_eq!(message, "network error: connection reset")
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn result_is_error_with_no_status_or_text_gets_generic_message() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "result",
            "cost_usd": 0.0,
            "is_error": true,
        }));
        match &events[1] {
            AgentEvent::Error { message } => {
                assert_eq!(message, "Agent encountered an error")
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn result_without_is_error_still_emits_done_as_before() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({ "type": "result", "cost_usd": 0.0, "result": "ok" }));
        match &events[1] {
            AgentEvent::Done { response, .. } => assert_eq!(response, "ok"),
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn message_start_with_user_role_extracts_tool_result() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "stream_event",
            "event": {
                "type": "message_start",
                "message": {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "t9",
                        "content": "output",
                        "is_error": false
                    }]
                }
            }
        }));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolResult {
                tool_use_id,
                output,
                is_error,
            } => {
                assert_eq!(tool_use_id, "t9");
                assert_eq!(output, &json!("output"));
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
        // Also lands in the transcript, same as the top-level `user` path.
        let done = t.translate(json!({ "type": "result", "cost_usd": 0.0 }));
        match &done[1] {
            AgentEvent::Done { transcript, .. } => {
                assert_eq!(transcript.len(), 1);
                assert_eq!(transcript[0].role, "tool_result");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn message_start_with_assistant_role_is_ignored() {
        let mut t = ClaudeTranslator::new();
        let events = t.translate(json!({
            "type": "stream_event",
            "event": {
                "type": "message_start",
                "message": { "role": "assistant", "content": [] }
            }
        }));
        assert!(events.is_empty());
    }
}
