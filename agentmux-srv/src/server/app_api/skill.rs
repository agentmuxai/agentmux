// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App API handlers for the v1 standalone Skill primitive.
//! skill.list / skill.get / skill.upsert / skill.delete / skill.bind / skill.unbind

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_skill_list(engine, state);
    register_skill_get(engine, state);
    register_skill_upsert(engine, state);
    register_skill_delete(engine, state);
    register_skill_bind(engine, state);
    register_skill_unbind(engine, state);
}

fn register_skill_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_SKILL_LIST,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.list: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                let skills = wstore.skill_list(&req.agent_id)
                    .map_err(|e| format!("skill.list: {e}"))?;
                Ok(Some(serde_json::to_value(&skills).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_skill_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_SKILL_GET,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.get: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                if !wstore.skill_is_accessible_to(&req.agent_id, &req.id)
                    .map_err(|e| format!("skill.get: {e}"))?
                {
                    return Err("FORBIDDEN: skill not accessible to this agent".to_string());
                }
                let skill = wstore.skill_get(&req.id)
                    .map_err(|e| format!("skill.get: {e}"))?;
                Ok(Some(serde_json::to_value(&skill).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_skill_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_UPSERT,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req {
                    agent_id: String,
                    #[serde(default)] id: String,
                    name: String,
                    #[serde(default)] trigger: String,
                    #[serde(default = "default_skill_type")] skill_type: String,
                    #[serde(default)] description: String,
                    #[serde(default)] content: String,
                }
                fn default_skill_type() -> String { "prompt".to_string() }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.upsert: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;

                let now = now_ms();
                // created_at is preserved across updates so the response never
                // reports a timestamp the DB won't persist (ON CONFLICT keeps
                // the original). For a new row it defaults to `now`.
                let (id, created_at) = if req.id.is_empty() {
                    (uuid::Uuid::new_v4().to_string(), now)
                } else {
                    if !wstore.skill_is_bound_to(&req.agent_id, &req.id)
                        .map_err(|e| format!("skill.upsert: {e}"))?
                    {
                        return Err("FORBIDDEN: skill not bound to this agent".to_string());
                    }
                    let existing = wstore.skill_get(&req.id)
                        .map_err(|e| format!("skill.upsert: {e}"))?;
                    match existing {
                        Some(s) if s.is_global => {
                            return Err("FORBIDDEN: cannot mutate a global skill".to_string());
                        }
                        Some(s) => (req.id.clone(), s.created_at),
                        None => (req.id.clone(), now),
                    }
                };

                let skill = crate::backend::storage::Skill {
                    id: id.clone(),
                    name: req.name,
                    trigger: req.trigger,
                    skill_type: req.skill_type,
                    description: req.description,
                    content: req.content,
                    is_global: false,
                    created_at,
                    updated_at: now,
                };
                // Atomic: name-uniqueness check + upsert + (bind on create) in one
                // transaction, so concurrent same-name upserts can't both pass.
                wstore.skill_upsert_unique(&req.agent_id, &skill, req.id.is_empty())
                    .map_err(|e| format!("skill.upsert: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "skills:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(serde_json::to_value(&skill).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_skill_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_DELETE,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.delete: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                if !wstore.skill_is_bound_to(&req.agent_id, &req.id)
                    .map_err(|e| format!("skill.delete: {e}"))?
                {
                    return Err("FORBIDDEN: skill not bound to this agent".to_string());
                }
                if let Some(existing) = wstore.skill_get(&req.id)
                    .map_err(|e| format!("skill.delete: {e}"))?
                {
                    if existing.is_global {
                        return Err("FORBIDDEN: cannot delete a global skill".to_string());
                    }
                }
                let deleted = wstore.skill_delete(&req.id)
                    .map_err(|e| format!("skill.delete: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }
                Ok(Some(json!({ "deleted": deleted })))
            })
        }),
    );
}

fn register_skill_bind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_SKILL_BIND,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, skill_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.bind: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                // Only global skills (or ones already bound) may be bound, so an
                // agent can't bootstrap read access to another agent's private skill.
                match wstore.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.bind: {e}"))?
                {
                    None => return Err("skill.bind: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        if !wstore.skill_is_bound_to(&req.agent_id, &req.skill_id)
                            .map_err(|e| format!("skill.bind: {e}"))?
                        {
                            return Err("FORBIDDEN: can only bind global skills".to_string());
                        }
                    }
                    Some(_) => {}
                }
                wstore.skill_bind(&req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.bind: {e}"))?;
                Ok(Some(json!({ "bound": true })))
            })
        }),
    );
}

fn register_skill_unbind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_SKILL_UNBIND,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, skill_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.unbind: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                let unbound = wstore.skill_unbind(&req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.unbind: {e}"))?;
                Ok(Some(json!({ "unbound": unbound })))
            })
        }),
    );
}
