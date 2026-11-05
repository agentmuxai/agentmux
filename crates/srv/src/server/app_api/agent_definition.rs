// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents update`, `bundles`, `hide` and `unhide` (`POST /api/v1/agent/{update,bundles,hide,unhide}`):
//! an agent edits an agent's definition, sets its Bundles list and hides or shows a picker template
//! without the operator's window, by the code the window's `updateagent`, `setagentbundles` and
//! `agentdefhide`/`agentdefunhide` run.
//!
//! - update: only the fields given change, and only ones that say how the agent is listed or when it runs:
//!   description, icon, auto_start, restart_on_crash and idle_timeout_minutes. What decides what an agent runs, or
//!   where its traffic goes or which project files it reads (shell, provider_flags, working_directory,
//!   model_vendor_base_url) is not taken, as `env_set` refuses the same: the operator sets those in My Agents. The
//!   name has its own action (`rename`), the provider is fixed when an agent is made, and environment variables,
//!   accounts, container settings and the seat have their own actions.
//! - bundles: with `bundle_ids`, replaces the agent's picked bundles, in order; without, reads them.
//! - hide, unhide: a seeded picker template; an agent you made is removed with `delete`.
//!
//! Each acts as the agent its token names; a request without one is refused, so a change is never attributed to
//! the operator, and each one, and each refusal, is recorded in the audit log under the calling agent.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::agent_lifecycle::{actor_of, audit, refuse, target_of, Actor, Refusal};
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The longest description or text field: what the definition form accepts in practice.
const MAX_TEXT_CHARS: usize = 4000;
/// A week: the longest idle timeout.
const MAX_IDLE_MINUTES: i64 = 7 * 24 * 60;

/// A field outside the list above is refused by name (`deny_unknown_fields`), never ignored.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateRequest {
    /// The agent's id, slug or name.
    agent: String,
    description: Option<String>,
    icon: Option<String>,
    auto_start: Option<bool>,
    restart_on_crash: Option<bool>,
    idle_timeout_minutes: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BundlesRequest {
    agent: String,
    /// The whole list, in order; absent reads the current one.
    bundle_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct HideRequest {
    /// The template's id or slug.
    template: String,
}

fn bad(message: impl Into<String>) -> Refusal {
    refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {}", message.into()))
}

fn text_field(name: &str, value: &str) -> Result<String, Refusal> {
    let value = value.trim();
    if value.chars().count() > MAX_TEXT_CHARS {
        return Err(bad(format!("{name} is at most {MAX_TEXT_CHARS} characters")));
    }
    Ok(value.to_string())
}

fn publish_changed(state: &AppState) {
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
}

/// Apply the fields `req` gives to the agent's stored definition. Returns the names of the fields changed.
fn update(state: &AppState, req: &UpdateRequest) -> Result<(String, Vec<&'static str>), Refusal> {
    let target = target_of(state, &req.agent)?;
    let internal = |e: crate::backend::storage::error::StoreError| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents update: {e}"));
    let mut agent = state
        .mstore
        .agent_def_list()
        .map_err(internal)?
        .into_iter()
        .find(|a| a.id == target.agent_id)
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("no agent '{}' on this machine", req.agent)))?;
    let mut changed: Vec<&'static str> = Vec::new();
    let mut set = |field: &'static str, slot: &mut String, value: &Option<String>| -> Result<(), Refusal> {
        if let Some(v) = value {
            *slot = text_field(field, v)?;
            changed.push(field);
        }
        Ok(())
    };
    set("description", &mut agent.description, &req.description)?;
    set("icon", &mut agent.icon, &req.icon)?;
    if let Some(on) = req.auto_start {
        agent.auto_start = i64::from(on);
        changed.push("auto_start");
    }
    if let Some(on) = req.restart_on_crash {
        agent.restart_on_crash = i64::from(on);
        changed.push("restart_on_crash");
    }
    if let Some(minutes) = req.idle_timeout_minutes {
        if !(0..=MAX_IDLE_MINUTES).contains(&minutes) {
            return Err(bad(format!("idle_timeout_minutes is 0 (never) to {MAX_IDLE_MINUTES}")));
        }
        agent.idle_timeout_minutes = minutes;
        changed.push("idle_timeout_minutes");
    }
    if changed.is_empty() {
        return Err(bad("nothing to change; give at least one of description, icon, auto_start, restart_on_crash, idle_timeout_minutes (name: rename; environment: env_set)"));
    }
    if !state.mstore.agent_def_update(&mut agent).map_err(internal)? {
        return Err(refuse(StatusCode::NOT_FOUND, format!("no agent '{}' on this machine", req.agent)));
    }
    publish_changed(state);
    Ok((target.agent_id, changed))
}

fn bundles(state: &AppState, req: &BundlesRequest) -> Result<Value, Refusal> {
    let target = target_of(state, &req.agent)?;
    let internal = |e: crate::backend::storage::error::StoreError| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents bundles: {e}"));
    let Some(ids) = &req.bundle_ids else {
        return Ok(json!({ "agent_id": target.agent_id, "bundle_ids": state.mstore.agent_bundle_ids(&target.agent_id).map_err(internal)? }));
    };
    for id in ids.iter().map(|id| id.trim()).filter(|id| !id.is_empty()) {
        match state.id_store.bundle_get(id).map_err(internal)? {
            Some(b) if !b.is_blank && !b.is_system => {}
            Some(_) => return Err(bad(format!("bundle {id} is the blank or a system entry, which an agent doesn't pick"))),
            None => return Err(refuse(StatusCode::NOT_FOUND, format!("no bundle {id}; Memory list shows the bundles"))),
        }
    }
    let stored = state.mstore.agent_bundles_set(&target.agent_id, ids).map_err(internal)?;
    Ok(json!({
        "agent_id": target.agent_id,
        "bundle_ids": stored,
        "note": "its next launch reads them: their instructions go into its startup file after Global Memory and their skills are added",
    }))
}

fn hide(state: &AppState, req: &HideRequest, hidden: bool) -> Result<Value, Refusal> {
    let verb = if hidden { "hide" } else { "unhide" };
    let named = req.template.trim();
    let internal = |e: crate::backend::storage::error::StoreError| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents {verb}: {e}"));
    let template = state
        .mstore
        .agent_def_list()
        .map_err(internal)?
        .into_iter()
        .find(|a| a.is_seeded == 1 && (a.id == named || a.slug == named))
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("no picker template '{named}'; only a seeded template can be hidden (an agent you made is removed with delete)")))?;
    state.mstore.agent_def_set_hidden(&template.id, hidden).map_err(internal)?;
    publish_changed(state);
    Ok(json!({ "template_id": template.id, "hidden": hidden }))
}

/// Run `op` as the calling agent, and record the outcome under it.
async fn run<F>(state: &AppState, caller: Option<&Caller>, action: &'static str, named: String, op: F) -> Response
where
    F: FnOnce(&AppState) -> Result<(Value, String), Refusal> + Send + 'static,
{
    let respond = |r: Refusal| (r.status, Json(json!({ "error": r.message }))).into_response();
    let actor: Actor = match actor_of(state, caller, action) {
        Ok(actor) => actor,
        Err(r) => return respond(r),
    };
    let blocking = state.clone();
    let outcome = tokio::task::spawn_blocking(move || op(&blocking))
        .await
        .unwrap_or_else(|e| Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents {action}: {e}"))));
    let audit_action = format!("agent.{action}");
    match outcome {
        Ok((mut body, note)) => {
            let id = audit(state, &actor, &audit_action, &named, "", true, None, &note);
            body["audit_id"] = json!(id);
            Json(body).into_response()
        }
        Err(r) => {
            audit(state, &actor, &audit_action, &named, "", false, Some(&r.message), "refused");
            respond(r)
        }
    }
}

pub(crate) async fn handle_agent_update(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<UpdateRequest>) -> Response {
    let caller = caller.map(|Extension(c)| c);
    let named = req.agent.trim().to_string();
    run(&state, caller.as_ref(), "update", named, move |s| {
        let (agent_id, changed) = update(s, &req)?;
        let note = format!("changed {} of {agent_id}", changed.join(", "));
        Ok((json!({ "agent_id": agent_id, "changed": changed, "note": "a running agent keeps what it launched with until it is launched again" }), note))
    })
    .await
}

pub(crate) async fn handle_agent_bundles(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<BundlesRequest>) -> Response {
    let caller = caller.map(|Extension(c)| c);
    let named = req.agent.trim().to_string();
    run(&state, caller.as_ref(), "bundles", named, move |s| {
        let body = bundles(s, &req)?;
        let note = if req.bundle_ids.is_some() { format!("set the bundles of {}", body["agent_id"]) } else { "read".to_string() };
        Ok((body, note))
    })
    .await
}

pub(crate) async fn handle_agent_hide(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<HideRequest>) -> Response {
    hide_or_show(state, caller, req, true).await
}

pub(crate) async fn handle_agent_unhide(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<HideRequest>) -> Response {
    hide_or_show(state, caller, req, false).await
}

async fn hide_or_show(state: AppState, caller: Option<Extension<Caller>>, req: HideRequest, hidden: bool) -> Response {
    let caller = caller.map(|Extension(c)| c);
    let named = req.template.trim().to_string();
    let action = if hidden { "hide" } else { "unhide" };
    run(&state, caller.as_ref(), action, named, move |s| {
        let body = hide(s, &req, hidden)?;
        let note = format!("{action} template {}", body["template_id"]);
        Ok((body, note))
    })
    .await
}

#[cfg(test)]
#[path = "tests/agent_definition_tests.rs"]
mod tests;
