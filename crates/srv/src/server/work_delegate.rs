// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! An agent accepts or sends back a task in Review:
//!
//!   POST /agentmux/work/:id/accept     { auth, text? }
//!   POST /agentmux/work/:id/send_back  { auth, text }
//!
//! Any agent may. The history carries the agent as the one who decided, and
//! says so plainly when the task was its own work; it is never recorded as the
//! operator. The caller is the agent whose signed identity (or token) verifies,
//! never one the body names.

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

#[derive(Debug, Deserialize)]
pub(crate) struct DecisionRequest {
    #[serde(flatten)]
    auth: UiAutomationAuth,
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
    let events = state.work_store.work_events(&task.item.id, 200).unwrap_or_default();
    let own_work = is_own_work(&task, &reg, &events);

    let outcome = check(&task, &req.text, verb);
    // One audit line per call, a refusal too: who, which task and why not.
    let request_id = uuid::Uuid::new_v4().to_string();
    let note = format!("#{num}{}", if own_work { " (its own work)" } else { "" });
    match &outcome {
        Err(r) => {
            tracing::info!(agent = %reg.agent_id, task = num, why = r.code, "kanban: an agent's decision was refused");
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
            // The history names the agent by its slug, as the other Kanban moves do.
            let slug = reg.uid.as_deref().and_then(|u| state.mstore.agent_names_by_id(u).ok().flatten()).map(|r| r.slug);
            let by = Decider::agent(slug.as_deref().unwrap_or(&reg.agent_id), own_work);
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
            tracing::info!(agent = %reg.agent_id, task = num, action = verb.audit_action(), ok = done.is_ok(), "kanban: an agent decided");
            super::work_queue::publish_changed(state);
            match done {
                Ok(v) => Json(v).into_response(),
                Err(e) => (StatusCode::CONFLICT, Json(json!({ "error": e }))).into_response(),
            }
        }
    }
}

/// The checks, in order; the first that fails answers.
fn check(task: &BoardTask, text: &str, verb: Verb) -> Result<(), Refusal> {
    if task.item.state != work_state::REVIEW {
        return Err(delegate::not_in_review(task.card.num, &task.item.state));
    }
    if verb == Verb::SendBack && text.trim().is_empty() {
        return Err(Refusal { code: "reason_required", words: "send_back needs a reason: it goes to the task's holder.".to_string() });
    }
    // A credential in the text is refused, not stored.
    if is_sensitive_message(text) {
        return Err(delegate::sensitive_quote());
    }
    Ok(())
}

#[cfg(test)]
#[path = "work_delegate_tests.rs"]
mod tests;
