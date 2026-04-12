// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session recovery after unclean shutdown (Phase 4.2 — ultra-long-sessions).
//!
//! When a persistent agent subprocess is running and the server is killed
//! (crash, OS reboot, task-kill), the subprocess dies with it but the block's
//! session history is preserved in FileStore. On next startup, we want to:
//!
//!   1. Detect that a session was running when the server died, so we can
//!      surface it to the user as "this session was interrupted".
//!   2. Let the user resume: the persistent controller already supports
//!      `--resume <session_id>` on the next input, so the recovery UX is
//!      simply "click resume, type your next message, get picked up mid-flight".
//!
//! The mechanism is a single boolean meta flag: `session:active_pid`. It's
//! set on subprocess spawn, cleared on clean exit (graceful or killed). If
//! this flag is still set when the server boots, the process is definitely
//! gone (old PID from a dead process), so we transfer it to
//! `session:was_interrupted = true`.
//!
//! `session:was_interrupted` is a frontend-only signal — the backend doesn't
//! consume it. The frontend `AgentControlBar` renders a banner when it's set,
//! and `service:update_object_meta` clears it when the user dismisses.

use std::sync::Arc;

use crate::backend::obj::{Block, MetaMapType};
use crate::backend::storage::wstore::WaveStore;

/// PID of the current running subprocess; 0 or missing = no process.
pub const META_SESSION_ACTIVE_PID: &str = "session:active_pid";
/// Set to `true` by startup scan when a pre-existing `active_pid` is found.
pub const META_SESSION_WAS_INTERRUPTED: &str = "session:was_interrupted";

/// Record that a subprocess with `pid` has been spawned for `block_id`.
/// Best-effort — logs on failure but never panics.
pub fn mark_active_pid(wstore: &Arc<WaveStore>, block_id: &str, pid: u32) {
    let mut meta = MetaMapType::new();
    meta.insert(META_SESSION_ACTIVE_PID.to_string(), serde_json::json!(pid));
    // Clear any stale interrupt flag — we're running again.
    meta.insert(META_SESSION_WAS_INTERRUPTED.to_string(), serde_json::Value::Null);
    let oref_str = format!("block:{}", block_id);
    if let Err(e) = crate::server::service::update_object_meta(wstore, &oref_str, &meta) {
        tracing::warn!(block_id = %block_id, error = %e, "session_recovery: failed to mark active pid");
    }
}

/// Clear `session:active_pid` — called when the subprocess exits for any reason.
pub fn clear_active_pid(wstore: &Arc<WaveStore>, block_id: &str) {
    let mut meta = MetaMapType::new();
    meta.insert(META_SESSION_ACTIVE_PID.to_string(), serde_json::Value::Null);
    let oref_str = format!("block:{}", block_id);
    if let Err(e) = crate::server::service::update_object_meta(wstore, &oref_str, &meta) {
        tracing::warn!(block_id = %block_id, error = %e, "session_recovery: failed to clear active pid");
    }
}

/// Scan all blocks at server startup. For any agent block that still has
/// `session:active_pid` set, transfer the flag to `session:was_interrupted`.
///
/// Returns the number of orphaned sessions found.
pub fn scan_orphans(wstore: &Arc<WaveStore>) -> u32 {
    let all_blocks = match wstore.get_all::<Block>() {
        Ok(blocks) => blocks,
        Err(e) => {
            tracing::warn!(error = %e, "session_recovery: scan_orphans get_all failed");
            return 0;
        }
    };

    let mut count = 0u32;
    for block in &all_blocks {
        // Only agent panes
        let view = block.meta.get("view").and_then(|v| v.as_str()).unwrap_or("");
        if view != "agent" {
            continue;
        }

        // Check for a stale active_pid
        let pid = block
            .meta
            .get(META_SESSION_ACTIVE_PID)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if pid == 0 {
            continue;
        }

        // Transfer: clear active_pid, set was_interrupted
        let mut update = MetaMapType::new();
        update.insert(META_SESSION_ACTIVE_PID.to_string(), serde_json::Value::Null);
        update.insert(
            META_SESSION_WAS_INTERRUPTED.to_string(),
            serde_json::json!(true),
        );
        let oref_str = format!("block:{}", block.oid);
        match crate::server::service::update_object_meta(wstore, &oref_str, &update) {
            Ok(()) => {
                count += 1;
                tracing::info!(
                    block_id = %block.oid,
                    stale_pid = pid,
                    "session_recovery: marked orphaned session as interrupted"
                );
            }
            Err(e) => {
                tracing::warn!(
                    block_id = %block.oid,
                    error = %e,
                    "session_recovery: failed to mark orphan"
                );
            }
        }
    }

    if count > 0 {
        tracing::info!(count = count, "session_recovery: scan_orphans complete");
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::Block;

    /// Verify that scan_orphans transfers active_pid → was_interrupted on
    /// agent blocks, and leaves non-agent blocks + blocks without active_pid
    /// untouched.
    #[test]
    fn test_scan_orphans_transfers_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let wstore = Arc::new(WaveStore::open(&tmp.path().join("wave.db")).unwrap());

        // ORef parser requires valid UUIDs, so generate them inline.
        let orphan_id = "11111111-1111-1111-1111-111111111111";
        let clean_id = "22222222-2222-2222-2222-222222222222";
        let term_id = "33333333-3333-3333-3333-333333333333";

        // Agent block with stale active_pid — should be flagged.
        let mut orphan = Block {
            oid: orphan_id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m.insert(META_SESSION_ACTIVE_PID.to_string(), serde_json::json!(12345u32));
                m
            },
            subblockids: None,
        };
        wstore.insert(&mut orphan).unwrap();

        // Agent block without active_pid — should be left alone.
        let mut clean = Block {
            oid: clean_id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m
            },
            subblockids: None,
        };
        wstore.insert(&mut clean).unwrap();

        // Non-agent block with active_pid (shouldn't exist in practice, but
        // sanity check the view filter) — should be left alone.
        let mut nonagent = Block {
            oid: term_id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("term"));
                m.insert(META_SESSION_ACTIVE_PID.to_string(), serde_json::json!(67890u32));
                m
            },
            subblockids: None,
        };
        wstore.insert(&mut nonagent).unwrap();

        let count = scan_orphans(&wstore);
        assert_eq!(count, 1, "exactly one agent orphan should be flagged");

        // Verify orphan block meta
        let after: Block = wstore.get(orphan_id).unwrap().unwrap();
        assert!(
            after.meta.get(META_SESSION_ACTIVE_PID).and_then(|v| v.as_u64()).unwrap_or(0) == 0,
            "active_pid should be cleared"
        );
        assert_eq!(
            after.meta.get(META_SESSION_WAS_INTERRUPTED).and_then(|v| v.as_bool()),
            Some(true),
            "was_interrupted should be true"
        );

        // Clean block untouched
        let clean_after: Block = wstore.get(clean_id).unwrap().unwrap();
        assert!(clean_after.meta.get(META_SESSION_WAS_INTERRUPTED).is_none());

        // Non-agent block untouched (still has active_pid)
        let term_after: Block = wstore.get(term_id).unwrap().unwrap();
        assert_eq!(
            term_after.meta.get(META_SESSION_ACTIVE_PID).and_then(|v| v.as_u64()),
            Some(67890),
        );
    }
}
