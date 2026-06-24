// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0008DefaultBundle;

impl Migration for M0008DefaultBundle {
    fn id(&self) -> &'static str { "0008_default_bundle" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Seed Default OAuth identity bundle from ambient credentials" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if let Some(parent) = ctx.channel_store_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MigrationError(format!("default_bundle: create db dir: {}", e)))?;
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("default_bundle: open wstore: {}", e)))?,
        );
        crate::identity::migration::run_default_bundle_migration(&wstore, None, None);
        Ok(())
    }
}
