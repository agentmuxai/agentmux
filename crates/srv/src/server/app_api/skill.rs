// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! MuxAPI handlers for the v1 standalone Skill primitive.
//! skill.list / skill.get / skill.upsert / skill.delete / skill.bind / skill.unbind
//!
//! Plus the Memory-level catalog (skill.catalog.*): global skills only,
//! window-scoped — no `agent_id`, no `check_s1` (mirrors bundle.* auth
//! shape, since Memory has no agent connection context to gate on).
//!
//! `db_skills` is authoritatively `identity_store` as of Phase 2 of
//! SPEC_DURABLE_BINDINGS_2026_09_10.md (§5.1/§5.4) — `mstore` stays the
//! ref-table owner (`db_agent_skills_ref`/`db_bundle_skills_ref` are not
//! promoted until Phase 3/6). Pure single-table reads/writes (`skill_get`,
//! `skill_upsert_unique_global`) call directly on `identity_store`; methods
//! that combine a ref table with the catalog now take `identity_store` as an
//! explicit `catalog` parameter while still being called ON `mstore` (the
//! ref-owning receiver).

use super::*;
use crate::backend::storage::skill_source::SkillSource;
use crate::backend::storage::skills::SkillCatalogItem;

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
    register_skill_catalog_set_reach(engine, state);
    register_skill_catalog_bind_all(engine, state);
    register_skill_catalog_import_preview(engine, state);
    register_skill_catalog_import_commit(engine, state);
    register_skill_catalog_bind_to_bundle(engine, state);
    register_skill_catalog_unbind_from_bundle(engine, state);
    register_skill_catalog_list_for_bundle(engine, state);
    register_skill_catalog_upsert_for_bundle(engine, state);
}

fn register_skill_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_SKILL_LIST,
        move |req: SkillAgentScopeData, ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move {
                check_s1(&mstore, &ctx, &req.agent_id)?;
                let skills = mstore.skill_list(&identity_store, &req.agent_id)
                    .map_err(|e| format!("skill.list: {e}"))?;
                Ok(skills)
            }
        },
    );
}

fn register_skill_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_SKILL_GET,
        move |req: SkillAgentItemData, ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move {
                check_s1(&mstore, &ctx, &req.agent_id)?;
                if !mstore.skill_is_accessible_to(&identity_store, &req.agent_id, &req.id)
                    .map_err(|e| format!("skill.get: {e}"))?
                {
                    return Err("FORBIDDEN: skill not accessible to this agent".to_string());
                }
                let skill = identity_store.skill_get(&req.id)
                    .map_err(|e| format!("skill.get: {e}"))?;
                Ok(skill)
            }
        },
    );
}

fn register_skill_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_UPSERT,
        move |req: CommandSkillUpsertData, ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                check_s1(&mstore, &ctx, &req.agent_id)?;

                let now = now_ms();
                // created_at is preserved across updates so the response never
                // reports a timestamp the DB won't persist (ON CONFLICT keeps
                // the original). For a new row it defaults to `now`.
                let (id, created_at) = if req.id.is_empty() {
                    (uuid::Uuid::new_v4().to_string(), now)
                } else {
                    if !mstore.skill_is_bound_to(&req.agent_id, &req.id)
                        .map_err(|e| format!("skill.upsert: {e}"))?
                    {
                        return Err("FORBIDDEN: skill not bound to this agent".to_string());
                    }
                    let existing = identity_store.skill_get(&req.id)
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
                // No longer a single transaction across the catalog insert and
                // the ref-bind (Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md
                // §5.4, accepted interim gap) — see
                // `Store::managed_upsert_unique`'s doc comment.
                mstore.skill_upsert_unique(&identity_store, &req.agent_id, &skill, req.id.is_empty())
                    .map_err(|e| format!("skill.upsert: {e}"))?;
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(skill)
            }
        },
    );
}

fn register_skill_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_DELETE,
        move |req: SkillAgentItemData, ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                check_s1(&mstore, &ctx, &req.agent_id)?;
                if !mstore.skill_is_bound_to(&req.agent_id, &req.id)
                    .map_err(|e| format!("skill.delete: {e}"))?
                {
                    return Err("FORBIDDEN: skill not bound to this agent".to_string());
                }
                if let Some(existing) = identity_store.skill_get(&req.id)
                    .map_err(|e| format!("skill.delete: {e}"))?
                {
                    if existing.is_global {
                        return Err("FORBIDDEN: cannot delete a global skill".to_string());
                    }
                }
                let deleted = mstore.skill_delete(&identity_store, &req.id)
                    .map_err(|e| format!("skill.delete: {e}"))?;
                if deleted {
                    broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                }
                Ok(SkillDeleteResult { deleted })
            }
        },
    );
}

fn register_skill_bind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_BIND,
        move |req: SkillAgentBindingData, ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                check_s1(&mstore, &ctx, &req.agent_id)?;
                // Only global skills (or ones already bound) may be bound, so an
                // agent can't bootstrap read access to another agent's private skill.
                match identity_store.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.bind: {e}"))?
                {
                    None => return Err("skill.bind: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        if !mstore.skill_is_bound_to(&req.agent_id, &req.skill_id)
                            .map_err(|e| format!("skill.bind: {e}"))?
                        {
                            return Err("FORBIDDEN: can only bind global skills".to_string());
                        }
                    }
                    Some(_) => {}
                }
                mstore.skill_bind(&identity_store, &req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.bind: {e}"))?;
                // An agent binding a skill to itself over its own authenticated
                // connection should reach an already-open Stash Skills tab for
                // that agent too — same reactivity as the catalog-tier bind.
                // reagentx P2 on PR #2329.
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(SkillBindResult { bound: true })
            }
        },
    );
}

fn register_skill_unbind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_UNBIND,
        move |req: SkillAgentBindingData, ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                check_s1(&mstore, &ctx, &req.agent_id)?;
                let unbound = mstore.skill_unbind(&req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.unbind: {e}"))?;
                if unbound {
                    broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                }
                Ok(SkillUnbindResult { unbound })
            }
        },
    );
}

fn register_skill_catalog_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_LIST,
        move |_req: SkillCatalogListData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move {
                let skills = mstore.skill_list_global(&identity_store)
                    .map_err(|e| format!("skill.catalog.list: {e}"))?;
                Ok(skills)
            }
        },
    );
}

fn register_skill_catalog_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_UPSERT,
        move |req: CommandSkillCatalogUpsertData, _ctx| {
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {

                let now = now_ms();
                let (id, created_at) = if req.id.is_empty() {
                    (uuid::Uuid::new_v4().to_string(), now)
                } else {
                    let existing = identity_store.skill_get(&req.id)
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
                // fix, reagent P1 on #1948. Pure single-table op — called
                // directly on identity_store.
                let is_new = created_at == now;
                identity_store.skill_upsert_unique_global(&skill)
                    .map_err(|e| format!("skill.catalog.upsert: {e}"))?;
                // A new catalog skill reaches the agents created after it, unless switched off
                // (the operator's default: new agents get the global skills, with a per-skill opt-out).
                if is_new {
                    identity_store
                        .skill_reach_default_for_new(&id, now)
                        .map_err(|e| format!("skill.catalog.upsert: {e}"))?;
                }
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(skill)
            }
        },
    );
}

// Catalog-tier sibling of register_skill_bind (above) — same DB write and
// "only global skills may be bound" safety check, but no check_s1: the
// Memory's WebSocket never authenticates as an agent (see this file's
// module doc comment), so a check_s1-gated bind can never be satisfied from
// that caller. skill.bind itself is left untouched — it may still be the
// intended surface for an agent binding a skill to itself over its own
// authenticated connection, and loosening its gate would silently widen
// what any authenticated agent connection can do (bind arbitrary skills to
// *other* agent ids), which nobody asked for.
// See docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md §2.4.
fn register_skill_catalog_bind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_BIND,
        move |req: SkillAgentBindingData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                // Only global skills (or ones already bound) may be bound, so the
                // catalog surface can't be used to bootstrap read access to
                // another agent's private skill — same rule as skill.bind.
                match identity_store.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind: {e}"))?
                {
                    None => return Err("skill.catalog.bind: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        if !mstore.skill_is_bound_to(&req.agent_id, &req.skill_id)
                            .map_err(|e| format!("skill.catalog.bind: {e}"))?
                        {
                            return Err("FORBIDDEN: can only bind global skills".to_string());
                        }
                    }
                    Some(_) => {}
                }
                mstore.skill_bind(&identity_store, &req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind: {e}"))?;
                // Lets any other open Stash/Memory view for this agent pick up
                // the new binding without a manual refresh — skill.bind (the
                // check_s1 agent-self-service path) intentionally left alone.
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(SkillBindResult { bound: true })
            }
        },
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
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_LIST_FOR_AGENT,
        move |req: SkillAgentScopeData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move {
                let skills = mstore.skill_list_global_for_agent(&identity_store, &req.agent_id)
                    .map_err(|e| format!("skill.catalog.list_for_agent: {e}"))?;
                Ok(skills)
            }
        },
    );
}

/// `skill.catalog.set_reach`: a global skill reaches every agent, the agents created from now on, or
/// only the agents it is bound to.
/// The switch lives in the identity store beside the catalog (`db_skill_reach`), so it survives an
/// upgrade, like an MCP server's (`mcp.catalog.set_reach`).
fn register_skill_catalog_set_reach(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_SET_REACH,
        move |req: CommandSkillCatalogSetReachData, _ctx| {
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                match identity_store.skill_get(&req.skill_id).map_err(|e| format!("skill.catalog.set_reach: {e}"))? {
                    Some(s) if s.is_global => {}
                    Some(_) => return Err("FORBIDDEN: a private skill has no catalog reach".to_string()),
                    None => return Err(format!("skill.catalog.set_reach: no skill {}", req.skill_id)),
                }
                identity_store
                    .skill_set_reach(&req.skill_id, req.all_agents, req.new_agents, now_ms())
                    .map_err(|e| format!("skill.catalog.set_reach: {e}"))?;
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(SkillBindResult { bound: req.all_agents || req.new_agents })
            }
        },
    );
}

/// `skill.catalog.bind_all`: a bind of one global skill to every agent that exists now. The agents
/// are this channel's own definitions (not the seeded templates), the same ones the per-agent bind
/// accepts.
fn register_skill_catalog_bind_all(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_BIND_ALL,
        move |req: CommandSkillCatalogBindAllData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                match identity_store.skill_get(&req.skill_id).map_err(|e| format!("skill.catalog.bind_all: {e}"))? {
                    Some(s) if s.is_global => {}
                    Some(_) => return Err("FORBIDDEN: can only bind global skills".to_string()),
                    None => return Err("skill.catalog.bind_all: skill not found".to_string()),
                }
                let mut bound: u64 = 0;
                for agent in mstore.agent_def_list().map_err(|e| format!("skill.catalog.bind_all: {e}"))? {
                    if agent.is_seeded == 1 {
                        continue;
                    }
                    // A definition of another channel is not in this one's registry and refuses a bind: skipped.
                    match mstore.skill_bind(&identity_store, &agent.id, &req.skill_id) {
                        Ok(()) => bound += 1,
                        Err(e) => tracing::debug!(agent_id = %agent.id, error = %e, "skill.catalog.bind_all: skipped an agent"),
                    }
                }
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(SkillBindAllResult { bound })
            }
        },
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
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_UNBIND,
        move |req: SkillAgentBindingData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                match identity_store.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.catalog.unbind: {e}"))?
                {
                    None => return Err("skill.catalog.unbind: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        return Err("FORBIDDEN: can only unbind global skills".to_string());
                    }
                    Some(_) => {}
                }
                let unbound = mstore.skill_unbind(&req.agent_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.unbind: {e}"))?;
                if unbound {
                    broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                }
                Ok(SkillUnbindResult { unbound })
            }
        },
    );
}

// Bundle-scoped sibling of register_skill_catalog_bind (above) — same DB
// write and "only global skills may be bound" safety check, keyed by
// bundle_id via bundle_skill_bind instead of agent_id/skill_bind.
// Composable model v2, docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md
// (GH issue #2024 item 3).
fn register_skill_catalog_bind_to_bundle(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_BIND_TO_BUNDLE,
        move |req: SkillBundleBindingData, _ctx| {
            let mstore = mstore.clone();
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                // Only global skills (or ones already bound) may be bound, so
                // this surface can't be used to bootstrap read access to
                // another entity's private skill — same rule as
                // skill.catalog.bind.
                match identity_store.skill_get(&req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind_to_bundle: {e}"))?
                {
                    None => return Err("skill.catalog.bind_to_bundle: skill not found".to_string()),
                    Some(s) if !s.is_global => {
                        if !mstore.bundle_skill_is_accessible_to(&identity_store, &req.bundle_id, &req.skill_id)
                            .map_err(|e| format!("skill.catalog.bind_to_bundle: {e}"))?
                        {
                            return Err("FORBIDDEN: can only bind global skills to a bundle".to_string());
                        }
                    }
                    Some(_) => {}
                }
                mstore.bundle_skill_bind(&identity_store, &id_store, &req.bundle_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.bind_to_bundle: {e}"))?;
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(SkillBindResult { bound: true })
            }
        },
    );
}

/// Bundle-scoped sibling of `skill.list` — GLOBAL SKILLS ONLY plus this
/// bundle's own referenced skills, each annotated with `bound_to_bundle`.
/// No `check_s1` (a bundle has no agent identity to gate on). Deliberately
/// restricted to global + bundle-bound rows only, same IDOR reasoning as
/// `skill_list_global_for_agent`'s doc comment — see `bundle_skill_list`'s
/// own implementation.
fn register_skill_catalog_list_for_bundle(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_LIST_FOR_BUNDLE,
        move |req: SkillBundleScopeData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move {
                let skills = mstore.bundle_skill_list(&identity_store, &req.bundle_id)
                    .map_err(|e| format!("skill.catalog.list_for_bundle: {e}"))?;
                Ok(skills)
            }
        },
    );
}

/// Creates a NEW, PRIVATE skill scoped directly to a bundle (never global)
/// — the bundle-level analog of `skill.upsert` (agent-scoped). See
/// `mcp.rs`'s `register_mcp_catalog_upsert_for_bundle` for the full
/// reasoning (reagentx P1 review on PR #2639).
fn register_skill_catalog_upsert_for_bundle(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_UPSERT_FOR_BUNDLE,
        move |req: CommandSkillCatalogUpsertForBundleData, _ctx| {
            let mstore = mstore.clone();
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {

                let now = now_ms();
                let (id, created_at) = if req.id.is_empty() {
                    (uuid::Uuid::new_v4().to_string(), now)
                } else {
                    if !mstore.bundle_skill_is_bound_to(&req.bundle_id, &req.id)
                        .map_err(|e| format!("skill.catalog.upsert_for_bundle: {e}"))?
                    {
                        return Err("FORBIDDEN: skill not bound to this bundle".to_string());
                    }
                    let existing = identity_store.skill_get(&req.id)
                        .map_err(|e| format!("skill.catalog.upsert_for_bundle: {e}"))?;
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
                mstore.bundle_skill_upsert_unique(&identity_store, &id_store, &req.bundle_id, &skill, req.id.is_empty())
                    .map_err(|e| format!("skill.catalog.upsert_for_bundle: {e}"))?;
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(skill)
            }
        },
    );
}

// Bundle-scoped sibling of register_skill_catalog_unbind (above). See
// mcp.rs's register_mcp_catalog_unbind_from_bundle for why this does NOT
// restrict to global-only, unlike its agent-level sibling.
fn register_skill_catalog_unbind_from_bundle(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_UNBIND_FROM_BUNDLE,
        move |req: SkillBundleBindingData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                let unbound = mstore.bundle_skill_unbind(&req.bundle_id, &req.skill_id)
                    .map_err(|e| format!("skill.catalog.unbind_from_bundle: {e}"))?;
                if unbound {
                    broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                }
                Ok(SkillUnbindResult { unbound })
            }
        },
    );
}

fn register_skill_catalog_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_DELETE,
        move |req: SkillCatalogItemData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                if let Some(existing) = identity_store.skill_get(&req.id)
                    .map_err(|e| format!("skill.catalog.delete: {e}"))?
                {
                    if !existing.is_global {
                        return Err("FORBIDDEN: cannot delete a private skill via the catalog".to_string());
                    }
                }
                let deleted = mstore.skill_delete(&identity_store, &req.id)
                    .map_err(|e| format!("skill.catalog.delete: {e}"))?;
                if deleted {
                    broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                }
                Ok(SkillDeleteResult { deleted })
            }
        },
    );
}


/// What a source holds: a folder, a git repository (https) or one SKILL.md at an https URL.
async fn read_skill_source(src: &SkillSource) -> Result<crate::backend::skill_import::Scan, String> {
    use crate::backend::skill_import as imp;
    match src.kind.as_str() {
        "folder" => imp::scan_dir(&crate::backend::base::expand_home_dir_safe(src.location.trim())),
        "git" => imp::fetch_git(&src.location, &src.git_ref).await,
        "url" => imp::fetch_url(&src.location).await,
        other => Err(format!("unknown source kind '{other}' (use folder, git or url)")),
    }
}

// Read a source and compare each skill in it with the catalog. Writes nothing: the person reads
// the whole text (and the diff for a skill that already exists) before `import_commit`.
fn register_skill_catalog_import_preview(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_IMPORT_PREVIEW,
        move |req: CommandSkillCatalogImportPreviewData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move {
                const E: &str = "skill.catalog.import_preview";
                // An update names a skill; its recorded source says where to read.
                let (source, only_key) = match (&req.source, req.skill_id.is_empty()) {
                    (_, false) => {
                        let src = identity_store
                            .skill_source_get(&req.skill_id)
                            .map_err(|e| format!("{E}: {e}"))?
                            .ok_or_else(|| format!("{E}: this skill was not imported from a source, so there is nothing to check"))?;
                        let key = src.item_key.clone();
                        (src, Some(key))
                    }
                    (Some(src), true) => (src.clone(), None),
                    (None, true) => return Err(format!("{E}: give a source or a skill_id")),
                };
                let scan = read_skill_source(&source).await.map_err(|e| format!("{E}: {e}"))?;
                let catalog = mstore.skill_list_global(&identity_store).map_err(|e| format!("{E}: {e}"))?;
                let mut warnings = scan.warnings;
                let mut items = Vec::new();
                for c in scan.candidates {
                    if only_key.as_deref().is_some_and(|k| k != c.key) {
                        continue;
                    }
                    let existing = if !req.skill_id.is_empty() {
                        catalog.iter().find(|i| i.skill.id == req.skill_id)
                    } else {
                        let from_here = |i: &&SkillCatalogItem| {
                            i.source.as_ref().is_some_and(|s| s.kind == source.kind && s.location == source.location && s.item_key == c.key)
                        };
                        catalog.iter().find(from_here).or_else(|| catalog.iter().find(|i| i.skill.name == c.name))
                    };
                    let (status, e) = match existing {
                        None => ("new", None),
                        Some(e) if e.skill.description == c.description && e.skill.content == c.content && e.skill.name == c.name => ("same", Some(e)),
                        Some(e) => ("changed", Some(e)),
                    };
                    items.push(SkillImportItem {
                        key: c.key,
                        name: c.name,
                        description: c.description,
                        content: c.content,
                        status: status.to_string(),
                        existing_id: e.map(|e| e.skill.id.clone()).unwrap_or_default(),
                        existing_description: e.map(|e| e.skill.description.clone()).unwrap_or_default(),
                        existing_content: e.map(|e| e.skill.content.clone()).unwrap_or_default(),
                    });
                }
                if items.is_empty() && warnings.is_empty() {
                    warnings.push(if only_key.is_some() { "the skill is no longer in its source" } else { "no SKILL.md found in that source" }.to_string());
                }
                Ok(SkillImportPreview { source, items, warnings })
            }
        },
    );
}

// Write the skills the person approved. A new one is global, unbound and outside "new agents get
// it": an imported skill tells every agent it reaches what to do, so reaching them is a separate,
// deliberate step. An existing one keeps its binds and reach.
fn register_skill_catalog_import_commit(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_SKILL_CATALOG_IMPORT_COMMIT,
        move |req: CommandSkillCatalogImportCommitData, _ctx| {
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            async move {
                const E: &str = "skill.catalog.import_commit";
                let now = now_ms();
                let mut saved = Vec::new();
                for item in req.items {
                    if item.name.trim().is_empty() || item.content.len() > 256 * 1024 {
                        return Err(format!("{E}: '{}' needs a name and at most 256 KB of text", item.key));
                    }
                    let existing = if item.existing_id.is_empty() {
                        None
                    } else {
                        match identity_store.skill_get(&item.existing_id).map_err(|e| format!("{E}: {e}"))? {
                            Some(s) if s.is_global => Some(s),
                            _ => return Err(format!("{E}: the skill to update is not in the catalog")),
                        }
                    };
                    let skill = crate::backend::storage::Skill {
                        id: existing.as_ref().map_or_else(|| uuid::Uuid::new_v4().to_string(), |s| s.id.clone()),
                        name: item.name.trim().to_string(),
                        trigger: existing.as_ref().map_or_else(|| item.name.trim().to_string(), |s| s.trigger.clone()),
                        skill_type: existing.as_ref().map_or_else(|| "agent-skill".to_string(), |s| s.skill_type.clone()),
                        description: item.description,
                        content: item.content,
                        is_global: true,
                        created_at: existing.as_ref().map_or(now, |s| s.created_at),
                        updated_at: now,
                    };
                    identity_store.skill_upsert_unique_global(&skill).map_err(|e| format!("{E}: {e}"))?;
                    if existing.is_none() {
                        identity_store.skill_set_reach(&skill.id, false, false, now).map_err(|e| format!("{E}: {e}"))?;
                    }
                    let source = SkillSource { item_key: item.key, ..req.source.clone() };
                    identity_store.skill_source_set(&skill.id, &source, now).map_err(|e| format!("{E}: {e}"))?;
                    saved.push(skill.id);
                }
                broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
                Ok(SkillImportCommitResult { saved })
            }
        },
    );
}

#[cfg(test)]
#[path = "skill_import_rpc_tests.rs"]
mod import_rpc_tests;
