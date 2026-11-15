// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a turn starts in a Claude stream-json transcript.

/// Whether `line` starts a turn in a Claude stream-json transcript: the same
/// rule as the frontend's `ClaudeTranslator.handleUserMessage` producing a
/// `user_message` — a `user` record whose content is a non-empty string, or
/// an array with an image and some text and no `tool_result` (a text-only
/// array is how the CLI records "[Request interrupted by user]" and other
/// meta lines, never rendered as a turn).
pub fn is_claude_turn_start(line: &str) -> bool {
    use serde_json::Value;
    if !line.contains("\"user\"") {
        return false;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return false;
    }
    match v.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(blocks)) => {
            let ty = |b: &Value| b.get("type").and_then(Value::as_str).map(str::to_owned);
            let has = |t: &str| blocks.iter().any(|b| ty(b).as_deref() == Some(t));
            !has("tool_result")
                && has("image")
                && blocks.iter().any(|b| {
                    ty(b).as_deref() == Some("text")
                        && b.get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|t| !t.is_empty())
                })
        }
        _ => false,
    }
}
