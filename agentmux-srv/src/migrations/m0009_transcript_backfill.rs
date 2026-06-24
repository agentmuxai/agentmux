// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::backend::storage::filestore::FileStore;
use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0009TranscriptBackfill;

impl Migration for M0009TranscriptBackfill {
    fn id(&self) -> &'static str { "0009_transcript_backfill" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Backfill agent conversation transcripts into global shared store" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let transcripts_dir = ctx.home.join("shared").join("agents").join("transcripts");
        std::fs::create_dir_all(&transcripts_dir)
            .map_err(|e| MigrationError(format!("transcript_backfill: create transcripts dir: {}", e)))?;

        let global = FileStore::open(&transcripts_dir.join("filestore.db"))
            .map_err(|e| MigrationError(format!("transcript_backfill: open global filestore: {}", e)))?;

        // Collect definition IDs to seed transcripts for.
        let def_ids: Vec<String> = {
            let def_dir = ctx.home.join("shared").join("agents").join("definitions");
            if def_dir.exists() {
                registry::DefinitionStore::open(def_dir)
                    .ok()
                    .and_then(|ds| ds.list_active().ok())
                    .map(|recs| recs.into_iter().map(|r| r.data.id).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        };

        crate::backend::transcript_backfill::backfill_transcripts_once(
            &ctx.home,
            &transcripts_dir,
            &def_ids,
            &global,
        );
        Ok(())
    }
}
