// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a leader is told about why a worker's turn failed, and what happens next (#2617): the class the
//! failure was stored under (`agent:last_failure`, written before the `agentfailure` event that
//! `worker_notes` hears), whether it is worth retrying, and who retries it.

use crate::agents::failure::FailureClass;
use crate::backend::obj::MetaMapType;

/// The failure stored in a block's meta, as a note reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredFailure {
    pub code: FailureClass,
    pub retryable: bool,
    pub resets_at_ms: Option<u64>,
}

/// The block's stored failure, or `None` when it has none this build can read.
pub(crate) fn stored(meta: &MetaMapType) -> Option<StoredFailure> {
    let f = meta.get(crate::backend::blockcontroller::core::META_LAST_FAILURE)?;
    Some(StoredFailure {
        code: serde_json::from_value(f.get("code")?.clone()).ok()?,
        retryable: f.get("retryable").and_then(|r| r.as_bool()).unwrap_or(false),
        resets_at_ms: f.get("resetsAtMs").and_then(|r| r.as_u64()),
    })
}

/// The stored failure's class as the wire names it (`rate_limited`, `network`...), for the `detail` of an
/// agent whose last run ended in an error: `error` alone doesn't say whether to retry.
pub fn code_name(meta: &MetaMapType) -> Option<String> {
    let code = stored(meta)?.code;
    serde_json::to_value(code).ok()?.as_str().map(str::to_string)
}

fn minutes_until(at_ms: u64, now_ms: u64) -> u64 {
    at_ms.saturating_sub(now_ms).div_ceil(60_000).max(1)
}

/// The sentence that follows "stopped with an error: <title>.": the class, and what happens to the
/// turn's message. Empty for a class with nothing useful to add.
pub(crate) fn what_next(f: &StoredFailure, now_ms: u64) -> String {
    match (f.code, f.resets_at_ms) {
        (FailureClass::UsageLimit, Some(at)) if at > now_ms => format!(
            "Its account is out of quota (usage limit), which lifts in about {} min; AgentMux wakes it a little after that. Messages sent to it wait until then.",
            minutes_until(at, now_ms)
        ),
        (FailureClass::UsageLimit, _) => {
            "Its account is out of quota (usage limit) and the provider gave no reset time; AgentMux can't wake it by itself.".to_string()
        }
        (FailureClass::Auth, _) => "Its sign-in was refused; it can't work until someone signs it in again.".to_string(),
        (FailureClass::RateLimited | FailureClass::Overloaded | FailureClass::Network, _) if f.retryable => {
            let why = match f.code {
                FailureClass::RateLimited => "The provider rate-limited it (HTTP 429)",
                FailureClass::Overloaded => "The provider was overloaded (HTTP 529)",
                _ => "It could not reach the provider",
            };
            // No `resets_at_ms` here is normal: a rate limit is a short burst, not an account quota with a reset.
            format!(
                "{why}; this is transient and has no reset time. The message it was working on was not answered, and AgentMux retries it by itself only while the worker's pane is open in a window. Otherwise send it another message to start a new turn."
            )
        }
        _ => String::new(),
    }
}

/// The body of the note for a failed worker.
pub(crate) fn failed_body(worker: &str, title: &str, meta: &MetaMapType, now_ms: u64) -> String {
    let base = format!("Your worker {worker} stopped with an error: {title}.");
    match stored(meta).map(|f| what_next(&f, now_ms)).filter(|s| !s.is_empty()) {
        Some(next) => format!("{base} {next}"),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(f: serde_json::Value) -> MetaMapType {
        MetaMapType::from([("agent:last_failure".to_string(), f)])
    }

    #[test]
    fn a_rate_limit_says_it_is_transient_has_no_reset_and_who_retries() {
        let m = meta(json!({ "code": "rate_limited", "title": "Rate limited", "detail": "", "retryable": true }));
        let body = failed_body("Pulp", "Rate limited", &m, 1_000);
        assert!(body.starts_with("Your worker Pulp stopped with an error: Rate limited. The provider rate-limited it (HTTP 429)"), "{body}");
        assert!(body.contains("transient and has no reset time"), "{body}");
        assert!(body.contains("only while the worker's pane is open"), "{body}");
        assert!(body.contains("send it another message"), "{body}");
    }

    #[test]
    fn a_usage_limit_names_its_reset_or_says_there_is_none() {
        let with = meta(json!({ "code": "usage_limit", "title": "Usage limit reached", "retryable": false, "resetsAtMs": 1_000 + 90 * 60_000 }));
        assert!(failed_body("W", "Usage limit reached", &with, 1_000).contains("lifts in about 90 min"));
        let without = meta(json!({ "code": "usage_limit", "title": "Usage limit reached", "retryable": false }));
        assert!(failed_body("W", "Usage limit reached", &without, 1_000).contains("no reset time"));
        let past = meta(json!({ "code": "usage_limit", "title": "x", "retryable": false, "resetsAtMs": 5 }));
        assert!(failed_body("W", "x", &past, 1_000).contains("no reset time"), "a reset already past says nothing to wait for");
    }

    #[test]
    fn the_error_states_reason_is_the_stored_class() {
        assert_eq!(code_name(&meta(json!({ "code": "rate_limited", "title": "x", "retryable": true }))).as_deref(), Some("rate_limited"));
        assert_eq!(code_name(&MetaMapType::new()), None);
    }

    #[test]
    fn a_failure_with_nothing_to_add_or_none_stored_keeps_the_plain_note() {
        let plain = "Your worker W stopped with an error: Boom.";
        assert_eq!(failed_body("W", "Boom", &MetaMapType::new(), 1), plain);
        let unknown = meta(json!({ "code": "unknown_non_zero", "title": "Boom", "retryable": false }));
        assert_eq!(failed_body("W", "Boom", &unknown, 1), plain);
        let unreadable = meta(json!({ "code": "no_such_class", "title": "Boom" }));
        assert_eq!(failed_body("W", "Boom", &unreadable, 1), plain);
        let not_retryable_429 = meta(json!({ "code": "rate_limited", "title": "Boom", "retryable": false }));
        assert_eq!(failed_body("W", "Boom", &not_retryable_429, 1), plain, "only a retryable one promises a retry");
    }
}
