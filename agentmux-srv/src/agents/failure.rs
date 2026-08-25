// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Classification of agent (Claude CLI) run failures into a small,
//! stable taxonomy with a human-readable explanation.
//!
//! `classify()` is a **pure** function: it takes the exit code, the
//! terminating signal, the tail of the child's stderr, and an optional
//! terminal `result` frame, and returns an [`AgentFailure`]. It does no
//! IO and is exhaustively unit-tested against the real Anthropic error
//! phrasings, so the "why" behind a non-zero exit can be surfaced to the
//! user instead of a bare `exit 1`.
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
    /// The process is still alive but has produced no meaningful output for
    /// `HealthMonitor::DEAD_SECS` during an active turn — not exit-based
    /// (unlike every other variant here), so it has no exit code/signal to
    /// report. Not retryable via the normal "re-send the last message" path
    /// (the process must be killed and respawned first, since it's the
    /// process itself that's wedged) — surfaced with a "Restart" action
    /// instead. See docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §4.
    Unresponsive,
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
    let frame_reported_error = result_frame.is_some_and(is_error_result_frame);
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
        || mentions_http_status(&hay, "429")
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
    if hay.contains("overloaded") || mentions_http_status(&hay, "529") {
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
    // The identity spawn gate's own refusal (identity/resolver/errors.rs's
    // `SpawnGateError::MissingCredentials` Display impl — a deliberate,
    // spec-owned wording per SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md
    // §2.2, not third-party CLI/API output, so matching on it is as stable
    // as matching our own code). This never reaches the API at all — the
    // spawn was refused before any CLI process existed to report a 401 —
    // so it's checked separately from the 401 branch below with its own
    // wording, but classifies the same way: no account bound is exactly as
    // actionable as a rejected credential, and needs the same relogin UI.
    // Before this, the message had no recognized keyword and fell through
    // to `UnknownNonZero`, which offers only "Retry" — a retry can never
    // succeed against a gate that blocks every respawn identically, so the
    // agent pane got stuck showing a dead-end error with no way out (see
    // retro-agentu-0.54.9-stuck-error-2026-08-03.md).
    if hay.contains("bind an account for this provider in the armory") {
        // codex P2 on PR #2413: this branch matches EVERY oauth-class
        // provider's MissingCredentials refusal (Codex, Gemini, OpenClaw,
        // Copilot — not just Claude), but a prior version of this message
        // hardcoded "No Claude account is bound". Extract the provider id
        // from the gate's own wording ("no credentials for {provider}: …")
        // instead of assuming — falls back to generic phrasing if the
        // extraction ever misses (wording drift in errors.rs), rather than
        // asserting a specific, possibly wrong, provider name.
        let provider_phrase = extract_spawn_gate_provider(&combined)
            .map(|p| format!("No {} account", capitalize_provider(&p)))
            .unwrap_or_else(|| "No account".to_string());
        return build(
            FailureClass::Auth,
            "No account linked",
            &format!(
                "{provider_phrase} is bound to this agent — the spawn was refused before it could run. \
                 Sign in to link one."
            ),
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    // The identity spawn gate's ambient-home-dir refusal (identity/resolver/
    // errors.rs's `SpawnGateError::AmbientHomeDirNotAllowed` Display impl —
    // same reasoning as the MissingCredentials branch above: this never
    // reaches the API, and a bare "Retry" would be a dead end since the
    // gate blocks every respawn identically until the identity is rebound.
    if hay.contains("instead of an isolated agentmux account") {
        let provider_phrase = extract_ambient_home_provider(&combined)
            .map(|p| format!("This agent's {} identity", capitalize_provider(&p)))
            .unwrap_or_else(|| "This agent's identity".to_string());
        return build(
            FailureClass::Auth,
            "Identity points at your personal login",
            &format!(
                "{provider_phrase} is bound directly to your personal CLI login \
                 directory, which AgentMux no longer allows. Re-bind it to an \
                 isolated account in Armory \u{2192} Accounts, then retry."
            ),
            false,
            exit_code,
            signal,
            &tail,
        );
    }
    if hay.contains("authentication_error")
        || hay.contains("authentication_failed")
        || hay.contains("invalid authentication")
        || hay.contains("invalid api key")
        || hay.contains("invalid x-api-key")
        || hay.contains("unauthorized")
        || hay.contains("please run /login")
        || mentions_http_status(&hay, "401")
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
    if frame_reported_error {
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

/// Pulls the provider id out of `SpawnGateError::MissingCredentials`'s own
/// Display wording ("no credentials for {provider}: the bound account was
/// deleted or is unresolvable. Bind an account for this provider in the
/// Armory." — identity/resolver/errors.rs). Returns `None` on any mismatch
/// (e.g. that wording drifts) rather than guessing — callers fall back to
/// generic phrasing instead of asserting a specific, possibly wrong,
/// provider name.
fn extract_spawn_gate_provider(combined: &str) -> Option<String> {
    let marker = "no credentials for ";
    let start = combined.find(marker)? + marker.len();
    let rest = &combined[start..];
    let end = rest.find(':')?;
    let provider = rest[..end].trim();
    if provider.is_empty() {
        None
    } else {
        Some(provider.to_string())
    }
}

/// Pulls the provider id out of
/// `SpawnGateError::AmbientHomeDirNotAllowed`'s own Display wording
/// ("this agent's {provider} identity points directly at your personal …" —
/// identity/resolver/errors.rs). Same "return None on any mismatch rather
/// than guessing" contract as `extract_spawn_gate_provider` above.
fn extract_ambient_home_provider(combined: &str) -> Option<String> {
    let marker = "this agent's ";
    let start = combined.find(marker)? + marker.len();
    let rest = &combined[start..];
    let end = rest.find(" identity points directly")?;
    let provider = rest[..end].trim();
    if provider.is_empty() {
        None
    } else {
        Some(provider.to_string())
    }
}

/// Title-cases a raw provider id ("claude" -> "Claude") for the
/// spawn-gate message. reagent P2 on PR #2413: the raw lowercase slug
/// read poorly next to catalog.ts's real display names ("Claude Code",
/// "Gemini CLI"); this doesn't reach for those (Rust has no access to
/// the frontend catalog and duplicating/syncing the full display-name
/// table across languages is disproportionate for one error string) —
/// just enough prettification that the message reads as a proper noun.
fn capitalize_provider(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => id.to_string(),
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
pub(crate) fn is_error_result_frame(frame: &Value) -> bool {
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

/// True if `hay` mentions HTTP status `code` in a status-like context
/// (e.g. "http 429", "status 429", "error 429", "[429]"). Anchoring on a
/// context word avoids the false positives a bare substring would hit —
/// an ISO date like `2026-04-29` contains "429". (reagent P2 on #1353.)
fn mentions_http_status(hay: &str, code: &str) -> bool {
    [
        format!("http {code}"),
        format!("http/1.1 {code}"),
        format!("http/2 {code}"),
        format!("status {code}"),
        format!("status: {code}"),
        format!("status code {code}"),
        format!("code {code}"),
        format!("code: {code}"),
        format!("error {code}"),
        format!("error: {code}"),
        format!("[{code}]"),
        format!("({code})"),
    ]
    .iter()
    .any(|p| hay.contains(p.as_str()))
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
    fn iso_date_in_stderr_does_not_false_positive() {
        // "2026-04-29" contains "429"; "2026-04-01" -> "401";
        // "2026-05-29" -> "529". A bare substring match would misclassify
        // these; anchored matching must not. (reagent P2 on #1353.)
        for stderr in [
            "panic at 2026-04-29T10:00:00: unexpected eof",
            "build 2026-04-01 failed: assertion",
            "ts=2026-05-29 fatal: segfault",
        ] {
            let f = classify(Some(2), None, stderr, None);
            assert_eq!(
                f.code,
                FailureClass::UnknownNonZero,
                "stderr {stderr:?} must not match an HTTP status"
            );
        }
    }

    #[test]
    fn anchored_http_status_still_classifies() {
        assert_eq!(
            classify(Some(1), None, "Error: HTTP 429 Too Many Requests", None).code,
            FailureClass::RateLimited
        );
        assert_eq!(
            classify(Some(1), None, "request failed with status 401", None).code,
            FailureClass::Auth
        );
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
    fn authentication_failed_string_is_auth() {
        // In-band 401: claude wraps the error in a synthetic assistant message
        // with `"error":"authentication_failed"`. The inband_text is appended
        // to the stderr tail before classify() is called, so it must match.
        let f = classify(
            Some(0),
            None,
            "authentication_failed Failed to authenticate. API Error: 401 Invalid authentication credentials",
            None,
        );
        assert_eq!(f.code, FailureClass::Auth);
        assert!(!f.retryable);
    }

    #[test]
    fn invalid_authentication_credentials_is_auth() {
        let f = classify(Some(0), None, "invalid authentication credentials", None);
        assert_eq!(f.code, FailureClass::Auth);
    }

    #[test]
    fn api_error_colon_401_is_auth() {
        // "API Error: 401" lowercased → "api error: 401" → matches "error: 401".
        let f = classify(Some(0), None, "api error: 401 invalid authentication credentials", None);
        assert_eq!(f.code, FailureClass::Auth);
    }

    #[test]
    fn spawn_gate_missing_credentials_is_auth_not_unknown() {
        // Exact frame shape agent_handlers/input.rs emits for a
        // SpawnGateError::MissingCredentials refusal (identity/resolver/
        // errors.rs's Display impl) — before this branch existed, this
        // frame had no recognized keyword and fell through to
        // UnknownNonZero, which offers only "Retry" against a gate that
        // blocks every respawn identically: a permanently stuck pane with
        // no relogin affordance (retro-agentu-0.54.9-stuck-error-2026-08-03.md).
        let frame = json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": { "message": "[AgentMux] no credentials for claude: the bound account was deleted or is unresolvable. Bind an account for this provider in the Armory." }
        });
        let f = classify(Some(1), None, "", Some(&frame));
        assert_eq!(f.code, FailureClass::Auth);
        assert!(f.detail.contains("Claude"), "detail: {}", f.detail);
    }

    #[test]
    fn spawn_gate_missing_credentials_names_the_actual_provider_not_claude() {
        // codex P2 on PR #2413: this branch matches EVERY oauth-class
        // provider's refusal, not just Claude's — a prior version
        // hardcoded "No Claude account is bound" regardless of which
        // provider the gate actually named.
        let frame = json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": { "message": "[AgentMux] no credentials for gemini: the bound account was deleted or is unresolvable. Bind an account for this provider in the Armory." }
        });
        let f = classify(Some(1), None, "", Some(&frame));
        assert_eq!(f.code, FailureClass::Auth);
        assert!(f.detail.contains("Gemini"), "detail: {}", f.detail);
        assert!(!f.detail.contains("Claude"), "detail: {}", f.detail);
    }

    #[test]
    fn spawn_gate_ambient_home_dir_is_auth_not_unknown() {
        // docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md
        // — same "must not fall through to a dead-end Retry" reasoning as
        // the MissingCredentials branch above, for the sibling
        // AmbientHomeDirNotAllowed gate refusal (identity/resolver/errors.rs).
        let frame = json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": { "message": "[AgentMux] this agent's claude identity points directly at your personal claude config directory (C:\\Users\\asafe\\.claude) instead of an isolated AgentMux account — AgentMux no longer allows spawning an agent against your own global CLI login. Re-bind this identity to an isolated account in Armory \u{2192} Accounts (delete the current claude account and log in again to create a fresh, isolated one), then retry." }
        });
        let f = classify(Some(1), None, "", Some(&frame));
        assert_eq!(f.code, FailureClass::Auth);
        assert!(!f.retryable, "a bare retry can never succeed against this gate");
        assert!(f.detail.contains("Claude"), "detail: {}", f.detail);
        assert!(f.title.to_lowercase().contains("identity"), "title: {}", f.title);
    }

    #[test]
    fn spawn_gate_ambient_home_dir_names_the_actual_provider_not_claude() {
        let frame = json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": { "message": "[AgentMux] this agent's codex identity points directly at your personal codex config directory (/home/user/.codex) instead of an isolated AgentMux account — AgentMux no longer allows spawning an agent against your own global CLI login. Re-bind this identity to an isolated account in Armory \u{2192} Accounts (delete the current codex account and log in again to create a fresh, isolated one), then retry." }
        });
        let f = classify(Some(1), None, "", Some(&frame));
        assert_eq!(f.code, FailureClass::Auth);
        assert!(f.detail.contains("Codex"), "detail: {}", f.detail);
        assert!(!f.detail.contains("Claude"), "detail: {}", f.detail);
    }

    #[test]
    fn extract_ambient_home_provider_reads_the_provider_out_of_the_gate_wording() {
        assert_eq!(
            extract_ambient_home_provider(
                "this agent's gemini identity points directly at your personal gemini config directory (/x)."
            ),
            Some("gemini".to_string()),
        );
    }

    #[test]
    fn extract_ambient_home_provider_returns_none_on_wording_mismatch() {
        assert_eq!(extract_ambient_home_provider("some unrelated error text"), None);
    }

    #[test]
    fn capitalize_provider_title_cases_the_raw_id() {
        assert_eq!(capitalize_provider("claude"), "Claude");
        assert_eq!(capitalize_provider("gemini"), "Gemini");
        assert_eq!(capitalize_provider(""), "");
    }

    #[test]
    fn spawn_gate_injection_unavailable_is_not_auth() {
        // The gate's OTHER error variant — a task-join/panic failure, not a
        // credentials problem — must not be swept into the same match.
        let frame = json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": { "message": "[AgentMux] credential injection could not run (panic); the spawn was refused rather than falling back to the global CLI login. Retry, and check `muxlog auth` if it persists." }
        });
        let f = classify(Some(1), None, "", Some(&frame));
        assert_eq!(f.code, FailureClass::UnknownNonZero);
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
    fn extract_spawn_gate_provider_reads_the_provider_out_of_the_gate_wording() {
        assert_eq!(
            extract_spawn_gate_provider(
                "no credentials for codex: the bound account was deleted or is unresolvable."
            ),
            Some("codex".to_string()),
        );
    }

    #[test]
    fn extract_spawn_gate_provider_returns_none_on_wording_mismatch() {
        assert_eq!(extract_spawn_gate_provider("some unrelated error text"), None);
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
