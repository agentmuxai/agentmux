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
//! Published as the per-pane `agent-api-retry` event, and answered for a pane
//! that opens mid-retry by `agentapiretry`. Cleared by the next frame that is
//! neither `system` nor `user` (a reply arrived, or the turn's `result`), and
//! when the process's stdout closes.
//!
//! The wait goes out as time remaining, computed when srv sends it, never as
//! a deadline: srv's clock and the renderer's need not agree. So the event is
//! not persisted (a replay would carry a stale remainder); srv keeps the
//! current retry here instead.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;

/// One retry the CLI is waiting on, on srv's clock.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRetry {
    pub attempt: u32,
    pub max_retries: u32,
    /// When the CLI makes the next attempt (srv's epoch ms).
    pub next_retry_at_ms: i64,
    pub error_status: Option<u16>,
    pub error: Option<String>,
}

impl PendingRetry {
    /// The retry as sent at `now_ms`.
    pub fn at(&self, now_ms: i64) -> ApiRetryState {
        ApiRetryState {
            attempt: self.attempt,
            max_retries: self.max_retries,
            retry_in_ms: u64::try_from(self.next_retry_at_ms.saturating_sub(now_ms)).unwrap_or(0),
            error_status: self.error_status,
            error: self.error.clone(),
        }
    }
}

/// One retry the CLI is waiting on, as the pane gets it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct ApiRetryState {
    /// The CLI's retry count, from 1, out of `max_retries`.
    pub attempt: u32,
    pub max_retries: u32,
    /// How long until the next attempt, as of sending; the receiver adds
    /// its own clock.
    #[ts(type = "number")]
    pub retry_in_ms: u64,
    /// The HTTP status of the failed request; `null` when none came back
    /// (the connection itself failed).
    pub error_status: Option<u16>,
    /// The CLI's error kind (`"unknown"` for a connection failure).
    pub error: Option<String>,
}

/// The `agent-api-retry` event's payload (`mps_events::AgentApiRetry`), and
/// `agentapiretry`'s answer: the retry in progress, or `null` when none is.
#[derive(Debug, Clone, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ApiRetryEvent {
    pub retry: Option<ApiRetryState>,
}

/// `agentapiretry`'s request: the pane asking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAgentApiRetryData {
    pub blockid: String,
}

/// What a frame changes about the pane's retry state.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiRetryUpdate {
    Set(PendingRetry),
    Clear,
}

/// The retry an `api_retry` frame announces, or `None` for any other frame.
pub fn retry_from_frame(frame: &Value, now_ms: i64) -> Option<PendingRetry> {
    if frame.get("type").and_then(Value::as_str) != Some("system")
        || frame.get("subtype").and_then(Value::as_str) != Some("api_retry")
    {
        return None;
    }
    let num = |key: &str| frame.get(key).and_then(Value::as_u64);
    let delay_ms = i64::try_from(num("retry_delay_ms").unwrap_or(0)).unwrap_or(i64::MAX);
    Some(PendingRetry {
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
/// sent once, not for every frame.
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

/// The retry each pane shows, and which process generation it came from.
#[derive(Debug, Default)]
pub struct ApiRetryRegistry {
    by_block: HashMap<String, (u64, PendingRetry)>,
}

impl ApiRetryRegistry {
    /// Applies an update from the stdout reader of process `generation`,
    /// `current` being the controller's generation now; returns the event to
    /// publish, if the pane's state changed. A replaced process's reader can
    /// still be draining its pipe: it may clear only a retry it set itself,
    /// and may not set one, or a stale row would offer to stop the new process.
    pub fn apply(
        &mut self,
        block_id: &str,
        generation: u64,
        current: u64,
        update: ApiRetryUpdate,
        now_ms: i64,
    ) -> Option<ApiRetryEvent> {
        match update {
            ApiRetryUpdate::Set(retry) if generation == current => {
                let event = ApiRetryEvent { retry: Some(retry.at(now_ms)) };
                self.by_block.insert(block_id.to_string(), (generation, retry));
                Some(event)
            }
            ApiRetryUpdate::Set(_) => None,
            ApiRetryUpdate::Clear => {
                let owned = self.by_block.get(block_id).is_some_and(|(g, _)| *g == generation);
                owned.then(|| {
                    self.by_block.remove(block_id);
                    ApiRetryEvent { retry: None }
                })
            }
        }
    }

    /// The pane's retry as of `now_ms`.
    pub fn current(&self, block_id: &str, now_ms: i64) -> ApiRetryEvent {
        ApiRetryEvent { retry: self.by_block.get(block_id).map(|(_, retry)| retry.at(now_ms)) }
    }
}

static REGISTRY: LazyLock<Mutex<ApiRetryRegistry>> = LazyLock::new(Mutex::default);

/// Records an update from the reader of process `generation` and publishes
/// the change, if any (see [`ApiRetryRegistry::apply`]).
pub fn publish_api_retry(
    broker: Option<&crate::backend::mps::Broker>,
    block_id: &str,
    generation: u64,
    current: u64,
    update: ApiRetryUpdate,
) {
    let now_ms = agentmux_common::time::now_ms();
    let event = REGISTRY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .apply(block_id, generation, current, update, now_ms);
    if let (Some(event), Some(broker)) = (event, broker) {
        broker.publish_event::<crate::backend::mps_events::AgentApiRetry>(&event, vec![format!("block:{block_id}")], 0);
    }
}

/// `agentapiretry`: the pane's retry as of now.
pub fn current_api_retry(block_id: &str) -> ApiRetryEvent {
    REGISTRY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .current(block_id, agentmux_common::time::now_ms())
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

    fn pending(attempt: u32) -> PendingRetry {
        PendingRetry { attempt, max_retries: 10, next_retry_at_ms: NOW + 10_000, error_status: None, error: None }
    }

    #[test]
    fn reads_the_clis_retry_frame() {
        assert_eq!(
            retry_from_frame(&retry_frame(), NOW),
            Some(PendingRetry {
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
    fn the_wait_is_sent_as_time_remaining_when_sent() {
        let mut r = ApiRetryRegistry::default();
        let sent = r.apply("b", 1, 1, ApiRetryUpdate::Set(pending(3)), NOW).unwrap();
        assert_eq!(sent.retry.unwrap().retry_in_ms, 10_000);
        // A pane opening 4 s later is told 6 s, and nothing once it is due.
        assert_eq!(r.current("b", NOW + 4_000).retry.unwrap().retry_in_ms, 6_000);
        assert_eq!(r.current("b", NOW + 60_000).retry.unwrap().retry_in_ms, 0);
        assert_eq!(r.current("other", NOW).retry, None);
    }

    #[test]
    fn the_owning_generation_clears() {
        let mut r = ApiRetryRegistry::default();
        r.apply("b", 1, 1, ApiRetryUpdate::Set(pending(3)), NOW);
        assert_eq!(r.apply("b", 1, 1, ApiRetryUpdate::Clear, NOW), Some(ApiRetryEvent { retry: None }));
        assert_eq!(r.current("b", NOW).retry, None);
        // Nothing left to clear: nothing to publish.
        assert_eq!(r.apply("b", 1, 1, ApiRetryUpdate::Clear, NOW), None);
    }

    #[test]
    fn a_replaced_process_changes_nothing_of_its_replacements() {
        let mut r = ApiRetryRegistry::default();
        r.apply("b", 2, 2, ApiRetryUpdate::Set(pending(5)), NOW);
        // Generation 1's reader, still draining after generation 2 started.
        assert_eq!(r.apply("b", 1, 2, ApiRetryUpdate::Set(pending(9)), NOW), None);
        assert_eq!(r.apply("b", 1, 2, ApiRetryUpdate::Clear, NOW), None);
        assert_eq!(r.current("b", NOW).retry.unwrap().attempt, 5);
    }

    #[test]
    fn a_replaced_process_still_clears_a_retry_it_set() {
        // Otherwise a row from the old process would stay until the new one retried.
        let mut r = ApiRetryRegistry::default();
        r.apply("b", 1, 1, ApiRetryUpdate::Set(pending(3)), NOW);
        assert_eq!(r.apply("b", 1, 2, ApiRetryUpdate::Clear, NOW), Some(ApiRetryEvent { retry: None }));
        assert_eq!(r.current("b", NOW).retry, None);
    }

    #[test]
    fn the_event_serializes_camel_case_and_null_when_cleared() {
        let set = ApiRetryEvent { retry: Some(pending(3).at(NOW)) };
        let v = serde_json::to_value(&set).unwrap();
        assert_eq!(v["retry"]["maxRetries"], json!(10));
        assert_eq!(v["retry"]["retryInMs"], json!(10_000));
        assert_eq!(v["retry"]["errorStatus"], Value::Null);
        assert!(v["retry"].get("nextRetryAtMs").is_none(), "no server-clock deadline on the wire: {v}");
        assert_eq!(serde_json::to_value(ApiRetryEvent { retry: None }).unwrap(), json!({ "retry": null }));
    }
}
