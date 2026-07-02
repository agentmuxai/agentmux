// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-time migration: per-block zones → per-agent zones.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::backend::obj::Block;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::super::helpers::{now_ms, write_zone_file};
use super::super::session_io::SNAPSHOT_FILE;
use super::super::zone_naming::{agent_archive_zone, agent_current_zone, is_valid_definition_id};

/// Marker file name for the per-data-dir one-shot migration gate.
pub const MIGRATION_MARKER_V1: &str = "migration_agent_zones_v1.flag";

/// Stats from `migrate_block_zones_v1`. Logged at INFO at startup.
#[derive(Debug, Clone, Default)]
pub struct MigrationStats {
    pub blocks_scanned: usize,
    pub archives_written: usize,
    pub current_zones_seeded: usize,
    pub skipped_no_snapshot: usize,
    pub failures: usize,
}

/// One-shot migration of per-block agent session zones to per-agent
/// zones. Gated by a marker file under `data_dir`; running twice is a
/// no-op.
///
/// Failure mode: per-block errors are logged + counted; we do NOT
/// abort startup. The marker file is written even on partial failure
/// so we don't retry indefinitely — operators can delete the marker
/// to force a re-run.
pub fn migrate_block_zones_v1(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    data_dir: &Path,
) -> MigrationStats {
    let marker_path = data_dir.join(MIGRATION_MARKER_V1);
    if marker_path.exists() {
        tracing::debug!(
            marker = %marker_path.display(),
            "agent_session migration: marker present, skipping"
        );
        return MigrationStats::default();
    }

    let mut stats = MigrationStats::default();

    let blocks: Vec<Block> = match wstore.get_all::<Block>() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agent_session migration: wstore.get_all<Block> failed; skipping migration"
            );
            // Don't write the marker — let the next start retry.
            return stats;
        }
    };

    // Track the most-recently-modified block snapshot per definition_id.
    // Value: (modts_ms, snapshot_bytes).
    let mut per_def_latest: HashMap<String, (i64, Vec<u8>)> = HashMap::new();

    for block in &blocks {
        let view = block.meta.get("view").and_then(|v| v.as_str()).unwrap_or("");
        if view != "agent" {
            continue;
        }
        // The agent definition id is stored under either `agentId`
        // (current shape, set by `agent.open` + frontend launch flow)
        // or the legacy `agent:id`. Skip blocks without an id.
        let def_id = block
            .meta
            .get("agentId")
            .and_then(|v| v.as_str())
            .or_else(|| block.meta.get("agent:id").and_then(|v| v.as_str()))
            .unwrap_or("");
        if !is_valid_definition_id(def_id) {
            continue;
        }
        stats.blocks_scanned += 1;

        // Read the per-block snapshot. Both missing and zero-byte are
        // "skip" — no point archiving an empty snapshot.
        let snapshot_stat = match filestore.stat(&block.oid, SNAPSHOT_FILE) {
            Ok(Some(f)) => f,
            Ok(None) => {
                stats.skipped_no_snapshot += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    block_id = %block.oid,
                    error = %e,
                    "agent_session migration: stat failed; skipping"
                );
                stats.failures += 1;
                continue;
            }
        };
        if snapshot_stat.size == 0 {
            stats.skipped_no_snapshot += 1;
            continue;
        }

        let snapshot_bytes = match filestore.read_file(&block.oid, SNAPSHOT_FILE) {
            Ok(Some(b)) => b,
            _ => {
                stats.failures += 1;
                continue;
            }
        };

        // 1) Backfill an archive zone keyed on the block snapshot's
        //    createdts (closest available proxy for "when this
        //    conversation started"). Falls back to modts when
        //    createdts is missing/zero.
        let mut archive_ts: u64 = if snapshot_stat.createdts > 0 {
            snapshot_stat.createdts as u64
        } else if snapshot_stat.modts > 0 {
            snapshot_stat.modts as u64
        } else {
            now_ms()
        };
        // Avoid collisions when multiple block zones share the same
        // createdts (test fixtures, second-precision rounding, etc.):
        // bump the timestamp by 1ms until the archive zone is unique.
        loop {
            let candidate = agent_archive_zone(def_id, archive_ts);
            let occupied = matches!(
                filestore.stat(&candidate, SNAPSHOT_FILE),
                Ok(Some(_))
            );
            if !occupied {
                break;
            }
            archive_ts += 1;
        }
        let archive_zone = agent_archive_zone(def_id, archive_ts);
        if let Err(e) = write_zone_file(filestore, &archive_zone, SNAPSHOT_FILE, &snapshot_bytes) {
            tracing::warn!(
                block_id = %block.oid,
                definition_id = %def_id,
                error = %e,
                "agent_session migration: archive write failed"
            );
            stats.failures += 1;
            continue;
        }
        stats.archives_written += 1;

        // 2) Track the most-recently-modified per definition so we
        //    can seed the `:current` zone after the scan.
        let entry = per_def_latest
            .entry(def_id.to_string())
            .or_insert_with(|| (0, Vec::new()));
        if snapshot_stat.modts > entry.0 {
            *entry = (snapshot_stat.modts, snapshot_bytes);
        }
    }

    // 3) Seed `:current` for each definition from its
    //    most-recently-modified per-block snapshot. If a `:current`
    //    zone is already populated (e.g. a partial prior migration
    //    left it behind), skip — we don't want to overwrite live data.
    for (def_id, (_modts, bytes)) in per_def_latest {
        let current_zone = agent_current_zone(&def_id);
        let already = matches!(
            filestore.stat(&current_zone, SNAPSHOT_FILE),
            Ok(Some(f)) if f.size > 0
        );
        if already {
            continue;
        }
        match write_zone_file(filestore, &current_zone, SNAPSHOT_FILE, &bytes) {
            Ok(()) => {
                stats.current_zones_seeded += 1;
            }
            Err(e) => {
                tracing::warn!(
                    definition_id = %def_id,
                    error = %e,
                    "agent_session migration: current-zone seed failed"
                );
                stats.failures += 1;
            }
        }
    }

    // Write marker — even on partial failure (see doc comment).
    if let Err(e) = std::fs::write(&marker_path, b"v1\n") {
        tracing::warn!(
            marker = %marker_path.display(),
            error = %e,
            "agent_session migration: marker write failed; migration may re-run on next startup"
        );
    }

    tracing::info!(
        blocks_scanned = stats.blocks_scanned,
        archives_written = stats.archives_written,
        current_zones_seeded = stats.current_zones_seeded,
        skipped_no_snapshot = stats.skipped_no_snapshot,
        failures = stats.failures,
        "agent_session migration: complete"
    );

    stats
}
