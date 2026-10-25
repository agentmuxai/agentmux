// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Retired: this used to seed six starter skills into a fresh install's
//! Skills catalog. They were bound to no agent and nobody used them, so the
//! seed is gone and a fresh install starts with an empty catalog.
//!
//! The id stays in the registry because it is recorded as applied in every
//! existing install's `db_migrations` and ids must not disappear.
//! `0041_retire_untouched_starter_skills` removes the seeded skills nobody
//! edited or bound.

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0015SeedStarterSkills;

impl Migration for M0015SeedStarterSkills {
    fn id(&self) -> &'static str { "0015_seed_starter_skills" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Retired: used to seed the global Skills catalog with a starter set" }

    fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;

    #[test]
    fn a_fresh_channel_gets_no_skills() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mstore = Store::open(tmp.path()).unwrap();
        let ctx = MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-store.db"),
            channel_store_path: tmp.path().to_path_buf(),
        };

        M0015SeedStarterSkills.up(&ctx).unwrap();

        assert!(mstore.skill_list_global(&mstore).unwrap().is_empty());
    }
}
