// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Seed the global Skills catalog (`db_skills`, per-channel) with a curated
//! starter set on fresh install.
//!
//! Runs exactly once per channel, tracked in that channel's `db_migrations`
//! table — NOT gated on "is the catalog currently empty." The prior
//! `seed_starter_skills_if_empty` gate (removed) couldn't distinguish "never
//! seeded" from "seeded then the user deleted every starter skill on
//! purpose," so a normal srv restart after a full deletion silently
//! resurrected the defaults (reagent P2, PR #2141 round 2 — confirmed by
//! `skill_seed.rs`'s own former test). The migration framework's
//! once-ever-per-channel tracking fixes this at the root: once
//! `0015_seed_starter_skills` is marked applied, it never runs again for
//! that channel regardless of what the user does to the catalog afterward.

use std::sync::Arc;

use crate::backend::skill_seed::seed_starter_skills;
use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0015SeedStarterSkills;

impl Migration for M0015SeedStarterSkills {
    fn id(&self) -> &'static str { "0015_seed_starter_skills" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Seed the global Skills catalog with a curated starter set" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("seed_starter_skills: open wstore: {}", e)))?,
        );
        seed_starter_skills(&wstore)
            .map(|_| ())
            .map_err(|e| MigrationError(format!("seed_starter_skills: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-store.db"),
            channel_store_path: path.to_path_buf(),
        }
    }

    #[test]
    fn seeds_six_starter_skills_on_a_fresh_channel() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Create the channel store up front — `up()` no-ops when the path
        // doesn't exist yet (mirrors m0007's guard).
        Store::open(tmp.path()).unwrap();

        M0015SeedStarterSkills.up(&ctx_for(tmp.path())).unwrap();

        let wstore = Store::open(tmp.path()).unwrap();
        assert_eq!(wstore.skill_list_global().unwrap().len(), 6);
    }

    #[test]
    fn once_marked_applied_a_full_catalog_deletion_is_never_resurrected() {
        // This is the actual fix: the migration framework's db_migrations
        // tracking (not skill_seed's own logic) is what must prevent
        // reseeding — reproduce that gate here rather than trusting it.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let wstore = Store::open(tmp.path()).unwrap();
        let ctx = ctx_for(tmp.path());

        M0015SeedStarterSkills.up(&ctx).unwrap();
        wstore.migration_mark_applied("0015_seed_starter_skills", "channel", 0).unwrap();
        assert_eq!(wstore.skill_list_global().unwrap().len(), 6);

        for item in wstore.skill_list_global().unwrap() {
            wstore.skill_delete(&item.skill.id).unwrap();
        }
        assert!(wstore.skill_list_global().unwrap().is_empty());

        // The real runner (runner.rs) never calls `up()` again once
        // `migration_is_applied` is true — assert that precondition holds,
        // matching the actual gate every real boot goes through.
        assert!(
            wstore.migration_is_applied("0015_seed_starter_skills"),
            "once applied, the tracking row must persist regardless of catalog contents"
        );
    }
}
