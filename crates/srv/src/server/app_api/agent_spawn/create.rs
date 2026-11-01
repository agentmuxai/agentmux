// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/create`, the `Agents` tool's `create`: a named agent made from a seeded template, the way the
//! new-agent picker makes one (`agentdefcreatefromtemplate`), so an agent can add to My Agents without the operator's
//! hands. It is not a worker: it has no leader, takes no place among the caller's live workers, can be given a seat
//! (`Agents set_seat`) and is started with `Agents open`.
//!
//! Where it runs is asked for as in `spawn` (`container::decide`): `runtime`, `container_image` and container
//! limits. Making an agent starts nothing and spends nothing, but it adds an agent the user will see, so it asks the
//! user as `spawn` does, unless the user's own turn asked for it.

use agentmux_common::api_types::CreateAgentRequest;

use super::container::{self, Placement};
use super::*;
use crate::backend::rpc_types::CommandAgentDefCreateFromTemplateData;

pub(crate) async fn handle_agent_create(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<CreateAgentRequest>,
) -> Response {
    match create(&state, caller.as_deref(), req).await {
        Ok(body) => Json(body).into_response(),
        Err(e) => (e.status, Json(json!({ "error": e.message }))).into_response(),
    }
}

/// The seeded template `wanted` names: its id, slug or name.
pub(super) fn find_template(state: &AppState, wanted: &str) -> Result<AgentDefinition, SpawnError> {
    let wanted = wanted.trim();
    let all = state
        .mstore
        .agent_def_list()
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent.create: {e}")))?;
    let templates: Vec<AgentDefinition> = all.into_iter().filter(|d| d.is_seeded == 1).collect();
    templates
        .iter()
        .find(|t| t.id == wanted || t.slug.eq_ignore_ascii_case(wanted) || t.name.eq_ignore_ascii_case(wanted))
        .cloned()
        .ok_or_else(|| {
            let known: Vec<String> = templates.iter().map(|t| t.slug.clone()).collect();
            refuse(StatusCode::NOT_FOUND, format!("UNKNOWN_TEMPLATE: no template {wanted:?}; the templates are: {}", known.join(", ")))
        })
}

fn message(leader: &str, name: &str, template: &str, placement: &Placement, account: Option<&WorkerAccount>) -> String {
    let on = account.map(|a| format!(" on the account {}", one_line(&a.label, 80))).unwrap_or_default();
    format!(
        "{} wants to add a new agent to My Agents: {}, from the template {}{}{on}. It is not started until someone starts it. Add it?",
        one_line(leader, 60),
        one_line(name, 60),
        one_line(template, 60),
        placement.describe(),
    )
}

async fn create(state: &AppState, caller: Option<&crate::server::caller::Caller>, req: CreateAgentRequest) -> Result<serde_json::Value, SpawnError> {
    let leader = leader_of(state, caller, &req.auth)?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: a name is required, at most {MAX_NAME_CHARS} characters, on one line")));
    }
    let template = find_template(state, &req.template)?;
    let placement = container::decide_here(state, &req.container, &leader.def, &template).await?;
    let provider = state.id_store.resolve_effective_provider_id(&template);
    // An account is linked when one is named, or the caller's can be used; an agent with none is bound at its first open.
    let account = match choose_account(state, &leader, &provider, req.account.as_deref()) {
        Ok(account) => Some(account),
        Err(e) if req.account.is_some() => return Err(e),
        Err(_) => None,
    };

    let provenance = blockcontroller::get_controller(&leader.block_id).and_then(|c| c.turn_provenance());
    let asked_user = if crate::sagas::self_quit::gate(provenance.as_ref(), &req.user_instruction).is_err() {
        tracing::info!(leader = %leader.def.name, "agent.create: asking the user");
        let question = json!({
            "kind": "consent",
            "title": format!("{} wants to add an agent", one_line(&leader.def.name, 60)),
            "message": message(&leader.def.name, name, &template.name, &placement, account.as_ref()),
            "checkbox": "",
            "ok_label": "Add it",
            "cancel_label": "Don't",
        });
        ask_consent(state, &leader, question, "an agent", "the agent").await?
    } else {
        false
    };

    make(state, &leader, &template, name, &placement, account.as_ref(), asked_user)
}

/// Make the agent, as `placement` says, and link its account; removed again if it can't be finished.
pub(super) fn make(
    state: &AppState,
    leader: &Leader,
    template: &AgentDefinition,
    name: &str,
    placement: &Placement,
    account: Option<&WorkerAccount>,
    asked_user: bool,
) -> Result<serde_json::Value, SpawnError> {
    let provider = state.id_store.resolve_effective_provider_id(template);
    let cmd = CommandAgentDefCreateFromTemplateData {
        template_id: template.id.clone(),
        name: name.to_string(),
        identity_id: None,
        memory_id: None,
        bundle_ids: None,
        agent_type: None,
        model_vendor_base_url: None,
        container_image: None,
    };
    let made = crate::server::agent_handlers::create_from_template(&state.mstore, &state.id_store, &state.broker, cmd).map_err(|e| {
        let status = if e.contains("already exists") || e.contains("still addresses") { StatusCode::CONFLICT } else { StatusCode::INTERNAL_SERVER_ERROR };
        refuse(status, e)
    })?;
    let mut def = state
        .mstore
        .agent_def_get(&made.definition_id)
        .ok()
        .flatten()
        .ok_or_else(|| refuse(StatusCode::INTERNAL_SERVER_ERROR, "the new agent is missing right after it was made"))?;
    let finished = container::apply(state, &mut def, placement).and_then(|()| match account {
        Some(a) => state.identity_store.agent_identity_link(&def.id, &a.id, &a.provider).map_err(|e| format!("link its account: {e}")),
        None => Ok(()),
    });
    if let Err(e) = finished {
        discard_created(state, &def.id);
        return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("the agent couldn't be set up, and was removed: {e}")));
    }
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);

    let (runtime, container_image) = runtime_of(&def);
    let (memory_mb, cpus) = state.mstore.container_limits_get(&def.id).ok().flatten().unwrap_or_default();
    tracing::info!(leader = %leader.def.name, agent = %def.name, template = %template.slug, %runtime, asked_user, "agent.create: an agent was made");
    Ok(json!({
        "agent_id": def.id,
        "name": def.name,
        "slug": def.slug,
        "message_to": def.slug,
        "template": template.slug,
        "provider": provider,
        "runtime": runtime,
        "container_image": container_image,
        "limits": if runtime == "container" { json!({ "memory_mb": memory_mb, "cpus": cpus }) } else { serde_json::Value::Null },
        "account": account.map(|a| a.label.clone()),
        "account_note": account.and_then(|a| a.note.clone()),
        "asked_user": asked_user,
        "note": "created, not started: Agents open starts it, and Agents set_seat gives it a seat",
    }))
}

/// Remove an agent whose set-up failed after it was made, as the `deleteagent` RPC deletes one.
fn discard_created(state: &AppState, def_id: &str) {
    if let Err(e) = state.mstore.agent_def_delete(def_id) {
        tracing::warn!(def_id, error = %e, "agent.create: the unfinished agent's definition is left behind");
        return;
    }
    crate::server::agent_handlers::purge_identity_store_rows(&state.identity_store, &state.id_store, def_id, "agent.create");
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
}
