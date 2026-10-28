// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/open_many`, the `Agents` tool's `open` with `agents`
//! (`SPEC_COUNTDOWN_BUTTONS_AND_BATCH_AGENT_ACTIONS_2026_10_28.md` §5): several agents opened at once,
//! all or none. Opening asks the user nothing, as for one. The agents that are not running must be
//! able to start together (room under the agent cap, without stopping anyone; the sender's starts per
//! minute) or the call is refused whole; a failed open closes the panes this call made.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::backend::rpc_types::{AgentOpenResult, CommandAgentOpenData};
use crate::server::agent_lifecycle::{admit_batch, release_batch, Wanted};
use crate::server::state::AppState;

/// The most agents one call opens: a roster, not a sweep.
pub(crate) const MAX_OPEN_MANY: usize = 16;

#[derive(Debug, Deserialize)]
pub(crate) struct OpenManyRequest {
    agents: Vec<CommandAgentOpenData>,
}

struct Refusal {
    status: StatusCode,
    message: String,
}

fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    Refusal { status, message: message.into() }
}

pub(crate) async fn handle_agent_open_many(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<OpenManyRequest>,
) -> Response {
    let sender = caller.as_ref().and_then(|c| c.0.uid().map(str::to_string)).unwrap_or_else(|| "agent.open_many".to_string());
    match open_many(&state, &sender, req.agents).await {
        Ok(opened) => Json(json!({ "opened": opened })).into_response(),
        Err(e) => (e.status, Json(json!({ "error": e.message }))).into_response(),
    }
}

/// The agent `named` is: a definition id, slug or name (any case) of an agent made on this machine.
fn resolve(state: &AppState, named: &str) -> Result<crate::backend::storage::agents::AgentDefinition, String> {
    let named = named.trim();
    let defs = state.mstore.agent_def_list().map_err(|e| format!("agent.open: {e}"))?;
    defs.into_iter()
        .find(|d| d.is_seeded == 0 && (d.id == named || d.slug.eq_ignore_ascii_case(named) || d.name.eq_ignore_ascii_case(named)))
        .ok_or_else(|| format!("AGENT_NOT_FOUND: no agent {named:?} (use an agent's id or name from Agents list; a template can't be opened)"))
}

async fn open_many(state: &AppState, sender: &str, asked: Vec<CommandAgentOpenData>) -> Result<Vec<AgentOpenResult>, Refusal> {
    if asked.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: agents is empty: give [{agent, tab_id?, focus?}]"));
    }
    if asked.len() > MAX_OPEN_MANY {
        return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: at most {MAX_OPEN_MANY} agents in one call (got {})", asked.len())));
    }
    // Everything that can be checked, before the first one opens.
    let mut seen = std::collections::HashSet::new();
    let mut cmds = Vec::with_capacity(asked.len());
    let mut defs = Vec::with_capacity(asked.len());
    for (i, mut cmd) in asked.into_iter().enumerate() {
        let def = resolve(state, &cmd.agent_id).map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("agent {}: {e}", i + 1)))?;
        if !seen.insert(def.id.clone()) {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: agent {}: {} appears twice in agents", i + 1, def.name)));
        }
        cmd.agent_id = def.id.clone();
        cmds.push(cmd);
        defs.push(def);
    }
    // Those already running (shown or not) need no room; the others are the batch's starts.
    let running: std::collections::HashSet<String> = crate::server::agent_lifecycle::live_agents(state).into_iter().map(|l| l.uid).collect();
    let wanted: Vec<Wanted> = defs.iter().filter(|d| !running.contains(&d.id)).map(|d| Wanted { key: d.id.clone(), label: d.name.clone() }).collect();
    admit_batch(state, &wanted, true).await.map_err(|e| refuse(StatusCode::CONFLICT, e))?;
    if let Err(e) = crate::server::start_rate::allow_n(state, sender, wanted.len()) {
        release_batch(state, &wanted);
        return Err(refuse(StatusCode::TOO_MANY_REQUESTS, e));
    }

    let total = cmds.len();
    let mut opened: Vec<AgentOpenResult> = Vec::with_capacity(total);
    for (i, cmd) in cmds.into_iter().enumerate() {
        match super::open_agent_impl(state, cmd).await {
            Ok(result) => opened.push(result),
            Err(e) => {
                // Close the panes this call made; one that was already open stays as it was.
                let made: Vec<&AgentOpenResult> = opened.iter().filter(|r| r.created).collect();
                for r in &made {
                    if let Err(close) = crate::sagas::delete_block::run(state, r.tab_id.clone(), r.block_id.clone()).await {
                        tracing::warn!(block_id = %r.block_id, error = %close, "agent.open_many: a pane it opened is left behind");
                    }
                }
                release_batch(state, &wanted);
                return Err(refuse(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("agent {} of {total} ({}) could not be opened, so none were opened ({} panes made by this call were closed): {e}", i + 1, defs[i].name, made.len()),
                ));
            }
        }
    }
    release_batch(state, &wanted);
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    fn ask(agent: &str) -> CommandAgentOpenData {
        CommandAgentOpenData { agent_id: agent.to_string(), tab_id: None, split_direction: None, split_reference_block_id: None, focus: None }
    }

    #[tokio::test]
    async fn an_empty_list_a_missing_agent_and_a_repeat_are_refused_before_anything_opens() {
        let state = test_state();
        let e = open_many(&state, "t", vec![]).await.err().expect("empty");
        assert!(e.message.starts_with("INVALID_ARG: agents is empty"), "{}", e.message);
        let e = open_many(&state, "t", vec![ask("no-such-agent-xyz")]).await.err().expect("missing");
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert!(e.message.starts_with("agent 1: AGENT_NOT_FOUND"), "{}", e.message);
        let many: Vec<_> = (0..=MAX_OPEN_MANY).map(|i| ask(&format!("a{i}"))).collect();
        let e = open_many(&state, "t", many).await.err().expect("too many");
        assert!(e.message.contains("at most 16 agents"), "{}", e.message);
    }

    #[tokio::test]
    async fn the_same_agent_twice_is_refused() {
        let state = test_state();
        let mut def = crate::backend::storage::agents::test_agent_def(&format!("om-{}", uuid::Uuid::new_v4()), "Open Many Twice", "claude", "agent", 1, "");
        def.slug = "open-many-twice".to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
        let e = open_many(&state, "t", vec![ask(&def.id), ask("OPEN MANY TWICE")]).await.err().expect("twice");
        assert!(e.message.contains("appears twice"), "{}", e.message);
    }
}
