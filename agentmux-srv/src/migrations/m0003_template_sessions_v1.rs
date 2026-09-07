// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use crate::backend::storage::store::Store;
use crate::backend::storage::filestore::FileStore;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0003TemplateSessionsV1;

impl Migration for M0003TemplateSessionsV1 {
    fn id(&self) -> &'static str { "0003_template_sessions_v1" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Promote template sessions to user-owned definitions" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("template_sessions_v1: open wstore: {}", e)))?,
        );
        let filestore_path = ctx.data_dir.join("db").join("filestore.db");
        let filestore = Arc::new(
            FileStore::open(&filestore_path)
                .map_err(|e| MigrationError(format!("template_sessions_v1: open filestore: {}", e)))?,
        );
        crate::backend::agent_session::migrate_promote_template_sessions_v1(&wstore, &filestore, &ctx.data_dir);
        Ok(())
    }

    // No `verify()` on purpose. SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03
    // Phase 1 lists 0003 alongside 0002 as "the same legacy-marker pattern",
    // but it is not: `migrate_promote_template_sessions_v1` explicitly
    // IGNORES its marker file and re-asserts the data invariant ("no seeded
    // definition has a session zone") on every startup — see
    // `agent_session/migrations/v1_templates.rs`'s doc comment. A mismatch
    // here is self-healing on the next start, so a doctor check would report
    // a condition that cannot persist. Reporting `NotVerifiable` is more
    // honest than a check whose "mismatch" means nothing.
}
