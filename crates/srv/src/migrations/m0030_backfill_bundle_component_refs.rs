// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill `db_bundle_skills_ref` from the inline `db_bundles.skills` JSON
//! column.
//!
//! **Skills only, since bundles stopped carrying MCP servers**
//! (`SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md` §3.1). This also
//! carried the inline `mcp_servers` column into `db_bundle_mcp_ref`;
//! `m0036_drop_bundle_mcp` now deletes those refs and clears that column, so
//! a store that hasn't run this yet would only have the servers dropped again
//! straight after. That half went with the store code it called.
//!
//! Phase 0b of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. The
//! two stores were never synced: binding a skill in the Armory wrote only a
//! ref row, while ABF export read only the inline column, so a bundle could
//! run with components it did not export and export components it did not run
//! with (§3.4). The ref tables become authoritative; this carries the inline
//! data across so nothing is lost when export stops reading the columns.
//!
//! **Ships in the same release as the read switch.** Export reading refs
//! before this has run would make every pre-existing bundle export empty.
//!
//! Two stores, deliberately, as of when this migration was written.
//! `db_bundles` lives in the shared store (`run_shared_store_schema`), while
//! `db_skills`/`db_mcp_servers` and both ref tables lived in the channel
//! store alone (`run_object_schema` in `migrations.rs`) — the ref tables had
//! to sit next to the catalog tables they carried foreign keys to, which is
//! also why they never had an FK to `db_bundles` (the comment above
//! `db_bundle_skills_ref` in `run_object_schema`). So this reads bundles from the shared store
//! and writes refs into the channel store, resolving the former exactly the
//! way `m0021` does. **Now stale in one respect** (Phase 2 of
//! `SPEC_DURABLE_BINDINGS_2026_09_10.md`, `m0032_drop_catalog_fk_from_ref_tables`):
//! `db_skills`/`db_mcp_servers` are authoritatively `identity_store` and the
//! ref tables no longer carry the FK described above at all — this
//! migration itself is unaffected (it ran, or runs, entirely within the
//! world this comment describes, since it always operated on `mstore`'s own
//! local catalog copy) and is left as a historical record of why the two
//! stores were split, not a claim about the current schema.
//!
//! Idempotent: `INSERT OR IGNORE` on the ref table. Re-running changes
//! nothing.
//!
//! Best-effort per bundle. One malformed `skills` blob must not abort a
//! migration that is otherwise carrying real data across — the alternative
//! leaves the store half-converted with the read switch already live. Every
//! skip is logged with the bundle id.

use std::sync::Arc;

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0030BackfillBundleComponentRefs;

/// Where `db_bundles` actually lives — the shared store, falling back to the
/// channel store when it cannot be opened.
///
/// Same resolution, and the same never-hard-fail posture, as
/// `m0021_backfill_agent_bundles::resolve_bundle_store`. An unusable shared
/// store means "not today" rather than a failed boot.
fn resolve_bundle_store(ctx: &MigrationContext, mstore: &Arc<Store>) -> Arc<Store> {
    match Store::open_shared(&ctx.shared_store_path) {
        Ok(shared) => Arc::new(shared),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %ctx.shared_store_path.display(),
                "backfill_bundle_component_refs: shared store unavailable, falling back to channel store"
            );
            mstore.clone()
        }
    }
}

/// Carry one bundle's inline skills across into the ref table.
///
/// Returns how many were bound. Never returns `Err` for bad data — see the
/// module doc on why one bad bundle must not stop the rest.
fn backfill_one_bundle(
    mstore: &Store,
    bundle_store: &Store,
    bundle_id: &str,
    skills_json: &str,
) -> usize {
    let mut skills_bound = 0usize;

    // ---- skills: the inline column holds bare ids into db_skills ----
    match serde_json::from_str::<Vec<String>>(skills_json) {
        Ok(ids) => {
            for id in ids {
                // A skill id whose row is already gone has nothing to bind;
                // the ref tables cannot represent it and the export could
                // never have rendered it either.
                match mstore.skill_get(&id) {
                    // `mstore` also serves as `catalog` here: this migration
                    // pre-dates Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md's
                    // catalog redirect, back when `db_skills` genuinely lived
                    // on the channel store alone — see this module's own doc
                    // comment on why `db_skills` lives here and `db_bundles`
                    // lives in `bundle_store`.
                    Ok(Some(_)) => match mstore.bundle_skill_bind(mstore, bundle_store, bundle_id, &id) {
                        Ok(()) => skills_bound += 1,
                        Err(e) => tracing::warn!(
                            bundle_id, skill_id = %id, error = %e,
                            "backfill_bundle_component_refs: skill bind failed"
                        ),
                    },
                    Ok(None) => tracing::warn!(
                        bundle_id, skill_id = %id,
                        "backfill_bundle_component_refs: inline skill id has no catalog row — dropped"
                    ),
                    Err(e) => tracing::warn!(
                        bundle_id, skill_id = %id, error = %e,
                        "backfill_bundle_component_refs: skill lookup failed"
                    ),
                }
            }
        }
        Err(e) if !skills_json.trim().is_empty() && skills_json.trim() != "[]" => {
            tracing::warn!(
                bundle_id, error = %e,
                "backfill_bundle_component_refs: malformed inline skills JSON — nothing carried across"
            );
        }
        Err(_) => {} // genuinely blank is not an error
    }

    skills_bound
}

impl Migration for M0030BackfillBundleComponentRefs {
    fn id(&self) -> &'static str {
        "0030_backfill_bundle_component_refs"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Carry inline bundle skills across into the bundle ref table"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let mstore = Arc::new(Store::open(&ctx.channel_store_path).map_err(|e| {
            MigrationError(format!("backfill_bundle_component_refs: open mstore: {e}"))
        })?);
        let bundle_store = resolve_bundle_store(ctx, &mstore);

        let bundles = bundle_store.bundle_list().map_err(|e| {
            MigrationError(format!("backfill_bundle_component_refs: list bundles: {e}"))
        })?;

        let mut total_skills = 0usize;
        for bundle in &bundles {
            total_skills += backfill_one_bundle(&mstore, &bundle_store, &bundle.id, &bundle.skills);
        }

        tracing::info!(
            bundles = bundles.len(),
            skills_bound = total_skills,
            "backfill_bundle_component_refs: complete"
        );
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        // Verifying "every inline component has a ref" needs both stores, and
        // the shared one may legitimately be unavailable (see
        // resolve_bundle_store). Report what we can rather than claiming a
        // mismatch we cannot substantiate.
        match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(conn)) => {
                let skills: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_bundle_skills_ref", [], |r| r.get(0))
                    .unwrap_or(0);
                VerifyOutcome::Ok(format!("{skills} skill ref(s)"))
            }
            Ok(None) => VerifyOutcome::Ok("no channel store".to_string()),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::skills::Skill;

    fn store() -> Store {
        // One in-memory store serves as both mstore and bundle store:
        // `run_object_schema` creates db_bundles alongside the ref tables, so
        // both sides resolve. The cross-store rule itself is pinned by
        // `mcp_servers.rs::bind_checks_bundle_existence_in_id_store_not_self`.
        Store::open_in_memory().unwrap()
    }

    fn insert_bundle(s: &Store, id: &str, skills: &str) {
        let bundle = crate::backend::storage::store::Bundle {
            id: id.to_string(),
            name: format!("Bundle {id}"),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: skills.to_string(),
            sort_order: 0,
            created_at: 1,
            updated_at: 1,
            is_system: false,
        };
        s.bundle_upsert(&bundle).unwrap();
    }

    fn insert_skill(s: &Store, id: &str, name: &str) {
        let skill = Skill {
            id: id.to_string(),
            name: name.to_string(),
            trigger: name.to_string(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: String::new(),
            content: "body".to_string(),
            is_global: true,
            created_at: 1,
            updated_at: 1,
        };
        s.skill_upsert_unique_global(&skill).unwrap();
    }

    #[test]
    fn carries_inline_skills_into_the_ref_table() {
        let s = store();
        insert_skill(&s, "skill-1", "Deploy");
        insert_bundle(&s, "bundle-1", r#"["skill-1"]"#);

        assert_eq!(backfill_one_bundle(&s, &s, "bundle-1", r#"["skill-1"]"#), 1);
        let bound_skills: Vec<_> = s
            .bundle_skill_list(&s, "bundle-1")
            .unwrap()
            .into_iter()
            .filter(|i| i.bound_to_bundle)
            .collect();
        assert_eq!(bound_skills.len(), 1, "skill must be bound to the bundle");

        // Re-running binds the same ref again, idempotently.
        backfill_one_bundle(&s, &s, "bundle-1", r#"["skill-1"]"#);
        let count = s
            .bundle_skill_list(&s, "bundle-1")
            .unwrap()
            .into_iter()
            .filter(|i| i.bound_to_bundle)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_malformed_blob_is_logged_not_fatal() {
        let s = store();
        insert_bundle(&s, "bundle-1", "not an array at all");
        assert_eq!(backfill_one_bundle(&s, &s, "bundle-1", "not an array at all"), 0);
    }

    #[test]
    fn an_inline_skill_id_with_no_catalog_row_is_dropped_not_fatal() {
        let s = store();
        insert_bundle(&s, "bundle-1", r#"["ghost"]"#);
        assert_eq!(backfill_one_bundle(&s, &s, "bundle-1", r#"["ghost"]"#), 0);
    }
}
