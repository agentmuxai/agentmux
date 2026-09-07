// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-time migration: per-block zones → per-agent zones.
//!
//! Gated by the migration framework (`db_migrations`, via
//! `migrations::m0002_block_zones_v1`), NOT by its own marker file — see
//! `migrate_block_zones_v1`'s doc comment for what the marker is still for.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::backend::obj::Block;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::super::helpers::{now_ms, write_zone_file};
use super::super::session_io::SNAPSHOT_FILE;
use super::super::zone_naming::{agent_archive_zone, agent_current_zone, is_valid_definition_id};

/// Marker file written under the data dir when the migration completes.
///
/// It is **evidence, not a gate**: `m0000_bootstrap` reads it (together with
/// [`block_zones_look_incomplete`]) to stamp `0002` as already applied on an
/// install that ran this migration before the framework existed, and
/// `m0002`'s doctor check reports it. The migration itself no longer
/// short-circuits on its presence — SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03
/// Phase 2: a marker whose mere *existence* is trusted as "done" is exactly
/// the shape that left `0007`'s target table empty (§1.2/F1); a restored
/// `objects.db` beside a stale flag would have skipped every block's
/// conversation for good. Whether work is needed is now decided from the
/// data, and the work is content-idempotent, so running twice is safe
/// without a flag.
pub const MIGRATION_MARKER_V1: &str = "migration_agent_zones_v1.flag";

/// Stats from `migrate_block_zones_v1`. Logged at INFO at startup.
#[derive(Debug, Clone, Default)]
pub struct MigrationStats {
    pub blocks_scanned: usize,
    pub archives_written: usize,
    /// Block snapshots whose bytes already existed as an archive for the
    /// same agent — a re-run over migrated data, written nothing.
    pub archives_already_present: usize,
    pub current_zones_seeded: usize,
    pub skipped_no_snapshot: usize,
    pub failures: usize,
}

/// The agent definition id a block carries, if it is an agent block with a
/// valid one. Stored under `agentId` (current shape, set by `agent.open` +
/// the frontend launch flow) or the legacy `agent:id`.
fn agent_definition_id(block: &Block) -> Option<&str> {
    let view = block.meta.get("view").and_then(|v| v.as_str()).unwrap_or("");
    if view != "agent" {
        return None;
    }
    let def_id = block
        .meta
        .get("agentId")
        .and_then(|v| v.as_str())
        .or_else(|| block.meta.get("agent:id").and_then(|v| v.as_str()))
        .unwrap_or("");
    is_valid_definition_id(def_id).then_some(def_id)
}

/// Every archive snapshot already stored for `def_id`, by bytes. Read once
/// per definition so the dedupe below is O(archives) per agent, not per
/// block. Enumerates zone ids directly rather than through `list_archives`,
/// which caps at 100 rows and reads previews this has no use for.
fn existing_archive_snapshots(filestore: &FileStore, def_id: &str) -> Vec<Vec<u8>> {
    let prefix = format!("agent:{}:archive:", def_id);
    let Ok(zones) = filestore.get_all_zone_ids() else {
        return Vec::new();
    };
    zones
        .iter()
        .filter(|z| z.starts_with(&prefix))
        .filter_map(|z| filestore.read_file(z, SNAPSHOT_FILE).ok().flatten())
        .collect()
}

/// Read-only content check: is there an agent block whose per-block
/// snapshot has bytes, while its agent's `:current` zone is still empty?
/// That is the state this migration exists to fix, so `true` means it has
/// not (fully) run on this data — regardless of what any marker says.
///
/// Used by `m0000_bootstrap` (never stamp `0002` off the flag alone) and by
/// `m0002`'s `verify()`. Never writes. `Err` when the blocks table cannot be
/// read at all — that is not "complete", and the callers must not treat it
/// as such (codex P1 on #3070): bootstrap fails rather than stamps, the
/// doctor reports an error.
pub fn block_zones_look_incomplete(wstore: &Store, filestore: &FileStore) -> Result<bool, String> {
    let blocks = wstore.get_all::<Block>().map_err(|e| format!("read blocks: {e}"))?;
    let mut current_populated: HashMap<String, bool> = HashMap::new();
    for block in &blocks {
        let Some(def_id) = agent_definition_id(block) else { continue };
        let has_snapshot = matches!(filestore.stat(&block.oid, SNAPSHOT_FILE), Ok(Some(f)) if f.size > 0);
        if !has_snapshot {
            continue;
        }
        let populated = *current_populated.entry(def_id.to_string()).or_insert_with(|| {
            matches!(filestore.stat(&agent_current_zone(def_id), SNAPSHOT_FILE), Ok(Some(f)) if f.size > 0)
        });
        if !populated {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One-shot migration of per-block agent session zones to per-agent
/// zones. Content-idempotent: an archive is only written when no archive
/// for that agent already holds the same bytes, and `:current` is only
/// seeded when empty — so running twice (or over a data dir that a stale
/// [`MIGRATION_MARKER_V1`] wrongly calls done) writes nothing twice. The
/// framework's `db_migrations` row is the gate; the marker is written at the
/// end as evidence only.
///
/// Failure mode: per-block errors are logged + counted and do not abort the
/// run. A `get_all::<Block>` failure is different — nothing was scanned — and
/// is returned as `Err` so `m0002::up` fails the migration instead of
/// letting the runner record it applied around a scan that never happened
/// (codex P1 on #3070; before Phase 2 this returned default stats and `up`
/// reported `Ok`).
pub fn migrate_block_zones_v1(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    data_dir: &Path,
) -> Result<MigrationStats, String> {
    let marker_path = data_dir.join(MIGRATION_MARKER_V1);
    let mut stats = MigrationStats::default();

    let blocks: Vec<Block> = wstore
        .get_all::<Block>()
        .map_err(|e| format!("agent_session migration: read blocks: {e}"))?;

    // Track the most-recently-modified block snapshot per definition_id.
    // Value: (modts_ms, snapshot_bytes).
    let mut per_def_latest: HashMap<String, (i64, Vec<u8>)> = HashMap::new();
    // Archive bytes already on disk per definition, loaded on first use and
    // extended as this run writes, so two blocks with identical snapshots
    // within one run also collapse to one archive.
    let mut known_archives: HashMap<String, Vec<Vec<u8>>> = HashMap::new();

    for block in &blocks {
        let Some(def_id) = agent_definition_id(block) else { continue };
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

        // 2) (tracked before 1 so a deduped archive still competes for
        //    `:current`) Track the most-recently-modified per definition so
        //    we can seed the `:current` zone after the scan.
        let entry = per_def_latest
            .entry(def_id.to_string())
            .or_insert_with(|| (0, Vec::new()));
        if snapshot_stat.modts > entry.0 {
            *entry = (snapshot_stat.modts, snapshot_bytes.clone());
        }

        // 1) Backfill an archive zone keyed on the block snapshot's
        //    createdts (closest available proxy for "when this
        //    conversation started"). Falls back to modts when
        //    createdts is missing/zero. Skipped when an archive with these
        //    exact bytes already exists for this agent — the re-run case.
        let archives = known_archives
            .entry(def_id.to_string())
            .or_insert_with(|| existing_archive_snapshots(filestore, def_id));
        if archives.iter().any(|a| *a == snapshot_bytes) {
            stats.archives_already_present += 1;
            continue;
        }
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
        archives.push(snapshot_bytes);
        stats.archives_written += 1;
    }

    // 3) Seed `:current` for each definition from its
    //    most-recently-modified per-block snapshot. If a `:current`
    //    zone is already populated (a prior run, or live data), skip — we
    //    don't want to overwrite it.
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

    // Write the marker — evidence for m0000's pre-framework stamping and
    // the doctor, never consulted by this function (see its doc comment).
    if let Err(e) = std::fs::write(&marker_path, b"v1\n") {
        tracing::warn!(
            marker = %marker_path.display(),
            error = %e,
            "agent_session migration: marker write failed"
        );
    }

    tracing::info!(
        blocks_scanned = stats.blocks_scanned,
        archives_written = stats.archives_written,
        archives_already_present = stats.archives_already_present,
        current_zones_seeded = stats.current_zones_seeded,
        skipped_no_snapshot = stats.skipped_no_snapshot,
        failures = stats.failures,
        "agent_session migration: complete"
    );

    Ok(stats)
}
