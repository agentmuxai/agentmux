// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents update`, `bundles`, `hide` and `unhide` (`POST /api/v1/agent/{update,bundles,hide,unhide}`):
//! an agent edits an agent's definition, sets its Bundles list and hides or shows a picker template
//! without the operator's window, by the code the window's `updateagent`, `setagentbundles` and
//! `agentdefhide`/`agentdefunhide` run.
//!
//! - update: only the fields given change, and only ones that say how the agent is listed or when it runs:
//!   description, icon, auto_start, restart_on_crash, idle_timeout_minutes, the model route's model and the agent's
//!   daily spend cap (spend_cap_usd, and reset_spend_today for a spend that was wrong; see below). What decides what an agent runs, or
//!   where its traffic goes or which project files it reads (shell, provider_flags, working_directory,
//!   model_vendor_base_url) is not taken, as `env_set` refuses the same: the operator sets those in My Agents. The
//!   name has its own action (`rename`), the provider is fixed when an agent is made, and environment variables,
//!   accounts, container settings and the seat have their own actions.
//! - spend cap: the agent's leader may set or reset another agent's cap; an agent may only lower its own. Each change
//!   is recorded with the cap before and after (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 3.2).
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
use crate::backend::storage::AgentDefinition;
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
    /// `""` takes the agent off its model route (its token stops working at once); the route it is on, or only
    /// `model`, changes its model on that route.
    route: Option<String>,
    model: Option<String>,
    /// Dollars a day through the model gateway: a number, or 0 / "none" for no cap.
    spend_cap_usd: Option<Value>,
    /// Zero the agent's spend counted for today.
    reset_spend_today: Option<bool>,
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

/// `route` and `model` of `Agents update`: leave a model route (`route: ""`), or change the model on the route the agent
/// is already on. Putting an agent onto a route spends the person's key, so that is asked of them when the agent is
/// made or in the composer's model panel, not done by a call that names no one to ask.
fn route_change(state: &AppState, agent: &AgentDefinition, req: &UpdateRequest) -> Result<Vec<&'static str>, Refusal> {
    use crate::backend::model_gateway::{launch, switch::{self, Target}};
    let current = launch::choice_of(&state.mstore, &agent.id);
    let route = req.route.as_deref().map(str::trim);
    let model = req.model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    if route == Some("") {
        if model.is_some() {
            return Err(bad("route \"\" leaves the model route; it takes no model"));
        }
        if current.is_none() {
            return Err(bad("the agent is not on a model route"));
        }
        let harness = state.id_store.resolve_effective_provider_id(agent);
        switch::agent_switch_allowed(&agent.id).map_err(bad)?;
        switch::move_to(&state.mstore, &agent.id, &harness, Target::OwnLogin, |_| true).map_err(bad)?;
        switch::count_agent_switch(&agent.id);
        restart_on_new_model(state, agent);
        return Ok(vec!["route"]);
    }
    let Some(current) = current else {
        return Err(bad("the agent is not on a model route, and putting one on a route spends the key in the Vault, which the person approves when the agent is made (Agents spawn or create with route) or in the model panel"));
    };
    let target = route.unwrap_or(&current.route);
    if target != current.route {
        return Err(bad(format!("the agent is on the {} route; moving it to {target} spends another key, which the person approves in the model panel", current.route)));
    }
    let harness = state.id_store.resolve_effective_provider_id(agent);
    let wanted = model.unwrap_or(&current.model);
    if wanted == current.model {
        return Ok(Vec::new());
    }
    switch::agent_switch_allowed(&agent.id).map_err(bad)?;
    let key_present = |r: &crate::backend::model_gateway::catalog::Route| crate::backend::vault::route_key::present(&state.identity_store, r.key_label);
    switch::move_to(&state.mstore, &agent.id, &harness, Target::Route { route: target, model: wanted }, key_present).map_err(bad)?;
    switch::count_agent_switch(&agent.id);
    restart_on_new_model(state, agent);
    Ok(vec!["model"])
}

/// The choice is read at every launch, so the agent's panes are restarted, with their conversation, at the end of
/// the turn they are in. A pane that cannot be restarted starts on it the next time it opens.
fn restart_on_new_model(state: &AppState, agent: &AgentDefinition) {
    let (_, failed) = super::agent_model::restart_panes(state, &agent.id);
    for why in failed {
        tracing::warn!(agent = %agent.slug, why, "Agents update: a pane was not restarted on the new model");
    }
}

/// What `spend_cap_usd` and `reset_spend_today` of `Agents update` ask for, once allowed.
struct SpendPlan {
    /// The new cap (`Some(None)`: no cap); absent when the request leaves it.
    cap: Option<Option<f64>>,
    reset: bool,
}

/// Checks `spend_cap_usd` and `reset_spend_today` before anything else in the request changes. The leader of an agent
/// sets and resets its cap; an agent may only lower its own, and not reset it, since a cap it could lift or clear would
/// stop nothing.
fn spend_plan(state: &AppState, actor_uid: &str, target_uid: &str, req: &UpdateRequest) -> Result<SpendPlan, Refusal> {
    use crate::backend::model_gateway::cap;
    let own = actor_uid == target_uid;
    let led = crate::backend::agent_seat::effective_seat_by_id(&state.mstore, target_uid).and_then(|s| s.lead).is_some_and(|lead| lead == actor_uid);
    let mut plan = SpendPlan { cap: None, reset: false };
    if let Some(value) = req.spend_cap_usd.as_ref().filter(|v| !v.is_null()) {
        let to = cap::parse_request(value).map_err(bad)?;
        let from = cap::agent_cap(&state.mstore, target_uid);
        if own {
            if !to.is_some_and(|to| from.is_none_or(|from| to <= from)) {
                return Err(refuse(StatusCode::FORBIDDEN, "an agent may lower its own spend cap, not raise or lift it; its leader or the person can"));
            }
        } else if !led {
            return Err(refuse(StatusCode::FORBIDDEN, "only an agent's leader sets its spend cap (Agents set_seat names the lead); you can lower your own"));
        }
        plan.cap = Some(to);
    }
    if req.reset_spend_today == Some(true) {
        if !led {
            return Err(refuse(StatusCode::FORBIDDEN, "only an agent's leader resets the spend counted for today"));
        }
        plan.reset = true;
    }
    if (plan.cap.is_some() || plan.reset) && crate::server::spend_watch::on_a_route(state, target_uid).is_err() {
        return Err(bad("the agent is not on a model route, so it has no spend cap (an agent on its own login has plan limits instead)"));
    }
    Ok(plan)
}

/// Does what [`spend_plan`] allowed. Returns what was done, for the audit row.
fn spend_apply(state: &AppState, target_uid: &str, plan: &SpendPlan) -> Result<Vec<String>, Refusal> {
    let internal = |e: String| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents update: {e}"));
    let mut done = Vec::new();
    if let Some(to) = plan.cap {
        let change = crate::server::spend_watch::change_cap(state, target_uid, to).map_err(internal)?;
        let show = |c: Option<f64>| c.map_or("none".to_string(), |c| format!("${c:.2}"));
        done.push(format!("spend cap {} to {}", show(change.from), show(change.to)));
    }
    if plan.reset {
        let removed = crate::server::spend_watch::reset_today(state, target_uid).map_err(internal)?;
        done.push(format!("reset today's spend (${removed:.2} counted)"));
    }
    Ok(done)
}

/// Apply the fields `req` gives to the agent's stored definition. Returns the names of the fields changed, and what
/// the audit row says beyond them.
fn update(state: &AppState, actor_uid: &str, req: &UpdateRequest) -> Result<(String, Vec<&'static str>, Vec<String>), Refusal> {
    let target = target_of(state, &req.agent)?;
    // Before anything is applied: a refused spend change must not leave a model change behind it.
    let plan = spend_plan(state, actor_uid, &target.agent_id, req)?;
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
    if req.route.is_some() || req.model.is_some() {
        changed.extend(route_change(state, &agent, req)?);
    }
    let spend_done = spend_apply(state, &target.agent_id, &plan)?;
    if changed.is_empty() && spend_done.is_empty() {
        return Err(bad("nothing to change; give at least one of description, icon, auto_start, restart_on_crash, idle_timeout_minutes, route, model, spend_cap_usd, reset_spend_today (name: rename; environment: env_set)"));
    }
    // A request that only changes the spend cap leaves the definition as it is.
    if !changed.is_empty() {
        if !state.mstore.agent_def_update(&mut agent).map_err(internal)? {
            return Err(refuse(StatusCode::NOT_FOUND, format!("no agent '{}' on this machine", req.agent)));
        }
        publish_changed(state);
    }
    Ok((target.agent_id, changed, spend_done))
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
            None => return Err(refuse(StatusCode::NOT_FOUND, format!("no bundle {id}; Identity presets lists the bundles"))),
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
    F: FnOnce(&AppState, &str) -> Result<(Value, String), Refusal> + Send + 'static,
{
    let respond = |r: Refusal| (r.status, Json(json!({ "error": r.message }))).into_response();
    let actor: Actor = match actor_of(state, caller, action) {
        Ok(actor) => actor,
        Err(r) => return respond(r),
    };
    let blocking = state.clone();
    let actor_uid = actor.uid().to_string();
    let outcome = tokio::task::spawn_blocking(move || op(&blocking, &actor_uid))
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
    run(&state, caller.as_ref(), "update", named, move |s, actor| {
        let (agent_id, changed, spend_done) = update(s, actor, &req)?;
        let mut parts: Vec<String> = Vec::new();
        if !changed.is_empty() {
            parts.push(format!("changed {} of {agent_id}", changed.join(", ")));
        }
        parts.extend(spend_done.iter().map(|d| format!("{d} of {agent_id}")));
        let note = parts.join("; ");
        let mut changed: Vec<String> = changed.iter().map(|c| c.to_string()).collect();
        changed.extend(spend_done.iter().map(|_| "spend".to_string()));
        Ok((json!({ "agent_id": agent_id, "changed": changed, "note": "a running agent keeps what it launched with until it is launched again; a spend cap applies at its next request" }), note))
    })
    .await
}

pub(crate) async fn handle_agent_bundles(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<BundlesRequest>) -> Response {
    let caller = caller.map(|Extension(c)| c);
    let named = req.agent.trim().to_string();
    run(&state, caller.as_ref(), "bundles", named, move |s, _| {
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
    run(&state, caller.as_ref(), action, named, move |s, _| {
        let body = hide(s, &req, hidden)?;
        let note = format!("{action} template {}", body["template_id"]);
        Ok((body, note))
    })
    .await
}

#[cfg(test)]
#[path = "tests/agent_definition_tests.rs"]
mod tests;
