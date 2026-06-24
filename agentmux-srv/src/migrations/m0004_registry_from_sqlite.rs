// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0004RegistryFromSqlite;

impl Migration for M0004RegistryFromSqlite {
    fn id(&self) -> &'static str { "0004_registry_from_sqlite" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Migrate named agent instances from SQLite to shared file registry" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let registry_root = ctx.home.join("shared").join("agents").join("registry");
        let reg = registry::Registry::open(registry_root)
            .map_err(|e| MigrationError(format!("registry_from_sqlite: open registry: {}", e)))?;
        registry::migrate_from_sqlite_once(&ctx.home, &reg)
            .map(|_| ())
            .map_err(|e| MigrationError(format!("registry_from_sqlite: {}", e)))
    }
}
