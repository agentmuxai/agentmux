// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `agentmux_session_outcome` frame a persistent controller writes when a
//! resume's fate is known.

use super::persistent_resume;

/// Builds the NDJSON line for a `persistent_resume::ResumeEffect::
/// EmitSessionOutcome` — a free function (not a method) so it's callable
/// directly from the stdout-reader, process-waiter, and stop-path match
/// arms below, which only hold `_read`/`_wait`-suffixed clones, not
/// `&self`. See SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.1.
pub(super) fn session_outcome_line(
    outcome: persistent_resume::SessionOutcome,
    attempted_sid: String,
    actual_sid: Option<String>,
) -> String {
    session_outcome_line_with(outcome, attempted_sid, actual_sid, false, false)
}

/// [`session_outcome_line`], plus whether the fresh session was given
/// AgentMux's record of the conversation (`continued`, SPEC_DURABLE_
/// CONVERSATION_MEMORY_2026_09_23.md §4.8). The outcome stays `fresh`: the
/// provider session is new, and every consumer that scopes scrollback on
/// `fresh` keeps doing so. Only the pane's label changes.
///
/// `idle_start`: no message sits behind the spawn (an eager spawn, a resume
/// the gate refused), so nobody is talking to the agent yet and the pane
/// should have it catch up. False when a message started the session. The
/// pane cannot tell these apart from its own turn state, which can be stale.
pub(super) fn session_outcome_line_with(
    outcome: persistent_resume::SessionOutcome,
    attempted_sid: String,
    actual_sid: Option<String>,
    continued: bool,
    idle_start: bool,
) -> String {
    let outcome_str = match outcome {
        persistent_resume::SessionOutcome::Resumed => "resumed",
        persistent_resume::SessionOutcome::Fresh => "fresh",
    };
    format!(
        "{}\n",
        serde_json::json!({
            "type": "system",
            "subtype": "agentmux_session_outcome",
            "outcome": outcome_str,
            "attempted_sid": attempted_sid,
            "actual_sid": actual_sid,
            "continued": continued,
            "idle_start": idle_start,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })
    )
}
