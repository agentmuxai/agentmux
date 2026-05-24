// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-anchored session zones (Option E, PR 1 of 2).
//!
//! A session zone is bound to the **agent definition** (`definition_id`),
//! not the identity bundle the agent currently uses. Two agents that
//! share an identity bundle keep separate session histories.
//!
//! ## Zone naming
//!
//! - Active:   `agent:<defId>:current`
//! - Archived: `agent:<defId>:archive:<unix_ms>`
//!
//! Files inside a zone keep the existing shape:
//! `output.state.json` (full UI snapshot) and `output` (raw NDJSON
//! stream for crash-recovery replay).
//!
//! ## Migration (`migrate_block_zones_v1`)
//!
//! On first srv startup after this PR ships, the per-block zones (the
//! pre-Option-E layout where each `blockId` owned its own zone) are
//! back-filled into the new per-agent layout:
//!
//! 1. For each `db_block` row with `meta.view = "agent"` and a
//!    non-empty `output.state.json` in its block-keyed zone, copy the
//!    contents to `agent:<defId>:archive:<block-zone-createdts>`.
//! 2. For each definition, copy the most-recently-modified per-block
//!    snapshot to `agent:<defId>:current`.
//! 3. Write the marker file `<data_dir>/migration_agent_zones_v1.flag`.
//!
//! The migration is read-only against the existing per-block zones —
//! GC of those is a later PR.
//!
//! See `docs/specs/SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::obj::Block;
use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};
use crate::backend::storage::wstore::WaveStore;

// ---------------------------------------------------------------------------
// File names within an agent session zone (mirrors per-block zone shape)
// ---------------------------------------------------------------------------

/// Full UI snapshot (JSON). Frontend reads this on pane mount.
pub const SNAPSHOT_FILE: &str = "output.state.json";
/// Raw NDJSON stream for crash-recovery replay.
pub const OUTPUT_FILE: &str = "output";

/// Marker file name for the per-data-dir one-shot migration gate.
pub const MIGRATION_MARKER_V1: &str = "migration_agent_zones_v1.flag";

// ---------------------------------------------------------------------------
// Zone helpers
// ---------------------------------------------------------------------------

/// Returns true if `s` matches `[A-Za-z0-9_-]+`. Rejects empty.
///
/// We're embedding `definition_id` into a zone name (a string the
/// frontend can supply via RPC), so anything outside the safe set would
/// let an attacker write/read arbitrary zones. UUIDs (the production
/// definition_id shape) are a strict subset of this character class.
pub fn is_valid_definition_id(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `agent:<definition_id>:current`. Panics in debug if `definition_id`
/// is invalid; callers should `validate_definition_id` first in release.
pub fn agent_current_zone(definition_id: &str) -> String {
    debug_assert!(
        is_valid_definition_id(definition_id),
        "agent_current_zone: invalid definition_id"
    );
    format!("agent:{}:current", definition_id)
}

/// `agent:<definition_id>:archive:<ts_ms>`.
pub fn agent_archive_zone(definition_id: &str, ts_ms: u64) -> String {
    debug_assert!(
        is_valid_definition_id(definition_id),
        "agent_archive_zone: invalid definition_id"
    );
    format!("agent:{}:archive:{}", definition_id, ts_ms)
}

/// Convenience: validate + build the current-zone string. Returns
/// `Err` with a stable error prefix on bad input so RPC callers see a
/// consistent message.
pub fn validate_and_current(definition_id: &str) -> Result<String, String> {
    if !is_valid_definition_id(definition_id) {
        return Err(format!(
            "INVALID_DEFINITION_ID: must match [A-Za-z0-9_-]+, got {:?}",
            definition_id
        ));
    }
    Ok(agent_current_zone(definition_id))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// FileStore operations on agent session zones
// ---------------------------------------------------------------------------

/// Write `content` to `output.state.json` in `agent:<defId>:current`.
/// Idempotent — creates the file if missing, overwrites otherwise.
pub fn write_session_state(
    filestore: &FileStore,
    definition_id: &str,
    content: &[u8],
) -> Result<(), String> {
    let zone = validate_and_current(definition_id)?;
    write_zone_file(filestore, &zone, SNAPSHOT_FILE, content)
}

/// Append `line` (with a trailing newline added if not present) to
/// `output` in `agent:<defId>:current`. Creates the file if missing.
pub fn append_session_output(
    filestore: &FileStore,
    definition_id: &str,
    line: &str,
) -> Result<u64, String> {
    let zone = validate_and_current(definition_id)?;
    // Normalize to NDJSON: each line ends with exactly one '\n'.
    let mut buf = line.as_bytes().to_vec();
    if !buf.ends_with(b"\n") {
        buf.push(b'\n');
    }
    ensure_file(filestore, &zone, OUTPUT_FILE)?;
    filestore
        .append_data(&zone, OUTPUT_FILE, &buf)
        .map_err(|e| format!("append_data: {e}"))?;
    Ok(buf.len() as u64)
}

/// Read `output.state.json` from `agent:<defId>:current`. Returns
/// `Ok((None, None))` when the zone doesn't exist — that's the
/// "fresh agent, nothing to restore" path and is NOT an error.
pub fn read_session_state(
    filestore: &FileStore,
    definition_id: &str,
) -> Result<(Option<String>, Option<i64>), String> {
    let zone = validate_and_current(definition_id)?;
    let stat = filestore
        .stat(&zone, SNAPSHOT_FILE)
        .map_err(|e| format!("stat: {e}"))?;
    let Some(file) = stat else {
        return Ok((None, None));
    };
    let bytes = filestore
        .read_file(&zone, SNAPSHOT_FILE)
        .map_err(|e| format!("read_file: {e}"))?
        .unwrap_or_default();
    let content = String::from_utf8_lossy(&bytes).into_owned();
    Ok((Some(content), Some(file.modts)))
}

/// Archive `agent:<defId>:current` to `agent:<defId>:archive:<now_ms>`.
///
/// Atomicity contract:
/// - We write the archive zone first, then clear the current zone.
/// - A crash between those two steps leaves both zones populated;
///   replay-time behaviour is "current wins" so this is safe.
/// - We never clear `:current` before the archive write has been
///   acked by FileStore, so the archive-missing case can't happen
///   without a FileStore I/O failure on the write itself (which is
///   surfaced as `Err` and aborts the clear step).
///
/// Returns:
/// - `Ok(Some((archive_zoneid, archived_at_ms)))` on successful
///   archive (current zone had content).
/// - `Ok(None)` when there was nothing to archive (no
///   `output.state.json` in :current, OR it was zero-byte). The
///   caller should treat this as "session was empty, nothing to do"
///   — we explicitly do NOT create an empty archive zone.
pub fn archive_session(
    filestore: &FileStore,
    definition_id: &str,
) -> Result<Option<(String, i64)>, String> {
    let current_zone = validate_and_current(definition_id)?;

    // Determine whether there's anything worth archiving.
    let state_stat = filestore
        .stat(&current_zone, SNAPSHOT_FILE)
        .map_err(|e| format!("stat current: {e}"))?;
    let has_state = match &state_stat {
        Some(f) => f.size > 0,
        None => false,
    };
    let output_stat = filestore
        .stat(&current_zone, OUTPUT_FILE)
        .map_err(|e| format!("stat current output: {e}"))?;
    let has_output = match &output_stat {
        Some(f) => f.size > 0,
        None => false,
    };
    if !has_state && !has_output {
        return Ok(None);
    }

    let ts = now_ms();
    let archive_zone = agent_archive_zone(definition_id, ts);

    // Copy snapshot first (the canonical "history" file).
    if has_state {
        let snapshot_bytes = filestore
            .read_file(&current_zone, SNAPSHOT_FILE)
            .map_err(|e| format!("read current snapshot: {e}"))?
            .unwrap_or_default();
        write_zone_file(filestore, &archive_zone, SNAPSHOT_FILE, &snapshot_bytes)?;
    }
    // Copy NDJSON output if present.
    if has_output {
        let output_bytes = filestore
            .read_file(&current_zone, OUTPUT_FILE)
            .map_err(|e| format!("read current output: {e}"))?
            .unwrap_or_default();
        if !output_bytes.is_empty() {
            write_zone_file(filestore, &archive_zone, OUTPUT_FILE, &output_bytes)?;
        }
    }

    // Archive write succeeded. Now safe to clear the current zone.
    if state_stat.is_some() {
        if let Err(e) = filestore.delete_file(&current_zone, SNAPSHOT_FILE) {
            tracing::warn!(
                definition_id = %definition_id,
                error = %e,
                "agent_session: failed to clear current snapshot after archive (archive already persisted)"
            );
        }
    }
    if output_stat.is_some() {
        if let Err(e) = filestore.delete_file(&current_zone, OUTPUT_FILE) {
            tracing::warn!(
                definition_id = %definition_id,
                error = %e,
                "agent_session: failed to clear current output after archive (archive already persisted)"
            );
        }
    }

    tracing::info!(
        definition_id = %definition_id,
        archive_zoneid = %archive_zone,
        archived_at_ms = ts,
        "agent_session: archived current session"
    );

    Ok(Some((archive_zone, ts as i64)))
}

/// List archive zones for `definition_id`, newest first.
///
/// Returns up to `limit` rows. `limit = 0` means "default 20"; caller
/// must clamp upper bounds. Each row carries a small preview lifted
/// from the archive's `output.state.json`.
pub fn list_archives(
    filestore: &FileStore,
    definition_id: &str,
    limit: usize,
) -> Result<Vec<ArchiveSummary>, String> {
    if !is_valid_definition_id(definition_id) {
        return Err(format!(
            "INVALID_DEFINITION_ID: must match [A-Za-z0-9_-]+, got {:?}",
            definition_id
        ));
    }
    let prefix = format!("agent:{}:archive:", definition_id);
    let limit = if limit == 0 { 20 } else { limit.min(100) };

    let all_zones = filestore
        .get_all_zone_ids()
        .map_err(|e| format!("get_all_zone_ids: {e}"))?;

    let mut matches: Vec<(u64, String)> = Vec::new();
    for zone in all_zones {
        if let Some(suffix) = zone.strip_prefix(&prefix) {
            if let Ok(ts) = suffix.parse::<u64>() {
                matches.push((ts, zone));
            }
        }
    }
    // Newest first.
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    matches.truncate(limit);

    let mut rows = Vec::with_capacity(matches.len());
    for (ts, zone) in matches {
        let (preview, node_count) = read_archive_preview(filestore, &zone);
        rows.push(ArchiveSummary {
            archive_zoneid: zone,
            archived_at_ms: ts as i64,
            preview,
            node_count,
        });
    }
    Ok(rows)
}

/// A single archive row. Mirrors the shape of `RecentSessionRow`'s
/// preview fields so the frontend can reuse the same row component.
#[derive(Debug, Clone)]
pub struct ArchiveSummary {
    pub archive_zoneid: String,
    pub archived_at_ms: i64,
    pub preview: String,
    pub node_count: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Ensure a file exists in `zone`. No-op when present.
fn ensure_file(filestore: &FileStore, zone: &str, name: &str) -> Result<(), String> {
    match filestore.stat(zone, name) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => filestore
            .make_file(zone, name, FileMeta::default(), FileOpts::default())
            .map_err(|e| format!("make_file: {e}")),
        Err(e) => Err(format!("stat: {e}")),
    }
}

/// Write the entire contents of a file in `zone`. Creates the file if
/// missing, otherwise replaces all parts atomically (FileStore single-tx).
fn write_zone_file(
    filestore: &FileStore,
    zone: &str,
    name: &str,
    content: &[u8],
) -> Result<(), String> {
    use crate::backend::storage::StoreError;
    match filestore.write_file(zone, name, content) {
        Ok(()) => Ok(()),
        Err(StoreError::NotFound) => {
            filestore
                .make_file(zone, name, FileMeta::default(), FileOpts::default())
                .map_err(|e| format!("make_file: {e}"))?;
            filestore
                .write_file(zone, name, content)
                .map_err(|e| format!("write_file: {e}"))
        }
        Err(e) => Err(format!("write_file: {e}")),
    }
}

/// Pull a small preview + node_count out of an archive's
/// `output.state.json`. Returns `("", 0)` on any error.
///
/// Mirrors the heuristics used by `read_session_preview` in
/// `agent_handlers.rs` (skip the bootstrap "# Session Context" message
/// when a later user_message exists; cap at 240 chars).
fn read_archive_preview(filestore: &FileStore, zone: &str) -> (String, usize) {
    let bytes = match filestore.read_file(zone, SNAPSHOT_FILE) {
        Ok(Some(b)) => b,
        _ => return (String::new(), 0),
    };
    if bytes.len() > 4 * 1024 * 1024 {
        return (String::new(), 0);
    }
    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return (String::new(), 0),
    };
    let nodes = match json.get("nodes").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return (String::new(), 0),
    };
    let node_count = nodes.len();
    let mut preview = String::new();
    for node in nodes {
        let ty = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "user_message" {
            continue;
        }
        let msg = node
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if msg.is_empty() {
            continue;
        }
        if preview.is_empty() && msg.starts_with("# Session Context") {
            preview = collapse_preview(msg);
            continue;
        }
        preview = collapse_preview(msg);
        break;
    }
    (preview, node_count)
}

fn collapse_preview(s: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut buf = String::with_capacity(s.len().min(MAX_CHARS + 4));
    let mut prev_space = false;
    for ch in s.chars() {
        if buf.chars().count() >= MAX_CHARS {
            buf.push('\u{2026}');
            return buf;
        }
        if ch.is_whitespace() {
            if !prev_space && !buf.is_empty() {
                buf.push(' ');
                prev_space = true;
            }
        } else {
            buf.push(ch);
            prev_space = false;
        }
    }
    buf
}

// ---------------------------------------------------------------------------
// One-time migration: per-block zones → per-agent zones
// ---------------------------------------------------------------------------

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
    wstore: &Arc<WaveStore>,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::MetaMapType;
    use crate::backend::storage::filestore::FileStore;
    use crate::backend::storage::wstore::WaveStore;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn fresh_filestore() -> Arc<FileStore> {
        Arc::new(FileStore::open_in_memory().unwrap())
    }

    #[test]
    fn zone_names_match_spec() {
        assert_eq!(
            agent_current_zone("def-abc"),
            "agent:def-abc:current"
        );
        assert_eq!(
            agent_archive_zone("def-abc", 1_700_000_000_000),
            "agent:def-abc:archive:1700000000000"
        );
    }

    #[test]
    fn validate_definition_id_rejects_bad_input() {
        assert!(is_valid_definition_id("abc-123_DEF"));
        assert!(is_valid_definition_id("a"));
        assert!(!is_valid_definition_id(""));
        // Path-traversal / zone-injection attempts.
        assert!(!is_valid_definition_id("../etc"));
        assert!(!is_valid_definition_id("a:b"));
        assert!(!is_valid_definition_id("a/b"));
        assert!(!is_valid_definition_id("a b"));
        assert!(!is_valid_definition_id("a\x00b"));
        // Unicode rejected — keeps the zone-name surface ASCII.
        assert!(!is_valid_definition_id("café"));
    }

    #[test]
    fn validate_and_current_surfaces_error_prefix() {
        let err = validate_and_current("../etc").unwrap_err();
        assert!(err.starts_with("INVALID_DEFINITION_ID:"));
    }

    #[test]
    fn read_returns_none_when_zone_missing() {
        let fs = fresh_filestore();
        // No prior write — no zone exists.
        let (content, modts) = read_session_state(&fs, "def-fresh").unwrap();
        assert!(content.is_none(), "missing zone should NOT be an error");
        assert!(modts.is_none());
    }

    #[test]
    fn read_rejects_invalid_definition_id() {
        let fs = fresh_filestore();
        let err = read_session_state(&fs, "../bad").unwrap_err();
        assert!(err.starts_with("INVALID_DEFINITION_ID:"));
    }

    #[test]
    fn write_then_read_roundtrip() {
        let fs = fresh_filestore();
        let payload = r#"{"nodes":[{"type":"user_message","message":"hi"}]}"#;
        write_session_state(&fs, "def-a", payload.as_bytes()).unwrap();
        let (content, modts) = read_session_state(&fs, "def-a").unwrap();
        assert_eq!(content.as_deref(), Some(payload));
        assert!(modts.unwrap_or(0) > 0);
    }

    #[test]
    fn write_is_idempotent_replaces_content() {
        let fs = fresh_filestore();
        write_session_state(&fs, "def-a", b"first").unwrap();
        write_session_state(&fs, "def-a", b"second").unwrap();
        let (content, _) = read_session_state(&fs, "def-a").unwrap();
        assert_eq!(content.as_deref(), Some("second"));
    }

    #[test]
    fn append_output_grows_ndjson_file() {
        let fs = fresh_filestore();
        let n1 = append_session_output(&fs, "def-a", "line1").unwrap();
        let n2 = append_session_output(&fs, "def-a", "line2\n").unwrap();
        // Each line is normalized to end with '\n'.
        assert_eq!(n1, b"line1\n".len() as u64);
        assert_eq!(n2, b"line2\n".len() as u64);
        let zone = agent_current_zone("def-a");
        let bytes = fs.read_file(&zone, OUTPUT_FILE).unwrap().unwrap();
        assert_eq!(bytes, b"line1\nline2\n");
    }

    #[test]
    fn archive_moves_content_and_clears_current() {
        let fs = fresh_filestore();
        let payload = br#"{"nodes":[{"type":"user_message","message":"x"}]}"#;
        write_session_state(&fs, "def-a", payload).unwrap();
        append_session_output(&fs, "def-a", "raw1").unwrap();

        let result = archive_session(&fs, "def-a").unwrap();
        let (zone, ts) = result.expect("archive should have happened");
        assert!(zone.starts_with("agent:def-a:archive:"));
        assert!(ts > 0);

        // Archive zone has the original snapshot.
        let archived = fs.read_file(&zone, SNAPSHOT_FILE).unwrap();
        assert_eq!(archived.as_deref(), Some(payload.as_slice()));
        // ...AND the NDJSON output.
        let archived_output = fs.read_file(&zone, OUTPUT_FILE).unwrap().unwrap();
        assert_eq!(archived_output, b"raw1\n");

        // Current zone snapshot is gone.
        let current_zone = agent_current_zone("def-a");
        let still_there = fs.stat(&current_zone, SNAPSHOT_FILE).unwrap();
        assert!(still_there.is_none(), ":current snapshot must be cleared");
        let still_output = fs.stat(&current_zone, OUTPUT_FILE).unwrap();
        assert!(still_output.is_none(), ":current output must be cleared");

        // Subsequent read returns None (fresh).
        let (content, _) = read_session_state(&fs, "def-a").unwrap();
        assert!(content.is_none());
    }

    #[test]
    fn archive_on_empty_current_is_noop() {
        let fs = fresh_filestore();
        // Nothing was ever written.
        let result = archive_session(&fs, "def-empty").unwrap();
        assert!(result.is_none(), "archive on empty :current should no-op");
        // No archive zones should exist.
        let zones = fs.get_all_zone_ids().unwrap();
        assert!(
            !zones.iter().any(|z| z.contains(":archive:")),
            "no archive zone should have been created"
        );
    }

    #[test]
    fn archive_on_zero_byte_state_is_noop() {
        let fs = fresh_filestore();
        // Touch the file but leave it empty.
        let zone = agent_current_zone("def-zero");
        fs.make_file(&zone, SNAPSHOT_FILE, FileMeta::default(), FileOpts::default())
            .unwrap();
        let result = archive_session(&fs, "def-zero").unwrap();
        assert!(result.is_none(), "zero-byte :current must NOT create archive");
    }

    /// Critical scoping invariant: agents are independent, even when
    /// they share an identity bundle. Writing to AgentA must NOT
    /// expose any data to AgentB.
    #[test]
    fn two_agents_have_independent_zones() {
        let fs = fresh_filestore();
        write_session_state(&fs, "def-A", br#"{"nodes":[{"type":"user_message","message":"A"}]}"#)
            .unwrap();

        // AgentB sees nothing.
        let (content_b, _) = read_session_state(&fs, "def-B").unwrap();
        assert!(content_b.is_none(), "AgentB must NOT see AgentA's data");

        // AgentA still has its content.
        let (content_a, _) = read_session_state(&fs, "def-A").unwrap();
        assert!(content_a.unwrap().contains("\"A\""));
    }

    #[test]
    fn list_archives_sorted_newest_first_with_previews() {
        let fs = fresh_filestore();
        // Seed three archive zones for the same def, varying timestamps.
        let make = |ts: u64, label: &str| {
            let zone = agent_archive_zone("def-a", ts);
            let payload = serde_json::json!({
                "nodes": [
                    {"type": "user_message", "message": label}
                ]
            });
            write_zone_file(&fs, &zone, SNAPSHOT_FILE, payload.to_string().as_bytes()).unwrap();
        };
        make(1_000, "old");
        make(3_000, "newest");
        make(2_000, "mid");

        let rows = list_archives(&fs, "def-a", 0).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].archived_at_ms, 3_000);
        assert_eq!(rows[0].preview, "newest");
        assert_eq!(rows[0].node_count, 1);
        assert_eq!(rows[1].archived_at_ms, 2_000);
        assert_eq!(rows[2].archived_at_ms, 1_000);
    }

    #[test]
    fn list_archives_respects_limit() {
        let fs = fresh_filestore();
        for ts in 1..=5u64 {
            let zone = agent_archive_zone("def-a", ts);
            fs.make_file(&zone, SNAPSHOT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
            fs.write_file(&zone, SNAPSHOT_FILE, b"{}").unwrap();
        }
        let rows = list_archives(&fs, "def-a", 2).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn list_archives_rejects_bad_definition_id() {
        let fs = fresh_filestore();
        assert!(list_archives(&fs, "../bad", 0).is_err());
    }

    // ---- Migration tests ----

    fn open_temp_wstore(dir: &Path) -> Arc<WaveStore> {
        let path = dir.join("objects.db");
        Arc::new(WaveStore::open(&path).expect("open wstore"))
    }

    fn insert_agent_block(wstore: &Arc<WaveStore>, def_id: &str) -> String {
        let oid = uuid::Uuid::new_v4().to_string();
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        meta.insert("agentId".to_string(), serde_json::json!(def_id));
        let mut block = Block {
            oid: oid.clone(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta,
            subblockids: None,
        };
        wstore.insert(&mut block).expect("insert block");
        oid
    }

    fn seed_block_snapshot(filestore: &Arc<FileStore>, block_id: &str, body: &str) {
        filestore
            .make_file(block_id, SNAPSHOT_FILE, FileMeta::default(), FileOpts::default())
            .unwrap();
        filestore.write_file(block_id, SNAPSHOT_FILE, body.as_bytes()).unwrap();
    }

    #[test]
    fn migration_backfills_archives_and_seeds_current() {
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        // Two blocks for the same definition. Block 2 is written later
        // → it should win the `:current` seed.
        let block1 = insert_agent_block(&wstore, "def-maks");
        seed_block_snapshot(
            &filestore,
            &block1,
            r#"{"nodes":[{"type":"user_message","message":"old"}]}"#,
        );
        // Sleep briefly so the second block's snapshot has a strictly
        // greater modts. FileStore stamps `Self::now_ms()` per write.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let block2 = insert_agent_block(&wstore, "def-maks");
        seed_block_snapshot(
            &filestore,
            &block2,
            r#"{"nodes":[{"type":"user_message","message":"newer"}]}"#,
        );

        // And one block for a different definition.
        let block_other = insert_agent_block(&wstore, "def-other");
        seed_block_snapshot(
            &filestore,
            &block_other,
            r#"{"nodes":[{"type":"user_message","message":"other"}]}"#,
        );

        let stats = migrate_block_zones_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.blocks_scanned, 3);
        assert_eq!(stats.archives_written, 3);
        assert_eq!(stats.current_zones_seeded, 2);
        assert_eq!(stats.failures, 0);

        // Marker file written.
        assert!(dir.path().join(MIGRATION_MARKER_V1).exists());

        // `:current` for def-maks must hold block2's content (the
        // most-recently-modified per-block snapshot).
        let (content, _) = read_session_state(&filestore, "def-maks").unwrap();
        assert!(content.unwrap().contains("newer"));

        // Both archives exist for def-maks.
        let archives = list_archives(&filestore, "def-maks", 0).unwrap();
        assert_eq!(archives.len(), 2);

        // Other def isolated.
        let (other, _) = read_session_state(&filestore, "def-other").unwrap();
        assert!(other.unwrap().contains("other"));
        let other_archives = list_archives(&filestore, "def-other", 0).unwrap();
        assert_eq!(other_archives.len(), 1);

        // Old block zones NOT deleted (GC is a later PR).
        let still_block1 = filestore.stat(&block1, SNAPSHOT_FILE).unwrap();
        assert!(still_block1.is_some(), "old block zone must remain");
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let block = insert_agent_block(&wstore, "def-a");
        seed_block_snapshot(
            &filestore,
            &block,
            r#"{"nodes":[{"type":"user_message","message":"x"}]}"#,
        );

        let first = migrate_block_zones_v1(&wstore, &filestore, dir.path());
        assert_eq!(first.archives_written, 1);
        assert_eq!(first.current_zones_seeded, 1);

        // Second run is gated by the marker.
        let second = migrate_block_zones_v1(&wstore, &filestore, dir.path());
        assert_eq!(second.blocks_scanned, 0);
        assert_eq!(second.archives_written, 0);
        assert_eq!(second.current_zones_seeded, 0);
    }

    #[test]
    fn migration_skips_non_agent_and_empty_blocks() {
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        // A "term" block (not agent) — must be skipped.
        let term_oid = uuid::Uuid::new_v4().to_string();
        let mut term_meta = MetaMapType::new();
        term_meta.insert("view".to_string(), serde_json::json!("term"));
        let mut term = Block {
            oid: term_oid.clone(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: term_meta,
            subblockids: None,
        };
        wstore.insert(&mut term).unwrap();
        seed_block_snapshot(&filestore, &term_oid, r#"{"nodes":[]}"#);

        // An agent block with NO snapshot — should count as skipped.
        let _empty = insert_agent_block(&wstore, "def-x");

        let stats = migrate_block_zones_v1(&wstore, &filestore, dir.path());
        // Only the empty agent block is "scanned" (view == "agent");
        // the term block is filtered out before the counter.
        assert_eq!(stats.blocks_scanned, 1);
        assert_eq!(stats.skipped_no_snapshot, 1);
        assert_eq!(stats.archives_written, 0);
        assert_eq!(stats.current_zones_seeded, 0);
    }
}
