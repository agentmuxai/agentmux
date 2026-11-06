// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The rules `classify` tries, in order, to name a failure. A rule answers `Some` failure when the
//! evidence is its cause, `None` when it is not.

use super::*;

/// What a classification rests on: the run's exit evidence and the text it left.
pub(super) struct Evidence<'a> {
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<i32>,
    pub(super) result_frame: Option<&'a Value>,
    pub(super) frame_reported_error: bool,
    /// The stderr, and the terminal frame's error text after it.
    pub(super) combined: String,
    /// `combined`, lower-cased once for the keyword matches.
    pub(super) hay: String,
    /// The last lines of `combined`, kept as the failure's evidence.
    pub(super) tail: String,
}

impl Evidence<'_> {
    /// A failure of this class, carrying the exit evidence and the stderr tail.
    fn failure(&self, code: FailureClass, title: &str, detail: &str, retryable: bool) -> AgentFailure {
        build(code, title, detail, retryable, self.exit_code, self.signal, &self.tail)
    }
}

type Rule = fn(&Evidence<'_>) -> Option<AgentFailure>;

/// The rules, most specific first. Order matters: rate-limit and overload are checked
/// before usage-limit because the server-side rate-limit message
/// literally contains "not your usage limit".
pub(super) const RULES: [Rule; 15] = [
    turn_cap_frame,
    killed,
    rate_limited,
    overloaded,
    missing_account,
    open_in_other_pane,
    live_elsewhere,
    agent_deleted,
    ambient_home,
    unprotected_config,
    not_authenticated,
    context_exceeded,
    turn_cap_text,
    usage_limit,
    network,
];

/// The terminal frame says the turn cap was reached.
fn turn_cap_frame(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // Explicit terminal-frame subtype: turn cap reached.
    if ev.result_frame
        .and_then(|f| f.get("subtype"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| s.contains("max_turns"))
    {
        return Some(ev.failure(
            FailureClass::MaxTurns,
            "Hit the turn limit",
            "The agent reached its configured `--max-turns` cap before finishing. Raise the cap or split the task.",
            false,
        ));
    }
    None
}

/// Killed by a signal (the OOM killer or an external stop).
fn killed(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // Killed by a signal (OOM killer or external stop). 137 == 128 + SIGKILL(9).
    if ev.signal.is_some() || ev.exit_code == Some(137) {
        return Some(ev.failure(
            FailureClass::Killed,
            "Agent process was killed",
            "Terminated by a signal — most often the OS out-of-memory killer or an external stop. Reduce concurrent load / memory pressure and retry.",
            false,
        ));
    }
    None
}

/// Server-side rate limit: transient, and not the account quota.
fn rate_limited(ev: &Evidence<'_>) -> Option<AgentFailure> {
    if ev.hay.contains("rate limited")
        || ev.hay.contains("temporarily limiting requests")
        || ev.hay.contains("rate_limit")
        || mentions_http_status(&ev.hay, "429")
    {
        return Some(ev.failure(
            FailureClass::RateLimited,
            "Rate-limited by the API",
            "Server-side rate limit — transient, and not your account quota. Retry shortly; consider lowering parallelism.",
            true,
        ));
    }
    None
}

/// The API is overloaded (HTTP 529).
fn overloaded(ev: &Evidence<'_>) -> Option<AgentFailure> {
    if ev.hay.contains("overloaded") || mentions_http_status(&ev.hay, "529") {
        return Some(ev.failure(
            FailureClass::Overloaded,
            "API temporarily overloaded",
            "The API reported it is overloaded (HTTP 529). Transient — retry with backoff.",
            true,
        ));
    }
    None
}

/// The identity spawn gate refused: no account is bound.
fn missing_account(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // The identity spawn gate's own refusal (identity/resolver/errors.rs's
    // `SpawnGateError::MissingCredentials` Display impl — a deliberate,
    // spec-owned wording per SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md
    // §2.2, not third-party CLI/API output, so matching on it is as stable
    // as matching our own code). This never reaches the API at all — the
    // spawn was refused before any CLI process existed to report a 401 —
    // so it's checked separately from the `not_authenticated` rule with its own
    // wording, but classifies the same way: no account bound is exactly as
    // actionable as a rejected credential, and needs the same relogin UI.
    // Before this, the message had no recognized keyword and fell through
    // to `UnknownNonZero`, which offers only "Retry" — a retry can never
    // succeed against a gate that blocks every respawn identically, so the
    // agent pane got stuck showing a dead-end error with no way out.
    // "in the armory" is the wording before the Armory became Connectors
    // (SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md §4.4);
    // a failure recorded then still has to classify.
    if ev.hay.contains("bind an account for this provider in connectors")
        || ev.hay.contains("bind an account for this provider in the armory")
    {
        // codex P2 on PR #2413: this branch matches EVERY oauth-class
        // provider's MissingCredentials refusal (Codex, Gemini, OpenClaw,
        // Copilot — not just Claude), but a prior version of this message
        // hardcoded "No Claude account is bound". Extract the provider id
        // from the gate's own wording ("no credentials for {provider}: …")
        // instead of assuming — falls back to generic phrasing if the
        // extraction ever misses (wording drift in errors.rs), rather than
        // asserting a specific, possibly wrong, provider name.
        let provider_phrase = extract_spawn_gate_provider(&ev.combined)
            .map(|p| format!("No {} account", capitalize_provider(&p)))
            .unwrap_or_else(|| "No account".to_string());
        return Some(ev.failure(
            FailureClass::Auth,
            "No account linked",
            &format!(
                "{provider_phrase} is bound to this agent — the spawn was refused before it could run. \
                 Sign in to link one."
            ),
            false,
        ));
    }
    None
}

/// Another pane of this AgentMux holds the agent.
fn open_in_other_pane(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // Another pane of this AgentMux holds the agent (`held_elsewhere_error`).
    // Checked before the instance refusal: both can name "another" holder.
    if let Some(holder) = open_in_other_pane_holder(&ev.combined) {
        let mut f = ev.failure(
            FailureClass::OpenInOtherPane,
            "Open in another pane",
            "This agent is already open in another pane. Go there to keep working in it, or move it here: the other pane stops and this one resumes the same conversation.",
            false,
        );
        f.holder_block_id = Some(holder);
        return Some(f);
    }
    None
}

/// Another AgentMux instance holds the agent.
fn live_elsewhere(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // A single-live-instance refusal (backend::agent_admission — our own
    // wording, recognised by the same markers that module uses). Checked
    // before the generic classes: its text is not provider output, and a
    // Retry cannot succeed while the other instance holds the agent.
    if crate::backend::agent_admission::is_admission_refusal(&ev.combined) {
        let taken_over = ev.hay.contains("was taken over by another agentmux instance");
        return Some(ev.failure(
            FailureClass::LiveElsewhere,
            if taken_over { "Taken over by another AgentMux instance" } else { "Running in another AgentMux instance" },
            frame_or_stderr_line(&ev.combined),
            false,
        ));
    }
    None
}

/// The identity spawn gate refused: the pane's agent was deleted.
fn agent_deleted(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // The identity spawn gate's deleted-agent refusal (identity/resolver/
    // errors.rs's `SpawnGateError::AgentDeleted` Display — our own wording).
    // Its own class, never Retry: the gate refuses every respawn the same
    // way, so a Retry would be the same dead end as the `missing_account` rule.
    if ev.hay.contains("was deleted, so this pane cannot start") {
        return Some(ev.failure(
            FailureClass::AgentDeleted,
            "Agent was deleted",
            "This pane's agent was deleted, so it cannot start. Close the pane, \
             or go back to the picker and launch another agent in it.",
            false,
        ));
    }
    None
}

/// The identity spawn gate refused: the identity points at the personal login directory.
fn ambient_home(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // The identity spawn gate's ambient-home-dir refusal (identity/resolver/
    // errors.rs's `SpawnGateError::AmbientHomeDirNotAllowed` Display impl —
    // same reasoning as the `missing_account` rule: this never
    // reaches the API, and a bare "Retry" would be a dead end since the
    // gate blocks every respawn identically until the identity is rebound.
    if ev.hay.contains("instead of an isolated agentmux account") {
        let provider_phrase = extract_ambient_home_provider(&ev.combined)
            .map(|p| format!("This agent's {} identity", capitalize_provider(&p)))
            .unwrap_or_else(|| "This agent's identity".to_string());
        return Some(ev.failure(
            FailureClass::Auth,
            "Identity points at your personal login",
            &format!(
                "{provider_phrase} is bound directly to your personal CLI login \
                 directory, which AgentMux no longer allows. Re-bind it to an \
                 isolated account in Connectors \u{2192} Accounts, then retry."
            ),
            false,
        ));
    }
    None
}

/// The identity spawn gate refused: the config directory could not be isolated.
fn unprotected_config(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // The identity spawn gate's CLAUDE.md isolation-seed refusal
    // (identity/resolver/errors.rs's `SpawnGateError::ClaudeMdSeedFailed`
    // Display impl — same reasoning as the `missing_account` and `ambient_home` rules: this never
    // reaches the API, so it needs its own title/detail instead of falling
    // through to the generic UnknownNonZero fallback. ReAgent P2 on PR
    // #2854 — added alongside the `ambient_home` rule but
    // initially missed for this newer variant.
    if ev.hay.contains("refusing to spawn with an unprotected config dir") {
        let provider_phrase = extract_claude_md_seed_provider(&ev.combined)
            .map(|p| format!("This agent's {} config directory", capitalize_provider(&p)))
            .unwrap_or_else(|| "This agent's config directory".to_string());
        return Some(ev.failure(
            FailureClass::Auth,
            "Could not isolate this agent's config",
            &format!(
                "{provider_phrase} could not be prepared for isolated use — the spawn was \
                 refused rather than risk it silently reading your personal CLI config. \
                 Retry, and check the directory's permissions if it persists."
            ),
            false,
        ));
    }
    None
}

/// The API rejected the credentials.
fn not_authenticated(ev: &Evidence<'_>) -> Option<AgentFailure> {
    if ev.hay.contains("authentication_error")
        || ev.hay.contains("authentication_failed")
        || ev.hay.contains("invalid authentication")
        || ev.hay.contains("invalid api key")
        || ev.hay.contains("invalid x-api-key")
        || ev.hay.contains("unauthorized")
        || ev.hay.contains("please run /login")
        // A login whose session ended: "Failed to authenticate: OAuth session expired and could not be
        // refreshed", also as the cause of "Prompt is too long · automatic compaction failed: …".
        || ev.hay.contains("failed to authenticate")
        || ev.hay.contains("oauth session expired")
        || mentions_http_status(&ev.hay, "401")
    {
        return Some(ev.failure(
            FailureClass::Auth,
            "Not authenticated",
            "The API rejected the credentials (HTTP 401). Re-authenticate via the Identity tab or `claude /login`.",
            false,
        ));
    }
    None
}

/// The conversation outgrew the context window.
fn context_exceeded(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // "Prompt is too long · automatic compaction failed: You've hit your weekly limit": the CLI
    // couldn't compact because the account is out of quota, so the limit is the cause, and
    // a move to an account with quota (failover) is what fixes it. It falls through to the `usage_limit` rule.
    if !names_plan_limit(&ev.hay)
        && (ev.hay.contains("model_context_window_exceeded")
            || ev.hay.contains("prompt is too long")
            || (ev.hay.contains("context") && (ev.hay.contains("exceed") || ev.hay.contains("window") || ev.hay.contains("too long"))))
    {
        return Some(ev.failure(
            FailureClass::ContextExceeded,
            "Context window exceeded",
            "The conversation exceeded the model's context window. Clear or compact the agent's history and retry.",
            false,
        ));
    }
    None
}

/// The turn cap, named in the text.
fn turn_cap_text(ev: &Evidence<'_>) -> Option<AgentFailure> {
    if ev.hay.contains("max turns") || ev.hay.contains("max-turns") || ev.hay.contains("maximum number of turns") {
        return Some(ev.failure(
            FailureClass::MaxTurns,
            "Hit the turn limit",
            "The agent reached its configured `--max-turns` cap before finishing. Raise the cap or split the task.",
            false,
        ));
    }
    None
}

/// The plan or usage limit is reached.
fn usage_limit(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // Claude's plan limits read "You've hit your weekly limit · resets …" (or
    // "session limit", or "monthly spend limit · raise it at …"), with no
    // "usage" in them.
    if names_plan_limit(&ev.hay)
        || ev.hay.contains("quota")
        || ev.hay.contains("billing")
        || ev.hay.contains("insufficient")
    {
        return Some(ev.failure(
            FailureClass::UsageLimit,
            "Usage limit reached",
            "Your plan or usage limit appears to be reached. Check plan / billing — retrying won't help until it resets.",
            false,
        ));
    }
    None
}

/// A network error reached the API.
fn network(ev: &Evidence<'_>) -> Option<AgentFailure> {
    // Claude Code ends a turn whose retries ran out with "API Error:
    // Connection refused — … (ECONNREFUSED)" when the network is down,
    // or "Unable to connect to API (ConnectionRefused)" when nothing listens.
    if ev.hay.contains("apiconnectionerror")
        || ev.hay.contains("unable to connect to api")
        || ev.hay.contains("connection error")
        || ev.hay.contains("connection refused")
        || ev.hay.contains("could not resolve")
        || ev.hay.contains("econnreset")
        || ev.hay.contains("econnrefused")
        || ev.hay.contains("enotfound")
        || ev.hay.contains("enetunreach")
        || ev.hay.contains("ehostunreach")
        || ev.hay.contains("etimedout")
        || ev.hay.contains("network")
        || ev.hay.contains("timed out")
        || ev.hay.contains("timeout")
        || ev.hay.contains("dns")
    {
        return Some(ev.failure(
            FailureClass::Network,
            "Network error reaching the API",
            "A network/connection error occurred talking to the API. Check connectivity and retry.",
            true,
        ));
    }
    None
}

/// No rule named a cause: say what is known.
pub(super) fn fallback(ev: &Evidence<'_>) -> AgentFailure {
    // Fallbacks.
    if ev.frame_reported_error {
        return ev.failure(
            FailureClass::UnknownNonZero,
            "Agent reported an error",
            "The agent reported a terminal error with no recognized cause. The raw output tail is included below.",
            false,
        );
    }
    if ev.exit_code == Some(0) {
        return ev.failure(
            FailureClass::NoOutput,
            "Agent produced no output",
            "The agent exited cleanly (code 0) but never produced a final result. Inspect the transcript / sidecar log.",
            false,
        );
    }
    ev.failure(
        FailureClass::UnknownNonZero,
        "Agent failed",
        "The agent exited with a non-zero status and no recognized error. The raw stderr tail is included below.",
        false,
    )
}
