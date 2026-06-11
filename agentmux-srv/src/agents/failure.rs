// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Classification of agent (Claude CLI) run failures into a small,
//! stable taxonomy with a human-readable explanation.
//!
//! `classify()` is a **pure** function — exit code + terminating signal
//! + the tail of the child's stderr + an optional terminal `result`
//! frame go in; an [`AgentFailure`] comes out. It does no IO and is
//! exhaustively unit-tested against the real Anthropic error phrasings,
//! so the "why" behind a non-zero exit can be surfaced to the user
//! instead of a bare `exit 1`.
//!
//! See `docs/specs/SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md`. This
//! is the P1 (capture + classify) slice; live-stream emission and UI
//! banners are P2/P3.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable failure taxonomy. Wire format is snake_case to match the rest
/// of the agent event surface (`frontend/types/gotypes.d.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Server-side rate limit (HTTP 429). Transient, not the account quota.
    RateLimited,
    /// API overloaded (HTTP 529). Transient.
    Overloaded,
    /// Plan / usage / billing limit reached. Not transient.
    UsageLimit,
    /// Credentials rejected (HTTP 401).
    Auth,
    /// Conversation exceeded the model's context window.
    ContextExceeded,
    /// Hit the configured `--max-turns` cap.
    MaxTurns,
    /// Network / connectivity error reaching the API.
    Network,
    /// Killed by a signal (OOM killer or external stop).
    Killed,
    /// Exited cleanly but produced no final result.
    NoOutput,
    /// Could not launch the `claude` binary at all.
    SpawnFailure,
    /// Non-zero exit with no recognized cause.
    UnknownNonZero,
}

/// A classified agent failure: the class, a user-facing title + detail,
/// the raw exit evidence, the stderr tail, and whether a retry is
/// worth attempting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentFailure {
    pub code: FailureClass,
    pub title: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stderr_tail: String,
    pub retryable: bool,
}

impl AgentFailure {
    /// Render the single-string explanation that becomes a run's
    /// terminal error (drone `error` column, sidecar log, future agent
    /// pane banner). Title + detail + exit evidence + retryable hint,
    /// followed by the raw stderr tail so the original text is never
    /// more than a glance away.
    pub fn explain(&self) -> String {
        let mut s = self.title.clone();
        if !self.detail.is_empty() {
            s.push_str(" — ");
            s.push_str(&self.detail);
        }
        match (self.signal, self.exit_code) {
            (Some(sig), _) => s.push_str(&format!(" [signal {sig}]")),
            (None, Some(code)) => s.push_str(&format!(" [exit {code}]")),
            (None, None) => {}
        }
        if self.retryable {
            s.push_str(" (retryable)");
        }
        if !self.stderr_tail.is_empty() {
            s.push_str("\n--- claude stderr (tail) ---\n");
            s.push_str(&self.stderr_tail);
        }
        s
    }
}

/// Classify an agent run failure.
///
/// `exit_code` / `signal` come from the child's `ExitStatus`. `stderr`
/// is the captured (capped) child stderr. `result_frame` is the
/// terminal stream-json `result` object when one was seen — the CLI
/// sometimes reports an error there while still exiting 0, so it is
/// folded into the evidence.
pub fn classify(
    exit_code: Option<i32>,
    signal: Option<i32>,
    stderr: &str,
    result_frame: Option<&Value>,
) -> AgentFailure {
    let frame_is_error = result_frame.is_some_and(frame_is_error);
    let frame_text = result_frame.map(frame_error_text).unwrap_or_default();

    let combined = if frame_text.is_empty() {
        stderr.to_string()
    } else {
        format!("{stderr}\n{frame_text}")
    };
    let tail = tail_lines(&combined, 24, 1600);
    let hay = combined.to_ascii_lowercase();

    // Explicit terminal-frame subtype: turn cap reached.
    if result_frame
        .and_then(|f| f.get("subtype"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| s.contains("max_turns"))
    {
        return build(
            FailureClass::MaxTurns,
            "Hit the turn limit",
            "The agent reached its configured `--max-turns` cap before finishing. Raise the cap or split the task.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }

    // Killed by a signal (OOM killer or external stop). 137 == 128 + SIGKILL(9).
    if signal.is_some() || exit_code == Some(137) {
        return build(
            FailureClass::Killed,
            "Agent process was killed",
            "Terminated by a signal — most often the OS out-of-memory killer or an external stop. Reduce concurrent load / memory pressure and retry.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }

    // Keyword matches. Order matters: rate-limit and overload are checked
    // before usage-limit because the server-side rate-limit message
    // literally contains "not your usage limit".
    if hay.contains("rate limited")
        || hay.contains("temporarily limiting requests")
        || hay.contains("rate_limit")
        || hay.contains("429")
    {
        return build(
            FailureClass::RateLimited,
            "Rate-limited by the API",
            "Server-side rate limit — transient, and not your account quota. Retry shortly; consider lowering parallelism.",
            true,
            exit_code,
            signal,
            &tail,
        );
    }
    if hay.contains("overloaded") || hay.contains("529") {
        return build(
            FailureClass::Overloaded,
            "API temporarily overloaded",
            "The API reported it is overloaded (HTTP 529). Transient — retry with backoff.",
            true,
            exit_code,
            signal,
            &tail,
        );
    }
    if hay.contains("authentication_error")
        || hay.contains("invalid api key")
        || hay.contains("invalid x-api-key")
        || hay.contains("unauthorized")
        || hay.contains("please run /login")
        || hay.contains("401")
    {
        return build(
            FailureClass::Auth,
            "Not authenticated",
            "The API rejected the credentials (HTTP 401). Re-authenticate via the Identity tab or `claude /login`.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    if hay.contains("model_context_window_exceeded")
        || hay.contains("prompt is too long")
        || (hay.contains("context") && (hay.contains("exceed") || hay.contains("window") || hay.contains("too long")))
    {
        return build(
            FailureClass::ContextExceeded,
            "Context window exceeded",
            "The conversation exceeded the model's context window. Clear or compact the agent's history and retry.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    if hay.contains("max turns") || hay.contains("max-turns") || hay.contains("maximum number of turns") {
        return build(
            FailureClass::MaxTurns,
            "Hit the turn limit",
            "The agent reached its configured `--max-turns` cap before finishing. Raise the cap or split the task.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    if hay.contains("usage limit")
        || hay.contains("out of credits")
        || hay.contains("quota")
        || hay.contains("billing")
        || hay.contains("insufficient")
    {
        return build(
            FailureClass::UsageLimit,
            "Usage limit reached",
            "Your plan or usage limit appears to be reached. Check plan / billing — retrying won't help until it resets.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    if hay.contains("apiconnectionerror")
        || hay.contains("connection error")
        || hay.contains("could not resolve")
        || hay.contains("econnreset")
        || hay.contains("network")
        || hay.contains("timed out")
        || hay.contains("timeout")
        || hay.contains("dns")
    {
        return build(
            FailureClass::Network,
            "Network error reaching the API",
            "A network/connection error occurred talking to the API. Check connectivity and retry.",
            true,
            exit_code,
            signal,
            &tail,
        );
    }

    // Fallbacks.
    if frame_is_error {
        return build(
            FailureClass::UnknownNonZero,
            "Agent reported an error",
            "The agent reported a terminal error with no recognized cause. The raw output tail is included below.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    if exit_code == Some(0) {
        return build(
            FailureClass::NoOutput,
            "Agent produced no output",
            "The agent exited cleanly (code 0) but never produced a final result. Inspect the transcript / sidecar log.",
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    build(
        FailureClass::UnknownNonZero,
        "Agent failed",
        "The agent exited with a non-zero status and no recognized error. The raw stderr tail is included below.",
        false,
        exit_code,
        signal,
        &tail,
    )
}

#[allow(clippy::too_many_arguments)]
fn build(
    code: FailureClass,
    title: &str,
    detail: &str,
    retryable: bool,
    exit_code: Option<i32>,
    signal: Option<i32>,
    tail: &str,
) -> AgentFailure {
    AgentFailure {
        code,
        title: title.to_string(),
        detail: detail.to_string(),
        exit_code,
        signal,
        stderr_tail: tail.to_string(),
        retryable,
    }
}

/// Best-effort error text from a terminal stream-json `result` frame:
/// `error.message`, else a string `error`/`result`, else the `subtype`.
fn frame_error_text(frame: &Value) -> String {
    if let Some(msg) = frame.pointer("/error/message").and_then(|v| v.as_str()) {
        return msg.to_string();
    }
    if let Some(err) = frame.get("error").and_then(|v| v.as_str()) {
        return err.to_string();
    }
    if let Some(res) = frame.get("result").and_then(|v| v.as_str()) {
        return res.to_string();
    }
    frame
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// True if a terminal `result` frame represents a failure.
fn frame_is_error(frame: &Value) -> bool {
    frame.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false)
        || frame
            .get("subtype")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.starts_with("error"))
}

/// Last `max_lines` non-empty lines of `s`, further capped to the last
/// `max_chars` characters (char-safe — never slices mid-codepoint).
fn tail_lines(s: &str, max_lines: usize, max_chars: usize) -> String {
    let mut lines: Vec<&str> = s
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    let joined = lines.join("\n");
    let n = joined.chars().count();
    if n > max_chars {
        joined.chars().skip(n - max_chars).collect()
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rate_limit_incident_string_classifies_retryable() {
        // The exact string this work was motivated by.
        let stderr =
            "API Error: Server is temporarily limiting requests (not your usage limit) · Rate limited";
        let f = classify(Some(1), None, stderr, None);
        assert_eq!(f.code, FailureClass::RateLimited);
        assert!(f.retryable);
        let e = f.explain();
        assert!(e.contains("Rate-limited"), "explain: {e}");
        assert!(e.contains("retryable"), "explain: {e}");
        assert!(e.to_lowercase().contains("rate limited"), "explain: {e}");
    }

    #[test]
    fn not_your_usage_limit_does_not_classify_as_usage_limit() {
        // Guard: the rate-limit message contains "usage limit"; it must
        // win as RateLimited (checked first), not UsageLimit.
        let f = classify(
            Some(1),
            None,
            "temporarily limiting requests (not your usage limit) · Rate limited",
            None,
        );
        assert_eq!(f.code, FailureClass::RateLimited);
    }

    #[test]
    fn overloaded_is_retryable() {
        let f = classify(Some(1), None, "overloaded_error: please retry", None);
        assert_eq!(f.code, FailureClass::Overloaded);
        assert!(f.retryable);
    }

    #[test]
    fn invalid_api_key_is_auth_not_retryable() {
        let f = classify(Some(1), None, "authentication_error: Invalid API key provided", None);
        assert_eq!(f.code, FailureClass::Auth);
        assert!(!f.retryable);
    }

    #[test]
    fn sigkill_is_killed() {
        let f = classify(None, Some(9), "", None);
        assert_eq!(f.code, FailureClass::Killed);
        assert_eq!(f.signal, Some(9));
        assert!(f.explain().contains("[signal 9]"));
    }

    #[test]
    fn exit_137_is_killed() {
        let f = classify(Some(137), None, "", None);
        assert_eq!(f.code, FailureClass::Killed);
    }

    #[test]
    fn context_window_exceeded_classifies() {
        let f = classify(
            Some(1),
            None,
            "prompt is too long: 1200000 tokens > 200000 maximum context window",
            None,
        );
        assert_eq!(f.code, FailureClass::ContextExceeded);
    }

    #[test]
    fn network_error_is_retryable() {
        let f = classify(Some(1), None, "APIConnectionError: Connection error.", None);
        assert_eq!(f.code, FailureClass::Network);
        assert!(f.retryable);
    }

    #[test]
    fn clean_exit_no_output_is_no_output() {
        let f = classify(Some(0), None, "", None);
        assert_eq!(f.code, FailureClass::NoOutput);
        assert!(!f.retryable);
    }

    #[test]
    fn unknown_nonzero_is_default() {
        let f = classify(Some(2), None, "something unexpected happened", None);
        assert_eq!(f.code, FailureClass::UnknownNonZero);
        assert!(f.explain().contains("[exit 2]"));
    }

    #[test]
    fn result_frame_max_turns_subtype() {
        let frame = json!({ "type": "result", "is_error": true, "subtype": "error_max_turns" });
        let f = classify(Some(0), None, "", Some(&frame));
        assert_eq!(f.code, FailureClass::MaxTurns);
    }

    #[test]
    fn result_frame_error_message_feeds_classification() {
        // CLI reported an overload on stdout but exited 0 — must still
        // classify, not fall through to NoOutput.
        let frame = json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": { "message": "overloaded_error: upstream busy" }
        });
        let f = classify(Some(0), None, "", Some(&frame));
        assert_eq!(f.code, FailureClass::Overloaded);
    }

    #[test]
    fn tail_lines_keeps_last_lines_char_safe() {
        let s = (0..100).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        let t = tail_lines(&s, 5, 1000);
        assert_eq!(t.lines().count(), 5);
        assert!(t.contains("line99"));
        assert!(!t.contains("line0\n"));
    }

    #[test]
    fn serializes_snake_case_code_camel_fields() {
        let f = classify(Some(1), None, "Rate limited", None);
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["code"], json!("rate_limited"));
        assert_eq!(v["retryable"], json!(true));
        assert!(v.get("stderrTail").is_some(), "camelCase stderrTail expected: {v}");
    }
}
