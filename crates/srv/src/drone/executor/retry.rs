// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Running a block again when it failed for a reason that is the world's, not
//! its own (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.4).
//!
//! A block opts in with `retry`: a number of retries (1 to 3), or
//! `{ "max": 1 to 3, "backoff_seconds": 1 to 300 }`. The wait doubles each time
//! from `backoff_seconds` (2 when not set) and never passes five minutes.
//!
//! Only **infrastructure** failures are retried: a network error, a 5xx or 429
//! answer from an API, a provider's rate limit, a machine out of memory. A block
//! that ran and failed on its own terms (a bad reducer, a refused URL, a
//! timeout, an agent that said `failed`) is never run again, because running
//! it again would only do the same thing, or do it twice.

use std::time::Duration;

use serde_json::Value;

use crate::drone::types::{BlockKind, FlowNode};

/// The most times a block is run again.
pub(super) const MAX_RETRIES: u32 = 3;
const DEFAULT_BACKOFF_SECONDS: u64 = 2;
const MAX_BACKOFF_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Retry {
    pub(super) max: u32,
    pub(super) backoff: Duration,
}

/// A block's `retry`, when it has one.
pub(super) fn retry_of(node: &FlowNode) -> Result<Option<Retry>, String> {
    let (max, backoff) = match node.data.get("retry") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(n)) => (n.as_u64(), Some(DEFAULT_BACKOFF_SECONDS)),
        Some(Value::Object(o)) => {
            let backoff = match o.get("backoff_seconds") {
                None | Some(Value::Null) => Some(DEFAULT_BACKOFF_SECONDS),
                Some(v) => v.as_u64(),
            };
            (o.get("max").and_then(Value::as_u64), backoff)
        }
        Some(_) => (None, None),
    };
    match (max, backoff) {
        (Some(max), Some(secs)) if (1..=u64::from(MAX_RETRIES)).contains(&max) && (1..=MAX_BACKOFF_SECONDS).contains(&secs) => {
            Ok(Some(Retry { max: max as u32, backoff: Duration::from_secs(secs) }))
        }
        _ => Err(format!(
            "{}: retry must be a number of retries from 1 to {MAX_RETRIES}, or {{ \"max\": 1 to {MAX_RETRIES}, \"backoff_seconds\": 1 to {MAX_BACKOFF_SECONDS} }}",
            node.id
        )),
    }
}

/// How long to wait before retry number `attempt` (1 is the first retry).
pub(super) fn delay(retry: &Retry, attempt: u32) -> Duration {
    let doubled = retry.backoff.saturating_mul(1u32 << attempt.saturating_sub(1).min(8));
    doubled.min(Duration::from_secs(MAX_BACKOFF_SECONDS))
}

/// Whether the error a block gave is the world's doing.
fn is_infrastructure(kind: BlockKind, error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    match kind {
        // `blocks::api`: the request did not complete, or its body did not arrive.
        BlockKind::Api => error.starts_with("request failed:") || error.starts_with("read body:"),
        // `blocks::agent`: no memory to start another agent, or the provider is limiting requests.
        BlockKind::Agent => lower.contains("memory full") || lower.contains("rate-limited") || lower.contains("rate limit"),
        _ => false,
    }
}

/// Whether a block that finished gave an answer that says the other side was
/// not ready: an API's 429 or 5xx.
fn is_infrastructure_output(kind: BlockKind, output: &Value) -> bool {
    kind == BlockKind::Api && output.get("status").and_then(Value::as_u64).is_some_and(|s| s == 429 || (500..600).contains(&s))
}

/// Whether to run the block again after `result`, having run it `ran` times.
pub(super) fn again(retry: Option<Retry>, kind: BlockKind, result: &Result<Value, String>, ran: u32) -> bool {
    let Some(retry) = retry else { return false };
    if ran > retry.max {
        return false;
    }
    match result {
        Err(e) => is_infrastructure(kind, e),
        Ok(output) => is_infrastructure_output(kind, output),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::drone::types::NodePosition;

    fn node(retry: Value) -> FlowNode {
        FlowNode { id: "a".to_string(), position: NodePosition::default(), data: json!({ "kind": "api", "retry": retry }), node_type: None }
    }

    #[test]
    fn a_retry_is_a_count_or_a_count_with_a_backoff() {
        assert_eq!(retry_of(&node(json!(2))).unwrap(), Some(Retry { max: 2, backoff: Duration::from_secs(2) }));
        assert_eq!(retry_of(&node(json!({ "max": 3, "backoff_seconds": 10 }))).unwrap(), Some(Retry { max: 3, backoff: Duration::from_secs(10) }));
        assert_eq!(retry_of(&node(json!({ "max": 1 }))).unwrap(), Some(Retry { max: 1, backoff: Duration::from_secs(2) }));
        assert_eq!(retry_of(&node(Value::Null)).unwrap(), None);
        for bad in [json!(0), json!(4), json!(-1), json!(1.5), json!("2"), json!(true), json!({}), json!({ "max": 2, "backoff_seconds": 0 }), json!({ "max": 2, "backoff_seconds": 301 })] {
            let e = retry_of(&node(bad.clone())).unwrap_err();
            assert!(e.contains("retry must be a number of retries from 1 to 3"), "{bad}: {e}");
        }
    }

    #[test]
    fn the_wait_doubles_and_stops_at_five_minutes() {
        let r = Retry { max: 3, backoff: Duration::from_secs(10) };
        let waits: Vec<u64> = (1..=3).map(|n| delay(&r, n).as_secs()).collect();
        assert_eq!(waits, [10, 20, 40]);
        let long = Retry { max: 3, backoff: Duration::from_secs(300) };
        assert_eq!(delay(&long, 3), Duration::from_secs(300));
    }

    #[test]
    fn only_the_worlds_failures_are_run_again() {
        let one = Some(Retry { max: 2, backoff: Duration::from_secs(1) });
        let api_err = |e: &str| Err::<Value, String>(e.to_string());
        assert!(again(one, BlockKind::Api, &api_err("request failed: connection reset"), 1));
        assert!(again(one, BlockKind::Api, &api_err("read body: error decoding"), 1));
        assert!(!again(one, BlockKind::Api, &api_err("host `localhost` is not allowed"), 1), "a refused URL stays refused");
        assert!(!again(one, BlockKind::Api, &api_err("timed out after 30 seconds: operation timed out"), 1), "a timeout is not retried");
        assert!(again(one, BlockKind::Api, &Ok(json!({ "status": 503 })), 1));
        assert!(again(one, BlockKind::Api, &Ok(json!({ "status": 429 })), 1));
        assert!(!again(one, BlockKind::Api, &Ok(json!({ "status": 404 })), 1), "a 404 is an answer");
        assert!(!again(one, BlockKind::Api, &Ok(json!({ "status": 200 })), 1));
        assert!(again(one, BlockKind::Agent, &api_err("agent block: agent run failed: Rate-limited by the provider"), 1));
        assert!(again(one, BlockKind::Agent, &api_err("agent block: memory full: 1 GB free"), 1));
        assert!(!again(one, BlockKind::Agent, &api_err("agent block: agent run failed: it said failed"), 1), "a finished agent is not asked again");
        assert!(!again(one, BlockKind::Message, &api_err("request failed: x"), 1), "only the blocks that know their failures");
    }

    #[test]
    fn a_block_is_run_again_at_most_max_times_and_never_without_a_retry() {
        let r = Some(Retry { max: 2, backoff: Duration::from_secs(1) });
        let down = Err::<Value, String>("request failed: down".to_string());
        assert!(again(r, BlockKind::Api, &down, 1), "first run failed: retry 1");
        assert!(again(r, BlockKind::Api, &down, 2), "second run failed: retry 2");
        assert!(!again(r, BlockKind::Api, &down, 3), "third run failed: that was the last");
        assert!(!again(None, BlockKind::Api, &down, 1));
    }
}
