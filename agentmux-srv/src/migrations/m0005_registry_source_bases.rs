// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0005RegistrySourceBases;

impl Migration for M0005RegistrySourceBases {
    fn id(&self) -> &'static str { "0005_registry_source_bases" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Backfill source_agents_base on registry instance records" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let registry_root = ctx.home.join("shared").join("agents").join("registry");
        let reg = registry::Registry::open(registry_root)
            .map_err(|e| MigrationError(format!("registry_source_bases: open registry: {}", e)))?;
        registry::backfill_source_bases_once(&ctx.home, &reg)
            .map(|_| ())
            .map_err(|e| MigrationError(format!("registry_source_bases: {}", e)))
    }
}
