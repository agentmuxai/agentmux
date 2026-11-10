// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The harness, model route and model a new agent is asked to run on, for `spawn`, `spawn_many` and `create`
//! (`docs/specs/SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md` sections 3 and 5). The rules are
//! `model_gateway::ask`'s; this adds what a spawn knows: the leader, its remembered runtime and the Vault.

use super::*;
use crate::backend::model_gateway::launch::{self, Choice};
use crate::backend::model_gateway::{ask, catalog::Route};
use crate::backend::vault::route_key;
use agentmux_common::api_types::RouteAsk;

/// Whether the Vault holds a route's key.
pub(super) fn key_present(state: &AppState) -> impl Fn(&Route) -> bool + '_ {
    move |route| route_key::present(&state.identity_store, route.key_label)
}

/// The harness the new agent runs: the one asked for, else its leader's.
fn harness_of(leader: &str, asked: Option<&str>) -> Result<String, SpawnError> {
    let Some(asked) = asked.map(str::trim).filter(|h| !h.is_empty()) else { return Ok(leader.to_string()) };
    let id = crate::backend::providers::resolve_provider_alias(asked);
    if crate::backend::providers::get_provider(id).is_none() {
        let known: Vec<&str> = crate::backend::providers::all_providers().map(|p| p.id).collect();
        return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: unknown harness {asked:?}; the harnesses are {}", known.join(", "))));
    }
    Ok(id.to_string())
}

/// The harness and runtime of a new worker: its model and effort over its leader's, on the leader's route unless
/// `ask` names another. A worker on a route keeps the leader's mode and effort and takes the route's model; a worker
/// on another harness takes none of its leader's remembered runtime, which is the leader's harness's.
pub(super) fn choose(state: &AppState, leader: &Leader, ask_route: &RouteAsk, model: Option<&str>, effort: Option<&str>) -> Result<(String, WorkerRuntime), SpawnError> {
    let leader_harness = state.id_store.resolve_effective_provider_id(&leader.def);
    let harness = harness_of(&leader_harness, ask_route.harness.as_deref())?;
    let same = harness == leader_harness;
    let inherited = if same { launch::choice_of(&state.mstore, &leader.def.id) } else { None };
    let bad = |e: String| {
        let status = if e.starts_with("KEY_MISSING") { StatusCode::CONFLICT } else { StatusCode::BAD_REQUEST };
        refuse(status, if e.starts_with("KEY_MISSING") { e } else { format!("INVALID_ARG: {e}") })
    };
    let picked = ask::resolve(&harness, ask_route.route.as_deref(), model, inherited.as_ref(), key_present(state)).map_err(bad)?;
    let leader_runtime = if same { state.mstore.agent_last_runtime_get(&leader.def.id).unwrap_or_default() } else { String::new() };
    // A worker on a route runs the route's model; the leader's own Claude model is not its to inherit. The harness
    // sizes its requests by the model it was started with, and a Claude one it takes per-turn effort for sends
    // `configuration_update` blocks the route's model answers with a 400 on the first turn.
    let leader_runtime = if picked.is_some() { without_model(&leader_runtime) } else { leader_runtime };
    let mut runtime = worker_runtime(&harness, &leader_runtime, if picked.is_some() { None } else { model }, effort)
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {e}")))?;
    if let Some(p) = &picked {
        let choice = Choice { route: p.route.id.to_string(), model: p.model.id.to_string() };
        let named = ask_route.route.as_deref().is_some_and(|r| !r.trim().is_empty());
        runtime.route_asked = named || inherited.as_ref() != Some(&choice);
        runtime.route = Some(choice);
        runtime.route_line = ask::consent_line(p);
    }
    if !same {
        runtime.harness = Some(harness.clone());
    }
    Ok((harness, runtime))
}

/// A remembered runtime (`agentlastruntime`) without its model; the other fields, and anything unreadable, as they are.
fn without_model(runtime: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(runtime) {
        Ok(serde_json::Value::Object(mut o)) => {
            o.remove("model");
            serde_json::Value::Object(o).to_string()
        }
        _ => runtime.to_string(),
    }
}

/// The account a new worker runs on. A worker on a route is billed to the route's key and signs in nowhere, so it
/// has none; its id is empty and nothing is linked.
pub(super) fn account(state: &AppState, leader: &Leader, harness: &str, runtime: &WorkerRuntime, requested: Option<&str>) -> Result<WorkerAccount, SpawnError> {
    if runtime.route.is_some() {
        if requested.is_some_and(|r| !r.trim().is_empty()) {
            return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: an agent on a model route has no account; leave account out"));
        }
        return Ok(WorkerAccount { id: String::new(), label: String::new(), provider: harness.to_string(), note: None });
    }
    choose_account(state, leader, harness, requested)
}

/// Gives the new worker its harness and route, as `runtime` says: srv writes both, so an agent cannot give itself a
/// route by writing its own definition (`written_only_by_srv`).
pub(super) fn apply(state: &AppState, worker: &mut AgentDefinition, runtime: &WorkerRuntime) -> Result<(), String> {
    if let Some(harness) = &runtime.harness {
        worker.provider = harness.clone();
        worker.provider_flags = String::new();
        worker.model_vendor_base_url = String::new();
        state.mstore.agent_def_update(worker).map_err(|e| format!("give it the {harness} harness: {e}"))?;
    }
    if let Some(choice) = &runtime.route {
        launch::set_choice(&state.mstore, &worker.id, &worker.provider, &choice.route, &choice.model).map_err(|e| format!("give it its model route: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_harness_that_is_not_one_is_refused_with_the_list() {
        assert_eq!(harness_of("claude", None).unwrap(), "claude");
        assert_eq!(harness_of("claude", Some(" codex ")).unwrap(), "codex");
        let e = harness_of("claude", Some("no-such-cli")).unwrap_err();
        assert!(e.message.contains("claude") && e.message.contains("no-such-cli"), "{}", e.message);
    }
}
