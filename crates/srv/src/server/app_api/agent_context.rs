// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents context` and `Agents compact`
//! (`docs/specs/SPEC_AGENT_SELF_COMPACTION_2026_10_23.md` §3.3): an agent reads
//! how full its own context is, and compacts itself when its turn ends.
//!
//! Both act on the caller's own pane only, resolved from its agent token's UID
//! (`actor::own_pane_by_token`), never from a name: there is no target.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::backend::context_reading;
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The longest a self-compaction waits for the caller's turn to end.
const TURN_END_WAIT: Duration = Duration::from_secs(30 * 60);
const TURN_END_POLL: Duration = Duration::from_millis(250);
/// The most focus text sent with `/compact`.
pub(crate) const FOCUS_MAX_CHARS: usize = 2_000;

#[derive(Debug, Deserialize)]
pub(crate) struct SelfContextRequest {
    /// The caller's slug, which its token must name.
    agent: String,
    /// `compact`: what to keep, for the summary.
    #[serde(default)]
    focus: Option<String>,
}

fn no_pane() -> Response {
    let error = "no pane for this agent token: Agents context and compact act only on the calling agent's own pane";
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": error }))).into_response()
}

/// `claude` or `codex` for a pane that can compact, with whether it runs in a container.
fn provider_of(state: &AppState, block_id: &str) -> Option<(String, bool)> {
    let block = state.mstore.get::<crate::backend::obj::Block>(block_id).ok()??;
    let raw = crate::backend::obj::meta_get_string(&block.meta, "agentProvider", "");
    let provider = crate::backend::providers::resolve_provider_alias(&raw).to_string();
    let container = crate::backend::runtime_adapter::RuntimeKind::of_block_meta(&block.meta).is_container();
    Some((provider, container))
}

/// `POST /api/v1/agent/self/context`
pub(crate) async fn handle_context_self(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<SelfContextRequest>,
) -> Response {
    let Some(block_id) = crate::server::actor::own_pane_by_token(&state.mstore, caller.as_deref(), req.agent.trim()) else {
        return no_pane();
    };
    let provider = provider_of(&state, &block_id).map(|(p, _)| p);
    let reading = context_reading::get(&block_id).unwrap_or_default();
    let why_none = reading.tokens.is_none().then_some(
        "no reading yet: srv reads it from the agent's calls (Claude) or after each turn (Codex)",
    );
    Json(json!({
        "provider": provider,
        "model": reading.model,
        "tokens": reading.tokens,
        "window": reading.window,
        "auto_compact_at": reading.auto_compact_at,
        "compacting": reading.compacting,
        "calls_since_compaction": reading.calls_since_compaction,
        "last_compaction_ms": reading.last_compaction_ms,
        "hint": context_reading::hint(&block_id),
        "note": why_none,
    }))
    .into_response()
}

/// The message that compacts a `provider` agent, or why it can't be: a Claude agent takes `/compact`
/// (with its focus) as a turn wherever it runs, and a container's turn is that same message on its
/// per-turn exec controller.
fn compact_request(provider: &str, container: bool, focus: Option<&str>) -> Result<String, &'static str> {
    match (provider, container) {
        ("claude", _) => Ok(compact_message(focus)),
        // Codex compacts through a short-lived App Server on the host (`codex_compact`); a
        // container's turn would take "/compact" as a prompt.
        ("codex", true) => Err("unsupported: a Codex agent in a container can't compact yet"),
        ("codex", false) => Ok("/compact".to_string()),
        _ if speaks_acp(provider) => Err("unsupported: this harness has no compaction AgentMux can start; call Agents fresh with a handoff to start a new session instead"),
        _ => Err("unsupported: only Claude and Codex agents can compact"),
    }
}

/// Whether `provider` is an Agent Client Protocol harness (dsh, Gemini, Copilot, ...): every one can open a new session.
fn speaks_acp(provider: &str) -> bool {
    crate::backend::providers::get_provider(provider).is_some_and(|p| p.controller_type_str() == "acp")
}

/// Whether an agent of `provider` can start a fresh session, and why not. A container's Claude or Codex agent runs one
/// `exec` per turn on the same controller a host Codex agent does, so ending its session is forgetting its id, as
/// there (`blockcontroller/subprocess/codex_fresh.rs`); the harnesses that speak ACP have no such controller in a container.
fn fresh_supported(provider: &str, container: bool) -> Result<(), &'static str> {
    match (provider, container) {
        ("claude" | "codex", _) => Ok(()),
        (_, true) => Err("unsupported in containers: only Claude and Codex agents can start fresh there"),
        _ if speaks_acp(provider) => Ok(()),
        _ => Err("unsupported: only Claude, Codex and ACP agents can start fresh"),
    }
}

/// `POST /api/v1/agent/self/compact`: `scheduled` (runs when the caller's turn
/// ends) or `refused` with why.
pub(crate) async fn handle_compact_self(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<SelfContextRequest>,
) -> Response {
    let Some(block_id) = crate::server::actor::own_pane_by_token(&state.mstore, caller.as_deref(), req.agent.trim()) else {
        return no_pane();
    };
    let refused = |why: &str| Json(json!({ "status": "refused", "reason": why })).into_response();
    let Some((provider, container)) = provider_of(&state, &block_id) else {
        return refused("the pane can't be read");
    };
    let message = match compact_request(&provider, container, req.focus.as_deref()) {
        Ok(message) => message,
        Err(why) => return refused(why),
    };
    if let Err(why) = context_reading::schedule_compact(&block_id, agentmux_common::time::now_ms_u64()) {
        return refused(why);
    }
    let focus_note = (provider == "codex" && req.focus.as_deref().is_some_and(|f| !f.trim().is_empty()))
        .then_some("Codex takes no focus: it compacts with its own prompt");
    tracing::info!(block_id = %block_id, provider = %provider, tokens = context_reading::tokens_now(&block_id), focus_chars = req.focus.as_deref().map_or(0, str::len), "self-compaction scheduled");
    tokio::spawn(compact_when_turn_ends(state, block_id, message, container));
    Json(json!({ "status": "scheduled", "runs": "when your current turn ends", "note": focus_note })).into_response()
}

/// The most handoff text a fresh session carries.
pub(crate) const HANDOFF_MAX_CHARS: usize = 4_000;
/// The longest a fresh session waits for the old CLI to exit.
const EXIT_WAIT: Duration = Duration::from_secs(30);

/// `POST /api/v1/agent/self/fresh`: `scheduled` (when the caller's turn ends,
/// its session ends and a new one starts with the handoff) or `refused`.
/// `focus` carries the handoff here.
pub(crate) async fn handle_fresh_self(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<SelfContextRequest>,
) -> Response {
    let Some(block_id) = crate::server::actor::own_pane_by_token(&state.mstore, caller.as_deref(), req.agent.trim()) else {
        return no_pane();
    };
    let refused = |why: &str| Json(json!({ "status": "refused", "reason": why })).into_response();
    let handoff = req.focus.as_deref().unwrap_or_default().trim().to_string();
    if handoff.is_empty() {
        return refused("a handoff is required: what your next session must know");
    }
    if handoff.chars().count() > HANDOFF_MAX_CHARS {
        return refused("the handoff is over 4,000 characters: shorten it, with the next step in it");
    }
    let Some((provider, container)) = provider_of(&state, &block_id) else {
        return refused("the pane can't be read");
    };
    if let Err(why) = fresh_supported(&provider, container) {
        return refused(why);
    }
    if let Err(why) = context_reading::schedule_compact(&block_id, agentmux_common::time::now_ms_u64()) {
        return refused(why);
    }
    tracing::info!(block_id = %block_id, provider = %provider, handoff_chars = handoff.len(), "fresh session scheduled");
    tokio::spawn(fresh_when_turn_ends(state, block_id, handoff_message(&handoff)));
    Json(json!({ "status": "scheduled", "runs": "when your current turn ends" })).into_response()
}

/// The new session's first message from AgentMux: the agent's own handoff and
/// a pointer to the old session. Nothing else of the conversation comes with it.
pub(crate) fn handoff_message(handoff: &str) -> String {
    format!(
        "[AgentMux] This is the fresh session you asked for. Nothing of the previous conversation is in your \
         context; it is in your record (Agents search_history finds it). Your handoff:\n\n{handoff}"
    )
}

/// The first message of the fresh session a model change starts (`agent.model.switch` and `Agents update` with
/// `fresh`): where the agent was moved to, that the earlier conversation is in its record and not in its context, and
/// what the caller said its next session must know (none: where to look for what it was doing).
pub(crate) fn switch_message(place: &str, handoff: &str) -> String {
    let said = match handoff.trim() {
        "" => "Read your task list (Tasks list) and your last status to see what you were doing, then carry on.",
        h => h,
    };
    format!(
        "[AgentMux] You were moved to {place} and this is a fresh session: nothing of the earlier conversation is in your \
         context, because a model that did not write a long conversation can read it badly. It is in your record \
         (Agents search_history finds it).\n\n{said}"
    )
}

/// Why the pane of a model change can't start a fresh session, or nothing: the test `Agents fresh` applies to its own pane.
pub(crate) fn fresh_refusal(state: &AppState, block_id: &str) -> Result<(), &'static str> {
    let (provider, container) = provider_of(state, block_id).ok_or("the pane can't be read")?;
    fresh_supported(&provider, container)
}

/// Start `block_id`'s fresh session after a model change: when its turn ends its session ends, and `first_message`
/// starts the new one on the model the agent is set to now. `false` when there is no async runtime to run it on.
pub(crate) fn start_fresh_after_switch(state: &AppState, block_id: &str, first_message: String) -> bool {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return false;
    };
    runtime.spawn(fresh_when_turn_ends(state.clone(), block_id.to_string(), first_message));
    true
}

/// Tries a fresh session makes when a message queued behind the turn starts
/// the next one before the old session can be ended.
const FRESH_ATTEMPTS: usize = 5;
/// Held for the agent's next turn when its fresh session couldn't start.
const FRESH_FAILED_NOTE_KEY: &str = "fresh-failed";
/// What `start_fresh_session` says when the turn hasn't ended, or the CLI is
/// already being stopped: the two it is worth trying again after.
const TURN_RUNNING: &str = "a turn is running";
const STOP_PENDING: &str = "the agent's process is already stopping or restarting";

/// End `block_id`'s provider session, so the next message starts a new one.
fn end_session(block_id: &str) -> Result<(), &'static str> {
    use crate::backend::blockcontroller::acp::AcpController;
    use crate::backend::blockcontroller::persistent::PersistentSubprocessController;
    use crate::backend::blockcontroller::subprocess::SubprocessController;
    let ctrl = crate::backend::blockcontroller::get_controller(block_id).ok_or("no controller for the pane")?;
    // An ACP pane: the process ends and its saved session id is forgotten, so the handoff opens a new session.
    if let Some(acp) = ctrl.as_any().downcast_ref::<AcpController>() {
        return acp.start_fresh_session().map(|_| ());
    }
    // A Codex pane: one `exec` per turn, so ending the thread is forgetting its id.
    if let Some(codex) = ctrl.as_any().downcast_ref::<SubprocessController>() {
        return codex.start_fresh_session().map(|_| ());
    }
    let p = ctrl.as_any().downcast_ref::<PersistentSubprocessController>().ok_or("the agent's controller can't start fresh")?;
    context_reading::mark_fresh_by_choice(block_id);
    p.start_fresh_session().map(|_| ()).inspect_err(|_| {
        context_reading::take_fresh_by_choice(block_id);
    })
}

/// Wait for the turn to end, end the session, then send `first_message` as
/// AgentMux's own turn, which starts the new session.
async fn fresh_when_turn_ends(state: AppState, block_id: String, first_message: String) {
    let mut ended = Err("the turn didn't end within 30 minutes");
    // A message queued behind the turn can start the next one between the wait
    // and the stop: wait again, a few times, before giving up.
    for _ in 0..FRESH_ATTEMPTS {
        if !wait_for_turn_end(&block_id).await {
            break;
        }
        ended = end_session(&block_id);
        if !matches!(ended, Err(TURN_RUNNING | STOP_PENDING)) {
            break;
        }
        tokio::time::sleep(TURN_END_POLL).await;
    }
    if let Err(why) = ended {
        tracing::warn!(block_id = %block_id, reason = why, "fresh session given up");
        context_reading::fresh_sent(&block_id, false);
        crate::backend::reactive::held_notes::hold(
            &block_id,
            FRESH_FAILED_NOTE_KEY,
            &format!("[AgentMux] Your fresh session did not start ({why}); this session continues. Call Agents fresh again when you want it."),
        );
        return;
    }
    // The old CLI exits before the handoff spawns the new one.
    let deadline = tokio::time::Instant::now() + EXIT_WAIT;
    while crate::backend::blockcontroller::get_block_controller_status(&block_id)
        .is_some_and(|s| s.shellprocstatus == crate::backend::blockcontroller::STATUS_RUNNING)
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(TURN_END_POLL).await;
    }
    let deps = crate::server::agent_handlers::AgentTurnDeps::from_state(&state);
    let outcome = crate::server::agent_handlers::run_agent_turn(
        &deps,
        block_id.clone(),
        first_message,
        None,
        crate::server::agent_handlers::TurnRegistration::Register,
        crate::backend::message_origin::MessageOrigin::system(),
        Vec::new(),
    )
    .await;
    context_reading::fresh_sent(&block_id, outcome.is_ok());
    match outcome {
        Ok(()) => tracing::info!(block_id = %block_id, "fresh session started"),
        Err(e) => tracing::warn!(block_id = %block_id, error = %e, "fresh session's handoff could not be sent"),
    }
}

/// Wait until `block_id` has no turn running; `false` after [`TURN_END_WAIT`].
async fn wait_for_turn_end(block_id: &str) -> bool {
    let deadline = tokio::time::Instant::now() + TURN_END_WAIT;
    while crate::backend::blockcontroller::get_block_controller_status(block_id).is_some_and(|s| s.turn_active) {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(TURN_END_POLL).await;
    }
    true
}

/// `/compact` with the focus, trimmed to [`FOCUS_MAX_CHARS`] and to one
/// paragraph's worth of whitespace.
pub(crate) fn compact_message(focus: Option<&str>) -> String {
    let focus: String = focus.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(FOCUS_MAX_CHARS).collect();
    if focus.is_empty() { "/compact".to_string() } else { format!("/compact {focus}") }
}

/// srv's own compaction of a route agent whose context is far over its budget: the compaction `Agents compact` schedules,
/// without a focus (the harness's summarizer decides what to keep). `Err(why)` when it can't be sent now.
pub(crate) fn compact_for_backstop(state: &AppState, block_id: &str) -> Result<(), &'static str> {
    let (provider, container) = provider_of(state, block_id).ok_or("the pane can't be read")?;
    let message = compact_request(&provider, container, None)?;
    context_reading::schedule_compact(block_id, agentmux_common::time::now_ms_u64())?;
    tokio::runtime::Handle::current().spawn(compact_when_turn_ends(state.clone(), block_id.to_string(), message, container));
    Ok(())
}

/// Send `message` as AgentMux's own turn once `block_id`'s turn has ended. A container agent's turn is not waited for: it
/// runs one turn at a time from a queue, and the compaction is placed at the head of that queue, so it starts the moment
/// the turn in flight ends and ahead of the messages already waiting. Waiting for a moment with no turn running could
/// take as long as the queue stays full, and the compaction would then go behind it (the queue is never empty for a busy
/// agent: the next waiting message starts the moment a turn ends).
async fn compact_when_turn_ends(state: AppState, block_id: String, message: String, container: bool) {
    if !container && !wait_for_turn_end(&block_id).await {
        tracing::warn!(block_id = %block_id, "self-compaction given up: the turn didn't end within 30 minutes");
        context_reading::compact_sent(&block_id, false);
        return;
    }
    let deps = crate::server::agent_handlers::AgentTurnDeps::from_state(&state);
    let outcome = crate::server::agent_handlers::run_agent_turn(
        &deps,
        block_id.clone(),
        message,
        None,
        crate::server::agent_handlers::TurnRegistration::Register,
        crate::backend::message_origin::MessageOrigin::system(),
        Vec::new(),
    )
    .await;
    context_reading::compact_sent(&block_id, outcome.is_ok());
    match outcome {
        Ok(()) => tracing::info!(block_id = %block_id, tokens = context_reading::tokens_now(&block_id), "self-compaction sent"),
        Err(e) => tracing::warn!(block_id = %block_id, error = %e, "self-compaction could not be sent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handoff_is_the_new_sessions_first_message_from_agentmux() {
        let m = handoff_message("Task #1791: PR 3 open as #5410; next: wait for review.");
        assert!(m.starts_with("[AgentMux] This is the fresh session you asked for."));
        assert!(m.contains("search_history"), "it points at the old session");
        assert!(m.ends_with("Task #1791: PR 3 open as #5410; next: wait for review."));
    }

    #[test]
    fn a_claude_agent_compacts_in_a_container_as_on_the_host() {
        assert_eq!(compact_request("claude", true, None), Ok("/compact".to_string()));
        assert_eq!(compact_request("claude", true, Some("keep: PR #1")), compact_request("claude", false, Some("keep: PR #1")));
        assert_eq!(compact_request("codex", false, None), Ok("/compact".to_string()));
        assert!(compact_request("codex", true, None).is_err_and(|why| why.contains("container")));
        assert!(compact_request("copilot", false, None).is_err_and(|why| why.contains("Agents fresh")), "an ACP harness is told what to do instead");
        assert!(compact_request("dsh", false, None).is_err_and(|why| why.contains("no compaction AgentMux can start")));
        assert!(compact_request("nonesuch", false, None).is_err_and(|why| why.contains("only Claude and Codex")));
    }

    #[test]
    fn fresh_is_offered_to_claude_codex_and_every_acp_harness_and_in_a_container_to_claude_and_codex() {
        for provider in ["claude", "codex", "dsh", "copilot"] {
            assert_eq!(fresh_supported(provider, false), Ok(()), "{provider}");
        }
        for provider in ["claude", "codex"] {
            assert_eq!(fresh_supported(provider, true), Ok(()), "a container's {provider} agent starts fresh as a host's does");
        }
        for provider in ["dsh", "copilot"] {
            assert!(fresh_supported(provider, true).is_err_and(|why| why.contains("containers") && why.contains("Claude and Codex")), "{provider}");
        }
        assert!(fresh_supported("nonesuch", false).is_err_and(|why| why.contains("ACP")));
    }

    #[test]
    fn the_focus_rides_after_compact_on_one_line_within_its_limit() {
        assert_eq!(compact_message(None), "/compact");
        assert_eq!(compact_message(Some("   ")), "/compact");
        assert_eq!(compact_message(Some("keep: PR #5401\n\nthe failing test")), "/compact keep: PR #5401 the failing test");
        let long = "x".repeat(FOCUS_MAX_CHARS + 50);
        assert_eq!(compact_message(Some(&long)).chars().count(), "/compact ".len() + FOCUS_MAX_CHARS);
    }
}
