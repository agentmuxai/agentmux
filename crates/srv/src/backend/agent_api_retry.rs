// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Claude CLI request that is being retried, from the CLI's own frames.
//!
//! When a request to the API fails in a way the CLI treats as transient (no
//! connection, a 5xx), it waits and tries again, up to its own limit, and
//! writes one `system`/`api_retry` frame per attempt:
//! `{attempt, max_retries, retry_delay_ms, error_status, error}`. With the
//! network down that is minutes of a busy-looking pane with nothing else on
//! stdout, so the pane shows the retry, with Stop.
//!
//! Published as the persisted per-pane `agent-api-retry` event. Cleared by
//! the next frame that is neither `system` nor `user` (a reply arrived, or
//! the turn's `result`), and when the process's stdout closes.

use serde_json::Value;

/// One retry the CLI is waiting on.
#[derive(Debug, Clone, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct ApiRetryState {
    /// The CLI's retry count, from 1, out of `max_retries`.
    pub attempt: u32,
    pub max_retries: u32,
    /// When the CLI makes that attempt (epoch ms).
    #[ts(type = "number")]
    pub next_retry_at_ms: i64,
    /// The HTTP status of the failed request; `null` when none came back
    /// (the connection itself failed).
    pub error_status: Option<u16>,
    /// The CLI's error kind (`"unknown"` for a connection failure).
    pub error: Option<String>,
}

/// The `agent-api-retry` event's payload (`mps_events::AgentApiRetry`):
/// the retry in progress, or `null` once there is none.
#[derive(Debug, Clone, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ApiRetryEvent {
    pub retry: Option<ApiRetryState>,
}

/// What a frame changes about the pane's retry state.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiRetryUpdate {
    Set(ApiRetryState),
    Clear,
}

/// The retry an `api_retry` frame announces, or `None` for any other frame.
pub fn retry_from_frame(frame: &Value, now_ms: i64) -> Option<ApiRetryState> {
    if frame.get("type").and_then(Value::as_str) != Some("system")
        || frame.get("subtype").and_then(Value::as_str) != Some("api_retry")
    {
        return None;
    }
    let num = |key: &str| frame.get(key).and_then(Value::as_u64);
    let delay_ms = i64::try_from(num("retry_delay_ms").unwrap_or(0)).unwrap_or(i64::MAX);
    Some(ApiRetryState {
        attempt: u32::try_from(num("attempt")?).ok()?,
        max_retries: u32::try_from(num("max_retries").unwrap_or(0)).unwrap_or(u32::MAX),
        next_retry_at_ms: now_ms.saturating_add(delay_ms),
        error_status: num("error_status").and_then(|s| u16::try_from(s).ok()),
        error: frame
            .get("error")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
            .map(str::to_string),
    })
}

/// One stdout reader's view of whether its CLI is retrying, so a clear is
/// published once, not for every frame.
#[derive(Debug, Default)]
pub struct ApiRetryTracker {
    active: bool,
}

impl ApiRetryTracker {
    /// The update `frame` makes, if any. A `user` frame doesn't clear: the
    /// CLI echoes a message it was sent mid-turn, and that is no reply.
    pub fn observe(&mut self, frame: &Value, now_ms: i64) -> Option<ApiRetryUpdate> {
        if let Some(retry) = retry_from_frame(frame, now_ms) {
            self.active = true;
            return Some(ApiRetryUpdate::Set(retry));
        }
        let kind = frame.get("type").and_then(Value::as_str);
        if self.active && !matches!(kind, Some("system") | Some("user")) {
            self.active = false;
            return Some(ApiRetryUpdate::Clear);
        }
        None
    }

    /// The process's stdout closed: a retry still shown is over.
    pub fn finish(&mut self) -> Option<ApiRetryUpdate> {
        std::mem::take(&mut self.active).then_some(ApiRetryUpdate::Clear)
    }
}

/// Publish an update, persisted, so a pane that mounts mid-retry sees it.
pub fn publish_api_retry(broker: &crate::backend::mps::Broker, block_id: &str, update: ApiRetryUpdate) {
    let retry = match update {
        ApiRetryUpdate::Set(retry) => Some(retry),
        ApiRetryUpdate::Clear => None,
    };
    broker.publish_event::<crate::backend::mps_events::AgentApiRetry>(
        &ApiRetryEvent { retry },
        vec![format!("block:{block_id}")],
        1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_800_000_000_000;

    fn retry_frame() -> Value {
        json!({
            "type": "system", "subtype": "api_retry", "attempt": 3, "max_retries": 10,
            "retry_delay_ms": 2238, "error_status": null, "error": "unknown",
            "session_id": "s", "uuid": "u",
        })
    }

    #[test]
    fn reads_the_clis_retry_frame() {
        assert_eq!(
            retry_from_frame(&retry_frame(), NOW),
            Some(ApiRetryState {
                attempt: 3,
                max_retries: 10,
                next_retry_at_ms: NOW + 2238,
                error_status: None,
                error: Some("unknown".into()),
            })
        );
    }

    #[test]
    fn keeps_an_http_status() {
        let mut frame = retry_frame();
        frame["error_status"] = json!(529);
        frame["error"] = json!("overloaded_error");
        let retry = retry_from_frame(&frame, NOW).unwrap();
        assert_eq!(retry.error_status, Some(529));
        assert_eq!(retry.error.as_deref(), Some("overloaded_error"));
    }

    #[test]
    fn other_frames_are_not_retries() {
        assert_eq!(retry_from_frame(&json!({ "type": "system", "subtype": "init" }), NOW), None);
        assert_eq!(retry_from_frame(&json!({ "type": "assistant", "subtype": "api_retry" }), NOW), None);
        // No attempt number: not a frame this can describe.
        assert_eq!(retry_from_frame(&json!({ "type": "system", "subtype": "api_retry" }), NOW), None);
    }

    #[test]
    fn a_retry_sets_and_the_next_reply_clears_once() {
        let mut t = ApiRetryTracker::default();
        assert!(matches!(t.observe(&retry_frame(), NOW), Some(ApiRetryUpdate::Set(r)) if r.attempt == 3));
        // Neither a system frame nor an echoed user message is a reply.
        assert_eq!(t.observe(&json!({ "type": "system", "subtype": "status" }), NOW), None);
        assert_eq!(t.observe(&json!({ "type": "user", "message": {} }), NOW), None);
        assert_eq!(t.observe(&json!({ "type": "stream_event", "event": {} }), NOW), Some(ApiRetryUpdate::Clear));
        assert_eq!(t.observe(&json!({ "type": "assistant", "message": {} }), NOW), None);
    }

    #[test]
    fn the_turns_result_clears() {
        let mut t = ApiRetryTracker::default();
        t.observe(&retry_frame(), NOW);
        let result = json!({ "type": "result", "is_error": true, "result": "API Error: Connection refused" });
        assert_eq!(t.observe(&result, NOW), Some(ApiRetryUpdate::Clear));
    }

    #[test]
    fn nothing_to_clear_without_a_retry() {
        let mut t = ApiRetryTracker::default();
        assert_eq!(t.observe(&json!({ "type": "result" }), NOW), None);
        assert_eq!(t.finish(), None);
    }

    #[test]
    fn stdout_closing_clears_a_retry_still_shown() {
        let mut t = ApiRetryTracker::default();
        t.observe(&retry_frame(), NOW);
        assert_eq!(t.finish(), Some(ApiRetryUpdate::Clear));
        assert_eq!(t.finish(), None);
    }

    #[test]
    fn the_event_serializes_camel_case_and_null_when_cleared() {
        let set = ApiRetryEvent { retry: retry_from_frame(&retry_frame(), NOW) };
        let v = serde_json::to_value(&set).unwrap();
        assert_eq!(v["retry"]["maxRetries"], json!(10));
        assert_eq!(v["retry"]["nextRetryAtMs"], json!(NOW + 2238));
        assert_eq!(v["retry"]["errorStatus"], Value::Null);
        assert_eq!(serde_json::to_value(ApiRetryEvent { retry: None }).unwrap(), json!({ "retry": null }));
    }
}
