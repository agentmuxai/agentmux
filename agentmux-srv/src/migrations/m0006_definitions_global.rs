// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0006DefinitionsGlobal;

impl Migration for M0006DefinitionsGlobal {
    fn id(&self) -> &'static str { "0006_definitions_global" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Migrate agent definitions from per-channel to global shared store" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let def_dir = ctx.home.join("shared").join("agents").join("definitions");
        let def_store = registry::DefinitionStore::open(def_dir)
            .map_err(|e| MigrationError(format!("definitions_global: open def store: {}", e)))?;
        registry::migrate_definitions_global_once(&ctx.home, &def_store)
            .map(|_| ())
            .map_err(|e| MigrationError(format!("definitions_global: {}", e)))
    }
}
