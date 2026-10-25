// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A leader accepts or sends back a task in Review on the operator's word
//! (docs/specs/SPEC_KANBAN_ACCEPT_ON_THE_OPERATORS_WORD_2026_10_23.md §4):
//!
//!   POST /agentmux/work/:id/accept     { auth, user_instruction, text? }
//!   POST /agentmux/work/:id/send_back  { auth, user_instruction, text }
//!
//! These are not `kanban.act`. That is the operator's own window, and what it
//! does is theirs alone; here a leader acts, and only when the operator's own
//! message to that leader, quoted, told it to. The caller is the agent whose
//! signed identity (or token) verifies, never one the body names.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use agentmux_common::api_types::UiAutomationAuth;
use agentmux_common::time::now_ms;

use super::app_api::kanban::{accept_task, send_back_task, Decider};
use super::caller::Caller;
use super::work_tasks::resolve_id;
use super::AppState;
use crate::backend::kanban_delegate::{self as delegate, Refusal};
use crate::backend::reactive::sanitize::is_sensitive_message;
use crate::backend::reactive::types::AgentRegistration;
use crate::backend::storage::work_board::BoardTask;
use crate::backend::storage::work_queue::work_state;

/// The longest quote kept: the operator's words that told the leader to act.
const MAX_QUOTE_CHARS: usize = 500;

#[derive(Debug, Deserialize)]
pub(crate) struct DecisionRequest {
    #[serde(flatten)]
    auth: UiAutomationAuth,
    /// The operator's own words, quoted from the message that started this turn.
    #[serde(default)]
    user_instruction: String,
    /// `accept`: an optional note. `send_back`: the reason, which goes to the holder.
    #[serde(default)]
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verb {
    Accept,
    SendBack,
}

impl Verb {
    fn audit_action(self) -> &'static str {
        match self {
            Verb::Accept => "kanban.accept",
            Verb::SendBack => "kanban.send_back",
        }
    }
}

pub(crate) async fn handle_work_accept(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<DecisionRequest>,
) -> Response {
    decide(&state, caller.as_deref(), &id, req, Verb::Accept).await
}

pub(crate) async fn handle_work_send_back(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<DecisionRequest>,
) -> Response {
    decide(&state, caller.as_deref(), &id, req, Verb::SendBack).await
}

fn refused(r: &Refusal) -> Response {
    (StatusCode::FORBIDDEN, Json(json!({ "error": r.words, "code": r.code }))).into_response()
}

/// Whether the caller is a leader: a seat whose role is leader, stored for its
/// definition (`agent.setSeat`, the operator's). Never a name.
fn is_leader(state: &AppState, uid: &str) -> bool {
    state.mstore.agent_seat_get(uid).is_some_and(|s| s.role == "leader")
}

/// The task is the caller's own: it holds it, or handed it in.
fn is_own_work(t: &BoardTask, reg: &AgentRegistration, events: &[crate::backend::storage::work_board::WorkEvent]) -> bool {
    let uid = reg.uid.as_deref().unwrap_or("");
    let holds = if t.item.claimed_by_uid.is_empty() {
        t.item.claimed_by.eq_ignore_ascii_case(&reg.agent_id)
    } else {
        !uid.is_empty() && t.item.claimed_by_uid == uid
    };
    holds || events.iter().any(|e| e.kind == "submitted" && e.actor.eq_ignore_ascii_case(&reg.agent_id))
}

/// When the task last reached Review, ms; `i64::MAX` when no event says so, so
/// that no earlier reply can be taken to have named it there.
fn in_review_since(events: &[crate::backend::storage::work_board::WorkEvent]) -> i64 {
    events.iter().rev().find(|e| e.to_state == work_state::REVIEW).map(|e| e.at).unwrap_or(i64::MAX)
}

/// The leader's reply before this turn, from the tail of its session output.
async fn previous_reply(state: &AppState, block_id: &str) -> Option<delegate::Reply> {
    let (mstore, filestore, block) = (state.mstore.clone(), state.filestore.clone(), block_id.to_string());
    tokio::task::spawn_blocking(move || {
        let (bytes, _) = crate::backend::session_archive::read_session_output(&mstore, &filestore, &block).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let start = text.rmatch_indices('\n').nth(delegate::OUTPUT_TAIL_LINES).map(|(i, _)| i + 1).unwrap_or(0);
        delegate::previous_reply(&text[start..])
    })
    .await
    .ok()
    .flatten()
}

async fn decide(state: &AppState, caller: Option<&Caller>, id: &str, req: DecisionRequest, verb: Verb) -> Response {
    let block_id = match super::ui_handlers::verified_block_id(state, caller, &req.auth) {
        Ok(b) => b,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))).into_response(),
    };
    let regs = state.reactive_handler.list_agents();
    let Some(reg) = regs.iter().find(|r| r.block_id == block_id).cloned() else {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "your pane isn't registered with AgentMux" }))).into_response();
    };
    let task_id = resolve_id(state, id);
    let task = match state.work_store.work_board_get(&task_id) {
        Ok(Some(t)) => t,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "no such task" }))).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    };
    let num = task.card.num;
    let quote = req.user_instruction.trim().chars().take(MAX_QUOTE_CHARS).collect::<String>();

    let outcome = check(state, &reg, &block_id, &task, &quote, &req.text, verb).await;
    // One audit line per call, a refusal too: who, which task, the quote and why not.
    let request_id = uuid::Uuid::new_v4().to_string();
    let note = format!("#{num} on the operator's word: \"{quote}\"");
    match &outcome {
        Err(r) => {
            tracing::info!(agent = %reg.agent_id, task = num, why = r.code, "kanban: a leader's decision was refused");
            state.reactive_handler.log_fleet_action_audit(
                Some(&reg.agent_id),
                &reg.agent_id,
                &block_id,
                verb.audit_action(),
                false,
                Some(r.code),
                &request_id,
                Some(&note),
            );
            refused(r)
        }
        Ok(()) => {
            let by = Decider::delegate(&reg.agent_id, &quote);
            let text = super::work_tasks::clean(&req.text);
            let now = now_ms();
            let done = match verb {
                Verb::Accept => accept_task(state, &task, &by, &text, now),
                Verb::SendBack => send_back_task(state, &regs, &task, &by, &text, now).await,
            };
            state.reactive_handler.log_fleet_action_audit(
                Some(&reg.agent_id),
                &reg.agent_id,
                &block_id,
                verb.audit_action(),
                done.is_ok(),
                done.as_ref().err().map(String::as_str),
                &request_id,
                Some(&note),
            );
            tracing::info!(agent = %reg.agent_id, task = num, action = verb.audit_action(), ok = done.is_ok(), "kanban: a leader decided on the operator's word");
            super::work_queue::publish_changed(state);
            match done {
                Ok(v) => Json(v).into_response(),
                Err(e) => (StatusCode::CONFLICT, Json(json!({ "error": e }))).into_response(),
            }
        }
    }
}

/// The checks of spec §4.2, in order; the first that fails answers.
async fn check(
    state: &AppState,
    reg: &AgentRegistration,
    block_id: &str,
    task: &BoardTask,
    quote: &str,
    text: &str,
    verb: Verb,
) -> Result<(), Refusal> {
    let num = task.card.num;
    // 1. A leader.
    match reg.uid.as_deref() {
        Some(uid) if is_leader(state, uid) => {}
        _ => return Err(delegate::not_a_leader()),
    }
    // 2. In Review.
    if task.item.state != work_state::REVIEW {
        return Err(delegate::not_in_review(num, &task.item.state));
    }
    // 3. Not its own work.
    let events = state.work_store.work_events(&task.item.id, 200).unwrap_or_default();
    if is_own_work(task, reg, &events) {
        return Err(delegate::own_work(num));
    }
    if verb == Verb::SendBack && text.trim().is_empty() {
        return Err(Refusal { code: "reason_required", words: "send_back needs a reason: it goes to the task's holder.".to_string() });
    }
    // A credential in the quote or the text is refused, not stored.
    if is_sensitive_message(quote) || is_sensitive_message(text) {
        return Err(delegate::sensitive_quote());
    }
    // 4. The operator's word.
    let provenance = crate::backend::blockcontroller::get_controller(block_id).and_then(|c| c.turn_provenance());
    let user_text = delegate::operators_word(provenance.as_ref(), quote, num)?;
    // 5. The words cover this task: its number is in them, else the leader's previous reply named it.
    let previous = if delegate::names_task(user_text, num) { None } else { previous_reply(state, block_id).await };
    delegate::covers(user_text, previous.as_ref(), num, in_review_since(&events))?;
    Ok(())
}

#[cfg(test)]
#[path = "work_delegate_tests.rs"]
mod tests;
