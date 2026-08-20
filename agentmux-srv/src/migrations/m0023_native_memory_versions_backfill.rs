// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill `db_agent_native_memory_versions` with one `source:
//! "agent_inferred"` version per existing `db_agent_native_memory` row —
//! reagent P1 (re-review of PR #2674): the spec this migration completes
//! (`docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`
//! §9 Rollout) explicitly calls for this backfill so history views aren't
//! empty for pre-existing memory on upgrade. Without it, an existing
//! user's first post-upgrade overwrite of a memory file records only the
//! NEW content as version 1 — the prior live content (everything that
//! file held before this feature shipped) is permanently unrecoverable
//! via `MemoryRevert`/`MemoryDiff`.
//!
//! Global scope: `db_agent_native_memory` (source) and
//! `db_agent_native_memory_versions` (destination) both live in the
//! shared store (`ctx.shared_store_path`) — the same store `id_store`
//! resolves to at runtime, and the only one every native-memory RPC
//! handler actually reads/writes. The per-channel `objects.db` copies of
//! both tables exist only as `id_store`'s own pre-`0011_shared_store_backfill`
//! fallback (see each table's own `CREATE TABLE` doc comment in
//! `migrations.rs`) — on any install that has already passed that
//! migration (a precondition of this one even existing, since v6/v8 of
//! the shared schema postdate it), they're not the live data path, so
//! this migration doesn't scan them.
//!
//! Idempotent: only backfills a `(agent_id, filename)` pair that has NO
//! existing version yet. This matters for correctness, not just
//! re-run-safety — a pair that already has one or more versions (a normal
//! write happened after this feature shipped but before this migration
//! ran) must NOT get a backfilled version inserted now, because it would
//! land at the END of that pair's chain (this migration runs after normal
//! startup writes are already possible) with content that's actually
//! OLDER than what's already recorded, corrupting the chain's chronology.

use std::sync::Arc;

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0023NativeMemoryVersionsBackfill;

impl Migration for M0023NativeMemoryVersionsBackfill {
    fn id(&self) -> &'static str { "0023_native_memory_versions_backfill" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Backfill one agent_inferred version per existing native memory file"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.shared_store_path.exists() {
            return Ok(());
        }
        let store = Arc::new(
            Store::open_shared(&ctx.shared_store_path)
                .map_err(|e| MigrationError(format!("native_memory_versions_backfill: open shared store: {e}")))?,
        );

        let rows = store
            .agent_native_memory_list_all_rows()
            .map_err(|e| MigrationError(format!("native_memory_versions_backfill: list rows: {e}")))?;

        let mut backfilled = 0usize;
        let mut skipped_already_versioned = 0usize;
        for row in rows {
            let existing = store
                .agent_native_memory_version_latest(&row.agent_id, &row.filename)
                .map_err(|e| {
                    MigrationError(format!(
                        "native_memory_versions_backfill: check existing for {}/{}: {e}",
                        row.agent_id, row.filename
                    ))
                })?;
            if existing.is_some() {
                skipped_already_versioned += 1;
                continue;
            }
            store
                .agent_native_memory_version_insert(
                    &row.agent_id,
                    &row.filename,
                    &row.content,
                    "agent_inferred",
                    "{}",
                    "",
                )
                .map_err(|e| {
                    MigrationError(format!(
                        "native_memory_versions_backfill: insert for {}/{}: {e}",
                        row.agent_id, row.filename
                    ))
                })?;
            backfilled += 1;
        }

        tracing::info!(
            backfilled,
            skipped_already_versioned,
            "native_memory_versions_backfill: complete"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(shared_store_path: std::path::PathBuf) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path,
            channel_store_path: std::env::temp_dir().join("unused-objects.db"),
        }
    }

    #[test]
    fn backfills_a_version_for_an_existing_mirror_row() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open_shared(tmp.path()).unwrap();
        store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "pre-upgrade content", None, "/some/path", 20, 1000)
            .unwrap();

        M0023NativeMemoryVersionsBackfill.up(&ctx_for(tmp.path().to_path_buf())).unwrap();

        let versions = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].source, "agent_inferred");

        let latest = store.agent_native_memory_version_latest("agent-1", "MEMORY.md").unwrap().unwrap();
        assert_eq!(latest.content, "pre-upgrade content");
    }

    #[test]
    fn skips_a_pair_that_already_has_a_version() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open_shared(tmp.path()).unwrap();
        store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "mirror content", None, "/some/path", 14, 1000)
            .unwrap();
        // Simulates a normal write that already happened (post-upgrade,
        // pre-migration) — this pair must NOT get a backfilled version
        // appended after it.
        store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "already-recorded content", "human", "{}", "")
            .unwrap();

        M0023NativeMemoryVersionsBackfill.up(&ctx_for(tmp.path().to_path_buf())).unwrap();

        let versions = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(versions.len(), 1, "must not append a backfilled version onto an already-versioned pair");
    }

    #[test]
    fn is_idempotent_on_a_second_run() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open_shared(tmp.path()).unwrap();
        store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "content", None, "/some/path", 7, 1000)
            .unwrap();

        M0023NativeMemoryVersionsBackfill.up(&ctx_for(tmp.path().to_path_buf())).unwrap();
        M0023NativeMemoryVersionsBackfill.up(&ctx_for(tmp.path().to_path_buf())).unwrap();

        let versions = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(versions.len(), 1, "second run must not duplicate the backfilled version");
    }

    #[test]
    fn backfills_every_agent_and_file_independently() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open_shared(tmp.path()).unwrap();
        store.agent_native_memory_upsert("agent-1", "MEMORY.md", "a1", None, "/p", 2, 1000).unwrap();
        store.agent_native_memory_upsert("agent-1", "topic.md", "a2", None, "/p", 2, 1000).unwrap();
        store.agent_native_memory_upsert("agent-2", "MEMORY.md", "b1", None, "/p", 2, 1000).unwrap();

        M0023NativeMemoryVersionsBackfill.up(&ctx_for(tmp.path().to_path_buf())).unwrap();

        assert_eq!(store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap().len(), 1);
        assert_eq!(store.agent_native_memory_version_list("agent-1", "topic.md").unwrap().len(), 1);
        assert_eq!(store.agent_native_memory_version_list("agent-2", "MEMORY.md").unwrap().len(), 1);
    }

    #[test]
    fn missing_shared_store_is_a_noop() {
        let ctx = ctx_for(std::path::Path::new("Z:/does/not/exist/store.db").to_path_buf());
        M0023NativeMemoryVersionsBackfill.up(&ctx).unwrap();
    }
}
