// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Starter Skills catalog seed: preloads a small set of curated global
//! skills (`db_skills`, the v1 standalone catalog) on fresh install.
//!
//! The actual "run exactly once, ever" gating lives in
//! `migrations::m0015_seed_starter_skills`, tracked in the channel's
//! `db_migrations` table — NOT in this module. An earlier version gated on
//! "is the catalog currently empty," which couldn't distinguish "never
//! seeded" from "seeded then the user deleted every starter skill on
//! purpose," silently resurrecting the defaults on the next restart after a
//! full deletion (reagent P2, PR #2141 round 2). This module now exposes
//! only the pure insert logic; the migration owns once-ever invocation.
//!
//! Distinct from `agent_seed.rs`, which seeds legacy per-agent skills into
//! `db_agent_skills` and re-seeds on every manifest version bump.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use uuid::Uuid;

use super::storage::skills::Skill;
use super::storage::store::Store;
use super::storage::StoreError;

/// One entry in the embedded starter-skills manifest. Field names mirror
/// `Skill` minus the store-generated `id`/`is_global`/timestamps.
#[derive(Debug, Deserialize)]
struct StarterSkill {
    name: String,
    trigger: String,
    skill_type: String,
    description: String,
    content: String,
}

/// The embedded starter-skills manifest JSON. Content is authored
/// externally and must not be edited here — see
/// `agentmux-srv/src/config/starter-skills.json`.
const STARTER_SKILLS_JSON: &str = include_str!("../config/starter-skills.json");

/// Report returned after a seed attempt.
pub struct SkillSeedReport {
    pub created: usize,
}

/// Parse the embedded manifest and insert every entry as a global skill via
/// the validated `skill_upsert_unique_global` path. Does NOT check whether
/// the catalog is already populated — the caller
/// (`migrations::m0015_seed_starter_skills`) owns run-once gating via
/// `db_migrations` tracking, not catalog contents. Exposed at `pub(crate)`
/// so the migration and tests can call it directly.
///
/// All-or-nothing: `skill_upsert_unique_global` commits each insert in its
/// own transaction (it isn't `StoreTx`-composable — see its own doc
/// comment), so a mid-loop failure can't be rolled back by the database
/// itself. If any insert fails, this compensates by deleting every skill
/// already inserted in THIS call before returning the error — otherwise a
/// retry (the migration framework re-runs `up()` on the next boot when a
/// migration returns `Err`, since it's never marked applied) would hit
/// `skill_upsert_unique_global`'s name-uniqueness rejection on the skills
/// already stranded from the failed attempt (reagent P2, PR #2141 round 1).
pub(crate) fn seed_starter_skills(wstore: &Arc<Store>) -> Result<SkillSeedReport, StoreError> {
    let manifest: Vec<StarterSkill> = serde_json::from_str(STARTER_SKILLS_JSON)
        .map_err(|e| StoreError::Other(format!("skill seed: parse manifest: {e}")))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut inserted_ids: Vec<String> = Vec::with_capacity(manifest.len());
    for entry in &manifest {
        let skill = Skill {
            id: Uuid::new_v4().to_string(),
            name: entry.name.clone(),
            trigger: entry.trigger.clone(),
            skill_type: entry.skill_type.clone(),
            description: entry.description.clone(),
            content: entry.content.clone(),
            is_global: true,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = wstore.skill_upsert_unique_global(&skill) {
            for id in &inserted_ids {
                if let Err(cleanup_err) = wstore.skill_delete(id) {
                    tracing::error!(
                        "skill seed: cleanup after partial failure could not remove {id}: {cleanup_err}"
                    );
                }
            }
            return Err(e);
        }
        inserted_ids.push(skill.id);
    }

    Ok(SkillSeedReport { created: inserted_ids.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_six_skills_into_an_empty_catalog() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        assert!(wstore.skill_list_global().unwrap().is_empty());

        let report = seed_starter_skills(&wstore).unwrap();

        assert_eq!(report.created, 6);
        let after = wstore.skill_list_global().unwrap();
        assert_eq!(after.len(), 6, "all six starter skills should be seeded");
        assert!(after.iter().all(|item| item.skill.is_global));
    }

    #[test]
    fn a_failed_insert_rolls_back_the_ones_already_seeded_this_call() {
        // Reagent P2 (PR #2141 round 1): skill_upsert_unique_global commits
        // each insert in its own transaction, so a mid-loop failure can't
        // be rolled back by the database. Simulate that failure mode by
        // pre-inserting a global skill whose NAME collides with one of the
        // starter skills (skill_upsert_unique_global rejects duplicate
        // names) — this forces seed_starter_skills to fail partway through
        // the manifest, and the catalog must end up back at exactly the
        // one pre-existing skill, not a stranded partial starter set.
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        // "Test-Driven Development" is the second entry in the manifest —
        // colliding on it guarantees at least one successful insert
        // (Systematic Debugging) precedes the failure, so the rollback
        // path actually has something to clean up.
        let colliding = Skill {
            id: "pre-existing-collision".to_string(),
            name: "Test-Driven Development".to_string(),
            trigger: "already-here".to_string(),
            skill_type: "prompt".to_string(),
            description: "Pre-existing skill that collides with a starter skill's name.".to_string(),
            content: "n/a".to_string(),
            is_global: true,
            created_at: now,
            updated_at: now,
        };
        wstore.skill_upsert_unique_global(&colliding).unwrap();

        let result = seed_starter_skills(&wstore);
        assert!(result.is_err(), "seeding must fail when a name collides");

        let after = wstore.skill_list_global().unwrap();
        assert_eq!(
            after.len(),
            1,
            "a failed seed must roll back every skill it inserted this call, leaving only the pre-existing one"
        );
        assert_eq!(after[0].skill.id, "pre-existing-collision");
    }
}
