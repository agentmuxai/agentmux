// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0010SessionIds;

impl Migration for M0010SessionIds {
    fn id(&self) -> &'static str { "0010_session_ids" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Backfill session_id on registry records for cross-channel resume" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let registry_root = ctx.home.join("shared").join("agents").join("registry");
        if !registry_root.exists() {
            return Ok(());
        }
        let reg = registry::Registry::open(registry_root)
            .map_err(|e| MigrationError(format!("session_ids: open registry: {}", e)))?;
        let shared_dir = ctx.home.join("shared");
        crate::backend::session_backfill::backfill_session_ids(&reg, &shared_dir);
        Ok(())
    }
}
