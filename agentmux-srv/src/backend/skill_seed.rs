// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Starter Skills catalog seed: preloads a small set of curated global
//! skills (`db_skills`, the v1 standalone catalog) on fresh install.
//!
//! This is a **one-time, idempotent seed** — not a schema migration (which
//! runs unconditionally every startup) and not a sync/upsert mechanism for
//! the starter set going forward. It is gated on the global catalog being
//! empty: if `skill_list_global()` returns any rows — including the case
//! where the user seeded then deleted every starter skill on purpose — the
//! seed step is a no-op. That gate can't distinguish "never seeded" from
//! "seeded then fully cleared", and per the design that's fine: both states
//! mean "the user should own an empty catalog from here," not "silently
//! resurrect defaults."
//!
//! Distinct from `agent_seed.rs`, which seeds legacy per-agent skills into
//! `db_agent_skills` and re-seeds on every manifest version bump. This
//! module seeds the newer standalone `db_skills` table exactly once and
//! never re-seeds or updates existing rows.

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

/// Seed the global Skills catalog (`db_skills WHERE is_global = 1`) from the
/// embedded starter set, but only if that catalog is currently empty.
///
/// Called once at startup, alongside `agent_seed::auto_seed_on_startup`.
/// Failures are logged and non-fatal — a broken seed should never block
/// server startup.
pub fn seed_starter_skills_if_empty(wstore: &Arc<Store>) {
    match wstore.skill_list_global() {
        Ok(existing) if !existing.is_empty() => {
            tracing::debug!(
                count = existing.len(),
                "skill seed: global catalog already has skills, skipping starter seed"
            );
        }
        Ok(_) => match seed_starter_skills(wstore) {
            Ok(report) => {
                if report.created > 0 {
                    tracing::info!(
                        "skill seed: seeded {} starter skills into the global catalog",
                        report.created
                    );
                }
            }
            Err(e) => tracing::error!("skill seed: failed: {e}"),
        },
        Err(e) => tracing::error!("skill seed: failed to check global catalog: {e}"),
    }
}

/// Parse the embedded manifest and insert every entry as a global skill via
/// the validated `skill_upsert_unique_global` path. Does NOT check whether
/// the catalog is already populated — callers (namely
/// `seed_starter_skills_if_empty`) own that gate. Exposed separately so
/// tests can exercise idempotency directly.
///
/// All-or-nothing: `skill_upsert_unique_global` commits each insert in its
/// own transaction (it isn't `StoreTx`-composable — see its own doc
/// comment), so a mid-loop failure can't be rolled back by the database
/// itself. If any insert fails, this compensates by deleting every skill
/// already inserted in THIS call before returning the error — otherwise
/// the catalog would be left non-empty-but-incomplete, and the
/// empty-catalog gate in `seed_starter_skills_if_empty` would treat that
/// partial state as "already seeded," permanently stranding it with no
/// retry path (reagent P2, PR #2141 round 1).
fn seed_starter_skills(wstore: &Arc<Store>) -> Result<SkillSeedReport, StoreError> {
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
    fn seeds_into_empty_catalog() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        assert!(wstore.skill_list_global().unwrap().is_empty());

        seed_starter_skills_if_empty(&wstore);

        let after = wstore.skill_list_global().unwrap();
        assert_eq!(after.len(), 6, "all six starter skills should be seeded");
        assert!(after.iter().all(|item| item.skill.is_global));
    }

    #[test]
    fn seeding_is_idempotent() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());

        seed_starter_skills_if_empty(&wstore);
        let first_count = wstore.skill_list_global().unwrap().len();
        assert_eq!(first_count, 6);

        // Running the gated entry point again must not duplicate rows —
        // the catalog is no longer empty, so this is a no-op.
        seed_starter_skills_if_empty(&wstore);
        let second_count = wstore.skill_list_global().unwrap().len();
        assert_eq!(second_count, first_count, "re-running the seed step must not duplicate rows");
    }

    #[test]
    fn skips_seeding_when_a_global_skill_already_exists() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());

        // Simulate a pre-existing global skill (e.g. user-created, or a
        // starter skill that survived from a previous partial state).
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let pre_existing = Skill {
            id: "user-skill-1".to_string(),
            name: "My Own Skill".to_string(),
            trigger: "my-own-skill".to_string(),
            skill_type: "prompt".to_string(),
            description: "A user-authored skill.".to_string(),
            content: "Do the thing.".to_string(),
            is_global: true,
            created_at: now,
            updated_at: now,
        };
        wstore.skill_upsert_unique_global(&pre_existing).unwrap();

        seed_starter_skills_if_empty(&wstore);

        let after = wstore.skill_list_global().unwrap();
        assert_eq!(
            after.len(),
            1,
            "seed step must not run when the global catalog is non-empty"
        );
        assert_eq!(after[0].skill.id, "user-skill-1");
    }

    #[test]
    fn deleting_all_seeded_skills_then_calling_again_does_reseed() {
        // Per the design: an empty-catalog gate can't distinguish "never
        // seeded" from "seeded then the user deleted everything on
        // purpose" — that's intentional (see the module doc comment). This
        // test verifies the actual consequence, not just the intermediate
        // state: once seeded-then-cleared, the catalog looks identical to
        // fresh, so a second call to `seed_starter_skills_if_empty` DOES
        // reseed. In production this never happens — the call site is
        // startup-only — but this proves the function-level behavior the
        // module comment claims, rather than asserting only the emptiness
        // in between and leaving the "would reseed" half unverified
        // (reagent P2, PR #2141 round 1).
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        seed_starter_skills_if_empty(&wstore);
        assert_eq!(wstore.skill_list_global().unwrap().len(), 6);

        for item in wstore.skill_list_global().unwrap() {
            wstore.skill_delete(&item.skill.id).unwrap();
        }
        assert!(wstore.skill_list_global().unwrap().is_empty());

        seed_starter_skills_if_empty(&wstore);
        assert_eq!(
            wstore.skill_list_global().unwrap().len(),
            6,
            "an empty catalog is indistinguishable from fresh, so calling the gated entry point again reseeds"
        );
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
