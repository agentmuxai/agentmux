// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App API handlers for the v1 standalone Skill primitive.
//! skill.list / skill.get / skill.upsert / skill.delete / skill.bind / skill.unbind
//!
//! Plus the Armory-level catalog (skill.catalog.*): global skills only,
//! window-scoped — no `agent_id`, no `check_s1` (mirrors bundle.* auth
//! shape, since the Armory has no agent connection context to gate on).

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_skill_list(engine, state);
    register_skill_get(engine, state);
    register_skill_upsert(engine, state);
    register_skill_delete(engine, state);
    register_skill_bind(engine, state);
    register_skill_unbind(engine, state);
    register_skill_catalog_list(engine, state);
    register_skill_catalog_upsert(engine, state);
    register_skill_catalog_delete(engine, state);
    register_skill_catalog_bind(engine, state);
    register_skill_catalog_list_for_agent(engine, state);
    register_skill_catalog_unbind(engine, state);
    register_skill_catalog_bind_to_bundle(engine, state);
    register_skill_catalog_unbind_from_bundle(engine, state);
    register_skill_catalog_list_for_bundle(engine, state);
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
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_BIND,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
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
                // An agent binding a skill to itself over its own authenticated
                // connection should reach an already-open Stash Skills tab for
                // that agent too — same reactivity as the catalog-tier bind.
                // reagentx P2 on PR #2329.
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "skills:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(json!({ "bound": true })))
            })
        }),
    );
}

fn register_skill_unbind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_UNBIND,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, skill_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.unbind: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                let unbound = wstore.skill_unbind(&req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.unbind: {e}"))?;
                if unbound {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }
                Ok(Some(json!({ "unbound": unbound })))
            })
        }),
    );
}

fn register_skill_catalog_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_LIST,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let skills = wstore.skill_list_global()
                    .map_err(|e| format!("skill.catalog.list: {e}"))?;
                Ok(Some(serde_json::to_value(&skills).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_skill_catalog_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_UPSERT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req {
                    #[serde(default)] id: String,
                    name: String,
                    #[serde(default)] trigger: String,
                    #[serde(default = "default_skill_type")] skill_type: String,
                    #[serde(default)] description: String,
                    #[serde(default)] content: String,
                }
                fn default_skill_type() -> String { "prompt".to_string() }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.upsert: {e}"))?;

                let now = now_ms();
                let (id, created_at) = if req.id.is_empty() {
                    (uuid::Uuid::new_v4().to_string(), now)
                } else {
                    let existing = wstore.skill_get(&req.id)
                        .map_err(|e| format!("skill.catalog.upsert: {e}"))?;
                    match existing {
                        // No agent_id/check_s1 here to verify ownership of a private
                        // skill, so the catalog surface may only create new global
                        // rows or edit ones that are ALREADY global — never promote
                        // a private skill into the catalog by supplying its id.
                        Some(s) if !s.is_global => {
                            return Err("FORBIDDEN: cannot promote a private skill via the catalog".to_string());
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
                    is_global: true,
                    created_at,
                    updated_at: now,
                };
                // Global-scoped uniqueness (not skill_upsert_unique's
                // per-agent check) — same defense-in-depth as the mcp.catalog
                // fix, reagent P1 on #1948.
                wstore.skill_upsert_unique_global(&skill)
                    .map_err(|e| format!("skill.catalog.upsert: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "skills:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(serde_json::to_value(&skill).map_err(|e| e.to_string())?))
            })
        }),
    );
}

// Catalog-tier sibling of register_skill_bind (above) — same DB write and
// "only global skills may be bound" safety check, but no check_s1: the
// Armory's WebSocket never authenticates as an agent (see this file's
// module doc comment), so a check_s1-gated bind can never be satisfied from
// that caller. skill.bind itself is left untouched — it may still be the
// intended surface for an agent binding a skill to itself over its own
// authenticated connection, and loosening its gate would silently widen
// what any authenticated agent connection can do (bind arbitrary skills to
// *other* agent ids), which nobody asked for.
// See docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md §2.4.
fn register_skill_catalog_bind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_BIND,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, skill_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.bind: {e}"))?;
                // Only global skills (or ones already bound) may be bound, so the
                // catalog surface can't be used to bootstrap read access to
                // another agent's private skill — same rule as skill.bind.
                match wstore.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind: {e}"))?
                {
                    None => return Err("skill.catalog.bind: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        if !wstore.skill_is_bound_to(&req.agent_id, &req.skill_id)
                            .map_err(|e| format!("skill.catalog.bind: {e}"))?
                        {
                            return Err("FORBIDDEN: can only bind global skills".to_string());
                        }
                    }
                    Some(_) => {}
                }
                wstore.skill_bind(&req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind: {e}"))?;
                // Lets any other open Stash/Armory view for this agent pick up
                // the new binding without a manual refresh — skill.bind (the
                // check_s1 agent-self-service path) intentionally left alone.
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "skills:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(json!({ "bound": true })))
            })
        }),
    );
}

/// Catalog-tier sibling of `skill.list` (above) — GLOBAL SKILLS ONLY, each
/// annotated with `bound_to_agent` for the caller-supplied `agent_id`, no
/// `check_s1`. AgentStashModal's Skills tab (the per-agent Stash view) runs
/// over the dashboard's connection, which is never agent-authenticated, so
/// `skill.list`'s gate can never be satisfied from that caller — the exact
/// same reasoning as `skill.catalog.bind` above. Deliberately does NOT
/// reuse `skill.list`'s full computation (global + this agent's own
/// private skills): with no `check_s1`, `agent_id` is unverified, and a
/// private skill's `content`/`description`/`trigger` can carry sensitive
/// agent-authored material — returning it for an arbitrary caller-chosen
/// `agent_id` would be an IDOR into every agent's private skills. See
/// `Store::skill_list_global_for_agent`'s doc comment and
/// docs/reports/REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md §2.2.
fn register_skill_catalog_list_for_agent(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_LIST_FOR_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.list_for_agent: {e}"))?;
                let skills = wstore.skill_list_global_for_agent(&req.agent_id)
                    .map_err(|e| format!("skill.catalog.list_for_agent: {e}"))?;
                Ok(Some(serde_json::to_value(&skills).map_err(|e| e.to_string())?))
            })
        }),
    );
}

/// Catalog-tier sibling of `skill.unbind` (above) — same DB write, no
/// `check_s1`. Restricted to global rows only, matching the guard
/// `register_skill_catalog_bind`/`register_skill_catalog_delete` already
/// apply in this file: any window connection can sever any agent's binding
/// to a *global* skill (an intentional, pre-existing trust boundary — bind
/// and delete already have the same or larger blast radius), but a private
/// row's own binding must not be touchable via this no-`check_s1` surface,
/// even though `skill_id` is never exposed by any no-`check_s1` command
/// today — defense in depth over relying solely on UUID secrecy.
/// reagentx P1 on PR #2329 (round 2).
fn register_skill_catalog_unbind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_UNBIND,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, skill_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.unbind: {e}"))?;
                match wstore.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.catalog.unbind: {e}"))?
                {
                    None => return Err("skill.catalog.unbind: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        return Err("FORBIDDEN: can only unbind global skills".to_string());
                    }
                    Some(_) => {}
                }
                let unbound = wstore.skill_unbind(&req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.unbind: {e}"))?;
                if unbound {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }
                Ok(Some(json!({ "unbound": unbound })))
            })
        }),
    );
}

// Bundle-scoped sibling of register_skill_catalog_bind (above) — same DB
// write and "only global skills may be bound" safety check, keyed by
// bundle_id via bundle_skill_bind instead of agent_id/skill_bind.
// Composable model v2, docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md
// (GH issue #2024 item 3).
fn register_skill_catalog_bind_to_bundle(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_BIND_TO_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { bundle_id: String, skill_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.bind_to_bundle: {e}"))?;
                // Only global skills (or ones already bound) may be bound, so
                // this surface can't be used to bootstrap read access to
                // another entity's private skill — same rule as
                // skill.catalog.bind.
                match wstore.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind_to_bundle: {e}"))?
                {
                    None => return Err("skill.catalog.bind_to_bundle: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        if !wstore.bundle_skill_is_accessible_to(&req.bundle_id, &req.skill_id)
                            .map_err(|e| format!("skill.catalog.bind_to_bundle: {e}"))?
                        {
                            return Err("FORBIDDEN: can only bind global skills to a bundle".to_string());
                        }
                    }
                    Some(_) => {}
                }
                wstore.bundle_skill_bind(&req.bundle_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind_to_bundle: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "skills:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(json!({ "bound": true })))
            })
        }),
    );
}

/// Bundle-scoped sibling of `skill.list` — GLOBAL SKILLS ONLY plus this
/// bundle's own referenced skills, each annotated with `bound_to_bundle`.
/// No `check_s1` (a bundle has no agent identity to gate on). Deliberately
/// restricted to global + bundle-bound rows only, same IDOR reasoning as
/// `skill_list_global_for_agent`'s doc comment — see `bundle_skill_list`'s
/// own implementation.
fn register_skill_catalog_list_for_bundle(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_LIST_FOR_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { bundle_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.list_for_bundle: {e}"))?;
                let skills = wstore.bundle_skill_list(&req.bundle_id)
                    .map_err(|e| format!("skill.catalog.list_for_bundle: {e}"))?;
                Ok(Some(serde_json::to_value(&skills).map_err(|e| e.to_string())?))
            })
        }),
    );
}

// Bundle-scoped sibling of register_skill_catalog_unbind (above) — same
// "global rows only" guard for the same defense-in-depth reasoning.
fn register_skill_catalog_unbind_from_bundle(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_UNBIND_FROM_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { bundle_id: String, skill_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.unbind_from_bundle: {e}"))?;
                match wstore.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.catalog.unbind_from_bundle: {e}"))?
                {
                    None => return Err("skill.catalog.unbind_from_bundle: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        return Err("FORBIDDEN: can only unbind global skills from a bundle".to_string());
                    }
                    Some(_) => {}
                }
                let unbound = wstore.bundle_skill_unbind(&req.bundle_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.unbind_from_bundle: {e}"))?;
                if unbound {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }
                Ok(Some(json!({ "unbound": unbound })))
            })
        }),
    );
}

fn register_skill_catalog_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_SKILL_CATALOG_DELETE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("skill.catalog.delete: {e}"))?;
                if let Some(existing) = wstore.skill_get(&req.id)
                    .map_err(|e| format!("skill.catalog.delete: {e}"))?
                {
                    if !existing.is_global {
                        return Err("FORBIDDEN: cannot delete a private skill via the catalog".to_string());
                    }
                }
                let deleted = wstore.skill_delete(&req.id)
                    .map_err(|e| format!("skill.catalog.delete: {e}"))?;
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
