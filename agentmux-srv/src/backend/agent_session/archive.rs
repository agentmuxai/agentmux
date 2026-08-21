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
    if let Some(archived) = archive_global_current(filestore, definition_id)? {
        clear_local_current_zone(filestore, &current_zone);
        clear_global_current_zone(definition_id);
        return Ok(Some(archived));
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
        // The tsidx sidecar travels with output (codex P2 on PR #2508):
        // without it, archived history loses its receive-time stamps and —
        // worse — a stale sidecar left in :current would mis-time the NEXT
        // session (fresh output restarts at offset 0 under old entries).
        // Best-effort: the sidecar is auxiliary and must never abort an
        // archive.
        copy_tsidx_best_effort(filestore, &current_zone, filestore, &archive_zone);
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
    // Clear the sidecar with output — a stale tsidx under a fresh (offset-0)
    // output would mis-time the next session's lines (codex P2 on PR #2508).
    if let Ok(Some(_)) = filestore.stat(&current_zone, TSIDX_FILE) {
        if let Err(e) = filestore.delete_file(&current_zone, TSIDX_FILE) {
            tracing::warn!(
                definition_id = %definition_id,
                error = %e,
                "agent_session: failed to clear current tsidx after archive"
            );
        }
    }
    // Clear the output.idx sidecar too (reagent P1 on #2701): same
    // stale-cache-by-coincidence risk as the tsidx case above, but for
    // blockfile:read_range's covered_size freshness check.
    if let Ok(Some(_)) = filestore.stat(&current_zone, "output.idx") {
        if let Err(e) = filestore.delete_file(&current_zone, "output.idx") {
            tracing::warn!(
                definition_id = %definition_id,
                error = %e,
                "agent_session: failed to clear current output.idx after archive"
            );
        }
    }

    // Clear the GLOBAL current zone in the same lifecycle. Without this, the
    // cross-channel read fallback (`read_session_state` / `blockfile:read_range`
    // / `blockfile:line_count`) would treat the intentionally-cleared local
    // `:current` as a cross-channel miss and resurrect the just-archived
    // conversation on the next open — so a "new session" for this definition
    // would inherit stale history. Best-effort, same as the per-channel clear.
    // (codex P1 on PR #1399.)
    clear_global_current_zone(definition_id);

    tracing::info!(
        definition_id = %definition_id,
        archive_zoneid = %archive_zone,
        archived_at_ms = ts,
        "agent_session: archived current session"
    );

    Ok(Some((archive_zone, ts as i64)))
}

/// Archive the agent's GLOBAL `agent:<defId>:current` content into a *local*
/// (per-`filestore`) archive zone, so a cross-channel viewer's conversation is
/// preserved + browsable in this channel before the global current is cleared.
///
/// Returns `Ok(Some((archive_zoneid, ts)))` when the global current held
/// content (snapshot or output), `Ok(None)` when there was nothing to archive
/// (no global store, or both files empty/absent). The archive lands in the
/// per-channel store because archive browsing (`list_archives`) is per-channel.
pub fn archive_global_current(
    filestore: &FileStore,
    definition_id: &str,
) -> Result<Option<(String, i64)>, String> {
    let Some(gfs) = global_transcript_store() else {
        return Ok(None);
    };
    let current_zone = validate_and_current(definition_id)?;

    let snap = read_snapshot_bytes(gfs, &current_zone, SNAPSHOT_FILE)?;
    let out = read_snapshot_bytes(gfs, &current_zone, OUTPUT_FILE)?;
    let has_snap = snap.as_ref().is_some_and(|b| !b.is_empty());
    let has_out = out.as_ref().is_some_and(|b| !b.is_empty());
    if !has_snap && !has_out {
        return Ok(None);
    }

    let ts = now_ms();
    let archive_zone = agent_archive_zone(definition_id, ts);
    if has_snap {
        write_zone_file(filestore, &archive_zone, SNAPSHOT_FILE, snap.as_ref().unwrap())?;
    }
    if has_out {
        write_zone_file(filestore, &archive_zone, OUTPUT_FILE, out.as_ref().unwrap())?;
        // Sidecar travels with output — same rationale as the per-channel
        // path (codex P2 on PR #2508).
        copy_tsidx_best_effort(gfs, &current_zone, filestore, &archive_zone);
    }
    tracing::info!(
        definition_id = %definition_id,
        archive_zoneid = %archive_zone,
        archived_at_ms = ts,
        "agent_session: archived cross-channel (global) session into local archive"
    );
    Ok(Some((archive_zone, ts as i64)))
}

/// Read a zone file's full bytes, mapping absence to `Ok(None)`.
fn read_snapshot_bytes(store: &FileStore, zone: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
    match store.stat(zone, name).map_err(|e| format!("stat: {e}"))? {
        Some(_) => store
            .read_file(zone, name)
            .map_err(|e| format!("read_file: {e}")),
        None => Ok(None),
    }
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
/// Also clears `output.idx` (reagent P1 on #2701, mirroring codex's P2 on the
/// global-zone twin of this function): `blockfile:read_range`'s freshness
/// check is a `covered_size` byte comparison against the current `output`, so
/// a stale index left behind after `output` is deleted and rewritten from
/// scratch could be spuriously accepted the moment the new output happens to
/// reach the same byte size, silently serving the old session's cached line
/// count/offsets.
pub fn clear_local_current_zone(filestore: &FileStore, zone: &str) {
    for name in [SNAPSHOT_FILE, OUTPUT_FILE, TSIDX_FILE, "output.idx"] {
        match filestore.stat(zone, name) {
            Ok(Some(_)) => {
                if let Err(e) = filestore.delete_file(zone, name) {
                    tracing::warn!(zone = %zone, file = %name, error = %e, "agent_session: failed to clear local current after global archive");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(zone = %zone, file = %name, error = %e, "agent_session: stat failed clearing local current"),
        }
    }
}

/// Delete `output.state.json` + `output` from the agent's GLOBAL
/// `agent:<defId>:current` zone, if a global store is installed. Best-effort:
/// a missing file is the expected "agent never mirrored" case (silent), other
/// errors are logged but never propagated. Keeps the global zone in lockstep
/// with the per-channel `:current` clear in [`archive_session`].
///
/// Also clears `output.idx` (codex P2 on #2701): its freshness check is a
/// `covered_size` byte comparison against the current `output`, so a stale
/// index left behind after `output` is deleted and rewritten from scratch
/// could be spuriously accepted as fresh the moment the new output happens
/// to reach the same byte size, silently reporting the old session's line
/// count. Deleting it here forces a rebuild against the new content instead.
pub fn clear_global_current_zone(definition_id: &str) {
    let Some(gfs) = global_transcript_store() else {
        return;
    };
    let Ok(zone) = validate_and_current(definition_id) else {
        return;
    };
    for name in [SNAPSHOT_FILE, OUTPUT_FILE, TSIDX_FILE, "output.idx"] {
        // Only delete what's present, so an absent file isn't logged as an error.
        match gfs.stat(&zone, name) {
            Ok(Some(_)) => {
                if let Err(e) = gfs.delete_file(&zone, name) {
                    tracing::warn!(
                        zone = %zone, file = %name, error = %e,
                        "global transcripts: failed to clear current zone on archive"
                    );
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(
                zone = %zone, file = %name, error = %e,
                "global transcripts: stat failed clearing current zone on archive"
            ),
        }
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
