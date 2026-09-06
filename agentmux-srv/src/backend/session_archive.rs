// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session archival and cleanup (Phase 3.3 — ultra-long-sessions).
//!
//! Responsibilities:
//!   1. Periodic sweep: archive sessions inactive for `inactive_days` days by
//!      compressing their FileStore "output" file to `archive_dir/<block_id>.jsonl.gz`
//!      and freeing the FileStore entry.
//!   2. Storage cap: after archiving, prune oldest `.gz` files until total archive
//!      disk usage is below `max_total_bytes` (default 2 GB).
//!
//! The archive/restore/export logic used by the sweep is the same as the
//! RPC handlers in `server/app_api.rs` — both call `archive_session_output`
//! and `read_session_output` from this module.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::backend::blockcontroller::session_stats::{
    META_SESSION_LAST_ACTIVITY_MS, META_SESSION_LINE_COUNT,
};
use crate::backend::obj::{Block, MetaMapType};
use crate::backend::storage::filestore::{FileStore, FileMeta, FileOpts};
use crate::backend::storage::store::Store;

// ---- Meta key constants for archival state ----

/// Unix ms when the session was last archived.
pub const META_SESSION_ARCHIVED_AT: &str = "session:archived_at";
/// Byte count before compression (original output size).
pub const META_SESSION_ARCHIVED_BYTES: &str = "session:archived_bytes";
/// Absolute path to the `.jsonl.gz` archive file.
pub const META_SESSION_ARCHIVE_PATH: &str = "session:archive_path";

/// FileStore filename for session output.
const OUTPUT_FILENAME: &str = "output";
/// Receive-time sidecar (see `agent_session::TSIDX_FILE`) — archived,
/// deleted, and restored in lockstep with `output` (codex P2 on PR #2508):
/// leaving it behind would mis-time a fresh session's lines (new output
/// restarts at offset 0 under stale entries), and dropping it from the
/// archive loses the timestamps the sidecar exists to preserve.
const TSIDX_FILENAME: &str = crate::backend::agent_session::session_io::TSIDX_FILE;

// ---------------------------------------------------------------------------
// Shared helpers — used by both the RPC handlers and the sweep
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

/// Compress `data` with gzip and write to `dest_path`.
fn write_gz(data: &[u8], dest_path: &Path) -> Result<(), String> {
    let file = std::fs::File::create(dest_path)
        .map_err(|e| format!("create archive file {}: {e}", dest_path.display()))?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(data)
        .map_err(|e| format!("compress: {e}"))?;
    encoder.finish()
        .map_err(|e| format!("gz finish: {e}"))?;
    Ok(())
}

/// Decompress a `.gz` file and return raw bytes.
fn read_gz(src_path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(src_path)
        .map_err(|e| format!("open archive {}: {e}", src_path.display()))?;
    let mut decoder = GzDecoder::new(file);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)
        .map_err(|e| format!("decompress: {e}"))?;
    Ok(out)
}

/// Ensure the archive directory exists.
fn ensure_archive_dir(archive_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(archive_dir)
        .map_err(|e| format!("create archive dir {}: {e}", archive_dir.display()))
}

// ---------------------------------------------------------------------------
// archive_session_output
// ---------------------------------------------------------------------------

/// Archive the FileStore "output" file for `block_id`:
///   1. Read bytes from FileStore.
///   2. Compress to `archive_dir/<block_id>.jsonl.gz`.
///   3. Delete the FileStore entry to reclaim SQLite space.
///   4. Write archive meta keys to the block.
///
/// Returns `(archived_bytes, archived_at_ms)`.
/// If the FileStore entry is missing or empty, returns `(0, now_ms)` — no-op.
pub fn archive_session_output(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    block_id: &str,
    archive_dir: &Path,
) -> Result<(u64, i64), String> {
    // Read existing data from FileStore
    let raw_bytes = match filestore.read_file(block_id, OUTPUT_FILENAME) {
        Ok(Some(b)) if !b.is_empty() => b,
        Ok(_) => {
            // Nothing to archive
            return Ok((0, now_ms()));
        }
        Err(e) => return Err(format!("filestore read: {e}")),
    };

    let archived_bytes = raw_bytes.len() as u64;

    ensure_archive_dir(archive_dir)?;

    let archive_path = archive_dir.join(format!("{}.jsonl.gz", block_id));
    write_gz(&raw_bytes, &archive_path)?;

    let archived_at = now_ms();

    // Write archive metadata BEFORE deleting the FileStore entry. If the meta
    // update fails, the session data is still retrievable from FileStore and
    // the orphaned .gz can be cleaned up by the next sweep. Deleting first
    // would orphan the session if the meta write then failed.
    let mut meta = MetaMapType::new();
    meta.insert(META_SESSION_ARCHIVED_AT.to_string(), serde_json::json!(archived_at));
    meta.insert(META_SESSION_ARCHIVED_BYTES.to_string(), serde_json::json!(archived_bytes));
    meta.insert(
        META_SESSION_ARCHIVE_PATH.to_string(),
        serde_json::json!(archive_path.to_string_lossy().as_ref()),
    );

    let oref_str = format!("block:{}", block_id);
    if let Err(e) = crate::server::service::update_object_meta(wstore, &oref_str, &meta) {
        // Roll back the archive file so we don't leak disk on retry
        let _ = std::fs::remove_file(&archive_path);
        return Err(format!("update_object_meta: {e}"));
    }

    // Meta is now persisted; safe to reclaim the FileStore entry
    if let Err(e) = filestore.delete_file(block_id, OUTPUT_FILENAME) {
        tracing::warn!(
            block_id = %block_id,
            error = %e,
            "session_archive: failed to delete filestore entry after archiving (meta already updated)"
        );
    }
    // Sidecar follows output: archive its bytes next to the .jsonl.gz
    // (best-effort — auxiliary timing data), then delete so a restarted
    // session can't inherit stale offset stamps.
    if let Ok(Some(ts_bytes)) = filestore.read_file(block_id, TSIDX_FILENAME) {
        if !ts_bytes.is_empty() {
            let ts_path = archive_dir.join(format!("{}.tsidx.gz", block_id));
            if let Err(e) = write_gz(&ts_bytes, &ts_path) {
                tracing::warn!(block_id = %block_id, error = %e, "session_archive: tsidx archive write failed (output archive unaffected)");
            }
        }
    }
    match filestore.stat(block_id, TSIDX_FILENAME) {
        Ok(Some(_)) => {
            if let Err(e) = filestore.delete_file(block_id, TSIDX_FILENAME) {
                tracing::warn!(block_id = %block_id, error = %e, "session_archive: failed to delete tsidx sidecar after archiving");
            }
        }
        _ => {}
    }

    tracing::info!(
        block_id = %block_id,
        archived_bytes = archived_bytes,
        archive_path = %archive_path.display(),
        "session archived"
    );

    Ok((archived_bytes, archived_at))
}

// ---------------------------------------------------------------------------
// restore_session_output
// ---------------------------------------------------------------------------

/// Restore an archived session back into FileStore:
///   1. Read `session:archive_path` from block meta.
///   2. Decompress and write bytes back via `make_file` + `append_data`.
///   3. Clear archive meta keys (keep the .gz file as backup).
pub fn restore_session_output(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    block_id: &str,
) -> Result<u64, String> {
    // Read archive path from block meta
    let block: Block = wstore
        .get(block_id)
        .map_err(|e| format!("wstore.get: {e}"))?
        .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", block_id))?;

    let archive_path_str = block
        .meta
        .get(META_SESSION_ARCHIVE_PATH)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("session:archive_path not set for block {}", block_id))?
        .to_string();

    let archive_path = PathBuf::from(&archive_path_str);
    let raw_bytes = read_gz(&archive_path)?;
    let restored_bytes = raw_bytes.len() as u64;

    // Recreate the FileStore entry (may already be gone after archival)
    let _ = filestore.delete_file(block_id, OUTPUT_FILENAME); // ignore "not found"
    filestore
        .make_file(block_id, OUTPUT_FILENAME, FileMeta::default(), FileOpts::default())
        .map_err(|e| format!("make_file: {e}"))?;
    filestore
        .append_data(block_id, OUTPUT_FILENAME, &raw_bytes)
        .map_err(|e| format!("append_data: {e}"))?;

    // Restore the tsidx sidecar when its archive exists (best-effort —
    // pre-sidecar archives simply have none, and restored history without
    // stamps degrades to the carry-forward day rule, not an error).
    let ts_path = archive_path
        .parent()
        .map(|d| d.join(format!("{}.tsidx.gz", block_id)))
        .filter(|p| p.exists());
    let _ = filestore.delete_file(block_id, TSIDX_FILENAME); // ignore "not found"
    if let Some(ts_path) = ts_path {
        match read_gz(&ts_path) {
            Ok(ts_bytes) if !ts_bytes.is_empty() => {
                let created = filestore
                    .make_file(block_id, TSIDX_FILENAME, FileMeta::default(), FileOpts::default());
                if created.is_ok() {
                    if let Err(e) = filestore.append_data(block_id, TSIDX_FILENAME, &ts_bytes) {
                        tracing::warn!(block_id = %block_id, error = %e, "session_archive: tsidx restore append failed");
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(block_id = %block_id, error = %e, "session_archive: tsidx restore read failed");
            }
        }
    }

    // Clear archive meta keys (keep the .gz as backup — don't delete it)
    let mut meta = MetaMapType::new();
    meta.insert(META_SESSION_ARCHIVED_AT.to_string(), serde_json::Value::Null);
    meta.insert(META_SESSION_ARCHIVED_BYTES.to_string(), serde_json::Value::Null);
    meta.insert(META_SESSION_ARCHIVE_PATH.to_string(), serde_json::Value::Null);

    let oref_str = format!("block:{}", block_id);
    crate::server::service::update_object_meta(wstore, &oref_str, &meta)
        .map_err(|e| format!("update_object_meta: {e}"))?;

    tracing::info!(
        block_id = %block_id,
        restored_bytes = restored_bytes,
        archive_path = %archive_path_str,
        "session restored"
    );

    Ok(restored_bytes)
}

// ---------------------------------------------------------------------------
// read_session_output — used by session:export
// ---------------------------------------------------------------------------

/// Read the raw session output bytes, whether from FileStore (live) or archive.
/// Returns `(bytes, line_count)`.
pub fn read_session_output(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    block_id: &str,
) -> Result<(Vec<u8>, u64), String> {
    // Check if session is archived
    let block: Block = wstore
        .get(block_id)
        .map_err(|e| format!("wstore.get: {e}"))?
        .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", block_id))?;

    let is_archived = block
        .meta
        .get(META_SESSION_ARCHIVED_AT)
        .and_then(|v| v.as_i64())
        .map(|v| v > 0)
        .unwrap_or(false);

    let raw_bytes = if is_archived {
        let archive_path_str = block
            .meta
            .get(META_SESSION_ARCHIVE_PATH)
            .and_then(|v| v.as_str())
            .ok_or_else(|| "session:archive_path not set".to_string())?
            .to_string();
        read_gz(Path::new(&archive_path_str))?
    } else {
        filestore
            .read_file(block_id, OUTPUT_FILENAME)
            .map_err(|e| format!("filestore read: {e}"))?
            .unwrap_or_default()
    };

    let line_count = bytecount_lines(&raw_bytes);
    Ok((raw_bytes, line_count))
}

/// Count non-empty lines in a byte buffer.
fn bytecount_lines(data: &[u8]) -> u64 {
    let text = String::from_utf8_lossy(data);
    text.lines().filter(|l| !l.trim().is_empty()).count() as u64
}

// ---------------------------------------------------------------------------
// Default archive directory
// ---------------------------------------------------------------------------

/// Returns `~/.agentmux/archives/`, or `None` if the home directory cannot be determined.
pub fn default_archive_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".agentmux").join("archives"))
}

// ---------------------------------------------------------------------------
// SessionArchiver — periodic sweep
// ---------------------------------------------------------------------------

/// Periodic session archival + storage cap enforcement.
pub struct SessionArchiver {
    wstore: Arc<Store>,
    filestore: Arc<FileStore>,
    /// Sessions with no activity for this many days get archived.
    pub inactive_days: u64,
    /// Maximum total bytes across all `.jsonl.gz` archives.
    pub max_total_bytes: u64,
    /// Directory where `.jsonl.gz` files are written.
    pub archive_dir: PathBuf,
}

/// Statistics from a single sweep run.
#[derive(Debug, Clone, Default)]
pub struct SessionArchiverStats {
    pub archived_count: u32,
    pub pruned_count: u32,
    pub bytes_freed: u64,
}

impl SessionArchiver {
    pub fn new(
        wstore: Arc<Store>,
        filestore: Arc<FileStore>,
        inactive_days: u64,
        max_total_bytes: u64,
        archive_dir: PathBuf,
    ) -> Self {
        Self { wstore, filestore, inactive_days, max_total_bytes, archive_dir }
    }

    /// Run one sweep:
    ///   1. Find all agent blocks inactive for `inactive_days`.
    ///   2. Archive each one (compress → delete FileStore).
    ///   3. Prune oldest archives if total disk usage exceeds `max_total_bytes`.
    pub async fn sweep(&self) -> Result<SessionArchiverStats, String> {
        let mut stats = SessionArchiverStats::default();
        let now = now_ms();
        let cutoff_ms = self.inactive_days as i64 * 86_400_000;

        // Collect all blocks
        let all_blocks: Vec<Block> = self
            .wstore
            .get_all::<Block>()
            .map_err(|e| format!("get_all blocks: {e}"))?;

        for block in &all_blocks {
            // Only agent panes
            let view = block.meta.get("view").and_then(|v| v.as_str()).unwrap_or("");
            if view != "agent" {
                continue;
            }

            // Skip if already archived
            let archived_at = block
                .meta
                .get(META_SESSION_ARCHIVED_AT)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if archived_at > 0 {
                continue;
            }

            // Skip if session:last_activity_ms is missing (fresh session)
            let last_activity = match block
                .meta
                .get(META_SESSION_LAST_ACTIVITY_MS)
                .and_then(|v| v.as_i64())
            {
                Some(v) if v > 0 => v,
                _ => continue,
            };

            // Skip if session has no lines
            let line_count = block
                .meta
                .get(META_SESSION_LINE_COUNT)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if line_count == 0 {
                continue;
            }

            // Check inactivity threshold
            if now - last_activity < cutoff_ms {
                continue;
            }

            // Archive it
            match archive_session_output(
                &self.wstore,
                &self.filestore,
                &block.oid,
                &self.archive_dir,
            ) {
                Ok((bytes, _)) => {
                    stats.archived_count += 1;
                    stats.bytes_freed += bytes;
                    tracing::info!(
                        block_id = %block.oid,
                        bytes_freed = bytes,
                        "session archiver: auto-archived inactive session"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        block_id = %block.oid,
                        error = %e,
                        "session archiver: failed to archive session"
                    );
                }
            }
        }

        // Prune oldest archives if over the storage cap
        stats.pruned_count = self.prune_archives(&mut stats.bytes_freed)?;

        Ok(stats)
    }

    /// Delete oldest `.jsonl.gz` files until total size is under `max_total_bytes`.
    /// Returns number of files deleted.
    fn prune_archives(&self, bytes_freed: &mut u64) -> Result<u32, String> {
        let Ok(entries) = std::fs::read_dir(&self.archive_dir) else {
            return Ok(0);
        };

        // Collect (mtime, path, size) for all .jsonl.gz files
        let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = entries
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|x| x == "gz")
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let mtime = meta.modified().ok()?;
                let size = meta.len();
                Some((mtime, e.path(), size))
            })
            .collect();

        // Sort oldest-first
        files.sort_by_key(|(mtime, _, _)| *mtime);

        let total: u64 = files.iter().map(|(_, _, s)| s).sum();
        if total <= self.max_total_bytes {
            return Ok(0);
        }

        let mut remaining = total;
        let mut pruned = 0u32;

        for (_, path, size) in &files {
            if remaining <= self.max_total_bytes {
                break;
            }
            if let Err(e) = std::fs::remove_file(path) {
                tracing::warn!(path = %path.display(), error = %e, "session archiver: failed to prune archive");
                continue;
            }
            remaining = remaining.saturating_sub(*size);
            *bytes_freed += size;
            pruned += 1;
            tracing::info!(
                path = %path.display(),
                size_freed = size,
                "session archiver: pruned archive"
            );
        }

        Ok(pruned)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::backend::storage::filestore::FileStore;
    use crate::backend::storage::store::Store;

    /// Verify that a block with old last_activity gets archived by the sweeper.
    /// Uses in-memory stores — no disk I/O needed for this unit test.
    #[tokio::test]
    async fn test_sweep_archives_inactive_session() {
        // Build a temp dir for archives
        let tmp_dir = tempfile::tempdir().expect("tempdir");
        let archive_dir = tmp_dir.path().to_path_buf();

        // In-memory FileStore with output data
        let filestore = Arc::new(
            FileStore::open_in_memory().expect("filestore"),
        );
        // Write a fake "output" file for the block. The ID must be a
        // valid UUID — `archive_session_output` calls
        // `update_object_meta`, which parses the ORef
        // `format!("block:{}", block_id)` via `ORef::parse` and
        // rejects non-UUID oids. The earlier "blk-sweep-test" string
        // failed silently inside the sweep's logged-but-suppressed
        // error path.
        let block_id_owned = uuid::Uuid::new_v4().to_string();
        let block_id = block_id_owned.as_str();
        filestore
            .make_file(block_id, OUTPUT_FILENAME, FileMeta::default(), FileOpts::default())
            .expect("make_file");
        filestore
            .append_data(block_id, OUTPUT_FILENAME, b"line1\nline2\n")
            .expect("append_data");

        // In-memory Store
        let db_dir = tmp_dir.path().join("wdb");
        std::fs::create_dir_all(&db_dir).unwrap();
        let wstore = Arc::new(
            Store::open(&db_dir.join("objects.db")).expect("wstore"),
        );

        // Insert a fake Block object with required meta
        use crate::backend::obj::{Block, MetaMapType};
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        // last_activity 10 days ago (well past the 7-day threshold)
        let ten_days_ago = now_ms() - 10 * 86_400_000;
        meta.insert(
            META_SESSION_LAST_ACTIVITY_MS.to_string(),
            serde_json::json!(ten_days_ago),
        );
        meta.insert(META_SESSION_LINE_COUNT.to_string(), serde_json::json!(2u64));

        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta,
            subblockids: None,
        };
        wstore.insert(&mut block).expect("wstore insert");

        // Run the archiver (1-day inactive threshold to keep test fast)
        let archiver = SessionArchiver::new(
            wstore.clone(),
            filestore.clone(),
            1, // 1 day inactive threshold
            2 * 1024 * 1024 * 1024,
            archive_dir.clone(),
        );

        let stats = archiver.sweep().await.expect("sweep");
        assert_eq!(stats.archived_count, 1, "should archive 1 session");
        assert!(stats.bytes_freed > 0, "should free bytes");

        // Verify the FileStore entry was deleted
        let remaining = filestore.stat(block_id, OUTPUT_FILENAME).unwrap();
        assert!(remaining.is_none(), "filestore entry should be deleted after archive");

        // Verify the .gz file was created
        let gz_path = archive_dir.join(format!("{}.jsonl.gz", block_id));
        assert!(gz_path.exists(), ".gz file should exist");

        // Verify the block meta was updated
        let updated_block: Option<Block> = wstore.get(block_id).unwrap();
        let updated_block = updated_block.expect("block still in store");
        let archived_at = updated_block
            .meta
            .get(META_SESSION_ARCHIVED_AT)
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        assert!(archived_at > 0, "session:archived_at should be set");
    }

    #[test]
    fn test_bytecount_lines() {
        assert_eq!(bytecount_lines(b""), 0);
        assert_eq!(bytecount_lines(b"line1\n"), 1);
        assert_eq!(bytecount_lines(b"line1\nline2\n"), 2);
        assert_eq!(bytecount_lines(b"  \n\nline3\n"), 1); // blanks excluded
    }

    #[test]
    fn test_gz_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let data = b"hello compressed world\nline2\n";
        write_gz(data, tmp.path()).unwrap();
        let out = read_gz(tmp.path()).unwrap();
        assert_eq!(out, data);
    }
}
