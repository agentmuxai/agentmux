// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Skills that ship with AgentMux: put into the catalog once, for every agent, and then left alone.
//!
//! - **Once.** Each skill has a stable id. The first start that sees an id not yet recorded in
//!   `db_builtin_seeds` creates the skill and records the id. The record stays when the user deletes the skill,
//!   so a delete is final; a release that adds a new skill to [`BUILTINS`] seeds only that one.
//! - **Never overwritten.** After the first write nothing here touches the row, so the user's edits stay. A
//!   later release that improves the text does not replace it.
//! - **Every agent.** The skill is global and reaches every agent (`db_skill_reach`, `all_agents`), so it is
//!   in each agent's skills without a bind, and the user can switch that off in Memory > Skills.
//! - **Quiet on conflict.** If the user already has a global skill with the same name, the built-in one is
//!   skipped and recorded as seeded, not retried at every start.
//!
//! Seeded into the identity store (the catalog), like the Global Memory seed. Skill text is public: it names
//! no host, account or private system.

use std::sync::Arc;

use super::storage::store::Store;
use super::storage::{Skill, StoreError};

/// One skill AgentMux ships.
pub struct Builtin {
    /// The skill's id for ever: the seed record and the catalog row are keyed on it.
    pub id: &'static str,
    pub name: &'static str,
    pub trigger: &'static str,
    pub description: &'static str,
    /// The skill's body (the `SKILL.md` text after the frontmatter).
    pub content: &'static str,
}

/// The skill format that becomes `.claude/skills/<name>/SKILL.md` (`agent_config::SKILL_TYPE_AGENT_SKILL`).
const SKILL_TYPE: &str = crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL;

pub const BUILTINS: &[Builtin] = &[Builtin {
    id: "7c1e5d2a-3b64-4f0e-9a58-0d15c4b0d1a2",
    name: "disk-cleanup",
    trigger: "disk-cleanup",
    description: "Free disk space safely: measure AgentMux's own leftovers (old version folders, dev homes, instances, caches), say what is safe to remove, and ask the user before deleting anything.",
    content: include_str!("builtin_skills/disk-cleanup.md"),
}];

/// The key a skill's seed record is kept under.
fn seed_key(id: &str) -> String {
    format!("skill:{id}")
}

/// Seeds [`BUILTINS`] into the catalog on a start. Failures are logged and retried at the next start.
pub fn seed_on_startup(store: &Arc<Store>) {
    let now = agentmux_common::time::now_ms();
    let seeded = seed(store, BUILTINS, now);
    if seeded > 0 {
        tracing::info!(seeded, "built-in skills: seeded");
    }
}

/// Seeds the skills of `list` that were never seeded; returns how many were created.
pub(crate) fn seed(store: &Store, list: &[Builtin], now_ms: i64) -> usize {
    let mut created = 0;
    for b in list {
        let key = seed_key(b.id);
        match store.builtin_seed_done(&key) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(skill = b.name, "built-in skills: could not read the seed record: {e}");
                continue;
            }
        }
        match seed_one(store, b, now_ms) {
            Ok(made) => {
                if let Err(e) = store.builtin_seed_mark(&key, now_ms) {
                    tracing::warn!(skill = b.name, "built-in skills: could not record the seed: {e}");
                    continue;
                }
                created += usize::from(made);
            }
            Err(e) => tracing::warn!(skill = b.name, "built-in skills: not seeded, will retry at the next start: {e}"),
        }
    }
    created
}

/// Creates the skill and its reach. `Ok(false)` when there was nothing to create: the row exists already, or a
/// global skill of the same name does (both count as seeded). A store failure is an `Err`, to retry.
fn seed_one(store: &Store, b: &Builtin, now_ms: i64) -> Result<bool, StoreError> {
    if store.skill_get(b.id)?.is_some() {
        return Ok(false);
    }
    let skill = Skill {
        id: b.id.to_string(),
        name: b.name.to_string(),
        trigger: b.trigger.to_string(),
        skill_type: SKILL_TYPE.to_string(),
        description: b.description.to_string(),
        content: b.content.to_string(),
        is_global: true,
        created_at: now_ms,
        updated_at: now_ms,
    };
    match store.skill_upsert_unique_global(&skill) {
        Ok(()) => {}
        Err(StoreError::Other(msg)) if msg.contains("already exists") => {
            tracing::info!(skill = b.name, "built-in skills: a global skill with this name exists; leaving it");
            return Ok(false);
        }
        Err(e) => return Err(e),
    }
    store.skill_set_reach(b.id, true, false, now_ms)?;
    Ok(true)
}

#[cfg(test)]
#[path = "builtin_skills_tests.rs"]
mod tests;
