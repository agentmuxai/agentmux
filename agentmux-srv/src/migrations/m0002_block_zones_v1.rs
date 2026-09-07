// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use crate::backend::storage::store::Store;
use crate::backend::storage::filestore::FileStore;
use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0002BlockZonesV1;

impl Migration for M0002BlockZonesV1 {
    fn id(&self) -> &'static str { "0002_block_zones_v1" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Migrate per-block agent session zones to per-agent zones" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("block_zones_v1: open wstore: {}", e)))?,
        );
        let filestore_path = ctx.data_dir.join("db").join("filestore.db");
        let filestore = Arc::new(
            FileStore::open(&filestore_path)
                .map_err(|e| MigrationError(format!("block_zones_v1: open filestore: {}", e)))?,
        );
        // A scan that could not even read the blocks table is a failed
        // migration, not an applied one (codex P1 on #3070).
        crate::backend::agent_session::migrate_block_zones_v1(&wstore, &filestore, &ctx.data_dir)
            .map_err(|e| MigrationError(format!("block_zones_v1: {}", e)))?;
        Ok(())
    }

    /// Doctor check, content-based since Phase 2 of
    /// SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03: the post-condition is
    /// "no agent block still holds a snapshot whose agent `:current` zone is
    /// empty" (`block_zones_look_incomplete`), read through the same
    /// read-only connections `--verify` uses everywhere. The marker file is
    /// reported as evidence only — Phase 1b's version of this check trusted
    /// its presence, which is the F1 shape Phase 2 exists to remove.
    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        if !ctx.channel_store_path.exists() {
            return VerifyOutcome::Ok("no channel store on this data dir; nothing to migrate".into());
        }
        let filestore_path = ctx.data_dir.join("db").join("filestore.db");
        if !filestore_path.exists() {
            return VerifyOutcome::Ok("no filestore on this data dir; no block snapshots to migrate".into());
        }
        // Read-only openers, like every other verifier: no WAL pragma, no
        // schema setup, no version stamp, works on a read-only backup
        // (codex P2 on #3070).
        let wstore = match Store::open_read_only(&ctx.channel_store_path) {
            Ok(s) => s,
            Err(e) => return VerifyOutcome::Error(format!("open channel store read-only: {}", e)),
        };
        let filestore = match FileStore::open_read_only(&filestore_path) {
            Ok(f) => f,
            Err(e) => return VerifyOutcome::Error(format!("open filestore read-only: {}", e)),
        };
        let marker = ctx.data_dir.join(crate::backend::agent_session::MIGRATION_MARKER_V1);
        let marker_note = if marker.exists() { "marker present" } else { "marker absent" };
        match crate::backend::agent_session::block_zones_look_incomplete(&wstore, &filestore) {
            Ok(true) => VerifyOutcome::Mismatch(format!(
                "an agent block still holds a snapshot whose agent :current zone is empty — recorded applied, zones not migrated ({})",
                marker_note
            )),
            Ok(false) => VerifyOutcome::Ok(format!("every block snapshot has a populated agent :current zone ({})", marker_note)),
            // Could not observe the blocks at all — that is not "complete".
            Err(e) => VerifyOutcome::Error(format!("{} ({})", e, marker_note)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_session::{agent_current_zone, MIGRATION_MARKER_V1};
    use crate::backend::obj::{Block, MetaMapType};
    use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

    fn data_dir_with_orphaned_block() -> (tempfile::TempDir, MigrationContext) {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let db = data_dir.join("db");
        std::fs::create_dir_all(&db).unwrap();
        let ctx = MigrationContext {
            home: dir.path().to_path_buf(),
            data_dir: data_dir.clone(),
            shared_store_path: dir.path().join("shared").join("store.db"),
            channel_store_path: db.join("objects.db"),
        };
        let wstore = Store::open(&ctx.channel_store_path).unwrap();
        let filestore = FileStore::open(&db.join("filestore.db")).unwrap();
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        meta.insert("agentId".to_string(), serde_json::json!("def-v"));
        let mut block = Block {
            oid: "block-v".to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta,
            subblockids: None,
        };
        wstore.insert(&mut block).unwrap();
        filestore.make_file("block-v", "output.state.json", FileMeta::default(), FileOpts::default()).unwrap();
        filestore.write_file("block-v", "output.state.json", br#"{"nodes":[]}"#).unwrap();
        (dir, ctx)
    }

    #[test]
    fn verify_is_content_based_and_reports_the_marker_only_as_a_note() {
        let (_dir, ctx) = data_dir_with_orphaned_block();
        // Orphaned snapshot, no marker: Mismatch.
        match M0002BlockZonesV1.verify(&ctx) {
            VerifyOutcome::Mismatch(m) => assert!(m.contains("marker absent"), "{m}"),
            other => panic!("expected Mismatch, got {other:?}"),
        }
        // A stale marker changes nothing about the verdict.
        std::fs::write(ctx.data_dir.join(MIGRATION_MARKER_V1), b"v1\n").unwrap();
        match M0002BlockZonesV1.verify(&ctx) {
            VerifyOutcome::Mismatch(m) => assert!(m.contains("marker present"), "{m}"),
            other => panic!("expected Mismatch, got {other:?}"),
        }
        // Once :current is populated the data is complete, marker or not.
        let filestore = FileStore::open(&ctx.data_dir.join("db").join("filestore.db")).unwrap();
        let zone = agent_current_zone("def-v");
        filestore.make_file(&zone, "output.state.json", FileMeta::default(), FileOpts::default()).unwrap();
        filestore.write_file(&zone, "output.state.json", br#"{"nodes":[]}"#).unwrap();
        std::fs::remove_file(ctx.data_dir.join(MIGRATION_MARKER_V1)).unwrap();
        assert!(matches!(M0002BlockZonesV1.verify(&ctx), VerifyOutcome::Ok(_)));
    }

    #[test]
    fn verify_opens_both_stores_read_only_and_never_creates_them() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(data_dir.join("db")).unwrap();
        let ctx = MigrationContext {
            home: dir.path().to_path_buf(),
            data_dir: data_dir.clone(),
            shared_store_path: dir.path().join("shared").join("store.db"),
            channel_store_path: data_dir.join("db").join("objects.db"),
        };
        assert!(matches!(M0002BlockZonesV1.verify(&ctx), VerifyOutcome::Ok(_)));
        assert!(!ctx.channel_store_path.exists(), "verify must not create the channel store");
        // A channel store without a filestore: still nothing to migrate, and
        // no filestore appears.
        drop(Store::open(&ctx.channel_store_path).unwrap());
        assert!(matches!(M0002BlockZonesV1.verify(&ctx), VerifyOutcome::Ok(_)));
        assert!(!data_dir.join("db").join("filestore.db").exists(), "verify must not create the filestore");
    }

    #[test]
    fn verify_reports_an_unreadable_blocks_table_as_an_error_not_complete() {
        let (_dir, ctx) = data_dir_with_orphaned_block();
        // Drop the blocks table: the scan cannot run, so "complete" cannot be
        // claimed (codex P1 on #3070).
        let conn = rusqlite::Connection::open(&ctx.channel_store_path).unwrap();
        conn.execute_batch("DROP TABLE db_block;").unwrap();
        drop(conn);
        assert!(matches!(M0002BlockZonesV1.verify(&ctx), VerifyOutcome::Error(_)));
    }
}
