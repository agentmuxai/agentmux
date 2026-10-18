// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Archive lifecycle, browsing, and previews for agent session zones.

use crate::backend::storage::filestore::FileStore;

use super::global_store::global_transcript_store;
use super::helpers::{now_ms, write_zone_file};
use super::session_io::{OUTPUT_FILE, SNAPSHOT_FILE, TSIDX_FILE};
use super::zone_naming::{agent_archive_zone, is_valid_definition_id, validate_and_current};

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
    archive_session_in(filestore, global_transcript_store().map(|g| g.as_ref()), definition_id)
}

/// [`archive_session`] with the global transcript store passed in (`None`
/// when there is none). The archive is written to the global store whenever
/// there is one: the agent's record (`history/record.rs`, behind
/// SearchHistory and the Agent History view) and the history summary read
/// archives only there, so an archive written to the per-channel store drops
/// out of the agent's history.
pub(crate) fn archive_session_in(
    filestore: &FileStore,
    global: Option<&FileStore>,
    definition_id: &str,
) -> Result<Option<(String, i64)>, String> {
    let current_zone = validate_and_current(definition_id)?;

    // Prefer the GLOBAL current zone as the archive source. It is the complete
    // cross-channel accumulation — and (via the hot-path mirror) the place the
    // `output` NDJSON is fully gathered — so the per-channel `:current` is at
    // most a subset (often just this channel's snapshot). Archiving from the
    // global store therefore preserves the full history in *every* case:
    //   - cross-channel viewer (empty local), AND
    //   - cross-channel session that also ran locally (non-empty local) —
    // both previously risked discarding global-only history.
    // We then clear BOTH currents so the read fallback can't resurrect the
    // just-archived conversation. Falls through to the per-channel path below
    // only when there's no global store or it holds nothing for this agent
    // (pure pre-global / same-channel-only data). (codex + reagent P1/P2 #1399.)
    if let Some(gfs) = global {
        match archive_global_current(gfs, definition_id)? {
            GlobalArchive::Archived(zone, ts) => {
                clear_local_current_zone(filestore, &current_zone);
                clear_global_current_zone_in(gfs, definition_id);
                return Ok(Some((zone, ts)));
            }
            // Another instance archived and cleared it first: this channel's
            // copy still goes, or its stale snapshot would bring it back.
            GlobalArchive::ArchivedElsewhere => {
                clear_local_current_zone(filestore, &current_zone);
                return Ok(None);
            }
            GlobalArchive::Nothing => {}
        }
    }

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

    // Per-channel content is archived into the global store too, when there is
    // one. It is written to a staging zone of this call's own and then moved
    // under a free archive name in one transaction, so two instances archiving
    // at once can't write into the same archive.
    let target = global.unwrap_or(filestore);
    let staging = format!("agent:{definition_id}:archiving:{}", uuid::Uuid::new_v4());

    // Copy snapshot first (the canonical "history" file).
    if has_state {
        let snapshot_bytes = filestore
            .read_file(&current_zone, SNAPSHOT_FILE)
            .map_err(|e| format!("read current snapshot: {e}"))?
            .unwrap_or_default();
        write_zone_file(target, &staging, SNAPSHOT_FILE, &snapshot_bytes)?;
    }
    // Copy NDJSON output if present.
    if has_output {
        let output_bytes = filestore
            .read_file(&current_zone, OUTPUT_FILE)
            .map_err(|e| format!("read current output: {e}"))?
            .unwrap_or_default();
        if !output_bytes.is_empty() {
            write_zone_file(target, &staging, OUTPUT_FILE, &output_bytes)?;
        }
        // The tsidx sidecar travels with output (codex P2 on PR #2508):
        // without it, archived history loses its receive-time stamps and —
        // worse — a stale sidecar left in :current would mis-time the NEXT
        // session (fresh output restarts at offset 0 under old entries).
        // Best-effort: the sidecar is auxiliary and must never abort an
        // archive.
        copy_tsidx_best_effort(filestore, &current_zone, target, &staging);
    }
    let (archive_zone, ts) = claim_archive_zone(target, &staging, definition_id)?;

    // Archive write succeeded. Now safe to clear the current zone: the files
    // that were archived, and both sidecars with `output` — a stale tsidx
    // under a fresh (offset-0) output would mis-time the next session's lines
    // (codex P2 on PR #2508), and a stale output.idx could pass
    // blockfile:read_range's covered_size check by coincidence (reagent P1 on
    // #2701). One transaction (5a-2b): a crash can't leave a sidecar
    // describing an `output` that is gone.
    let mut clear: Vec<&str> = Vec::with_capacity(4);
    if state_stat.is_some() {
        clear.push(SNAPSHOT_FILE);
    }
    if output_stat.is_some() {
        clear.push(OUTPUT_FILE);
    }
    clear.extend([TSIDX_FILE, "output.idx"]);
    if let Err(e) = filestore.delete_files(&current_zone, &clear) {
        tracing::warn!(
            definition_id = %definition_id,
            error = %e,
            "agent_session: failed to clear current zone after archive (archive already persisted)"
        );
    }

    // Clear the GLOBAL current zone in the same lifecycle. Without this, the
    // cross-channel read fallback (`read_session_state` / `blockfile:read_range`
    // / `blockfile:line_count`) would treat the intentionally-cleared local
    // `:current` as a cross-channel miss and resurrect the just-archived
    // conversation on the next open — so a "new session" for this definition
    // would inherit stale history. Best-effort, same as the per-channel clear.
    // (codex P1 on PR #1399.)
    if let Some(gfs) = global {
        clear_global_current_zone_in(gfs, definition_id);
    }

    tracing::info!(
        definition_id = %definition_id,
        archive_zoneid = %archive_zone,
        archived_at_ms = ts,
        "agent_session: archived current session"
    );

    Ok(Some((archive_zone, ts as i64)))
}

/// Archive the agent's GLOBAL `agent:<defId>:current` into a global
/// `agent:<defId>:archive:<ms>` zone, where the agent's record reads it.
///
/// The files are moved (renamed) inside the database, in one transaction: a
/// conversation of any size never passes through memory, and only one
/// archive can take it. The output's timestamps and the running summary go
/// with it.
fn archive_global_current(gfs: &FileStore, definition_id: &str) -> Result<GlobalArchive, String> {
    let current_zone = validate_and_current(definition_id)?;
    let has = |name: &str| -> Result<bool, String> {
        Ok(gfs.stat(&current_zone, name).map_err(|e| format!("stat: {e}"))?.is_some_and(|f| f.size > 0))
    };
    // A snapshot written through the cache may not be in the database yet.
    gfs.flush_cache().map_err(|e| format!("flush before archive: {e}"))?;
    if !has(SNAPSHOT_FILE)? && !has(OUTPUT_FILE)? {
        return Ok(GlobalArchive::Nothing);
    }

    let files = [SNAPSHOT_FILE, OUTPUT_FILE, TSIDX_FILE, crate::backend::continuity_state::STATE_FILE];
    // The store is shared by every instance on the machine. The files are
    // moved, not copied, so only one archive can take them; another instance
    // can still take the free name between the check and the move.
    for _ in 0..8 {
        let (archive_zone, ts) = free_archive_zone(gfs, definition_id)?;
        match gfs
            .move_files_to_zone(&current_zone, &archive_zone, &files)
            .map_err(|e| format!("move to archive: {e}"))?
        {
            // The name was taken in between: try the next one.
            None => continue,
            // Nothing left to move: another instance archived it first.
            Some(0) => return Ok(GlobalArchive::ArchivedElsewhere),
            Some(_) => {
                tracing::info!(
                    definition_id = %definition_id,
                    archive_zoneid = %archive_zone,
                    archived_at_ms = ts,
                    "agent_session: archived the global session"
                );
                return Ok(GlobalArchive::Archived(archive_zone, ts as i64));
            }
        }
    }
    Err(format!("no free archive zone for {definition_id}"))
}

/// What [`archive_global_current`] did.
enum GlobalArchive {
    Archived(String, i64),
    /// The global current held nothing to archive.
    Nothing,
    /// Another instance archived and cleared the global current first.
    ArchivedElsewhere,
}

/// Move what [`archive_session_in`] staged in `staging` under a free archive
/// name. The move fails, and the next name is tried, when another instance
/// took the name first.
fn claim_archive_zone(store: &FileStore, staging: &str, definition_id: &str) -> Result<(String, u64), String> {
    // The staged files were written through the cache; the move reads the database.
    store.flush_cache().map_err(|e| format!("flush before archive: {e}"))?;
    for _ in 0..8 {
        let (zone, ts) = free_archive_zone(store, definition_id)?;
        let moved = store
            .move_files_to_zone(staging, &zone, &[SNAPSHOT_FILE, OUTPUT_FILE, TSIDX_FILE])
            .map_err(|e| format!("move to archive: {e}"))?;
        if moved.is_some() {
            return Ok((zone, ts));
        }
    }
    Err(format!("no free archive zone for {definition_id}; the conversation stays in {staging}"))
}

/// An archive zone for now that holds nothing yet in `store`. Two archives in
/// the same millisecond would otherwise share a name, and the second would
/// overwrite the first.
fn free_archive_zone(store: &FileStore, definition_id: &str) -> Result<(String, u64), String> {
    let mut ts = now_ms();
    for _ in 0..1000 {
        let zone = agent_archive_zone(definition_id, ts);
        if store.zone_ids_with_prefix(&zone).map_err(|e| format!("zone_ids_with_prefix: {e}"))?.iter().all(|z| *z != zone) {
            return Ok((zone, ts));
        }
        ts += 1;
    }
    Err(format!("no free archive zone for {definition_id}"))
}

/// Copy the `output.tsidx` sidecar from `src` zone to `dst` zone,
/// best-effort: absence is normal (pre-sidecar history), and no failure
/// here may abort the surrounding archive — the sidecar is auxiliary
/// timing data, not conversation content.
fn copy_tsidx_best_effort(src_store: &FileStore, src_zone: &str, dst_store: &FileStore, dst_zone: &str) {
    let bytes = match src_store.stat(src_zone, TSIDX_FILE) {
        Ok(Some(f)) if f.size > 0 => match src_store.read_file(src_zone, TSIDX_FILE) {
            Ok(Some(b)) if !b.is_empty() => b,
            _ => return,
        },
        _ => return,
    };
    if let Err(e) = write_zone_file(dst_store, dst_zone, TSIDX_FILE, &bytes) {
        tracing::warn!(src = %src_zone, dst = %dst_zone, error = %e, "agent_session: tsidx sidecar copy failed (archive content unaffected)");
    }
}

/// Delete `output.state.json` + `output` from a per-channel `:current` zone,
/// only for files that are present (so absence isn't logged as an error).
/// Best-effort — used after the global-preferred archive has persisted the
/// content, to retire this channel's (subset) copy.
///
/// Also clears `output.idx` (#2701): an index left behind after `output` is
/// deleted and rewritten could otherwise be accepted for a new output of the
/// same byte size, serving the old session's line count/offsets. Since #3634
/// the index also carries the output generation it was built for, which
/// catches that too; deleting it here keeps the zone clean regardless.
pub fn clear_local_current_zone(filestore: &FileStore, zone: &str) {
    // One transaction (5a-2b); absent files are not an error.
    if let Err(e) = filestore.delete_files(zone, &[SNAPSHOT_FILE, OUTPUT_FILE, TSIDX_FILE, "output.idx"]) {
        tracing::warn!(zone = %zone, error = %e, "agent_session: failed to clear local current after global archive");
    }
}

/// Delete `output.state.json` + `output` from the agent's GLOBAL
/// `agent:<defId>:current` zone in `gfs`. Best-effort:
/// a missing file is the expected "agent never mirrored" case (silent), other
/// errors are logged but never propagated. Keeps the global zone in lockstep
/// with the per-channel `:current` clear in [`archive_session`].
///
/// Also clears `output.idx` (#2701): an index left behind after `output` is
/// deleted and rewritten could otherwise be accepted for a new output of the
/// same byte size. Since #3634 the index also carries the output generation it
/// was built for, which catches that too; deleting it here forces a rebuild
/// against the new content regardless.
fn clear_global_current_zone_in(gfs: &FileStore, definition_id: &str) {
    let Ok(zone) = validate_and_current(definition_id) else {
        return;
    };
    // One transaction (5a-2b); absent files are not an error.
    if let Err(e) = gfs.delete_files(
        &zone,
        &[
            SNAPSHOT_FILE,
            OUTPUT_FILE,
            TSIDX_FILE,
            "output.idx",
            // The running summary describes the conversation being archived.
            crate::backend::continuity_state::STATE_FILE,
        ],
    ) {
        tracing::warn!(
            zone = %zone, error = %e,
            "global transcripts: failed to clear current zone on archive"
        );
    }
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
    list_archives_in(filestore, global_transcript_store().map(|g| g.as_ref()), definition_id, limit)
}

/// [`list_archives`] over the global store, where archives are written, and
/// the per-channel store, which holds archives written before that.
pub(crate) fn list_archives_in(
    filestore: &FileStore,
    global: Option<&FileStore>,
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

    let mut matches: Vec<(u64, String, &FileStore)> = Vec::new();
    for store in global.into_iter().chain(std::iter::once(filestore)) {
        let zones = store
            .zone_ids_with_prefix(&prefix)
            .map_err(|e| format!("zone_ids_with_prefix: {e}"))?;
        for zone in zones {
            let Some(Ok(ts)) = zone.strip_prefix(&prefix).map(str::parse::<u64>) else {
                continue;
            };
            // The global copy wins when both stores hold the same archive.
            if !matches.iter().any(|(_, z, _)| *z == zone) {
                matches.push((ts, zone, store));
            }
        }
    }
    // Newest first.
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    matches.truncate(limit);

    let mut rows = Vec::with_capacity(matches.len());
    for (ts, zone, store) in matches {
        let (preview, node_count) = read_archive_preview(store, &zone);
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

/// Pull a small preview + node_count out of an archive's
/// `output.state.json`. Returns `("", 0)` on any error.
fn read_archive_preview(filestore: &FileStore, zone: &str) -> (String, usize) {
    let bytes = match filestore.read_file(zone, SNAPSHOT_FILE) {
        Ok(Some(b)) => b,
        _ => return (String::new(), 0),
    };
    if bytes.len() > MAX_PREVIEW_SNAPSHOT_BYTES {
        return (String::new(), 0);
    }
    snapshot_preview(&bytes)
}

/// Snapshots larger than this are not parsed for a preview, so a corrupted one
/// can't stall a listing.
pub(crate) const MAX_PREVIEW_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

/// `(preview, node_count)` from an `output.state.json` snapshot's bytes, or
/// `("", 0)` when they don't parse or have no `nodes` array. The preview is
/// the first `user_message`, skipping a leading "# Session Context" bootstrap
/// message when a later one exists, collapsed to one line of at most 240 chars.
pub(crate) fn snapshot_preview(bytes: &[u8]) -> (String, usize) {
    let json: serde_json::Value = match serde_json::from_slice(bytes) {
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
            // Stash as fallback in case there's no later user_message.
            preview = collapse_preview(msg);
            continue;
        }
        preview = collapse_preview(msg);
        break;
    }
    (preview, node_count)
}

/// Collapse newlines + extra whitespace, cap at 240 chars. Output is
/// safe to render inline in a single-line preview row.
pub(crate) fn collapse_preview(s: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut buf = String::with_capacity(s.len().min(MAX_CHARS + 4));
    let mut prev_space = false;
    for ch in s.chars() {
        if buf.chars().count() >= MAX_CHARS {
            buf.push('\u{2026}'); // "…"
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

#[cfg(test)]
mod preview_tests {
    use super::snapshot_preview;

    #[test]
    fn snapshot_preview_skips_a_leading_session_context_and_counts_nodes() {
        let snap = serde_json::json!({ "nodes": [
            { "type": "user_message", "message": "# Session Context\nboilerplate" },
            { "type": "markdown", "content": "ack" },
            { "type": "user_message", "message": "  fix the\n\nbug  " },
        ]});
        let bytes = serde_json::to_vec(&snap).unwrap();
        assert_eq!(snapshot_preview(&bytes), ("fix the bug".to_string(), 3));
        assert_eq!(snapshot_preview(b"not json"), (String::new(), 0));
        assert_eq!(snapshot_preview(br#"{"nodes":{}}"#), (String::new(), 0));
    }
}
