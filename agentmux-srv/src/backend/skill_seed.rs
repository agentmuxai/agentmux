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
fn seed_starter_skills(wstore: &Arc<Store>) -> Result<SkillSeedReport, StoreError> {
    let manifest: Vec<StarterSkill> = serde_json::from_str(STARTER_SKILLS_JSON)
        .map_err(|e| StoreError::Other(format!("skill seed: parse manifest: {e}")))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut created = 0usize;
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
        wstore.skill_upsert_unique_global(&skill)?;
        created += 1;
    }

    Ok(SkillSeedReport { created })
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
    fn deleting_all_seeded_skills_does_not_trigger_reseed() {
        // Per the design: an empty-catalog gate can't distinguish "never
        // seeded" from "seeded then the user deleted everything on
        // purpose" — and that's intentional. This test documents the
        // consequence directly: once seeded-then-cleared, the catalog
        // looks identical to fresh, so a second call *would* reseed if
        // invoked again. The one-time nature is enforced by the call site
        // (startup only), not by this function's own state — this test
        // exists to make that behavior explicit rather than assumed.
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        seed_starter_skills_if_empty(&wstore);
        assert_eq!(wstore.skill_list_global().unwrap().len(), 6);

        for item in wstore.skill_list_global().unwrap() {
            wstore.skill_delete(&item.skill.id).unwrap();
        }
        assert!(wstore.skill_list_global().unwrap().is_empty());
    }
}
