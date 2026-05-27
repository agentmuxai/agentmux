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
use crate::backend::storage::store::Store;

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

// ---------------------------------------------------------------------------
// Two-tier picker — Phase 1 migration (seeded-def → user-agent promote)
// ---------------------------------------------------------------------------

/// Marker file name for the Phase 1 two-tier-picker migration.
///
/// **Vestigial.** Originally gated `migrate_promote_template_sessions_v1`
/// as a one-shot. The 2026-05-24 self-idempotency rework moved gating
/// to the data invariant ("no seeded def has a session zone"), so the
/// migration runs on every startup and is a no-op when the invariant
/// already holds. The constant + `data_dir` parameter on the migration
/// function are kept for API/import compatibility and so the legacy
/// marker file (if present from an earlier portable run) isn't
/// resurrected. Operators may delete the file; the migration ignores
/// it either way.
pub const TEMPLATE_PROMOTE_MARKER_V1: &str = "migration_template_promote_v1.flag";

/// Stats from `migrate_promote_template_sessions_v1`. Logged at INFO.
#[derive(Debug, Clone, Default)]
pub struct TemplatePromoteStats {
    pub templates_scanned: usize,
    pub templates_promoted: usize,
    /// Total archive zones moved across all promotions.
    pub archives_moved: usize,
    /// Total instances repointed via
    /// `wstore.instance_repoint_definition`.
    pub instances_repointed: usize,
    pub failures: usize,
}

/// Phase 1 two-tier picker migration: promote any seeded template that
/// carries a session zone into a fresh user-owned definition, then move
/// its `:current` + `:archive:*` zones onto the new definition_id.
///
/// Why this exists (Q1 = Option C in
/// `docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md`):
/// after the picker UI split, clicking a "template" card in the
/// Templates section MUST create a new agent — not silently append to
/// whatever session the user previously ran against that template
/// directly (e.g. "Maks's conversation" living at `agent:claude:current`).
/// Without this migration the template card would either reattach to
/// the existing session (broken — wrong intent) or be effectively
/// non-functional. The migration moves any such pre-existing session
/// out of the template namespace onto a new user-owned definition so
/// the template is pristine post-migration.
///
/// Algorithm:
/// 1. List zone ids; partition the `agent:<id>:current` and
///    `agent:<id>:archive:*` zones by definition id.
/// 2. For each definition id with at least one zone, look up the
///    matching `db_agent_definitions` row.
///    - Skip if missing (zone refers to a deleted definition).
///    - Skip if `is_seeded = 0` (already user-owned — no work).
///    - Otherwise: clone the template into a new user definition
///      (mirrors `agent_def_create_from_template` semantics).
/// 3. Pick the new name: most-recently-active named instance's
///    `instance_name` if any exists, else fall back to the template's
///    own `name`.
/// 4. Move every matching zone (`:current` + every `:archive:*`)
///    from the old defId to the new defId via FileStore's existing
///    write-then-delete pattern.
/// 5. Repoint every `db_agent_instances` row that referenced the old
///    defId to point at the new defId (preserves the
///    `continueOfInstanceId` reattach flow).
///
/// Idempotency: the migration is **self-gated on the data invariant**
/// ("no seeded def has a session zone"). It runs on every startup;
/// when the invariant already holds the inner loop has zero
/// iterations and returns the default stats in sub-ms. There used to
/// be a marker-file gate (`TEMPLATE_PROMOTE_MARKER_V1`), but it
/// produced an "early-marker" failure mode: a portable launched at v
/// N had no seeded-def zones, set the marker, and on v N+1 startups
/// (when seeded-def zones DID exist from prior real use) the marker
/// caused the migration to skip. The 2026-05-24 rework dropped the
/// marker check; this is safe because the seeded-def-with-zone
/// invariant is detectable per-startup at constant cost. `data_dir`
/// is retained for API compatibility.
///
/// Failure mode: per-template errors are logged + counted; we DO NOT
/// abort startup. Errors that prevent a template from being promoted
/// leave its zones in place; the next startup retries (no marker
/// gate to block retry).
pub fn migrate_promote_template_sessions_v1(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    _data_dir: &Path,
) -> TemplatePromoteStats {

    let mut stats = TemplatePromoteStats::default();

    let all_zones = match filestore.get_all_zone_ids() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "template_promote migration: get_all_zone_ids failed; aborting (will retry next start)"
            );
            return stats;
        }
    };

    // Group zone ids by definition id. A zone counts if it matches
    // `agent:<id>:current` OR `agent:<id>:archive:<ts>`. Anything else
    // (e.g. legacy per-block zones the prior migration didn't sweep)
    // is ignored by this migration.
    let mut per_def_zones: HashMap<String, Vec<String>> = HashMap::new();
    for zone in &all_zones {
        let rest = match zone.strip_prefix("agent:") {
            Some(r) => r,
            None => continue,
        };
        // `<defId>:current` or `<defId>:archive:<ts>`
        let (def_id, tail) = match rest.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        if !is_valid_definition_id(def_id) {
            continue;
        }
        let is_current = tail == "current";
        let is_archive = tail.starts_with("archive:");
        if !is_current && !is_archive {
            continue;
        }
        per_def_zones
            .entry(def_id.to_string())
            .or_default()
            .push(zone.clone());
    }

    // Fetch all definitions ONCE so per-template lookups don't re-hit
    // SQLite in a loop.
    let defs = match wstore.agent_def_list() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "template_promote migration: agent_def_list failed; aborting (will retry next start)"
            );
            return stats;
        }
    };

    for (old_def_id, zones) in per_def_zones {
        // Look up the definition row this zone is bound to.
        let template = match defs.iter().find(|d| d.id == old_def_id) {
            Some(d) => d,
            None => {
                // Zone points at a deleted definition — leave it
                // alone; a future GC pass can clean orphans.
                continue;
            }
        };
        // Only seeded templates need promotion. User-owned defs are
        // already on the new model.
        if template.is_seeded != 1 {
            continue;
        }
        stats.templates_scanned += 1;

        // Pick the new agent name: most-recently-active named instance
        // for this template, else fall back to the template's own name.
        // `instance_list_named` already filters to non-hidden + named
        // rows + sorts by `started_at DESC`, so the first row is the
        // pick.
        // Include continuations: a user who clicked Maks today and
        // resumed three times has only continuation rows for that
        // definition; the head row is whatever they originally
        // named the agent. Picking the most-recent continuation
        // surfaces the same `instance_name` they used last.
        let new_name = match wstore.instance_list_named(
            1,
            Some(&old_def_id),
            /* identity_id */ None,
            /* include_continuations */ true,
        ) {
            Ok(rows) => rows
                .into_iter()
                .next()
                .map(|i| i.instance_name)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| template.name.clone()),
            Err(e) => {
                tracing::warn!(
                    template_id = %old_def_id,
                    error = %e,
                    "template_promote migration: instance_list_named failed; using template name"
                );
                template.name.clone()
            }
        };

        // Idempotency: the migration uses a DETERMINISTIC clone id
        // (`template-promote-v1-<template_id>`) so every retry of
        // every partial-failure scenario targets the same clone.
        // Successful prior steps (zone moves, instance repoints)
        // are reused; failed steps re-attempt against the same
        // destination. There is no way to "fork" the migration
        // into a different clone id, so the unbounded-duplicate
        // failure modes from codex P1 rounds 1+2 cannot recur:
        //
        //   1. Insert def: idempotent via `SELECT WHERE id = ?1`
        //      first; new row only on absence. PK uniqueness on
        //      the deterministic id catches any race.
        //   2. move_zone: write-then-delete; replay copies the
        //      same content to the same destination (no-op when
        //      already moved), retries the source delete.
        //   3. instance_repoint_definition: UPDATE on rows whose
        //      definition_id = old; rows already at new are a
        //      no-op SET.
        //
        // The deterministic id also distinguishes the migration's
        // own clone from any user-created "+ New from template"
        // clone (which lives under a fresh UUID), so we never
        // clobber a user's live session.
        let promote_target_id =
            format!("template-promote-v1-{}", template.id);
        debug_assert!(
            is_valid_definition_id(&promote_target_id),
            "deterministic promote-target id must satisfy the zone-id charset"
        );

        let existing_target = match wstore.agent_def_get(&promote_target_id) {
            Ok(Some(def)) => Some(def),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(
                    template_id = %old_def_id,
                    promote_target_id = %promote_target_id,
                    error = %e,
                    "template_promote migration: agent_def_get failed; aborting this template"
                );
                stats.failures += 1;
                continue;
            }
        };
        let new_def = if let Some(existing) = existing_target {
            tracing::info!(
                template_id = %old_def_id,
                promote_target_id = %promote_target_id,
                "template_promote migration: reusing prior promote-target clone (idempotent retry)"
            );
            existing
        } else {
            // Clone the template into a new user-owned definition
            // at the deterministic id. Field copies mirror
            // `agent_def_create_from_template`.
            let now = now_ms() as i64;
            let mut new_def = crate::backend::storage::store::AgentDefinition {
                id: promote_target_id.clone(),
                slug: String::new(),
                name: new_name.clone(),
                icon: template.icon.clone(),
                provider: template.provider.clone(),
                description: template.description.clone(),
                working_directory: String::new(),
                shell: template.shell.clone(),
                provider_flags: template.provider_flags.clone(),
                auto_start: 0,
                restart_on_crash: template.restart_on_crash,
                idle_timeout_minutes: template.idle_timeout_minutes,
                created_at: now,
                agent_type: template.agent_type.clone(),
                environment: template.environment.clone(),
                agent_bus_id: String::new(),
                is_seeded: 0,
                accounts: String::new(),
                parent_id: template.id.clone(),
                branch_label: String::new(),
                updated_at: now,
                user_hidden: 0,
            };
            if let Err(e) = wstore.agent_def_insert(&mut new_def) {
                tracing::warn!(
                    template_id = %old_def_id,
                    promote_target_id = %promote_target_id,
                    error = %e,
                    "template_promote migration: agent_def_insert failed; skipping this template"
                );
                stats.failures += 1;
                continue;
            }
            new_def
        };

        // Move every matching zone (current + archives) onto the new
        // definition id. Per-zone failures are logged but don't abort
        // the whole template — best-effort.
        let mut archives_for_this_def: usize = 0;
        for old_zone in &zones {
            // Build the new zone id by swapping the def-id segment.
            // We know `old_zone` starts with `agent:<old_def_id>:`
            // (per the bucketing above), so substring-replace is safe.
            let suffix = match old_zone.strip_prefix(&format!("agent:{}:", old_def_id)) {
                Some(s) => s,
                None => continue,
            };
            let new_zone = format!("agent:{}:{}", new_def.id, suffix);
            let is_archive = suffix.starts_with("archive:");

            if let Err(e) = move_zone(filestore, old_zone, &new_zone) {
                tracing::warn!(
                    template_id = %old_def_id,
                    old_zone = %old_zone,
                    new_zone = %new_zone,
                    error = %e,
                    "template_promote migration: move_zone failed"
                );
                stats.failures += 1;
                continue;
            }
            if is_archive {
                archives_for_this_def += 1;
            }
        }

        // Repoint any in-DB instances referencing this template at
        // the new user-owned definition. Without this, the existing
        // continueOfInstanceId reattach flow would still look up the
        // template and pass through the un-promoted definition_id.
        let repointed = match wstore.instance_repoint_definition(&old_def_id, &new_def.id) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(
                    template_id = %old_def_id,
                    new_definition_id = %new_def.id,
                    error = %e,
                    "template_promote migration: instance_repoint_definition failed"
                );
                stats.failures += 1;
                0
            }
        };
        stats.instances_repointed += repointed;
        stats.archives_moved += archives_for_this_def;
        stats.templates_promoted += 1;
        tracing::info!(
            template_id = %old_def_id,
            template_name = %template.name,
            new_definition_id = %new_def.id,
            new_name = %new_def.name,
            archives_moved = archives_for_this_def,
            instances_repointed = repointed,
            "template_promote migration: promoted template into user agent"
        );
    }

    // Marker write removed in the 2026-05-24 self-idempotency rework
    // (see doc comment above). The invariant "no seeded def carries a
    // session zone" is checked on every startup; when it already holds
    // this function is a sub-ms no-op.

    tracing::info!(
        templates_scanned = stats.templates_scanned,
        templates_promoted = stats.templates_promoted,
        archives_moved = stats.archives_moved,
        instances_repointed = stats.instances_repointed,
        failures = stats.failures,
        "template_promote migration: complete"
    );

    stats
}

/// Per-file decision inside `move_zone`'s retry-aware loop. See the
/// doc comment in `move_zone` for which round each variant addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyAction {
    /// Destination missing the file (R5 partial-copy fill).
    Copy,
    /// Source strictly newer than destination (R6 newer-source promotion).
    Overwrite,
    /// Destination strictly newer than source (R4 user-continuation
    /// on destination clone) — or equal-modts + equal bytes.
    Preserve,
    /// Equal modts; need to read both sides and compare bytes.
    TieBreakByBytes,
    /// Equal modts but bytes differ — neither side is canonical
    /// (R7 same-ms conflict). Preserve destination, leave source.
    Conflict,
}

/// Move every file in `old_zone` to `new_zone`, preserving names + bytes.
/// Implemented as read-write-delete because FileStore doesn't expose a
/// native rename; the cost is bounded by the per-zone file count (1-2
/// in practice — `output.state.json` + `output`).
fn move_zone(
    filestore: &FileStore,
    old_zone: &str,
    new_zone: &str,
) -> Result<(), String> {
    let files = filestore
        .list_files(old_zone)
        .map_err(|e| format!("list_files: {e}"))?;
    if files.is_empty() {
        return Ok(());
    }
    // Per-file recency-aware copy (codex P1 rounds 4 + 5 + 6 on
    // PR #1017). Three retry shapes need to coexist on the same
    // retry path:
    //
    //   R4 — partial-failure, user continued on the destination
    //        clone (`:current` of the new def). Destination has
    //        NEWER bytes than source. Keep destination; drop
    //        source.
    //   R5 — partial-failure, prior `move_zone` wrote SOME of the
    //        destination files before crashing. Destination has
    //        only some files; the missing ones must be copied
    //        from source. Don't drop source until every source
    //        file has a counterpart at the destination.
    //   R6 — partial-failure, `instance_repoint_definition` was
    //        the step that failed. Instances still point at the
    //        seeded def, user continued — SOURCE bytes are newer
    //        than destination's stale copy. Source must NOT be
    //        dropped without first promoting its newer content
    //        to the destination.
    //
    // Resolve all three via a per-file recency-aware copy:
    //   - destination missing the file → COPY (R5).
    //   - destination has the file, src.modts ≤ dest.modts → keep
    //     destination, no copy (R4).
    //   - destination has the file, src.modts > dest.modts → copy
    //     source over destination (R6).
    // After the loop, every source file has a counterpart at the
    // destination; source can be safely deleted.
    //
    // `modts` ties (or zero on either side) are resolved in favor
    // of keeping the destination, matching the R4 semantics — the
    // common case for a clean first-time retry where both sides
    // hold identical bytes.
    let dest_meta: std::collections::HashMap<String, crate::backend::storage::filestore::WaveFile> = filestore
        .list_files(new_zone)
        .map_err(|e| format!("list_files (new): {e}"))?
        .into_iter()
        .map(|f| (f.name.clone(), f))
        .collect();
    let mut copied = 0usize;
    let mut overwritten = 0usize;
    let mut preserved = 0usize;
    let mut conflicts = 0usize;
    for f in &files {
        let dest = dest_meta.get(&f.name);
        let action = match dest {
            None => CopyAction::Copy, // R5: destination missing
            Some(d) if f.modts > d.modts => CopyAction::Overwrite, // R6
            Some(d) if d.modts > f.modts => CopyAction::Preserve, // R4
            Some(_) => CopyAction::TieBreakByBytes, // R7: equal modts
        };
        let resolved = match action {
            CopyAction::Copy | CopyAction::Overwrite => action,
            CopyAction::Preserve => action,
            CopyAction::Conflict => action, // unreachable from the matcher above; explicit for exhaustiveness
            CopyAction::TieBreakByBytes => {
                // R7 — equal modts (millisecond-granular filestore
                // can write source + destination within the same
                // ms on a real retry). Read both sides and
                // disambiguate by bytes.
                let src_bytes = filestore
                    .read_file(old_zone, &f.name)
                    .map_err(|e| format!("read_file {}: {e}", f.name))?
                    .unwrap_or_default();
                let dest_bytes = filestore
                    .read_file(new_zone, &f.name)
                    .map_err(|e| format!("read_file (dest) {}: {e}", f.name))?
                    .unwrap_or_default();
                if src_bytes == dest_bytes {
                    CopyAction::Preserve
                } else {
                    // Conflict: can't tell which side is canonical.
                    // Preserve destination (matches the round-4
                    // semantics — keep what the user might be
                    // looking at), but refuse to delete source so
                    // the operator (or a future GC pass that can
                    // compare timestamps at a higher resolution)
                    // can resolve. The post-loop missing-files
                    // check would still pass, so we signal the
                    // conflict via a separate counter.
                    CopyAction::Conflict
                }
            }
        };
        match resolved {
            CopyAction::Copy => {
                let bytes = filestore
                    .read_file(old_zone, &f.name)
                    .map_err(|e| format!("read_file {}: {e}", f.name))?
                    .unwrap_or_default();
                write_zone_file(filestore, new_zone, &f.name, &bytes)?;
                copied += 1;
            }
            CopyAction::Overwrite => {
                let bytes = filestore
                    .read_file(old_zone, &f.name)
                    .map_err(|e| format!("read_file {}: {e}", f.name))?
                    .unwrap_or_default();
                write_zone_file(filestore, new_zone, &f.name, &bytes)?;
                overwritten += 1;
            }
            CopyAction::Preserve => {
                preserved += 1;
            }
            CopyAction::Conflict => {
                conflicts += 1;
                tracing::warn!(
                    old_zone = %old_zone,
                    new_zone = %new_zone,
                    file = %f.name,
                    modts = f.modts,
                    "template_promote migration: same-ms conflict — bytes differ at equal modts; preserving destination + leaving source for manual recovery"
                );
            }
            CopyAction::TieBreakByBytes => unreachable!("resolved above"),
        }
    }
    if preserved > 0 || overwritten > 0 || conflicts > 0 {
        tracing::info!(
            old_zone = %old_zone,
            new_zone = %new_zone,
            copied,
            overwritten,
            preserved,
            conflicts,
            "template_promote migration: per-file move (R4 user-continuation, R5 partial-copy fill, R6 newer-source promotion, R7 same-ms conflict)"
        );
    }
    if conflicts > 0 {
        // R7: an equal-modts byte-diff was detected. We don't know
        // which side is canonical, so we preserve both: destination
        // keeps its content, source is left in place for operator
        // / GC recovery. Migration converges next run only if the
        // operator resolves the conflict externally.
        return Ok(());
    }
    // Verify every source file has a counterpart at the
    // destination before dropping source — protects against the
    // R5 partial-write case where write_zone_file silently leaves
    // a file absent at the destination despite returning Ok (no
    // current call path does so, but defending the invariant here
    // is cheap and future-proofs the helper).
    let post_dest: std::collections::HashSet<String> = filestore
        .list_files(new_zone)
        .map_err(|e| format!("list_files (new, post): {e}"))?
        .into_iter()
        .map(|f| f.name)
        .collect();
    let missing: Vec<&str> = files
        .iter()
        .map(|f| f.name.as_str())
        .filter(|n| !post_dest.contains(*n))
        .collect();
    if !missing.is_empty() {
        tracing::warn!(
            old_zone = %old_zone,
            new_zone = %new_zone,
            missing = ?missing,
            "template_promote migration: destination missing files post-copy; leaving source in place for retry"
        );
        return Ok(());
    }
    // Delete the source files only after every write has succeeded.
    // delete_zone wipes the whole zone in one transaction.
    if let Err(e) = filestore.delete_zone(old_zone) {
        // Source delete failure is non-fatal — the new zone has the
        // data; the old zone is now stale duplicate, GC concern.
        tracing::warn!(
            old_zone = %old_zone,
            error = %e,
            "template_promote migration: delete_zone failed after copy; source remains"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::MetaMapType;
    use crate::backend::storage::filestore::FileStore;
    use crate::backend::storage::store::Store;
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

    fn open_temp_wstore(dir: &Path) -> Arc<Store> {
        let path = dir.join("objects.db");
        Arc::new(Store::open(&path).expect("open wstore"))
    }

    fn insert_agent_block(wstore: &Arc<Store>, def_id: &str) -> String {
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

    // ---- Two-tier picker Phase 1 migration tests ----

    use crate::backend::storage::store::{AgentDefinition, AgentInstance, InstanceStatus};

    fn insert_template(
        wstore: &Arc<Store>,
        id: &str,
        name: &str,
        provider: &str,
    ) -> AgentDefinition {
        let mut def = AgentDefinition {
            id: id.to_string(),
            slug: String::new(),
            name: name.to_string(),
            icon: String::new(),
            provider: provider.to_string(),
            description: format!("{name} template"),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1, // template
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
        };
        wstore.agent_def_insert(&mut def).unwrap();
        def
    }

    fn insert_named_instance(
        wstore: &Arc<Store>,
        id: &str,
        def_id: &str,
        instance_name: &str,
        started_at: i64,
    ) {
        let inst = AgentInstance {
            id: id.to_string(),
            definition_id: def_id.to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at,
            ended_at: 0,
            created_at: started_at,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: instance_name.to_string(),
            working_directory: String::new(),
            display_hidden: false,
        };
        wstore.instance_create(&inst).unwrap();
    }

    #[test]
    fn template_promote_clones_template_and_moves_zones() {
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        // Seeded template "Claude Code" with a current session zone +
        // one archive zone (the pre-existing "Maks" conversation).
        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        insert_named_instance(&wstore, "inst-maks", &template.id, "Maks", 1_700_000_100_000);
        write_session_state(
            &filestore,
            &template.id,
            br#"{"nodes":[{"type":"user_message","message":"hi"}]}"#,
        )
        .unwrap();
        // Pre-existing archive (simulates a prior + New session).
        let archive_zone = agent_archive_zone(&template.id, 1_699_000_000_000);
        write_zone_file(&filestore, &archive_zone, SNAPSHOT_FILE, b"archived").unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_scanned, 1);
        assert_eq!(stats.templates_promoted, 1);
        assert_eq!(stats.archives_moved, 1);
        assert_eq!(stats.instances_repointed, 1);
        assert_eq!(stats.failures, 0);

        // Template's current zone is gone — no `agent:tpl-claude:current`.
        let stale_current = agent_current_zone(&template.id);
        let stale = filestore.list_files(&stale_current).unwrap();
        assert!(stale.is_empty(), "template current zone should be empty post-promote");
        // Template's archive zone is gone.
        let stale_archive = filestore.list_files(&archive_zone).unwrap();
        assert!(stale_archive.is_empty(), "template archive zone should be empty post-promote");

        // Find the new user-owned definition. Use the most-recent
        // instance name ("Maks") as the new name per spec.
        let all = wstore.agent_def_list().unwrap();
        let new_def = all
            .iter()
            .find(|d| d.is_seeded == 0 && d.parent_id == template.id)
            .expect("a new user-owned definition should exist");
        assert_eq!(new_def.name, "Maks");
        assert_eq!(new_def.provider, "claude");

        // Zones present on the NEW defId.
        let new_current = agent_current_zone(&new_def.id);
        let new_files = filestore.list_files(&new_current).unwrap();
        assert!(
            new_files.iter().any(|f| f.name == SNAPSHOT_FILE),
            "new current zone should have output.state.json"
        );
        let new_archive = agent_archive_zone(&new_def.id, 1_699_000_000_000);
        let new_archive_files = filestore.list_files(&new_archive).unwrap();
        assert!(
            new_archive_files.iter().any(|f| f.name == SNAPSHOT_FILE),
            "new archive zone should be populated"
        );

        // Instance is repointed.
        let inst = wstore.instance_get("inst-maks").unwrap().unwrap();
        assert_eq!(
            inst.definition_id, new_def.id,
            "instance should now reference new user-agent def"
        );

        // Template definition is still around (still seeded), but the
        // session it carried is gone.
        let still_seeded = all.iter().find(|d| d.id == template.id).unwrap();
        assert_eq!(still_seeded.is_seeded, 1);

        // Marker file is intentionally NOT written under the
        // self-idempotency model (constant still exists for legacy
        // file compatibility — see the doc comment on
        // `TEMPLATE_PROMOTE_MARKER_V1`).
        assert!(!dir.path().join(TEMPLATE_PROMOTE_MARKER_V1).exists());
    }

    #[test]
    fn template_promote_is_idempotent_on_second_run() {
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        write_session_state(&filestore, &template.id, br#"{"nodes":[]}"#).unwrap();

        let first = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(first.templates_promoted, 1);

        let second = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(second.templates_scanned, 0);
        assert_eq!(second.templates_promoted, 0);
        assert_eq!(second.archives_moved, 0);
        assert_eq!(second.instances_repointed, 0);
    }

    #[test]
    fn template_promote_runs_when_seeded_def_grows_zone_after_first_run() {
        // Regression test for the 2026-05-24 "Maks not under My Agents"
        // failure mode. Under the old marker-file gate, this scenario
        // played out:
        //
        //   1. Portable v N starts: no seeded defs have session zones
        //      (fresh data dir). Migration runs, no-ops, writes marker.
        //   2. User clicks "Claude Code" template, has a real
        //      conversation. Session zone now lives at
        //      `agent:tpl-claude:current` (a seeded def carrying a
        //      session — invariant violation).
        //   3. Portable v N+1 starts. Marker present → migration
        //      skips. Seeded def keeps its session zone forever; the
        //      picker can't show the user's agent under My Agents
        //      because there is no user-clone definition.
        //
        // The self-idempotency rework dropped the marker gate and
        // re-runs the migration on every startup. This test simulates
        // that exact sequence and asserts the second run DOES promote.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        // First startup: a seeded template with no session zone yet.
        // Migration finds nothing to do (templates_scanned == 0).
        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        let first = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(first.templates_scanned, 0);
        assert_eq!(first.templates_promoted, 0);
        // (Under the old marker-gated model the marker was written here.)
        assert!(!dir.path().join(TEMPLATE_PROMOTE_MARKER_V1).exists());

        // Between startups: user opens a conversation on the seeded
        // template — invariant now violated.
        write_session_state(&filestore, &template.id, br#"{"nodes":[]}"#).unwrap();

        // Second startup: under the OLD gate this would be a no-op
        // (marker still present). Under the new self-idempotent model
        // it MUST detect the invariant violation and promote.
        let second = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(second.templates_scanned, 1);
        assert_eq!(second.templates_promoted, 1);
        assert_eq!(second.failures, 0);

        // User-owned definition exists post-promotion.
        let all = wstore.agent_def_list().unwrap();
        assert!(
            all.iter().any(|d| d.is_seeded == 0 && d.parent_id == template.id),
            "second-run promotion should create a user-owned def"
        );

        // Third startup: invariant restored, migration no-ops cleanly.
        let third = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(third.templates_scanned, 0);
        assert_eq!(third.templates_promoted, 0);
    }

    #[test]
    fn template_promote_does_not_reuse_clone_with_active_zone() {
        // Codex P1 round 2 on PR #1017: the reuse path must not
        // pick a user-clone whose own `agent:<clone_id>:current`
        // zone is populated — that clone was created by the user
        // through "+ New from template" and has a real conversation
        // in it. Reusing it would let `move_zone` overwrite the
        // user's live session with the seeded template's session.
        // The reuse target must be an empty-zone clone (partial-
        // failure shape) only.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        // A pre-existing user-clone created via "+ New from
        // template" — it has its OWN active conversation in its
        // own zone.
        let now = now_ms() as i64;
        let mut user_clone = crate::backend::storage::store::AgentDefinition {
            id: "user-made-clone".to_string(),
            slug: String::new(),
            name: "MyAgent".to_string(),
            icon: template.icon.clone(),
            provider: template.provider.clone(),
            description: template.description.clone(),
            working_directory: String::new(),
            shell: template.shell.clone(),
            provider_flags: template.provider_flags.clone(),
            auto_start: 0,
            restart_on_crash: template.restart_on_crash,
            idle_timeout_minutes: template.idle_timeout_minutes,
            created_at: now - 2_000,
            agent_type: template.agent_type.clone(),
            environment: template.environment.clone(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: template.id.clone(),
            branch_label: String::new(),
            updated_at: now - 2_000,
            user_hidden: 0,
        };
        wstore.agent_def_insert(&mut user_clone).unwrap();
        // The user's clone has its OWN active conversation.
        write_session_state(
            &filestore,
            &user_clone.id,
            br#"{"nodes":[{"type":"user_message","message":"mine"}]}"#,
        )
        .unwrap();

        // Seeded template ALSO has a session zone (the invariant
        // violation we're recovering from).
        write_session_state(
            &filestore,
            &template.id,
            br#"{"nodes":[{"type":"user_message","message":"theirs"}]}"#,
        )
        .unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_promoted, 1);

        // The user's clone must NOT have been used as the promote
        // target — a fresh clone with a new id must have been
        // created instead, with its OWN promoted zone.
        let user_zone_files = filestore
            .list_files(&agent_current_zone(&user_clone.id))
            .unwrap();
        let user_snapshot = user_zone_files
            .iter()
            .find(|f| f.name == SNAPSHOT_FILE)
            .expect("user-clone's own zone snapshot must still exist");
        let user_bytes = filestore
            .read_file(&agent_current_zone(&user_clone.id), &user_snapshot.name)
            .unwrap()
            .unwrap_or_default();
        assert!(
            std::str::from_utf8(&user_bytes).unwrap().contains("mine"),
            "user-clone's existing conversation must NOT be overwritten by the seeded session"
        );

        // A NEW clone (id != user-made-clone) must own the promoted
        // seeded session.
        let all = wstore.agent_def_list().unwrap();
        let new_clone = all
            .iter()
            .find(|d| d.is_seeded == 0 && d.parent_id == template.id && d.id != "user-made-clone")
            .expect("a NEW clone must have been created (not reusing the user's clone)");
        let new_zone_bytes = filestore
            .read_file(&agent_current_zone(&new_clone.id), SNAPSHOT_FILE)
            .unwrap()
            .unwrap_or_default();
        assert!(
            std::str::from_utf8(&new_zone_bytes).unwrap().contains("theirs"),
            "promoted session must land under the fresh clone's id"
        );
    }

    #[test]
    fn template_promote_preserves_user_continuation_on_clone() {
        // Codex P1 round 4 on PR #1017: data-loss scenario.
        // Sequence:
        //   1. Run 1 copies seeded `:current` → clone `:current`
        //      OK, but `delete_zone` on the seeded source fails.
        //   2. User opens the clone, continues the conversation —
        //      the clone's `:current` now has NEWER content.
        //   3. Run 2 sees the invariant still violated and would
        //      re-copy the (older) seeded bytes onto the clone,
        //      rolling back the user's continuation.
        // The fix: `move_zone` detects a non-empty destination and
        // drops the stale source instead of copying.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        // Prior partial run: deterministic-id clone def already
        // exists.
        let promote_target_id = format!("template-promote-v1-{}", template.id);
        let now = now_ms() as i64;
        let mut prior_target = crate::backend::storage::store::AgentDefinition {
            id: promote_target_id.clone(),
            slug: String::new(),
            name: "Claude Code".to_string(),
            icon: template.icon.clone(),
            provider: template.provider.clone(),
            description: template.description.clone(),
            working_directory: String::new(),
            shell: template.shell.clone(),
            provider_flags: template.provider_flags.clone(),
            auto_start: 0,
            restart_on_crash: template.restart_on_crash,
            idle_timeout_minutes: template.idle_timeout_minutes,
            created_at: now - 1_000,
            agent_type: template.agent_type.clone(),
            environment: template.environment.clone(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: template.id.clone(),
            branch_label: String::new(),
            updated_at: now - 1_000,
            user_hidden: 0,
        };
        wstore.agent_def_insert(&mut prior_target).unwrap();
        // Seeded `:current` has the OLDER stale snapshot the prior
        // run's `delete_zone` failed to remove. Write it FIRST so
        // its modts is earlier than the clone's continuation.
        write_session_state(
            &filestore,
            &template.id,
            br#"{"nodes":[{"type":"user_message","message":"old-stale-seeded"}]}"#,
        )
        .unwrap();
        // Force a modts gap so the modts-aware copy rule picks
        // destination (R4 user-continuation). 10ms is reliable on
        // every platform we ship to.
        std::thread::sleep(std::time::Duration::from_millis(10));
        // Clone's `:current` has the user's NEWER continuation.
        write_session_state(
            &filestore,
            &promote_target_id,
            br#"{"nodes":[{"type":"user_message","message":"my-newer-message"}]}"#,
        )
        .unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_promoted, 1);

        // The user's newer content is INTACT on the clone.
        let clone_bytes = filestore
            .read_file(&agent_current_zone(&promote_target_id), SNAPSHOT_FILE)
            .unwrap()
            .unwrap_or_default();
        let clone_str = std::str::from_utf8(&clone_bytes).unwrap();
        assert!(
            clone_str.contains("my-newer-message"),
            "user's newer continuation must survive the partial-failure retry; got: {clone_str}"
        );
        assert!(
            !clone_str.contains("old-stale-seeded"),
            "stale seeded content must NOT overwrite user's newer continuation"
        );

        // The seeded current zone is drained (source deleted).
        let seeded_files = filestore
            .list_files(&agent_current_zone(&template.id))
            .unwrap();
        assert!(
            seeded_files.is_empty(),
            "seeded current zone must be drained after the retry's safety drop"
        );
    }

    #[test]
    fn template_promote_recovers_partial_copy_at_zone() {
        // Codex P1 round 5 on PR #1017: a prior `move_zone` that
        // wrote SOME destination files but failed before the rest
        // must not be mistaken for "fully migrated" — dropping the
        // source there would lose the unwritten files forever.
        //
        // Setup: seeded `:current` has both files (snapshot +
        // output stream); the clone's `:current` has only the
        // snapshot (the prior copy crashed before the second
        // file). After retry: clone has BOTH files; seeded zone
        // is drained.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        // Prior partial run already created the deterministic-id
        // clone def.
        let promote_target_id = format!("template-promote-v1-{}", template.id);
        let now = now_ms() as i64;
        let mut prior_target = crate::backend::storage::store::AgentDefinition {
            id: promote_target_id.clone(),
            slug: String::new(),
            name: "Claude Code".to_string(),
            icon: template.icon.clone(),
            provider: template.provider.clone(),
            description: template.description.clone(),
            working_directory: String::new(),
            shell: template.shell.clone(),
            provider_flags: template.provider_flags.clone(),
            auto_start: 0,
            restart_on_crash: template.restart_on_crash,
            idle_timeout_minutes: template.idle_timeout_minutes,
            created_at: now - 1_000,
            agent_type: template.agent_type.clone(),
            environment: template.environment.clone(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: template.id.clone(),
            branch_label: String::new(),
            updated_at: now - 1_000,
            user_hidden: 0,
        };
        wstore.agent_def_insert(&mut prior_target).unwrap();

        // Seeded `:current` has BOTH files.
        let seeded_current = agent_current_zone(&template.id);
        write_zone_file(&filestore, &seeded_current, SNAPSHOT_FILE, b"seeded-snapshot").unwrap();
        write_zone_file(&filestore, &seeded_current, OUTPUT_FILE, b"seeded-output-stream").unwrap();

        // Clone `:current` already has ONLY the snapshot (prior
        // copy got that far, then failed on OUTPUT_FILE).
        write_zone_file(
            &filestore,
            &agent_current_zone(&promote_target_id),
            SNAPSHOT_FILE,
            b"seeded-snapshot",
        )
        .unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_promoted, 1);

        // Clone now has BOTH files (snapshot preserved, output
        // copied over from the source).
        let clone_zone = agent_current_zone(&promote_target_id);
        let clone_files = filestore.list_files(&clone_zone).unwrap();
        let clone_names: std::collections::HashSet<String> =
            clone_files.iter().map(|f| f.name.clone()).collect();
        assert!(
            clone_names.contains(SNAPSHOT_FILE),
            "snapshot file must remain at destination"
        );
        assert!(
            clone_names.contains(OUTPUT_FILE),
            "output file must be copied over from source on retry (codex R5)"
        );
        let output_bytes = filestore
            .read_file(&clone_zone, OUTPUT_FILE)
            .unwrap()
            .unwrap_or_default();
        assert_eq!(
            output_bytes, b"seeded-output-stream",
            "the unwritten file from the partial copy must arrive intact"
        );

        // Source is drained — every source file has a destination
        // counterpart now.
        let seeded_files = filestore.list_files(&seeded_current).unwrap();
        assert!(
            seeded_files.is_empty(),
            "seeded current zone must be drained after the complete copy"
        );
    }

    #[test]
    fn template_promote_promotes_newer_source_over_stale_destination() {
        // Codex P1 round 6 on PR #1017: the inverse of R4. If the
        // prior run's `instance_repoint_definition` failed,
        // instances stay pointed at the SEEDED def — the user's
        // continuation lands in the SEEDED zone, not the clone.
        // On retry, the SEEDED side has newer bytes. The fix
        // promotes the newer source over the stale destination
        // (and resolves R4 the other way when destination is
        // newer instead).
        //
        // Test setup: write destination FIRST (older modts), then
        // source SECOND (newer modts). After retry: destination
        // has the source's bytes; source drained.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        let promote_target_id = format!("template-promote-v1-{}", template.id);
        let now = now_ms() as i64;
        let mut prior_target = crate::backend::storage::store::AgentDefinition {
            id: promote_target_id.clone(),
            slug: String::new(),
            name: "Claude Code".to_string(),
            icon: template.icon.clone(),
            provider: template.provider.clone(),
            description: template.description.clone(),
            working_directory: String::new(),
            shell: template.shell.clone(),
            provider_flags: template.provider_flags.clone(),
            auto_start: 0,
            restart_on_crash: template.restart_on_crash,
            idle_timeout_minutes: template.idle_timeout_minutes,
            created_at: now - 1_000,
            agent_type: template.agent_type.clone(),
            environment: template.environment.clone(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: template.id.clone(),
            branch_label: String::new(),
            updated_at: now - 1_000,
            user_hidden: 0,
        };
        wstore.agent_def_insert(&mut prior_target).unwrap();

        // Destination has the prior copy (will become OLDER).
        let clone_zone = agent_current_zone(&promote_target_id);
        write_zone_file(&filestore, &clone_zone, SNAPSHOT_FILE, b"stale-old-copy").unwrap();
        // Sleep just long enough to push modts forward.
        // filestore's modts comes from system time; 10ms is enough
        // on every platform we ship to.
        std::thread::sleep(std::time::Duration::from_millis(10));
        // Seeded source has the user's newer continuation (the
        // instance_repoint failed in the prior run, so user kept
        // typing at the seeded def).
        let seeded_zone = agent_current_zone(&template.id);
        write_zone_file(&filestore, &seeded_zone, SNAPSHOT_FILE, b"user-newer-continuation").unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_promoted, 1);

        // Destination now carries the SOURCE's newer bytes.
        let clone_bytes = filestore
            .read_file(&clone_zone, SNAPSHOT_FILE)
            .unwrap()
            .unwrap_or_default();
        let clone_str = std::str::from_utf8(&clone_bytes).unwrap();
        assert!(
            clone_str.contains("user-newer-continuation"),
            "user's newer continuation must be promoted from seeded source to clone; got: {clone_str}"
        );
        assert!(
            !clone_str.contains("stale-old-copy"),
            "stale older destination bytes must be replaced by the newer source"
        );

        // Source drained.
        let seeded_files = filestore.list_files(&seeded_zone).unwrap();
        assert!(seeded_files.is_empty(), "seeded zone drained after promotion");
    }

    #[test]
    fn template_promote_uses_deterministic_clone_id() {
        // Every run of `migrate_promote_template_sessions_v1` for
        // the same template MUST produce a clone at the same
        // deterministic id (`template-promote-v1-<template_id>`).
        // This is the convergence invariant that makes retries
        // safe under any partial-failure mode without ever
        // splitting one logical agent across multiple clone ids.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        write_session_state(
            &filestore,
            &template.id,
            br#"{"nodes":[{"type":"user_message","message":"hi"}]}"#,
        )
        .unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_promoted, 1);

        let expected_id = format!("template-promote-v1-{}", template.id);
        let clone = wstore.agent_def_get(&expected_id).unwrap();
        assert!(clone.is_some(), "promote target must be created at the deterministic id");
        assert_eq!(clone.unwrap().parent_id, template.id);
    }

    #[test]
    fn template_promote_idempotent_under_partial_failure_at_archive_move() {
        // Codex P1 round 3 on PR #1017: when a prior run copies
        // the seeded `:current` zone successfully but leaves at
        // least one seeded zone behind (e.g. `move_zone` succeeds
        // for `:current` but the source delete fails OR a later
        // `:archive:*` move fails), the next startup re-enters
        // migration for that template. The deterministic clone id
        // means the retry hits the SAME clone — never splitting
        // history across clone ids. Reuses the existing clone def,
        // re-runs move_zone (idempotent: write replaces if newer,
        // delete is best-effort), and converges.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        insert_named_instance(&wstore, "inst-maks", &template.id, "Maks", 1_700_000_100_000);

        // Simulate the partial-failure state: prior run created
        // the deterministic-id clone, moved :current successfully
        // (clone has data), but failed to remove the seeded
        // :archive:* zone (still on the seeded id).
        let promote_target_id = format!("template-promote-v1-{}", template.id);
        let now = now_ms() as i64;
        let mut prior_target = crate::backend::storage::store::AgentDefinition {
            id: promote_target_id.clone(),
            slug: String::new(),
            name: "Maks".to_string(),
            icon: template.icon.clone(),
            provider: template.provider.clone(),
            description: template.description.clone(),
            working_directory: String::new(),
            shell: template.shell.clone(),
            provider_flags: template.provider_flags.clone(),
            auto_start: 0,
            restart_on_crash: template.restart_on_crash,
            idle_timeout_minutes: template.idle_timeout_minutes,
            created_at: now - 1_000,
            agent_type: template.agent_type.clone(),
            environment: template.environment.clone(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: template.id.clone(),
            branch_label: String::new(),
            updated_at: now - 1_000,
            user_hidden: 0,
        };
        wstore.agent_def_insert(&mut prior_target).unwrap();
        // Realistic partial-failure shape: run 1 copied :current
        // successfully (dest and source have IDENTICAL bytes from
        // that copy), and run 1's archive-move failed (archive
        // still on the seeded side, never copied to the clone).
        // Use identical bytes for :current so the modts-aware
        // copy gate treats it as no-op (no conflict).
        let snapshot_bytes = b"snapshot-from-prior-run".as_slice();
        write_zone_file(&filestore, &agent_current_zone(&promote_target_id), SNAPSHOT_FILE, snapshot_bytes).unwrap();
        write_zone_file(&filestore, &agent_current_zone(&template.id), SNAPSHOT_FILE, snapshot_bytes).unwrap();
        let stale_archive = agent_archive_zone(&template.id, 1_699_000_000_000);
        write_zone_file(&filestore, &stale_archive, SNAPSHOT_FILE, b"old archive").unwrap();

        // Pre-condition: exactly one user-clone DEF (the
        // deterministic-id one). Use the dedicated
        // `db_agent_definitions` scan (not `agent_def_list`, which
        // reads `db_agents` and surfaces template-instance
        // projection rows).
        let clones_pre = wstore.user_clone_defs_for_template(&template.id).unwrap();
        assert_eq!(clones_pre.len(), 1, "test setup: one prior clone at deterministic id");
        assert_eq!(clones_pre[0].id, promote_target_id);

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_scanned, 1);
        assert_eq!(stats.templates_promoted, 1);

        // Still exactly one user-clone def — the retry reused the
        // deterministic-id clone instead of inserting another.
        let clones_post = wstore.user_clone_defs_for_template(&template.id).unwrap();
        assert_eq!(
            clones_post.len(),
            1,
            "deterministic-id reuse must not create a duplicate clone on partial-failure retry"
        );
        assert_eq!(clones_post[0].id, promote_target_id);

        // Both seeded zones are now drained onto the clone.
        let seeded_current = filestore
            .list_files(&agent_current_zone(&template.id))
            .unwrap();
        assert!(
            seeded_current.is_empty(),
            "seeded current zone should be empty after the retry's successful move"
        );
        let seeded_archive_files = filestore.list_files(&stale_archive).unwrap();
        assert!(
            seeded_archive_files.is_empty(),
            "seeded archive zone should be empty after the retry's successful move"
        );

        // Re-run after convergence — pure no-op.
        let stats2 = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats2.templates_scanned, 0);
        assert_eq!(stats2.templates_promoted, 0);
    }

    #[test]
    fn template_promote_ignores_legacy_marker_file() {
        // Backward-compat: an existing v1 marker file from a portable
        // running pre-self-idempotency code must NOT prevent the
        // migration from running. The 2026-05-24 rework leaves any
        // existing marker file in place but doesn't read it.
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        // Place a vestigial marker as if a prior startup wrote one.
        std::fs::write(dir.path().join(TEMPLATE_PROMOTE_MARKER_V1), b"v1\n").unwrap();

        // Now set up an invariant violation.
        let template = insert_template(&wstore, "tpl-claude", "Claude Code", "claude");
        write_session_state(&filestore, &template.id, br#"{"nodes":[]}"#).unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        // Must NOT skip — the legacy marker is ignored.
        assert_eq!(stats.templates_scanned, 1);
        assert_eq!(stats.templates_promoted, 1);
    }

    #[test]
    fn template_promote_falls_back_to_template_name_when_no_named_instance() {
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        let template = insert_template(&wstore, "tpl-x", "Cursor", "cursor");
        write_session_state(&filestore, &template.id, br#"{"nodes":[]}"#).unwrap();
        // NO instances inserted.

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_promoted, 1);

        let all = wstore.agent_def_list().unwrap();
        let new_def = all
            .iter()
            .find(|d| d.is_seeded == 0 && d.parent_id == template.id)
            .expect("should clone the template");
        // Falls back to template name when no named instance exists.
        assert_eq!(new_def.name, "Cursor");
    }

    #[test]
    fn template_promote_skips_already_user_owned_definitions() {
        let dir = tempdir().unwrap();
        let wstore = open_temp_wstore(dir.path());
        let filestore = fresh_filestore();

        // A user-owned definition (is_seeded = 0) with a session — the
        // migration should leave it alone.
        let mut user_def = AgentDefinition {
            id: "user-abc".to_string(),
            slug: String::new(),
            name: "My Agent".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
        };
        wstore.agent_def_insert(&mut user_def).unwrap();
        write_session_state(&filestore, &user_def.id, br#"{"nodes":[]}"#).unwrap();

        let stats = migrate_promote_template_sessions_v1(&wstore, &filestore, dir.path());
        assert_eq!(stats.templates_scanned, 0);
        assert_eq!(stats.templates_promoted, 0);

        // Original definition untouched.
        let all = wstore.agent_def_list().unwrap();
        let still_there = all.iter().find(|d| d.id == "user-abc").unwrap();
        assert_eq!(still_there.is_seeded, 0);

        // Session zone still present.
        let cur = agent_current_zone(&user_def.id);
        let files = filestore.list_files(&cur).unwrap();
        assert!(!files.is_empty());
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
