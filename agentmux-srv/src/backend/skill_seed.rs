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
//!
//! **Ids are deterministic, not randomly minted** (Phase 1 of
//! `SPEC_DURABLE_BINDINGS_2026_09_10.md`). Every store that seeds "Systematic
//! Debugging" produces the exact same row id for it, derived from the
//! manifest entry's `trigger` — the stable slash-command slug, not `name`
//! (display text; changing it must not change identity) and not `content`
//! (edited over time for typos/clarity; a wording fix is not a new skill).
//! This is what a later cross-store reconciliation needs to recognize "these
//! are the same seeded skill" without a name-matching heuristic that a
//! user-created private skill sharing that name could collide with — see
//! `starter_skill_id`'s own doc comment for why a bare name match is unsafe
//! in general. Existing installs, already seeded under this migration before
//! this change shipped, keep whatever random id they already have; this only
//! changes what a FUTURE seed produces.

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

/// Deterministic row id for a starter skill, derived from its `trigger`.
///
/// A bare name match is not a safe way to recognize "the same skill" across
/// stores in general — `managed_upsert_unique` enforces name uniqueness only
/// WITHIN one owner (a bundle, or the global tier), so two different bundles
/// can each legitimately hold a private skill called the same thing with
/// different content (`SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.3). Collapsing
/// on name alone would discard one payload and cross-link edits between two
/// owners that never agreed to share a row.
///
/// The ambiguity does not apply to a SEEDED global row specifically: it is
/// owned by nobody, so two stores minting one under the same key are, by
/// construction, minting the same resource. Giving that case a stable id up
/// front means a later cross-store reconciliation does not need the
/// name-matching heuristic at all for anything this function produced — only
/// for genuinely user-created rows, which is the case that heuristic was
/// actually built for.
///
/// `trigger`, not `name` or `content`: `name` is display text a maintainer
/// could reword without changing what skill this is, and `content` gets
/// wording fixes over time — neither should mint a new identity. `trigger`
/// is the stable slash-command slug already surfaced to the user
/// (`docs/CLAUDE.md`'s own "Available Skills" list uses it as the anchor),
/// so a change to it is already understood elsewhere in the codebase as
/// meaning "this is a different skill."
///
/// The namespace is itself derived via `Uuid::new_v5` from a fixed, documented
/// seed string rather than a hand-picked random constant, so the value here
/// is reproducible and auditable from this source file alone rather than an
/// opaque UUID nobody can regenerate if this file is ever lost.
pub(crate) fn starter_skill_id(trigger: &str) -> Uuid {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"https://agentmux.ai/catalog/skills/v1");
    Uuid::new_v5(&namespace, trigger.as_bytes())
}

/// Guard against a manifest-authoring mistake: two entries sharing a
/// `trigger`.
///
/// Deterministic ids remove a safety net that random ids provided by
/// accident. Before this change, a duplicate `trigger` with a different
/// `name` would insert as two independent rows (no id collision, no name
/// collision — nobody was checking `trigger` for uniqueness either way,
/// this was already a latent gap). After this change it is worse in a
/// different way: both entries now mint the SAME id, so the second insert's
/// `managed_upsert_unique_global` duplicate-name check
/// (`WHERE name = ?1 AND id <> ?2`) excludes the first entry's own row from
/// consideration — because its id now equals the second entry's id — and the
/// `INSERT ... ON CONFLICT(id) DO UPDATE` silently overwrites it instead of
/// erroring. `seed_starter_skills`'s `created` count would even report six
/// when only five distinct rows exist (reagent P2, PR #3175).
///
/// Unreachable today — `starter-skills.json` has six distinct triggers, see
/// the test pinning that — but it must fail LOUDLY the moment it stops being
/// unreachable, the same way the pre-existing name-uniqueness check already
/// does for other manifest mistakes, rather than silently merging two
/// authored skills into one.
fn reject_duplicate_triggers(manifest: &[StarterSkill]) -> Result<(), StoreError> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for entry in manifest {
        if !seen.insert(entry.trigger.as_str()) {
            return Err(StoreError::Other(format!(
                "skill seed: manifest has more than one entry with trigger '{}' — \
                 this is an authoring bug in starter-skills.json, not a runtime \
                 condition to recover from",
                entry.trigger
            )));
        }
    }
    Ok(())
}

/// True if any global skill whose name matches a starter-skill's name
/// already exists — i.e. this channel already effectively has the starter
/// set (or a name collision with it), whether from a prior run of this
/// migration, the retired pre-migration startup-seed path (#2141, shipped
/// before this migration existed — every channel that has already booted
/// once has these names present), or the user coincidentally naming their
/// own skill the same thing.
///
/// The caller (`migrations::m0015_seed_starter_skills`) uses this to skip
/// seeding entirely rather than attempting an insert that would collide on
/// `skill_upsert_unique_global`'s name-uniqueness check and permanently
/// fail the migration on every subsequent boot (reagent P1, PR #2144).
pub(crate) fn any_starter_skill_name_exists(wstore: &Arc<Store>) -> Result<bool, StoreError> {
    let manifest: Vec<StarterSkill> = serde_json::from_str(STARTER_SKILLS_JSON)
        .map_err(|e| StoreError::Other(format!("skill seed: parse manifest: {e}")))?;
    let existing = wstore.skill_list_global()?;
    Ok(manifest
        .iter()
        .any(|entry| existing.iter().any(|item| item.skill.name == entry.name)))
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
    reject_duplicate_triggers(&manifest)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut inserted_ids: Vec<String> = Vec::with_capacity(manifest.len());
    for entry in &manifest {
        let skill = Skill {
            id: starter_skill_id(&entry.trigger).to_string(),
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

    fn starter(trigger: &str, name: &str) -> StarterSkill {
        StarterSkill {
            name: name.to_string(),
            trigger: trigger.to_string(),
            skill_type: "prompt".to_string(),
            description: "d".to_string(),
            content: "c".to_string(),
        }
    }

    #[test]
    fn reject_duplicate_triggers_fails_loudly_on_a_colliding_manifest() {
        // reagent P2, PR #3175: without this guard, two entries sharing a
        // trigger now mint the SAME deterministic id, and the second insert
        // silently overwrites the first instead of erroring — see this
        // function's own doc comment for the full mechanism. Must fail
        // BEFORE any row is touched, the way the old random-id behavior
        // failed loudly on a duplicate NAME.
        let manifest = vec![
            starter("dup", "First Skill"),
            starter("dup", "Second Skill, Different Name"),
        ];
        let err = reject_duplicate_triggers(&manifest).unwrap_err();
        assert!(format!("{err}").contains("dup"), "error must name the offending trigger: {err}");
    }

    #[test]
    fn reject_duplicate_triggers_accepts_the_real_manifest() {
        // The guard must never fire on starter-skills.json as it ships.
        let manifest: Vec<StarterSkill> = serde_json::from_str(STARTER_SKILLS_JSON).unwrap();
        assert!(reject_duplicate_triggers(&manifest).is_ok());
    }

    #[test]
    fn starter_skill_ids_are_identical_across_two_independently_seeded_stores() {
        // The property Phase 1 of SPEC_DURABLE_BINDINGS_2026_09_10.md exists
        // for: two stores that have never seen each other seed the SAME
        // starter skill under the SAME id. This is what lets a later
        // cross-store reconciliation recognize them as one resource without
        // guessing from the name.
        let store_a = Arc::new(Store::open_in_memory().unwrap());
        let store_b = Arc::new(Store::open_in_memory().unwrap());

        seed_starter_skills(&store_a).unwrap();
        seed_starter_skills(&store_b).unwrap();

        let mut ids_a: Vec<(String, String)> = store_a
            .skill_list_global()
            .unwrap()
            .into_iter()
            .map(|item| (item.skill.name, item.skill.id))
            .collect();
        let mut ids_b: Vec<(String, String)> = store_b
            .skill_list_global()
            .unwrap()
            .into_iter()
            .map(|item| (item.skill.name, item.skill.id))
            .collect();
        ids_a.sort();
        ids_b.sort();

        assert_eq!(ids_a.len(), 6);
        assert_eq!(ids_a, ids_b, "the same starter skill must get the same id in every store");
    }

    #[test]
    fn starter_skill_id_changes_only_with_trigger_not_with_name_or_content() {
        // Renaming the DISPLAY text or fixing a typo in the body must not
        // mint a new identity — only the trigger (the stable slug) does.
        let id_before = starter_skill_id("tdd");
        let id_after_reword = starter_skill_id("tdd");
        assert_eq!(id_before, id_after_reword, "identical trigger must always produce the identical id");

        let id_different_trigger = starter_skill_id("test-driven-development");
        assert_ne!(
            id_before, id_different_trigger,
            "a different trigger is treated as a different skill, as intended"
        );
    }

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
