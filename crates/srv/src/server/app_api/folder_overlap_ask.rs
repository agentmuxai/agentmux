// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents ask_go_ahead` (`POST /api/v1/agent/folder-overlap-ask`): an agent that was refused for a host agent that
//! shares a folder with a container agent asks the person to go ahead with that pair. srv keeps the ask
//! (`backend::folder_overlap_asks`), the window shows it as a card in the asking agent's pane, and the person answers
//! with one click, which only their window can do (`app_api::folder_overlap`). Asking again says where the ask
//! stands: `asked` while the person has not answered, `chosen` once they went ahead, `declined` after a no.
//!
//! The caller is the agent its token names; a request without one is refused, so an ask is never the person's. The
//! ask is recorded in the audit log under the asking agent, and the person's answer under the person.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::folder_overlap as overlap;
use crate::backend::folder_overlap_asks::{asks, Asked, FolderAsk};
use crate::backend::storage::agents::AgentDefinition;
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The audit entry's action for an ask.
pub(crate) const AUDIT_ASK: &str = "folder-overlap.ask";

#[derive(Debug, Deserialize)]
pub(crate) struct AskRequest {
    /// The host agent, by id or slug.
    pub(crate) host: String,
    /// The container agent, by id or slug.
    pub(crate) container: String,
}

#[derive(Debug)]
pub(crate) struct Refusal {
    pub status: StatusCode,
    pub message: String,
}

fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    Refusal { status, message: message.into() }
}

/// The agent asking, and the pane its card shows in (empty when it has none).
pub(crate) struct Asker {
    pub slug: String,
    pub name: String,
    pub block_id: String,
}

fn asker_of(state: &AppState, caller: Option<&Caller>) -> Result<Asker, Refusal> {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err(refuse(StatusCode::UNAUTHORIZED, "no agent token: Agents ask_go_ahead is an agent's ask; the person goes ahead from the AgentMux window"));
    };
    let def = state
        .mstore
        .agent_def_get(uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("ask_go_ahead: {e}")))?
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
    let block_id = crate::backend::reactive::handler::get_global_handler().block_for_uid(uid).unwrap_or_default();
    Ok(Asker { slug: def.slug, name: def.name, block_id })
}

fn agent_named(state: &AppState, named: &str, what: &str) -> Result<AgentDefinition, Refusal> {
    let named = named.trim();
    crate::backend::agent_resolve::resolve_agent_id(&state.mstore, named)
        .ok()
        .and_then(|id| state.mstore.agent_def_get(&id).ok().flatten())
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("no agent '{named}' on this machine ({what})")))
}

/// Where an ask for the pair stands, asking the person when they have not been asked.
pub(crate) fn ask(state: &AppState, asker: &Asker, req: &AskRequest, now_ms: u64) -> Result<Value, Refusal> {
    let host = agent_named(state, &req.host, "host")?;
    let container = agent_named(state, &req.container, "container")?;
    if !overlap::is_a_pair(&state.mstore, &host.id, &container.id) {
        return Err(refuse(StatusCode::BAD_REQUEST, "Agents ask_go_ahead needs host: a host agent and container: a container agent"));
    }
    if overlap::accepted(&host.id, &container.id) {
        return Ok(json!({ "status": "chosen", "note": "the person has gone ahead with these two agents: retry what was refused" }));
    }
    let shared = overlap::of(&state.mstore, &host, Some(&overlap::open_folder(&host))).iter().any(|o| o.container_id == container.id);
    if !shared {
        return Ok(json!({ "status": "not_needed", "note": "these two agents share no folder, so there is nothing to go ahead with" }));
    }
    if asker.block_id.is_empty() {
        return Err(refuse(StatusCode::CONFLICT, "Agents ask_go_ahead shows its card in the asking agent's pane, and this agent has no open pane to show it in"));
    }
    let ask = FolderAsk {
        host_id: host.id.clone(),
        host_name: host.name.clone(),
        container_id: container.id.clone(),
        container_name: container.name.clone(),
        agent: asker.name.clone(),
        block_id: asker.block_id.clone(),
        requested_ms: now_ms,
    };
    match asks().ask(ask, now_ms) {
        Asked::Waiting { new, .. } => {
            super::folder_overlap::publish_asks(state);
            if new {
                super::folder_overlap::audit(state, Some(&asker.slug), AUDIT_ASK, &host.id, &container.id, &asker.block_id);
            }
            Ok(json!({
                "status": "asked",
                "note": "the person is asked in your pane; ask_go_ahead again for their answer, then retry what was refused",
            }))
        }
        Asked::Declined { retry_after_ms } => Ok(json!({
            "status": "declined",
            "retry_after_seconds": retry_after_ms.div_ceil(1000),
            "note": "the person said no to these two agents; give them separate workspaces, or bind the shared folder read-only into the container agent (a volume ending in :ro)",
        })),
    }
}

pub(crate) async fn handle_folder_overlap_ask(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<AskRequest>) -> Response {
    let caller = caller.map(|Extension(c)| c);
    let run = || -> Result<Value, Refusal> {
        let asker = asker_of(&state, caller.as_ref())?;
        ask(&state, &asker, &req, agentmux_common::time::now_ms_u64())
    };
    match run() {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}
