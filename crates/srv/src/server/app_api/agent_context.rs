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
    let container = crate::backend::obj::meta_get_string(&block.meta, "agentMode", "host") == "container";
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
    let message = match (provider.as_str(), container) {
        // A container pane runs on the per-turn docker-exec controller, which
        // srv's context reading doesn't follow.
        (_, true) => return refused("unsupported in containers"),
        ("claude", false) => compact_message(req.focus.as_deref()),
        ("codex", false) => "/compact".to_string(),
        _ => return refused("unsupported: only Claude and Codex agents can compact"),
    };
    if let Err(why) = context_reading::schedule_compact(&block_id, agentmux_common::time::now_ms_u64()) {
        return refused(why);
    }
    let focus_note = (provider == "codex" && req.focus.as_deref().is_some_and(|f| !f.trim().is_empty()))
        .then_some("Codex takes no focus: it compacts with its own prompt");
    tracing::info!(block_id = %block_id, provider = %provider, focus_chars = req.focus.as_deref().map_or(0, str::len), "self-compaction scheduled");
    tokio::spawn(compact_when_turn_ends(state, block_id, message));
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
    let handoff: String = req.focus.as_deref().unwrap_or_default().trim().chars().take(HANDOFF_MAX_CHARS).collect();
    if handoff.is_empty() {
        return refused("a handoff is required: what your next session must know");
    }
    let Some((provider, container)) = provider_of(&state, &block_id) else {
        return refused("the pane can't be read");
    };
    match (provider.as_str(), container) {
        (_, true) => return refused("unsupported in containers"),
        ("claude" | "codex", false) => {}
        _ => return refused("unsupported: only Claude and Codex agents can start fresh"),
    }
    if let Err(why) = context_reading::schedule_compact(&block_id, agentmux_common::time::now_ms_u64()) {
        return refused(why);
    }
    tracing::info!(block_id = %block_id, provider = %provider, handoff_chars = handoff.len(), "fresh session scheduled");
    tokio::spawn(fresh_when_turn_ends(state, block_id, provider, handoff));
    Json(json!({ "status": "scheduled", "runs": "when your current turn ends" })).into_response()
}

/// The new session's first message from AgentMux: the agent's own handoff. A
/// fresh Claude session also gets the continuation packet ahead of it.
pub(crate) fn handoff_message(handoff: &str) -> String {
    format!("[AgentMux] This is the fresh session you asked for. Your previous session's handoff:\n\n{handoff}")
}

/// Wait for the turn to end, end the session, then send the handoff as
/// AgentMux's own turn, which starts the new session.
async fn fresh_when_turn_ends(state: AppState, block_id: String, provider: String, handoff: String) {
    if !wait_for_turn_end(&block_id).await {
        tracing::warn!(block_id = %block_id, "fresh session given up: the turn didn't end within 30 minutes");
        context_reading::fresh_sent(&block_id, false);
        return;
    }
    let ended = match crate::backend::blockcontroller::get_controller(&block_id) {
        Some(ctrl) if provider == "claude" => {
            use crate::backend::blockcontroller::persistent::PersistentSubprocessController;
            match ctrl.as_any().downcast_ref::<PersistentSubprocessController>() {
                Some(p) => p.start_fresh_session().map(|_| ()),
                None => Err("the agent's controller can't start fresh"),
            }
        }
        Some(ctrl) => {
            use crate::backend::blockcontroller::subprocess::SubprocessController;
            match ctrl.as_any().downcast_ref::<SubprocessController>() {
                Some(s) => {
                    s.forget_session();
                    crate::backend::blockcontroller::core::persist_session_id(&block_id, "", &Some(state.mstore.clone()), &None);
                    Ok(())
                }
                None => Err("the agent's controller can't start fresh"),
            }
        }
        None => Err("no controller for the pane"),
    };
    if let Err(why) = ended {
        tracing::warn!(block_id = %block_id, reason = why, "fresh session could not end the old one");
        context_reading::fresh_sent(&block_id, false);
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
        handoff_message(&handoff),
        None,
        crate::server::agent_handlers::TurnRegistration::Register,
        crate::backend::blockcontroller::health::TurnOrigin::System,
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

/// Wait for `block_id`'s turn to end, then send `message` as AgentMux's own turn.
async fn compact_when_turn_ends(state: AppState, block_id: String, message: String) {
    let deadline = tokio::time::Instant::now() + TURN_END_WAIT;
    while crate::backend::blockcontroller::get_block_controller_status(&block_id).is_some_and(|s| s.turn_active) {
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(block_id = %block_id, "self-compaction given up: the turn didn't end within 30 minutes");
            context_reading::compact_sent(&block_id, false);
            return;
        }
        tokio::time::sleep(TURN_END_POLL).await;
    }
    let deps = crate::server::agent_handlers::AgentTurnDeps::from_state(&state);
    let outcome = crate::server::agent_handlers::run_agent_turn(
        &deps,
        block_id.clone(),
        message,
        None,
        crate::server::agent_handlers::TurnRegistration::Register,
        crate::backend::blockcontroller::health::TurnOrigin::System,
        Vec::new(),
    )
    .await;
    context_reading::compact_sent(&block_id, outcome.is_ok());
    match outcome {
        Ok(()) => tracing::info!(block_id = %block_id, "self-compaction sent"),
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
        assert!(m.ends_with("Task #1791: PR 3 open as #5410; next: wait for review."));
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
