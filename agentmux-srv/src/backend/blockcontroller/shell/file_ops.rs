// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Block-file append/persist helpers and global-transcript mirroring.

use std::sync::Arc;

use base64::Engine as _;

use crate::backend::storage::filestore::FileStore;
use crate::backend::wps;

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
        let needs_create = match fs.stat(block_id, filename) {
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
        if let Err(e) = fs.append_data(block_id, filename, data) {
            tracing::warn!(
                block_id = %block_id, filename = %filename, error = %e,
                "persist_silent: append_data failed"
            );
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
pub fn handle_append_block_file(
    broker: &wps::Broker,
    block_id: &str,
    filename: &str,
    data: &[u8],
    filestore: Option<&Arc<FileStore>>,
    global_output_zone: Option<&str>,
) {
    let data64 = base64::engine::general_purpose::STANDARD.encode(data);

    let event_data = wps::WSFileEventData {
        zoneid: block_id.to_string(),
        filename: filename.to_string(),
        fileop: wps::FILE_OP_APPEND.to_string(),
        data64,
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
        let needs_create = match fs.stat(block_id, filename) {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id,
                    filename = %filename,
                    error = %e,
                    "filestore stat failed; skipping write-through"
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

        if let Err(e) = fs.append_data(block_id, filename, data) {
            tracing::warn!(
                block_id = %block_id,
                filename = %filename,
                error = %e,
                "filestore append_data failed"
            );
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
    if let Err(e) = gfs.append_data(zone, OUTPUT_FILE, data) {
        tracing::warn!(zone = %zone, error = %e, "global transcripts: append_data failed");
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
