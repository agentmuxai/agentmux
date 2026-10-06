// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a running Claude CLI will auto-compact, from its own answer.
//!
//! The context meter's countdown used to assume the CLI's default
//! (window − 33K). The real point moves with `CLAUDE_CODE_AUTO_COMPACT_WINDOW`,
//! the `autoCompactWindow` setting and `CLAUDE_CODE_DISABLE_1M_CONTEXT`, and
//! auto-compaction can be off altogether. The CLI says which: the
//! `get_context_usage` control request (`detail: "summary"`, answered locally
//! with no inference call) returns `autoCompactThreshold` (absent when off),
//! `isAutoCompactEnabled`, `maxTokens` (the auto-compact window) and the
//! model. Verified on CLI 2.1.288:
//! docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md §9.1.
//!
//! Asked at the same points as `get_settings` (`agent_runtime.rs`): at spawn
//! and at every turn boundary, of a control-protocol process only. Published
//! as the persisted per-pane `agentcontextusage` event.

use serde_json::{json, Value};

/// Prefix of the `request_id` of our `get_context_usage` requests, so the
/// answer can be told from the answers to anything else.
pub const CONTEXT_USAGE_REQUEST_PREFIX: &str = "agentmux-context-usage-";

/// What the CLI reported about auto-compaction for its current model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContextUsage {
    /// The model the answer is for, as the CLI names it (a raw id, e.g.
    /// `claude-haiku-4-5-20251001`).
    pub model: Option<String>,
    /// The window auto-compaction is measured against; smaller than the
    /// model's context window when an override applies.
    pub auto_compact_window: Option<u64>,
    /// The prompt size at which the CLI auto-compacts; `None` when it is off.
    pub auto_compact_threshold: Option<u64>,
    pub auto_compact_enabled: bool,
}

/// The `get_context_usage` request. `summary` is answered from local state;
/// `full` would call the token-count API.
pub fn context_usage_request_line() -> String {
    json!({
        "type": "control_request",
        "request_id": format!("{CONTEXT_USAGE_REQUEST_PREFIX}{}", uuid::Uuid::new_v4()),
        "request": { "subtype": "get_context_usage", "detail": "summary" },
    })
    .to_string()
}

/// A positive count no larger than any real window (100M), else `None`.
fn count(v: Option<&Value>) -> Option<u64> {
    v.and_then(Value::as_u64)
        .filter(|n| *n > 0 && *n <= 100_000_000)
}

/// The usage in a `get_context_usage` answer, or `None` for any other frame,
/// an error answer (a CLI without the request) or one missing
/// `isAutoCompactEnabled`. Shape: `{type: control_response, response:
/// {subtype: success, request_id, response: {model, maxTokens,
/// autoCompactThreshold?, isAutoCompactEnabled, ...}}}`.
pub fn context_usage_from_control_response(frame: &Value) -> Option<AgentContextUsage> {
    if frame.get("type").and_then(Value::as_str) != Some("control_response") {
        return None;
    }
    let resp = frame.get("response")?;
    if resp.get("subtype").and_then(Value::as_str) != Some("success") {
        return None;
    }
    if !resp
        .get("request_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with(CONTEXT_USAGE_REQUEST_PREFIX))
    {
        return None;
    }
    let body = resp.get("response")?.as_object()?;
    let enabled = body.get("isAutoCompactEnabled")?.as_bool()?;
    Some(AgentContextUsage {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
        auto_compact_window: count(body.get("maxTokens")),
        // A threshold reported while disabled means nothing.
        auto_compact_threshold: if enabled {
            count(body.get("autoCompactThreshold"))
        } else {
            None
        },
        auto_compact_enabled: enabled,
    })
}

/// The `agentcontextusage` event data for one pane.
pub fn agent_context_usage_event(block_id: &str, usage: &AgentContextUsage) -> Value {
    json!({
        "blockid": block_id,
        "model": usage.model,
        "auto_compact_window": usage.auto_compact_window,
        "auto_compact_threshold": usage.auto_compact_threshold,
        "auto_compact_enabled": usage.auto_compact_enabled,
    })
}

/// Publish it, persisted, so a pane that mounts later still gets the latest.
pub fn publish_agent_context_usage(
    broker: &crate::backend::mps::Broker,
    block_id: &str,
    usage: &AgentContextUsage,
) {
    use crate::backend::mps::{MuxEvent, EVENT_AGENT_CONTEXT_USAGE};
    broker.publish(MuxEvent {
        event: EVENT_AGENT_CONTEXT_USAGE.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 1,
        data: Some(agent_context_usage_event(block_id, usage)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(request_id: &str, body: Value) -> Value {
        json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id, "response": body },
        })
    }

    fn ours() -> String {
        format!("{CONTEXT_USAGE_REQUEST_PREFIX}abc")
    }

    #[test]
    fn the_request_is_a_local_summary_with_our_prefix() {
        let line: Value = serde_json::from_str(&context_usage_request_line()).unwrap();
        assert_eq!(line["type"], "control_request");
        assert_eq!(line["request"]["subtype"], "get_context_usage");
        assert_eq!(line["request"]["detail"], "summary");
        assert!(line["request_id"]
            .as_str()
            .unwrap()
            .starts_with(CONTEXT_USAGE_REQUEST_PREFIX));
    }

    #[test]
    fn reads_the_default_answer() {
        // CLI 2.1.288, Sonnet 5.5, no overrides.
        let frame = answer(
            &ours(),
            json!({
                "model": "claude-sonnet-5-5", "maxTokens": 1_000_000, "rawMaxTokens": 1_000_000,
                "autoCompactThreshold": 967_000, "isAutoCompactEnabled": true,
                "autocompactSource": "model-default", "totalTokens": 37_475, "categories": [],
            }),
        );
        assert_eq!(
            context_usage_from_control_response(&frame),
            Some(AgentContextUsage {
                model: Some("claude-sonnet-5-5".into()),
                auto_compact_window: Some(1_000_000),
                auto_compact_threshold: Some(967_000),
                auto_compact_enabled: true,
            })
        );
    }

    #[test]
    fn reads_an_overridden_window() {
        // CLAUDE_CODE_AUTO_COMPACT_WINDOW=300000.
        let frame = answer(
            &ours(),
            json!({ "model": "claude-sonnet-5-5", "maxTokens": 300_000, "autoCompactThreshold": 267_000,
                    "isAutoCompactEnabled": true, "autocompactSource": "env" }),
        );
        let u = context_usage_from_control_response(&frame).unwrap();
        assert_eq!(
            (u.auto_compact_window, u.auto_compact_threshold),
            (Some(300_000), Some(267_000))
        );
    }

    #[test]
    fn disabled_has_no_threshold() {
        let frame = answer(
            &ours(),
            json!({ "model": "claude-sonnet-5-5", "maxTokens": 1_000_000, "autoCompactThreshold": 967_000,
                    "isAutoCompactEnabled": false }),
        );
        let u = context_usage_from_control_response(&frame).unwrap();
        assert!(!u.auto_compact_enabled);
        assert_eq!(u.auto_compact_threshold, None);
    }

    #[test]
    fn ignores_other_answers_errors_and_junk() {
        let body = json!({ "model": "m", "maxTokens": 200_000, "autoCompactThreshold": 167_000, "isAutoCompactEnabled": true });
        // Someone else's request (get_settings).
        assert_eq!(
            context_usage_from_control_response(&answer("agentmux-settings-x", body.clone())),
            None
        );
        // An error answer from a CLI without the request.
        let err = json!({ "type": "control_response",
                          "response": { "subtype": "error", "request_id": ours(), "error": "Unsupported" } });
        assert_eq!(context_usage_from_control_response(&err), None);
        // Not a control response.
        assert_eq!(
            context_usage_from_control_response(&json!({ "type": "result" })),
            None
        );
        // No enabled flag: not an answer we understand.
        assert_eq!(
            context_usage_from_control_response(&answer(&ours(), json!({ "maxTokens": 1 }))),
            None
        );
        // Implausible counts are dropped, not trusted.
        let u = context_usage_from_control_response(&answer(
            &ours(),
            json!({ "maxTokens": 0, "autoCompactThreshold": -5, "isAutoCompactEnabled": true }),
        ))
        .unwrap();
        assert_eq!(
            (u.auto_compact_window, u.auto_compact_threshold),
            (None, None)
        );
    }

    #[test]
    fn the_event_carries_the_fields_snake_cased() {
        let u = AgentContextUsage {
            model: Some("claude-haiku-4-5-20251001".into()),
            auto_compact_window: Some(200_000),
            auto_compact_threshold: Some(167_000),
            auto_compact_enabled: true,
        };
        assert_eq!(
            agent_context_usage_event("blk", &u),
            json!({ "blockid": "blk", "model": "claude-haiku-4-5-20251001", "auto_compact_window": 200_000,
                    "auto_compact_threshold": 167_000, "auto_compact_enabled": true })
        );
    }
}
