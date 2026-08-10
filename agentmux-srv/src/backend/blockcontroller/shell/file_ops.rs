// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Block-file append/persist helpers and global-transcript mirroring.

use std::sync::Arc;

use base64::Engine as _;

use crate::backend::storage::filestore::FileStore;
use crate::backend::wps;

/// Current unix time in ms, or 0 if the clock is before the epoch (never in
/// practice; 0 reads as "unknown" on the consumer side, same as no stamp).
fn unix_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Append one receive-time record to a zone's `output.tsidx` sidecar:
/// NDJSON `{"off":<byte offset the batch starts at in output>,"ms":<unix ms>}`.
/// Batch granularity is deliberate — one flush of one turn's output; sub-batch
/// precision has no UI meaning
/// (SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §4.4).
/// Fire-and-forget like every other write on this path: any failure logs at
/// debug and the transcript append is unaffected. Only called for agent
/// transcript streams (never PTY `term` data).
fn append_tsidx_entry(fs: &Arc<FileStore>, zone: &str, batch_start: u64, now_ms: i64) {
    use crate::backend::agent_session::TSIDX_FILE;
    use crate::backend::storage::error::StoreError;

    if let Ok(None) = fs.stat(zone, TSIDX_FILE) {
        if let Err(e) = fs.make_file(
            zone,
            TSIDX_FILE,
            std::collections::HashMap::new(),
            crate::backend::storage::filestore::FileOpts::default(),
        ) {
            if !matches!(e, StoreError::AlreadyExists) {
                tracing::debug!(zone = %zone, error = %e, "tsidx: make_file failed; skipping stamp");
                return;
            }
        }
    }
    let line = format!("{{\"off\":{batch_start},\"ms\":{now_ms}}}\n");
    if let Err(e) = fs.append_data(zone, TSIDX_FILE, line.as_bytes()) {
        tracing::debug!(zone = %zone, error = %e, "tsidx: append failed");
    }
}

/// Persist `data` to the block's output file and the global transcript zone
/// **without** publishing a WPS event. Used by the persistent controller to
/// record user-message lines for future history loads (so that
/// `parseHistoryLines` can reconstruct `user_message` nodes on reopen) without
/// triggering a live-stream append that would produce a duplicate node alongside
/// the `agent-message-accepted` UUID node. Same lazy-create semantics as
/// `handle_append_block_file`.
pub fn persist_to_blockfile_silent(
    block_id: &str,
    filename: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global_output_zone: Option<&str>,
) {
    if let Some(fs) = filestore {
        let needs_create: bool = match fs.stat(block_id, filename) {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id, filename = %filename, error = %e,
                    "persist_silent: stat failed; skipping"
                );
                return;
            }
        };
        if needs_create {
            if let Err(e) = fs.make_file(
                block_id,
                filename,
                std::collections::HashMap::new(),
                crate::backend::storage::filestore::FileOpts::default(),
            ) {
                use crate::backend::storage::error::StoreError;
                if !matches!(e, StoreError::AlreadyExists) {
                    tracing::warn!(
                        block_id = %block_id, filename = %filename, error = %e,
                        "persist_silent: make_file failed; skipping"
                    );
                    return;
                }
            }
        }
        // append_data_at returns the offset the batch ACTUALLY landed at,
        // read under the store's own lock - a caller-side pre-append stat
        // can be stale under concurrent appenders (codex P2 on PR #2508).
        match fs.append_data_at(block_id, filename, data) {
            Ok(actual_start) => {
                // Only agent transcript streams carry a global zone; those are
                // the streams the tsidx sidecar exists for (§4.4).
                if global_output_zone.is_some() {
                    append_tsidx_entry(fs, block_id, actual_start.max(0) as u64, unix_ms_now());
                }
            }
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id, filename = %filename, error = %e,
                    "persist_silent: append_data failed"
                );
            }
        }
    }
    if let Some(zone) = global_output_zone {
        if let Some(gfs) = crate::backend::agent_session::global_transcript_store() {
            mirror_append_to_global(gfs, zone, data);
        }
    }
}

/// Append data to a block's terminal output file, publish a WPS event,
/// and write-through to FileStore (if provided).
///
/// Port of Go's `HandleAppendBlockFile`.
///
/// The FileStore write is fire-and-forget: if it fails we emit a warning but
/// never propagate the error back to the hot stdout-reader path.
/// Outcome of the up-front `stat()` this function needs regardless (to know
/// whether to lazily `make_file` before appending) — reused to compute the
/// chunk's start offset for the broadcast event too, rather than stat-ing
/// twice. `Error` preserves the original behavior of skipping write-through
/// entirely (and omitting `offset` from the broadcast) on a stat failure.
enum ExistingFileStat {
    NeedsCreate,
    Exists { size: u64 },
    Error,
}

pub fn handle_append_block_file(
    broker: &wps::Broker,
    block_id: &str,
    filename: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global_output_zone: Option<&str>,
) {
    let data64 = base64::engine::general_purpose::STANDARD.encode(data);

    // Stat once, up front — needed both to decide whether make_file() is
    // required below AND to compute this chunk's start offset (the file's
    // size immediately before this append) for the broadcast event, so a
    // client reconnecting mid-stream can reconcile a chunk arriving during
    // its own reconnect fetch against what that fetch already covers.
    // Before the write-through fix (§2.1 of the spec below), "term" reads
    // always 404'd, so this race was latent — a reconnecting TermWrap had
    // nothing from the fetch to double-count against. Now that reads
    // return real content, a chunk landing in the subscribe-then-fetch-
    // then-flush-held-data window could be written twice (once from the
    // fetch's response, once from the live/held replay), corrupting
    // ptyOffset for future reconnects.
    // See SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1 follow-up.
    let existing_stat = match filestore {
        None => None,
        Some(fs) => Some(match fs.stat(block_id, filename) {
            Ok(None) => ExistingFileStat::NeedsCreate,
            Ok(Some(info)) => ExistingFileStat::Exists { size: info.size as u64 },
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id,
                    filename = %filename,
                    error = %e,
                    "filestore stat failed; skipping write-through"
                );
                ExistingFileStat::Error
            }
        }),
    };
    let start_offset = match &existing_stat {
        None | Some(ExistingFileStat::Error) => None,
        Some(ExistingFileStat::NeedsCreate) => Some(0),
        Some(ExistingFileStat::Exists { size }) => Some(*size),
    };

    let event_data = wps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: filename.to_string(),
        fileop: wps::FILE_OP_APPEND.to_string(),
        data64,
        offset: start_offset,
    };

    let event = wps::WaveEvent {
        event: wps::EVENT_BLOCK_FILE.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 0,
        data: serde_json::to_value(&event_data).ok(),
    };

    broker.publish(event);

    // Write-through to FileStore for persistent history (Phase 1.3).
    // Create the file lazily on first append; if the file already exists
    // we skip make_file and go straight to append_data.
    if let Some(fs) = filestore {
        let needs_create = match existing_stat {
            Some(ExistingFileStat::NeedsCreate) => true,
            Some(ExistingFileStat::Exists { .. }) => false,
            // Error already warned above; preserve the original
            // skip-write-through-entirely behavior.
            Some(ExistingFileStat::Error) | None => return,
        };

        if needs_create {
            if let Err(e) = fs.make_file(
                block_id,
                filename,
                std::collections::HashMap::new(),
                crate::backend::storage::filestore::FileOpts::default(),
            ) {
                // AlreadyExists is benign (race between two appends); anything
                // else is worth warning about.
                use crate::backend::storage::error::StoreError;
                if !matches!(e, StoreError::AlreadyExists) {
                    tracing::warn!(
                        block_id = %block_id,
                        filename = %filename,
                        error = %e,
                        "filestore make_file failed; skipping write-through"
                    );
                    return;
                }
            }
        }

        match fs.append_data_at(block_id, filename, data) {
            Ok(actual_start) => {
                // Receive-time stamp for this batch (§4.4). Gated on
                // `global_output_zone` — only agent transcript appends carry
                // one; PTY `term` data and non-agent blocks never get a
                // sidecar. Stamped at the offset the append itself reports
                // (read under the store lock), not the pre-append stat —
                // codex P2 on PR #2508: the stat can go stale when writers
                // interleave, and a mis-keyed stamp gives replayed lines
                // another batch's time. (The broadcast event's `offset`
                // still uses the pre-append stat — pre-existing contract,
                // reconcile-only consumer, unchanged here.)
                if global_output_zone.is_some() {
                    append_tsidx_entry(fs, block_id, actual_start.max(0) as u64, unix_ms_now());
                }
            }
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id,
                    filename = %filename,
                    error = %e,
                    "filestore append_data failed"
                );
            }
        }
        // Note: output.idx is NOT updated incrementally here. It is a lazily-built,
        // self-validating cache rebuilt by the read path whenever output grows (see
        // `rebuild_output_idx` below, invoked from the blockfile:read_range handler
        // in app_api.rs). This avoids every incremental-index failure mode (desync on
        // write failure, chunk-split lines, blank-line miscounting) at the cost of one
        // rescan per output-size change.
    }

    // Mirror the agent's `output` stream into the GLOBAL transcript zone
    // (`agent:<defId>:current`) so the conversation loads when this agent is
    // opened from another build/channel. Fire-and-forget, exactly like the
    // per-channel write above: a global-store hiccup must never stall the live
    // pane. Callers pass `Some(zone)` only for the agent output stream, so we
    // never mirror PTY `term` data or non-agent blocks.
    if let Some(zone) = global_output_zone {
        if let Some(gfs) = crate::backend::agent_session::global_transcript_store() {
            mirror_append_to_global(gfs, zone, data);
        }
    }
}

/// Append `data` to the global transcript zone's `output` file, creating it
/// lazily on first write. Mirrors the per-channel write-through in
/// [`handle_append_block_file`]; all errors are logged and swallowed so the
/// hot stdout path is never blocked by the global store.
pub(super) fn mirror_append_to_global(gfs: &Arc<FileStore>, zone: &str, data: &[u8]) {
    use crate::backend::agent_session::OUTPUT_FILE;
    use crate::backend::storage::error::StoreError;

    match gfs.stat(zone, OUTPUT_FILE) {
        Ok(None) => {
            if let Err(e) = gfs.make_file(
                zone,
                OUTPUT_FILE,
                std::collections::HashMap::new(),
                crate::backend::storage::filestore::FileOpts::default(),
            ) {
                if !matches!(e, StoreError::AlreadyExists) {
                    tracing::warn!(zone = %zone, error = %e, "global transcripts: make_file failed; skipping mirror");
                    return;
                }
            }
        }
        Ok(Some(_)) => {}
        Err(e) => {
            tracing::warn!(zone = %zone, error = %e, "global transcripts: stat failed; skipping mirror");
            return;
        }
    }
    match gfs.append_data_at(zone, OUTPUT_FILE, data) {
        Ok(actual_start) => {
            // Global zones are agent transcripts by construction — always
            // stamp (§4.4), keyed at the offset this append actually
            // landed at (codex P2 on PR #2508 — cross-channel mirrors from
            // concurrent srv instances can interleave; the store-lock-read
            // offset is exact where a pre-append stat is not).
            append_tsidx_entry(gfs, zone, actual_start.max(0) as u64, unix_ms_now());
        }
        Err(e) => {
            tracing::warn!(zone = %zone, error = %e, "global transcripts: append_data failed");
        }
    }
}

/// Resolve a block's GLOBAL transcript zone (`agent:<defId>:current`) from its
/// `agentId` meta, looking the block up in `wstore`. Returns `None` for
/// non-agent blocks, when there's no store, or when the block can't be loaded —
/// the caller then passes `None` and no global mirror happens. Shared by the
/// subprocess / persistent / acp agent controllers.
pub(crate) fn resolve_global_output_zone(
    wstore: &Option<Arc<crate::backend::storage::store::Store>>,
    block_id: &str,
) -> Option<String> {
    let store = wstore.as_ref()?;
    let block = store
        .must_get::<crate::backend::obj::Block>(block_id)
        .ok()?;
    crate::backend::agent_session::agent_zone_for_block_meta(&block.meta)
}

/// Truncate a block's terminal output file and publish a WPS event.
/// Port of Go's `HandleTruncateBlockFile`.
#[allow(dead_code)]
pub fn handle_truncate_block_file(broker: &wps::Broker, block_id: &str, filename: &str) {
    let event_data = wps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: filename.to_string(),
        fileop: wps::FILE_OP_TRUNCATE.to_string(),
        data64: String::new(),
        offset: None,
    };

    let event = wps::WaveEvent {
        event: wps::EVENT_BLOCK_FILE.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 0,
        data: serde_json::to_value(&event_data).ok(),
    };

    broker.publish(event);
}
