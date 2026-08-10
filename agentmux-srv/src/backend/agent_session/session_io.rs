// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Read/write/append snapshot state for the `agent:<defId>:current` zone,
//! including the global-mirror normalization + one-shot heal.

use crate::backend::storage::filestore::FileStore;

use super::global_store::global_transcript_store;
use super::helpers::{ensure_file, write_zone_file};
use super::zone_naming::{agent_current_zone, is_valid_definition_id, validate_and_current};

// ---------------------------------------------------------------------------
// File names within an agent session zone (mirrors per-block zone shape)
// ---------------------------------------------------------------------------

/// Full UI snapshot (JSON). Frontend reads this on pane mount.
pub const SNAPSHOT_FILE: &str = "output.state.json";
/// Raw NDJSON stream for crash-recovery replay.
pub const OUTPUT_FILE: &str = "output";
/// Receive-time sidecar for `output`: one NDJSON `{"off":<byte offset of the
/// appended batch>,"ms":<unix ms>}` record per append batch. A pure addition —
/// `output`'s own format and every existing parser are untouched; a missing or
/// corrupt sidecar degrades reads to "no stamps" (today's behavior). Gives
/// replayed transcript nodes real timestamps (hover peek after restore, day
/// separators). Spec:
/// SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §4.4.
pub const TSIDX_FILE: &str = "output.tsidx";

/// Write `content` to `output.state.json` in `agent:<defId>:current`.
/// Idempotent — creates the file if missing, overwrites otherwise.
///
/// The per-channel write is primary (preserving all existing same-channel
/// behaviour); the snapshot overlay is *additionally* mirrored into the GLOBAL
/// transcript store (when installed) so a cross-channel open — and the
/// migrated-agent backfill — find a coherent zone (overlay + `output`
/// together). Mirroring is best-effort: a global-store failure is logged, never
/// propagated. See `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`.
pub fn write_session_state(
    filestore: &FileStore,
    definition_id: &str,
    content: &[u8],
) -> Result<(), String> {
    let zone = validate_and_current(definition_id)?;
    write_zone_file(filestore, &zone, SNAPSHOT_FILE, content)?;
    if let Some(gfs) = global_transcript_store() {
        // The GLOBAL mirror must be AGENT-anchored, not channel-anchored.
        // `sourceBlockId` names the LOCAL block whose `output` a restore reads,
        // and that id only exists in the writing channel. A different channel
        // opening the agent would read from a block it doesn't have, and the read
        // fallback (`global_output_source`) — which resolves the agent zone via
        // that block's LOCAL meta — can't anchor it, so history renders empty.
        // Strip it to "" in the global copy so a cross-channel open anchors on its
        // own fresh local block (which maps to the agent). The per-channel copy
        // above keeps the real id for same-channel restore. See
        // docs/retro/retro-legacy-agent-history-cross-channel-2026-06-16.md.
        let global_content = normalize_snapshot_for_global(content);
        if let Err(e) = write_zone_file(gfs, &zone, SNAPSHOT_FILE, &global_content) {
            tracing::warn!(zone = %zone, error = %e, "global transcripts: snapshot mirror failed");
        }
    }
    Ok(())
}

/// Strip `sourceBlockId` to "" for the GLOBAL (cross-channel) snapshot mirror.
///
/// In a local snapshot `sourceBlockId` names the block whose per-block `output` a
/// restore reads. That id is channel-scoped, so a global copy carrying it is only
/// usable by the writing channel — every other channel's reader can't anchor it.
/// "" is the agent-anchored sentinel that makes the reader fall back to the opening
/// channel's own block. Best-effort: returns the input unchanged if it isn't a
/// JSON object.
pub fn normalize_snapshot_for_global(content: &[u8]) -> Vec<u8> {
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(content) else {
        return content.to_vec();
    };
    if let Some(obj) = v.as_object_mut() {
        if obj.contains_key("sourceBlockId") {
            obj.insert(
                "sourceBlockId".to_string(),
                serde_json::Value::String(String::new()),
            );
        }
    }
    let result = serde_json::to_vec(&v).unwrap_or_else(|_| content.to_vec());
    // Invariant G1: global snapshot must NEVER carry a non-empty sourceBlockId.
    // A non-empty id is channel-local and breaks cross-channel opens.
    debug_assert!(
        serde_json::from_slice::<serde_json::Value>(&result)
            .ok()
            .and_then(|v| v.get("sourceBlockId").and_then(|s| s.as_str()).map(|s| s.is_empty()))
            .unwrap_or(true),
        "normalize_snapshot_for_global: G1 violated — sourceBlockId was not stripped"
    );
    result
}

/// One-shot heal for global snapshots poisoned before the normalize-on-mirror fix
/// (a channel-local `sourceBlockId` was mirrored into `agent:<defId>:current`,
/// breaking cross-channel opens). For each `def_id`, rewrite its global snapshot's
/// `sourceBlockId` to "" iff it isn't already. Idempotent and cheap (one small
/// JSON per agent); returns the number healed. Best-effort per agent.
pub fn heal_global_snapshot_source_block_ids(gfs: &FileStore, def_ids: &[String]) -> usize {
    let mut healed = 0;
    for def_id in def_ids {
        if !is_valid_definition_id(def_id) {
            continue;
        }
        let zone = agent_current_zone(def_id);
        let bytes = match gfs.read_file(&zone, SNAPSHOT_FILE) {
            Ok(Some(b)) => b,
            _ => continue, // no global snapshot for this agent
        };
        // Only rewrite when currently poisoned (non-empty sourceBlockId).
        let poisoned = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| {
                v.get("sourceBlockId")
                    .and_then(|s| s.as_str())
                    .map(|s| !s.is_empty())
            })
            .unwrap_or(false);
        if !poisoned {
            continue;
        }
        let fixed = normalize_snapshot_for_global(&bytes);
        if write_zone_file(gfs, &zone, SNAPSHOT_FILE, &fixed).is_ok() {
            healed += 1;
            tracing::info!(zone = %zone, "global transcripts: healed poisoned snapshot sourceBlockId");
        }
    }
    healed
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
///
/// Reads the GLOBAL store first (preferred). The global copy always holds an
/// agent-anchored snapshot (`sourceBlockId = ""`), which works in any channel.
/// Per-channel is the fallback for old builds that pre-date the global store.
/// Symmetric with the `blockfile:read_range` fallback in `app_api.rs`.
/// See `docs/specs/SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16.md` invariant G2.
pub fn read_session_state(
    filestore: &FileStore,
    definition_id: &str,
) -> Result<(Option<String>, Option<i64>), String> {
    let zone = validate_and_current(definition_id)?;
    // Read the global store first: it always holds the agent-anchored
    // (sourceBlockId="") snapshot. The per-channel copy carries the
    // writing channel's local block id, which is stale the moment any
    // OTHER block opens the agent (same channel, different tab/build).
    // Preferring global ensures cross-channel opens never get a foreign
    // sourceBlockId — the root cause of the blank-pane regression.
    // Per-channel is the fallback for pre-global-store builds only.
    if let Some(gfs) = global_transcript_store() {
        if let Some(found) = read_snapshot_from(gfs, &zone)? {
            return Ok((Some(found.0), Some(found.1)));
        }
    }
    if let Some(found) = read_snapshot_from(filestore, &zone)? {
        return Ok((Some(found.0), Some(found.1)));
    }
    Ok((None, None))
}

/// Read `output.state.json` from `zone` in `store`. `Ok(None)` when absent.
pub(super) fn read_snapshot_from(store: &FileStore, zone: &str) -> Result<Option<(String, i64)>, String> {
    let stat = store
        .stat(zone, SNAPSHOT_FILE)
        .map_err(|e| format!("stat: {e}"))?;
    let Some(file) = stat else {
        return Ok(None);
    };
    let bytes = store
        .read_file(zone, SNAPSHOT_FILE)
        .map_err(|e| format!("read_file: {e}"))?
        .unwrap_or_default();
    Ok(Some((String::from_utf8_lossy(&bytes).into_owned(), file.modts)))
}
