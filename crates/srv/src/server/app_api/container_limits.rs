// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container agent's CPU and memory limits: `Agents set_container_limits` (`POST /api/v1/agent/container/limits`),
//! and the window's `agent.getContainerLimits` / `agent.setContainerLimits`, which share [`view`] and [`set`].
//! `docs/specs/SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md` §3, §4.
//!
//! A change is stored (`db_agent_container_limits`), put on the running container with `docker update` at once, and
//! shown in the agent's pane with who made it. A container agent can read limits but not change them, its own
//! included (the operator's decision of 2026-10-29): it asks a host agent or the operator.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::container::limits::{normalize, ContainerLimits, Target};
use crate::backend::rpc_types::AgentContainerLimitsResult;
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// A refusal: the status it travels with, and what to tell the caller.
pub(crate) type Refusal = (StatusCode, String);

fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    (status, message.into())
}

/// The container agent `named` (an id or slug): its uid and definition.
pub(crate) fn container_agent(state: &AppState, named: &str) -> Result<(String, crate::backend::storage::AgentDefinition), Refusal> {
    let named = named.trim();
    let uid = crate::backend::agent_resolve::resolve_agent_id(&state.mstore, named)
        .map_err(|_| refuse(StatusCode::NOT_FOUND, format!("no agent '{named}' on this machine")))?;
    let def = state
        .mstore
        .agent_def_get(&uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("container limits: {e}")))?
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("no agent '{named}' on this machine")))?;
    if def.agent_type != "container" {
        return Err(refuse(StatusCode::CONFLICT, format!("{} does not run in a container, so it has no container limits", def.name)));
    }
    Ok((uid, def))
}

fn mb_of(bytes: i64) -> u64 {
    bytes as u64 / (1024 * 1024)
}

/// The limits stored for `uid`, what is in force, and what the daemon has.
pub(crate) async fn view(state: &AppState, uid: &str, applied: Option<bool>) -> Result<AgentContainerLimitsResult, Refusal> {
    let (memory_mb, cpus) = state
        .mstore
        .container_limits_get(uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("container limits: {e}")))?
        .unwrap_or_default();
    let mut out = AgentContainerLimitsResult {
        memory_mb: memory_mb.clone(),
        cpus: cpus.clone(),
        effective_memory_mb: None,
        effective_cpus: None,
        daemon_memory_mb: None,
        daemon_cpus: None,
        applied,
    };
    if let Some(cm) = state.container_manager.get().await {
        let name = crate::backend::container::container_name_for_slug(uid);
        let wanted = cm.wanted_limits_for_container(&name, ContainerLimits::from_strings(&memory_mb, &cpus)).await;
        if let Target::Set(bytes) = wanted.memory {
            out.effective_memory_mb = Some(mb_of(bytes));
        }
        if let Target::Set(nano) = wanted.cpus {
            out.effective_cpus = Some(nano as f64 / 1e9);
        }
        if let Some(capacity) = cm.daemon_capacity().await {
            out.daemon_memory_mb = Some(capacity.mem_total / (1024 * 1024));
            out.daemon_cpus = Some(capacity.ncpu);
        }
    }
    Ok(out)
}

/// Stores the limits given (`None` leaves one as it is), puts them on the container now and says so in the pane.
/// `by` is who is shown as having made the change.
pub(crate) async fn set(
    state: &AppState,
    uid: &str,
    def: &crate::backend::storage::AgentDefinition,
    memory: Option<&Value>,
    cpus: Option<&Value>,
    by: &str,
) -> Result<AgentContainerLimitsResult, Refusal> {
    let memory = memory.map(|v| normalize(v, "memory_mb")).transpose().map_err(|m| refuse(StatusCode::BAD_REQUEST, m))?;
    let cpus = cpus.map(|v| normalize(v, "cpus")).transpose().map_err(|m| refuse(StatusCode::BAD_REQUEST, m))?;
    let (old_memory, old_cpus) = state.mstore.container_limits_get(uid).ok().flatten().unwrap_or_default();
    let (memory, cpus) = (memory.unwrap_or(old_memory), cpus.unwrap_or(old_cpus));
    state
        .mstore
        .container_limits_set(uid, &memory, &cpus)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("container limits: {e}")))?;

    // Put on the running container now. A container that is not there takes them when it is made.
    let mut applied = None;
    if let Some(cm) = state.container_manager.get().await {
        let name = crate::backend::container::container_name_for_slug(uid);
        applied = cm.limits_apply(&name, ContainerLimits::from_strings(&memory, &cpus)).await;
    }
    let result = view(state, uid, applied).await?;
    tracing::info!(agent = %def.slug, %by, memory_mb = %memory, cpus = %cpus, ?applied, "container limits set");
    note_in_pane(state, uid, def, &result, by);
    Ok(result)
}

/// One pane row saying the limits changed, so a change is visible whoever makes it.
fn note_in_pane(state: &AppState, uid: &str, def: &crate::backend::storage::AgentDefinition, result: &AgentContainerLimitsResult, by: &str) {
    let Some(registration) = crate::backend::reactive::handler::get_global_handler().get_agent(&def.slug) else { return };
    let block_id = registration.block_id;
    let provider = match state.mstore.get::<crate::backend::obj::Block>(&block_id) {
        Ok(Some(block)) => crate::backend::obj::meta_get_string(&block.meta, "agentProvider", "claude"),
        _ => return,
    };
    let (state_name, error) = match result.applied {
        Some(false) => ("failed", Some("Docker did not take the new limits; the container keeps the ones it has")),
        _ => ("changed", None),
    };
    let frame = crate::backend::cli_notice::container_limit_frame(
        &provider,
        &block_id,
        (uid, &def.name),
        state_name,
        result.effective_memory_mb,
        result.effective_cpus,
        Some(by),
        error,
    );
    crate::backend::cli_notice::append_to_pane(&state.broker, &state.filestore, &state.mstore, &block_id, &frame);
}

/// `POST /api/v1/agent/container/limits`: `Agents set_container_limits`. With neither `memory_mb` nor `cpus` it only
/// reads.
#[derive(Debug, Deserialize)]
pub(crate) struct LimitsRequest {
    agent: String,
    /// MB, `"auto"`, or `0` for no limit.
    #[serde(default)]
    memory_mb: Option<Value>,
    /// CPUs (fractions allowed), `"auto"`, or `0` for no limit.
    #[serde(default)]
    cpus: Option<Value>,
}

async fn run(state: &AppState, caller: Option<&Caller>, req: LimitsRequest) -> Result<Value, Refusal> {
    let (uid, def) = container_agent(state, &req.agent)?;
    let changing = req.memory_mb.is_some() || req.cpus.is_some();
    let mut by = "you".to_string();
    if let Some(caller_uid) = caller.and_then(Caller::uid) {
        let caller_def = state
            .mstore
            .agent_def_get(caller_uid)
            .ok()
            .flatten()
            .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
        if changing && caller_def.agent_type == "container" {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                "CONTAINER_AGENT: a container agent cannot change a container's limits, its own included. Ask a host agent, \
                 your steward or the operator to raise it, saying how much memory the work needs and why",
            ));
        }
        by = caller_def.name;
    }
    let result = if changing {
        set(state, &uid, &def, req.memory_mb.as_ref(), req.cpus.as_ref(), &by).await?
    } else {
        view(state, &uid, None).await?
    };
    Ok(json!({
        "agent": def.name,
        "limits": result,
        "note": "memory_mb and cpus are what is set (empty: auto, \"0\": no limit); effective_* is what is in force now, \
                 and daemon_* what Docker has in total. A change is put on the running container at once",
    }))
}

pub(crate) async fn handle_container_limits(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<LimitsRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match run(&state, caller.as_ref(), req).await {
        Ok(body) => Json(body).into_response(),
        Err((status, message)) => (status, Json(json!({ "error": message }))).into_response(),
    }
}
