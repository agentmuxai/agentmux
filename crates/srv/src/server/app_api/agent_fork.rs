// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/fork`, the `Agents` tool's `fork`
//! (docs/specs/SPEC_AGENT_WORKERS_2026_10_15.md §3.1, Phase A): the calling
//! agent, the leader, launches a worker, a fork of itself on the model it
//! names, in a pane to the right of its own. The worker starts a fresh
//! session; the MCP server then sends it the brief, signed as the leader.
//!
//! Launching an agent spends tokens until someone stops it, so (§3.5): only
//! a verified host agent forks, a worker doesn't, a leader has at most
//! [`MAX_LIVE_WORKERS`], and unless the user's own turn asked for it the user
//! is asked first.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use agentmux_common::api_types::ForkAgentRequest;

use super::*;

/// The worker pane's leader, `{agent, id, block_id}`: the leader's name, its
/// definition id and its pane. srv writes it when it opens the worker; no
/// client can (`browser_owner::srv_only_keys`).
pub(crate) const COACHED_BY_META_KEY: &str = "agent:coached_by";

/// Live workers a leader may have at once (§3.5).
pub(crate) const MAX_LIVE_WORKERS: usize = 3;

const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, serde::Serialize)]
pub(crate) struct ForkAgentResult {
    /// The worker's definition id.
    pub agent_id: String,
    pub name: String,
    /// What `Message` `send` addresses it by (its AGENTMUX_AGENT_ID).
    pub message_to: String,
    pub block_id: String,
    pub tab_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Whether the user was asked (the turn wasn't the user's own).
    pub asked_user: bool,
}

#[derive(Debug)]
pub(crate) struct ForkError {
    status: StatusCode,
    message: String,
}

fn refuse(status: StatusCode, message: impl Into<String>) -> ForkError {
    ForkError { status, message: message.into() }
}

pub(crate) async fn handle_agent_fork(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<ForkAgentRequest>,
) -> Response {
    match fork_worker(&state, caller.as_deref(), req).await {
        Ok(result) => Json(json!(result)).into_response(),
        Err(e) => (e.status, Json(json!({ "error": e.message }))).into_response(),
    }
}

/// The leader as srv knows it: its pane and its definition.
struct Leader {
    block_id: String,
    def: AgentDefinition,
}

async fn fork_worker(
    state: &AppState,
    caller: Option<&crate::server::caller::Caller>,
    req: ForkAgentRequest,
) -> Result<ForkAgentResult, ForkError> {
    let leader = leader_of(state, caller, &req)?;
    let brief = req.brief.trim();
    if brief.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: a brief is required: the worker's task, its first message"));
    }
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if let Some(n) = name {
        if n.chars().count() > MAX_NAME_CHARS || n.chars().any(char::is_control) {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: a name is at most {MAX_NAME_CHARS} characters, on one line")));
        }
    }
    let provider = state.id_store.resolve_effective_provider_id(&leader.def);
    let leader_runtime = state.mstore.agent_last_runtime_get(&leader.def.id).unwrap_or_default();
    let runtime = worker_runtime(&provider, &leader_runtime, req.model.as_deref(), req.effort.as_deref())
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {e}")))?;
    let tab_id = super::resolve_tab_id_for_block(&state.mstore, &leader.block_id)
        .map_err(|e| refuse(StatusCode::CONFLICT, format!("NO_TAB: your pane isn't in a tab: {e}")))?;
    let live = live_workers_of(&state.mstore, &leader.def.id);
    if live >= MAX_LIVE_WORKERS {
        return Err(refuse(
            StatusCode::CONFLICT,
            format!(
                "TOO_MANY_WORKERS: you have {live} workers open, the most at once; close one \
                 (Stop pane) when its work is done, then fork again"
            ),
        ));
    }

    let asked_user = consent(state, &leader, name, runtime.model.as_deref(), brief, &req.user_instruction).await?;

    let (fork, opened) = launch(state, &leader, name, &tab_id, &runtime).await?;
    tracing::info!(
        leader = %leader.def.name,
        worker = %fork.name,
        block_id = %opened.block_id,
        model = ?runtime.model,
        asked_user,
        "agent.fork: a worker was launched"
    );
    Ok(ForkAgentResult {
        agent_id: fork.id,
        name: fork.name,
        message_to: fork.slug,
        block_id: opened.block_id,
        tab_id: opened.tab_id,
        provider: opened.provider,
        model: runtime.model,
        effort: runtime.effort,
        asked_user,
    })
}

/// Fork the leader's definition and open the fork as its worker. A fork that
/// doesn't open is deleted again: the user didn't get a worker, so there is
/// no definition of one in My Agents either.
async fn launch(
    state: &AppState,
    leader: &Leader,
    name: Option<&str>,
    tab_id: &str,
    runtime: &WorkerRuntime,
) -> Result<(AgentDefinition, AgentOpenResult), ForkError> {
    let fork = crate::server::agent_handlers::fork_definition(
        &state.mstore,
        &state.id_store,
        &state.identity_store,
        &leader.def.id,
        name.unwrap_or(""),
    )
    .map_err(|e| {
        let status = if e.contains("already exists") { StatusCode::CONFLICT } else { StatusCode::INTERNAL_SERVER_ERROR };
        refuse(status, e)
    })?;
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
    match open_worker(state, leader, &fork, tab_id, runtime).await {
        Ok(opened) => Ok((fork, opened)),
        Err(e) => {
            discard_definition(state, &fork.id);
            Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("the worker couldn't be opened, and was removed: {e}")))
        }
    }
}

/// The verified caller, if it may fork: a host agent's own pane, not a
/// worker's.
fn leader_of(
    state: &AppState,
    caller: Option<&crate::server::caller::Caller>,
    req: &ForkAgentRequest,
) -> Result<Leader, ForkError> {
    let block_id = crate::server::ui_handlers::verified_block_id(state, caller, &req.auth)
        .map_err(|e| refuse(StatusCode::UNAUTHORIZED, format!("could not verify which agent is asking: {e}")))?;
    let block = state
        .mstore
        .get::<Block>(&block_id)
        .ok()
        .flatten()
        .ok_or_else(|| refuse(StatusCode::CONFLICT, "NOT_AN_AGENT_PANE: your pane is gone"))?;
    if block.meta.get(COACHED_BY_META_KEY).is_some_and(|v| !v.is_null()) {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            "WORKER_CANNOT_FORK: you are a worker; only the agent that coaches you can start workers",
        ));
    }
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    let def = state
        .mstore
        .agent_def_list()
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent.fork: {e}")))?
        .into_iter()
        .find(|d| !def_id.is_empty() && d.id == def_id)
        .ok_or_else(|| refuse(StatusCode::CONFLICT, "NOT_AN_AGENT_PANE: your pane isn't an agent from My Agents"))?;
    if def.agent_type == "container" {
        return Err(refuse(StatusCode::FORBIDDEN, "CONTAINER_AGENT: a container agent can't start workers"));
    }
    Ok(Leader { block_id, def })
}

/// The worker's remembered runtime (`agentlastruntime`): the leader's, with
/// the requested model and effort over it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkerRuntime {
    /// The normalized JSON to store; empty when there is nothing to remember.
    stored: String,
    model: Option<String>,
    effort: Option<String>,
}

pub(crate) fn worker_runtime(
    provider: &str,
    leader_runtime: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<WorkerRuntime, String> {
    use crate::backend::agent_runtime::Remembered;
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let effort = effort.map(str::trim).filter(|e| !e.is_empty());
    let leader = Remembered::parse(provider, leader_runtime);
    if provider != "claude" {
        if model.is_some() || effort.is_some() {
            return Err(format!("a model and effort can be chosen only for a Claude worker for now; this agent runs {provider}"));
        }
        return Ok(WorkerRuntime::default());
    }
    let mut obj = serde_json::Map::new();
    if let Some(mode) = &leader.permission_mode {
        obj.insert("permissionMode".into(), json!(mode));
    }
    if let Some(m) = model.map(str::to_string).or(leader.model.clone()) {
        obj.insert("model".into(), json!(m));
    }
    if let Some(e) = effort.map(str::to_string).or(leader.effort.clone()) {
        obj.insert("effort".into(), json!(e));
    }
    let raw = serde_json::Value::Object(obj).to_string();
    // What a launch will read back: a value it would drop is refused here.
    let parsed = Remembered::parse(provider, &raw);
    if model.is_some() && parsed.model.as_deref() != model {
        return Err(format!("unknown model {:?}: use opus, sonnet, haiku or a claude-… model id", model.unwrap_or("")));
    }
    if effort.is_some() && parsed.effort.as_deref() != effort {
        return Err(format!("unknown effort {:?}: use low, medium, high, xhigh or max", effort.unwrap_or("")));
    }
    let stored = crate::backend::storage::agents::normalize_last_runtime(&raw)?;
    Ok(WorkerRuntime { stored, model: parsed.model, effort: parsed.effort })
}

/// How many open panes `leader_def_id` coaches: workers in some tab.
pub(crate) fn live_workers_of(mstore: &Store, leader_def_id: &str) -> usize {
    let Ok(tabs) = mstore.get_all::<Tab>() else {
        return 0;
    };
    tabs.iter()
        .flat_map(|t| t.blockids.iter())
        .filter_map(|id| mstore.get::<Block>(id).ok().flatten())
        .filter(|b| coached_by_id(&b.meta) == Some(leader_def_id))
        .count()
}

fn coached_by_id(meta: &MetaMapType) -> Option<&str> {
    meta.get(COACHED_BY_META_KEY)?.get("id")?.as_str()
}

/// May the fork go ahead? At once when the user's own turn asked for it (the
/// `QuitSelf` rule, `self_quit::gate`), otherwise only when the user says so
/// in the approval window, which no agent can answer. Returns whether the
/// user was asked.
async fn consent(
    state: &AppState,
    leader: &Leader,
    name: Option<&str>,
    model: Option<&str>,
    brief: &str,
    user_instruction: &str,
) -> Result<bool, ForkError> {
    let provenance = blockcontroller::get_controller(&leader.block_id).and_then(|c| c.turn_provenance());
    let Err(why) = crate::sagas::self_quit::gate(provenance.as_ref(), user_instruction) else {
        return Ok(false);
    };
    tracing::info!(leader = %leader.def.name, why = why.as_str(), "agent.fork: asking the user");
    let question = json!({
        "kind": "consent",
        "title": format!("{} wants to start a worker", one_line(&leader.def.name, 60)),
        "message": consent_message(&leader.def.name, name, model, brief),
        "checkbox": "",
        "ok_label": "Start it",
        "cancel_label": "Don't",
    });
    let answer = super::connections::ask_user(state, &leader.block_id, question)
        .await
        .map_err(|e| refuse(StatusCode::FORBIDDEN, format!("NOT_ALLOWED: starting a worker needs the user's go-ahead, and {e}")))?;
    if answer.answered && answer.approve {
        Ok(true)
    } else if answer.answered {
        Err(refuse(StatusCode::FORBIDDEN, "NOT_ALLOWED: the user said no to starting the worker"))
    } else {
        Err(refuse(StatusCode::FORBIDDEN, "NOT_ALLOWED: the user didn't answer, so no worker was started"))
    }
}

fn consent_message(leader: &str, name: Option<&str>, model: Option<&str>, brief: &str) -> String {
    let who = match name {
        Some(n) => format!("a new agent, {}", one_line(n, 60)),
        None => "a new agent".to_string(),
    };
    let on = model.map(|m| format!(" on {}", one_line(m, 40))).unwrap_or_default();
    format!(
        "{} wants to start {who}{on}, a fork of itself, in a pane next to its own, with this task:\n\n{}\n\n\
         It spends tokens until it is stopped. Start it?",
        one_line(leader, 60),
        one_line(brief, 400),
    )
}

/// `s` on one line, at most `max` characters.
fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>())
    }
}

/// Remember the worker's runtime, then open it right of the leader's pane,
/// marked as the leader's worker from the moment its block exists.
async fn open_worker(
    state: &AppState,
    leader: &Leader,
    fork: &AgentDefinition,
    tab_id: &str,
    runtime: &WorkerRuntime,
) -> Result<AgentOpenResult, String> {
    if !runtime.stored.is_empty() {
        state
            .mstore
            .agent_last_runtime_set(&fork.id, &runtime.stored)
            .map_err(|e| format!("remember its model: {e}"))?;
    }
    let mut meta = MetaMapType::new();
    meta.insert(
        COACHED_BY_META_KEY.to_string(),
        json!({ "agent": leader.def.name, "id": leader.def.id, "block_id": leader.block_id }),
    );
    super::agent_open::open_agent_inner(
        state,
        CommandAgentOpenData {
            agent_id: fork.id.clone(),
            tab_id: Some(tab_id.to_string()),
            split_direction: Some("right".to_string()),
            split_reference_block_id: Some(leader.block_id.clone()),
            focus: Some(false),
        },
        None,
        meta,
    )
    .await
}

/// Delete a fork that never got its pane, as the `deleteagent` RPC does.
fn discard_definition(state: &AppState, def_id: &str) {
    if let Err(e) = state.mstore.agent_def_delete(def_id) {
        tracing::warn!(def_id, error = %e, "agent.fork: the unopened worker's definition is left behind");
        return;
    }
    crate::server::agent_handlers::purge_identity_store_rows(&state.identity_store, &state.id_store, def_id, "agent.fork");
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
}

#[cfg(test)]
#[path = "tests/agent_fork_tests.rs"]
mod tests;
