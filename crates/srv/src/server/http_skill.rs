// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/skill`: an agent manages the skill catalog (list, get, upsert, delete, bind,
//! unbind, reach), the same operations Memory > Skills offers a person. Backs the `Skills` MCP tool.
//! The request carries the agent's own token; every change is recorded as that agent.

use super::*;
use crate::backend::storage::Skill;

#[derive(serde::Deserialize)]
pub(super) struct AgentSkillRequest {
    pub(super) agent_id: String,
    pub(super) action: String,
    #[serde(default)]
    pub(super) skill_id: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) trigger: Option<String>,
    #[serde(default)]
    pub(super) skill_type: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) content: Option<String>,
    /// bind, unbind: the agent's id; default the calling agent.
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) all_agents: Option<bool>,
    #[serde(default)]
    pub(super) new_agents: Option<bool>,
}

pub(super) async fn handle_agent_skill(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Json(req): Json<AgentSkillRequest>,
) -> impl IntoResponse {
    let (slug, uid) = match actor::require_token_actor(&state, caller.as_deref(), actor::ActorSite::SkillManage, &req.agent_id).await {
        Ok(who) => who,
        Err(e) => return app_api_response::<()>(Err(e)),
    };
    let result = tokio::task::spawn_blocking(move || run(&state, &slug, &uid, req))
        .await
        .unwrap_or_else(|e| Err(format!("skill: {e}")));
    app_api_response(result)
}

/// A skill as the tool shows it: the text only when asked for (`get`).
fn summary(skill: &Skill, bound_count: i64, all_agents: bool, new_agents: bool, with_content: bool) -> serde_json::Value {
    let mut v = json!({
        "id": skill.id,
        "name": skill.name,
        "skill_type": skill.skill_type,
        "trigger": skill.trigger,
        "description": skill.description,
        "bound_count": bound_count,
        "all_agents": all_agents,
        "new_agents": new_agents,
    });
    if with_content {
        v["content"] = json!(skill.content);
    }
    v
}

fn global_skill(state: &AppState, id: &str) -> Result<Skill, String> {
    if id.is_empty() {
        return Err("skill: skill_id is required".to_string());
    }
    match state.identity_store.skill_get(id).map_err(|e| format!("skill: {e}"))? {
        Some(s) if s.is_global => Ok(s),
        Some(_) => Err("FORBIDDEN: only catalog skills can be managed here, not an agent's private skill".to_string()),
        None => Err(format!("skill: no skill {id}")),
    }
}

fn run(state: &AppState, slug: &str, uid: &str, req: AgentSkillRequest) -> Result<serde_json::Value, String> {
    let (mstore, catalog) = (&state.mstore, &state.identity_store);
    let changed = || state.broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
    let target = || req.target.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| uid.to_string());
    match req.action.as_str() {
        "list" => {
            let items = mstore.skill_list_global(catalog).map_err(|e| format!("skill: {e}"))?;
            Ok(json!(items
                .iter()
                .map(|i| summary(&i.skill, i.bound_count, i.all_agents, i.new_agents, false))
                .collect::<Vec<_>>()))
        }
        "get" => {
            let skill = global_skill(state, &req.skill_id)?;
            let items = mstore.skill_list_global(catalog).map_err(|e| format!("skill: {e}"))?;
            let item = items.iter().find(|i| i.skill.id == skill.id);
            Ok(summary(
                &skill,
                item.map_or(0, |i| i.bound_count),
                item.is_some_and(|i| i.all_agents),
                item.is_some_and(|i| i.new_agents),
                true,
            ))
        }
        "upsert" => {
            let now = agentmux_common::time::now_ms();
            let existing = if req.skill_id.is_empty() { None } else { global_skill_or_new(state, &req.skill_id)? };
            let is_new = existing.is_none();
            let id = match &existing {
                Some(s) => s.id.clone(),
                None if req.skill_id.is_empty() => uuid::Uuid::new_v4().to_string(),
                None => req.skill_id.clone(),
            };
            // An edit keeps what the request leaves out; a new skill needs a name.
            let name = match (&req.name, &existing) {
                (Some(n), _) if !n.trim().is_empty() => n.trim().to_string(),
                (_, Some(s)) => s.name.clone(),
                _ => return Err("skill: name is required".to_string()),
            };
            let pick = |given: &Option<String>, old: Option<&String>, default: &str| {
                given.clone().or_else(|| old.cloned()).unwrap_or_else(|| default.to_string())
            };
            let skill = Skill {
                id: id.clone(),
                name,
                trigger: pick(&req.trigger, existing.as_ref().map(|s| &s.trigger), ""),
                skill_type: pick(&req.skill_type, existing.as_ref().map(|s| &s.skill_type), "agent-skill"),
                description: pick(&req.description, existing.as_ref().map(|s| &s.description), ""),
                content: pick(&req.content, existing.as_ref().map(|s| &s.content), ""),
                is_global: true,
                created_at: existing.as_ref().map_or(now, |s| s.created_at),
                updated_at: now,
            };
            catalog.skill_upsert_unique_global(&skill).map_err(|e| format!("skill: {e}"))?;
            if is_new {
                catalog.skill_reach_default_for_new(&id, now).map_err(|e| format!("skill: {e}"))?;
            }
            tracing::info!(agent = %slug, skill = %id, "skill: upserted by an agent");
            changed();
            Ok(json!({ "id": id, "name": skill.name }))
        }
        "delete" => {
            let skill = global_skill(state, &req.skill_id)?;
            let deleted = mstore.skill_delete(catalog, &skill.id).map_err(|e| format!("skill: {e}"))?;
            tracing::info!(agent = %slug, skill = %skill.id, "skill: deleted by an agent");
            if deleted {
                changed();
            }
            Ok(json!({ "deleted": deleted }))
        }
        "bind" => {
            let skill = global_skill(state, &req.skill_id)?;
            mstore.skill_bind(catalog, &target(), &skill.id).map_err(|e| format!("skill: {e}"))?;
            changed();
            Ok(json!({ "bound": true }))
        }
        "unbind" => {
            let skill = global_skill(state, &req.skill_id)?;
            let unbound = mstore.skill_unbind(&target(), &skill.id).map_err(|e| format!("skill: {e}"))?;
            if unbound {
                changed();
            }
            Ok(json!({ "unbound": unbound }))
        }
        "reach" => {
            let skill = global_skill(state, &req.skill_id)?;
            let current = catalog.skill_reach_map().map_err(|e| format!("skill: {e}"))?.get(&skill.id).copied().unwrap_or_default();
            let all_agents = req.all_agents.unwrap_or(current.all_agents);
            let new_agents = req.new_agents.unwrap_or(current.new_agents_since > 0 || all_agents);
            catalog.skill_set_reach(&skill.id, all_agents, new_agents, agentmux_common::time::now_ms()).map_err(|e| format!("skill: {e}"))?;
            tracing::info!(agent = %slug, skill = %skill.id, all_agents, new_agents, "skill: reach changed by an agent");
            changed();
            Ok(json!({ "all_agents": all_agents, "new_agents": new_agents }))
        }
        other => Err(format!("skill: unknown action {other:?} (list, get, upsert, delete, bind, unbind, reach)")),
    }
}

/// `Some(skill)` for an existing catalog skill, `None` for an id nothing has used yet (a caller may
/// choose its own id); an agent's private skill is refused.
fn global_skill_or_new(state: &AppState, id: &str) -> Result<Option<Skill>, String> {
    match state.identity_store.skill_get(id).map_err(|e| format!("skill: {e}"))? {
        Some(s) if s.is_global => Ok(Some(s)),
        Some(_) => Err("FORBIDDEN: cannot promote a private skill via the catalog".to_string()),
        None => Ok(None),
    }
}
