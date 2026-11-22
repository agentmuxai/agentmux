// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents rotate_token` (`POST /api/v1/agent/rotate-token`): an agent replaces the token of an agent on this
//! machine. The token is part of the agent's spawn environment, so the new one reaches the agent only through a
//! restart; the old value stops working the moment the stored row is replaced (`Store::agent_token_rotate`).
//!
//! The caller is the agent its token names; a request without one is refused, so a change is never attributed
//! to the operator. Each rotation, each wait for the person's window and each refusal is recorded in the audit
//! log under the calling agent. The answer, the note a caller is sent and the audit never carry a token value.
//!
//! - Not your own agent: a restart of yourself is not yours to run; another agent rotates yours.
//! - A stopped agent: the row is replaced and nothing restarts; it takes the new token when it next starts.
//! - A running agent: it is stopped as `Agents stop` stops it (the person's 30 s window to keep it running,
//!   except for a worker the caller opened, which is stopped at once), its row is replaced once it has stopped,
//!   and it starts again in the pane and conversation it had. If its user keeps it running, nothing changes and
//!   the caller is told.

use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::agent_lifecycle::{actor_of, audit, may_have_process, refuse, row_json, stop, target_of, Actor, LifecycleRequest, Refusal};
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The audit entry's action for a rotation.
pub(crate) const AUDIT_ACTION: &str = "agent.rotate_token";

#[derive(Debug, Deserialize)]
pub(crate) struct RotateRequest {
    /// The agent's id, slug or name.
    agent: String,
}

/// Replace the stored token (and its index entry): off the async workers, as it takes the store's lock.
async fn replace_row(state: &AppState, agent_id: &str) -> Result<(), String> {
    let store = state.mstore.clone();
    let uid = agent_id.to_string();
    tokio::task::spawn_blocking(move || store.agent_token_rotate(&uid))
        .await
        .map_err(|e| format!("Agents rotate_token: {e}"))?
        .map_err(|e| format!("Agents rotate_token: {e}"))
}

/// How a restart in the agent's own pane went.
enum Restart {
    Done,
    /// The agent has no block any more (its tab was closed meanwhile): it is left stopped.
    NoBlock,
    Failed(String),
}

/// Start the agent again, in the block (pane) it has, with its session resumed: its spawn environment is built
/// afresh and carries the replaced token.
async fn restart_in_place(state: &AppState, agent_id: &str) -> Restart {
    match super::agent_block::find_agent_block_by_uid(&state.mstore, agent_id) {
        Ok(Some(_)) => match super::agent_attach::start_agent(state, agent_id).await {
            Ok(_) => Restart::Done,
            Err(e) => Restart::Failed(e),
        },
        Ok(None) => Restart::NoBlock,
        Err(e) => Restart::Failed(e),
    }
}

fn after_restart_note(name: &str, restart: &Restart) -> String {
    match restart {
        Restart::Done => format!("{name}'s token is replaced and it restarted in its pane with its conversation."),
        Restart::NoBlock => format!("{name}'s token is replaced; it has no pane or block now, so it is stopped and takes the new token when it is next started (Agents start)."),
        Restart::Failed(e) => format!("{name}'s token is replaced but it did not restart ({e}): it is stopped, and Agents start runs it with the new token."),
    }
}

async fn rotate(state: &AppState, actor: &Actor, req: &RotateRequest) -> Result<Value, Refusal> {
    let blocking = state.clone();
    let named = req.agent.clone();
    let target = tokio::task::spawn_blocking(move || target_of(&blocking, &named))
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents rotate_token: {e}")))??;
    if target.agent_id == actor.uid() {
        return Err(refuse(
            StatusCode::CONFLICT,
            "SELF_ROTATE: that is you. A new token reaches an agent only through a restart, which you cannot run on yourself: ask another agent on this machine (your leader or a peer) to run Agents rotate_token with your id, and you restart in your pane",
        ));
    }
    let running = may_have_process(&target);
    // A restart is a start: the sender's per-minute limit counts it, and it is checked before the person's window opens.
    if running {
        crate::server::start_rate::allow(state, actor.uid()).map_err(|e| refuse(StatusCode::TOO_MANY_REQUESTS, e))?;
    }
    let stopped = stop(state, actor, &LifecycleRequest { agent: req.agent.clone() }).await?;
    let (name, agent_id) = (target.name.clone(), target.agent_id.clone());

    // The person has their window to keep it running: replace the token and restart only if it goes on to stop.
    if let Some(pending) = stopped.get("pending").filter(|p| !p.is_null()) {
        let Some(request_id) = pending["request_id"].as_str().map(str::to_string) else {
            return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "Agents rotate_token: the stop request has no id"));
        };
        tokio::spawn(finish_after_window(state.clone(), actor.clone(), agent_id.clone(), name.clone(), request_id));
        return Ok(json!({
            "agent": name, "agent_id": agent_id, "rotated": false, "restarted": false, "pending": pending,
            "note": "its user has 30 seconds to keep it running; if it stops, its token is replaced and it restarts in its pane, and you are told by a note either way",
            "runtime": row_json(state, &agent_id),
        }));
    }

    replace_row(state, &agent_id).await.map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if stopped["stopped"] != true {
        return Ok(json!({
            "agent": name, "agent_id": agent_id, "rotated": true, "restarted": false,
            "note": "it was not running: its token is replaced and it takes the new one when it is next started",
            "runtime": row_json(state, &agent_id),
        }));
    }
    let restart = restart_in_place(state, &agent_id).await;
    Ok(json!({
        "agent": name, "agent_id": agent_id, "rotated": true, "restarted": matches!(restart, Restart::Done),
        "block_id": stopped["block_id"], "note": after_restart_note(&name, &restart),
        "runtime": row_json(state, &agent_id),
    }))
}

/// Wait for the stop request to end: `Ok` once the agent has stopped (or already had), else why it did not.
async fn wait_for_stop(request_id: &str) -> Result<(), String> {
    let give_up = Instant::now() + crate::sagas::pending_shutdown::OVERRIDE_WINDOW + Duration::from_secs(60);
    loop {
        match crate::sagas::pending_shutdown::status(request_id) {
            None => return Err("the stop request was lost".to_string()),
            Some(v) => match v.status {
                "pending" | "proceeding" => {}
                "shut_down" | "superseded" => return Ok(()),
                "kept_by_user" => return Err("its user chose to keep it running".to_string()),
                _ => return Err(v.error.unwrap_or_else(|| format!("the stop ended as {}", v.status))),
            },
        }
        if Instant::now() >= give_up {
            return Err("the stop did not finish in time".to_string());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// After the person's window: if the agent stopped, replace its token and start it again; tell the caller how it
/// ended, by a held `[AgentMux]` note that does not wake it, and record it under the caller.
async fn finish_after_window(state: AppState, actor: Actor, agent_id: String, name: String, request_id: String) {
    let stopped = wait_for_stop(&request_id).await;
    settle(&state, &actor, &agent_id, &name, &request_id, stopped).await;
}

async fn settle(state: &AppState, actor: &Actor, agent_id: &str, name: &str, request_id: &str, stopped: Result<(), String>) {
    let (ok, text) = match stopped {
        Err(why) => (false, format!("[AgentMux] Agents rotate_token for {name} was not done: {why}. Its token is unchanged; run it again when the agent can be stopped.")),
        Ok(()) => match replace_row(state, agent_id).await {
            Err(e) => (false, format!("[AgentMux] Agents rotate_token for {name} failed after it stopped: {e}. Its token is unchanged and it is stopped; Agents start runs it again.")),
            Ok(()) => {
                let restart = restart_in_place(state, agent_id).await;
                (true, format!("[AgentMux] Agents rotate_token: {}", after_restart_note(name, &restart)))
            }
        },
    };
    audit(state, actor, AUDIT_ACTION, agent_id, "", ok, (!ok).then_some(text.as_str()), "after the user's window");
    if let Err(e) = state.reactive_handler.hold_system_note(actor.slug(), &format!("rotate-token:{request_id}"), &text) {
        tracing::debug!(by = actor.slug(), "rotate_token note not held: {e}");
    }
}

async fn run(state: &AppState, caller: Option<&Caller>, req: RotateRequest) -> Result<Value, Refusal> {
    let actor = actor_of(state, caller, "rotate_token")?;
    match rotate(state, &actor, &req).await {
        Ok(mut body) => {
            let block_id = body["block_id"].as_str().unwrap_or_default().to_string();
            let target = body["agent_id"].as_str().unwrap_or_default().to_string();
            let note = body["note"].as_str().unwrap_or_default().to_string();
            body["audit_id"] = json!(audit(state, &actor, AUDIT_ACTION, &target, &block_id, true, None, &note));
            Ok(body)
        }
        Err(refusal) => {
            audit(state, &actor, AUDIT_ACTION, req.agent.trim(), "", false, Some(&refusal.message), "refused");
            Err(refusal)
        }
    }
}

pub(crate) async fn handle_agent_rotate_token(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<RotateRequest>) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match run(&state, caller.as_ref(), req).await {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}

#[cfg(test)]
#[path = "tests/agent_rotate_token_tests.rs"]
mod tests;
