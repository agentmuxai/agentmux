// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Remove the six starter skills (`0015_seed_starter_skills` seeded them on a
//! fresh install; it no longer does) from a catalog where nobody used them.
//!
//! A starter skill is deleted only when all of these hold:
//! - its row has the id the seed gave it (`frozen_starter_skill_id`);
//! - it is still global and its name, trigger, type, description and content
//!   hash to the shipped text, so an edited or renamed skill stays;
//! - no agent and no bundle holds a ref to it, in this channel or in any other
//!   channel's `objects.db` (the identity store's catalog is shared by all of
//!   them, so another channel may still bind it). A sibling store that cannot
//!   be read counts as "bound": a skill is never deleted on a guess.
//!
//! Both copies go: the identity store's row (the catalog) and this channel's
//! own `db_skills` copy that `0031_carry_skills_and_mcp_servers_to_identity_store`
//! left behind. Idempotent: a second run finds nothing to delete. It only
//! deletes rows and changes no table or schema version.
//!
//! The shipped texts are gone from the tree, so the check keeps a SHA-256 of
//! each rather than the text. The id derivation and the hash input are frozen
//! copies on purpose (`migrations/mod.rs`): this must recognise the rows the
//! seed wrote on every machine it runs on, whatever later releases do.

use std::path::{Path, PathBuf};

use rusqlite::params;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::backend::storage::skills::Skill;
use crate::backend::storage::store::Store;
use crate::registry;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0041RetireUntouchedStarterSkills;

/// `(name, trigger, SHA-256 of `fingerprint`)` for each starter skill as
/// `0015_seed_starter_skills` shipped it. All six were of type "prompt".
const FROZEN_STARTERS: &[(&str, &str, &str)] = &[
    ("Systematic Debugging", "systematic-debugging", "17ec313359cc8f4f1b00e4cf3ae282d684c421c22c8fe80bbc58d2e543ce70e9"),
    ("Test-Driven Development", "tdd", "85f2a34983dee0321bc850cd341e9a35e621aecd9e7a5072c0f62b41d300d4d4"),
    ("Code Review — Requesting & Receiving", "code-review", "b650dd27f0e24c5a470eb0dc15fd23652b55f8b2247bc2e37cdbae2a86140ab8"),
    ("Git Commit & Branch Hygiene", "commit-hygiene", "b1daa6d425b73f729588acc447fe3942fdbb0872bf65e76ea4d3410b7d35bd91"),
    ("Verification Before Completion", "verification-before-completion", "fde1ab55fe243ddd534ccc98d4cfa1fd051e904b3d5468e5b59763335f64835c"),
    ("Security Review Basics", "security-basics", "57dc504a0f6df88b02ab2f7f4ea509333ddec11ad1aac8d6b20b424f21c21911"),
];

/// Frozen copy of the seed's id derivation (`skill_seed::starter_skill_id`),
/// also frozen in `0031`.
fn frozen_starter_skill_id(trigger: &str) -> String {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"https://agentmux.ai/catalog/skills/v1");
    Uuid::new_v5(&namespace, trigger.as_bytes()).to_string()
}

/// What the hash covers: every field the seed wrote and an edit can change.
fn fingerprint(skill: &Skill) -> String {
    let input = [
        skill.name.as_str(),
        skill.trigger.as_str(),
        skill.skill_type.as_str(),
        skill.description.as_str(),
        skill.content.as_str(),
    ]
    .join("\0");
    hex::encode(Sha256::digest(input.as_bytes()))
}

fn is_untouched(skill: &Skill, expected_hash: &str) -> bool {
    skill.is_global && fingerprint(skill) == expected_hash
}

/// True when `store` has an agent or bundle ref to the skill `id`, or to a
/// global skill called `name` (a channel that has not run `0031` yet still
/// has the seed's older random id).
fn holds_a_ref(store: &Store, id: &str, name: &str) -> Result<bool, rusqlite::Error> {
    let conn = store.conn().lock().unwrap();
    for table in ["db_agent_skills_ref", "db_bundle_skills_ref"] {
        let by_id = format!("SELECT COUNT(*) FROM {table} WHERE skill_id = ?1");
        let by_name = format!(
            "SELECT COUNT(*) FROM {table} WHERE skill_id IN (SELECT id FROM db_skills WHERE is_global = 1 AND name = ?1)"
        );
        for (sql, key) in [(by_id, id), (by_name, name)] {
            match conn.query_row(&sql, params![key], |r| r.get::<_, i64>(0)) {
                Ok(0) => {}
                Ok(_) => return Ok(true),
                // An older schema without the table holds no refs.
                Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(false)
}

/// True unless `path` was read and holds no ref to the skill.
fn sibling_may_hold_a_ref(path: &Path, id: &str, name: &str) -> bool {
    let store = match Store::open_source_readonly(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "retire_untouched_starter_skills: sibling unreadable, keeping the skill");
            return true;
        }
    };
    holds_a_ref(&store, id, name).unwrap_or_else(|e| {
        tracing::warn!(path = %path.display(), error = %e, "retire_untouched_starter_skills: sibling refs unreadable, keeping the skill");
        true
    })
}

/// Delete each starter skill nobody uses and nobody edited. `identity` is the
/// shared catalog (`None` when there is none yet), `mstore` this channel's
/// store and `siblings` the other channels' `objects.db` files. Returns how
/// many starter skills were removed.
fn retire_untouched_starters(mstore: &Store, identity: Option<&Store>, siblings: &[PathBuf]) -> Result<usize, String> {
    let mut removed = 0usize;
    for (name, trigger, hash) in FROZEN_STARTERS {
        let id = frozen_starter_skill_id(trigger);
        let in_catalog = match identity {
            Some(s) => s.skill_get(&id).map_err(|e| format!("read catalog skill {name}: {e}"))?,
            None => None,
        };
        let in_channel = mstore.skill_get(&id).map_err(|e| format!("read channel skill {name}: {e}"))?;
        // The catalog row is the authoritative one; the channel's copy only
        // decides when there is no catalog row.
        let Some(authoritative) = in_catalog.as_ref().or(in_channel.as_ref()) else {
            continue;
        };
        if !is_untouched(authoritative, hash) {
            continue;
        }
        if holds_a_ref(mstore, &id, name).map_err(|e| format!("count refs to {name}: {e}"))? {
            continue;
        }
        if siblings.iter().any(|path| sibling_may_hold_a_ref(path, &id, name)) {
            continue;
        }
        if let (Some(s), Some(row)) = (identity, in_catalog.as_ref()) {
            s.skill_delete(s, &row.id).map_err(|e| format!("delete catalog skill {name}: {e}"))?;
        }
        if let Some(row) = in_channel.as_ref().filter(|row| is_untouched(row, hash)) {
            mstore.skill_delete(mstore, &row.id).map_err(|e| format!("delete channel skill {name}: {e}"))?;
        }
        removed += 1;
    }
    Ok(removed)
}

impl Migration for M0041RetireUntouchedStarterSkills {
    fn id(&self) -> &'static str {
        "0041_retire_untouched_starter_skills"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Remove the starter skills no agent uses and nobody edited"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let mstore = Store::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("retire_untouched_starter_skills: open channel store: {e}")))?;
        let identity = match registry::resolve_identity_store_path().filter(|p| p.is_file()) {
            Some(path) => Some(
                Store::open_identity_store(&path)
                    .map_err(|e| MigrationError(format!("retire_untouched_starter_skills: open identity store: {e}")))?,
            ),
            None => None,
        };
        let siblings: Vec<PathBuf> = registry::enumerate_objects_dbs(&ctx.home)
            .into_iter()
            .filter(|p| p != &ctx.channel_store_path)
            .collect();
        let removed = retire_untouched_starters(&mstore, identity.as_ref(), &siblings).map_err(MigrationError)?;
        if removed > 0 {
            tracing::info!(removed, "retire_untouched_starter_skills: removed unused starter skills");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "m0041_retire_untouched_starter_skills_tests.rs"]
mod tests;
