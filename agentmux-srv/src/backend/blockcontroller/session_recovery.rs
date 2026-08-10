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
//!
//! A second, unrelated flag lives here for the same reason: `session:resume_failed`
//! (SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.2) marks a
//! block whose `--resume <sid>` was rejected by the CLI (stale/unreachable
//! session id — see `persistent.rs`'s `poison_resume`) and silently fell
//! through to a fresh conversation. Same shape, same frontend-only contract,
//! different trigger (a resume rejection mid-session, not a stale PID found
//! at boot) — kept in this file rather than a new module since it's the
//! established home for "frontend-only session-state signal flags."

use std::sync::Arc;

use crate::backend::eventbus::EventBus;
use crate::backend::obj::{Block, MetaMapType};
use crate::backend::storage::store::Store;

/// PID of the current running subprocess; 0 or missing = no process.
pub const META_SESSION_ACTIVE_PID: &str = "session:active_pid";
/// Set to `true` by startup scan when a pre-existing `active_pid` is found.
pub const META_SESSION_WAS_INTERRUPTED: &str = "session:was_interrupted";
/// Set to `true` when a `--resume <sid>` was rejected by the CLI and the
/// controller silently started a fresh conversation instead.
pub const META_SESSION_RESUME_FAILED: &str = "session:resume_failed";

/// Record that a subprocess with `pid` has been spawned for `block_id`.
/// Best-effort — logs on failure but never panics.
pub fn mark_active_pid(wstore: &Arc<Store>, block_id: &str, pid: u32) {
    let mut meta = MetaMapType::new();
    meta.insert(META_SESSION_ACTIVE_PID.to_string(), serde_json::json!(pid));
    // Clear any stale interrupt flag — we're running again.
    meta.insert(META_SESSION_WAS_INTERRUPTED.to_string(), serde_json::Value::Null);
    let oref_str = format!("block:{}", block_id);
    if let Err(e) = crate::server::service::update_object_meta(wstore, &oref_str, &meta) {
        tracing::warn!(block_id = %block_id, error = %e, "session_recovery: failed to mark active pid");
    }
}

/// Mark that `block_id`'s `--resume <sid>` was rejected and the controller
/// fell through to a fresh conversation. Best-effort — logs on failure but
/// never panics, matching `mark_active_pid`'s contract. Called from
/// `persistent.rs`'s stderr reader the moment it detects the CLI's "No
/// conversation found with session ID" line, right alongside the existing
/// `core::persist_session_id(block_id, "", ...)` clear.
///
/// Broadcasts `waveobj:update` on success (reagent P1 on the initial PR):
/// this fires while the user may be actively watching the pane that just
/// lost its resume, so — unlike `mark_active_pid`/`scan_orphans`, both of
/// which run before any frontend subscriber could be watching (spawn time /
/// server boot) — the write must reach an already-open `blockAtom` live, not
/// only on the pane's next reload/reopen. Mirrors `persist_session_id`'s
/// broadcast in `core.rs`. `event_bus: None` (tests / no live subscribers)
/// silently skips the broadcast, matching every other call site's contract.
pub fn mark_resume_failed(wstore: &Arc<Store>, event_bus: &Option<Arc<EventBus>>, block_id: &str) {
    let mut meta = MetaMapType::new();
    meta.insert(META_SESSION_RESUME_FAILED.to_string(), serde_json::json!(true));
    let oref_str = format!("block:{}", block_id);
    if let Err(e) = crate::server::service::update_object_meta(wstore, &oref_str, &meta) {
        tracing::warn!(block_id = %block_id, error = %e, "session_recovery: failed to mark resume_failed");
        return;
    }
    let Some(ref bus) = event_bus else {
        return;
    };
    if let Ok(updated_block) = wstore.must_get::<Block>(block_id) {
        let update_data = serde_json::to_value(&crate::backend::obj::WaveObjUpdate {
            updatetype: "update".into(),
            otype: "block".into(),
            oid: block_id.to_string(),
            obj: Some(crate::backend::obj::wave_obj_to_value(&updated_block)),
        })
        .ok();
        bus.broadcast_event(&crate::backend::eventbus::WSEventType {
            eventtype: "waveobj:update".to_string(),
            oref: oref_str,
            data: update_data,
        });
    }
}

/// Clear `session:active_pid` — called when the subprocess exits for any reason.
pub fn clear_active_pid(wstore: &Arc<Store>, block_id: &str) {
    let mut meta = MetaMapType::new();
    meta.insert(META_SESSION_ACTIVE_PID.to_string(), serde_json::Value::Null);
    let oref_str = format!("block:{}", block_id);
    if let Err(e) = crate::server::service::update_object_meta(wstore, &oref_str, &meta) {
        tracing::warn!(block_id = %block_id, error = %e, "session_recovery: failed to clear active pid");
    }
}

/// Clear `session:active_pid` **only if it still equals `expected_pid`** —
/// a transactional compare-and-clear for exit-handlers that know which
/// process instance they are cleaning up after.
///
/// Issue #2363: `PersistentSubprocessController`'s fallback respawn
/// (`respawn_once_for_leftover_queue`) runs on a separate tokio task from
/// the dying process's own exit-handler. The exit-handler's cleanup is
/// gated on a spawn-generation check, but that gate is read once under the
/// controller's lock while the fallback registers its own fresh PID in
/// parallel — the unconditional [`clear_active_pid`] could then wipe the
/// NEW process's registration, leaving it untracked for crash recovery
/// until the next natural re-registration. The whole read-compare-clear
/// here runs inside one `Store::with_tx` (a single connection lock across
/// read+merge+write — see `object_helpers::update_object_meta`'s doc
/// comment for the transactional contract), so a concurrent
/// [`mark_active_pid`] strictly serializes against it: whichever commits
/// second sees the other's completed write, and a mismatch means "a newer
/// process already registered — leave it alone."
///
/// Best-effort like the unconditional variant: logs on failure, never
/// panics. Returns nothing — callers do not branch on the outcome.
pub fn clear_active_pid_if_pid(wstore: &Arc<Store>, block_id: &str, expected_pid: u32) {
    let block_id_owned = block_id.to_string();
    let result = wstore.with_tx(|tx| {
        let mut block = tx.must_get::<Block>(&block_id_owned)?;
        let current = block
            .meta
            .get(META_SESSION_ACTIVE_PID)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if current != expected_pid as u64 {
            // A newer process re-registered (or the pid was already
            // cleared) — not ours to clear.
            return Ok(false);
        }
        block.meta.remove(META_SESSION_ACTIVE_PID);
        tx.update(&mut block)?;
        Ok(true)
    });
    match result {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(
                block_id = %block_id,
                expected_pid = expected_pid,
                "session_recovery: active pid changed since this process registered — skipping clear"
            );
        }
        Err(e) => {
            tracing::warn!(block_id = %block_id, error = %e, "session_recovery: compare-and-clear active pid failed");
        }
    }
}

/// Scan all blocks at server startup. For any agent block that still has
/// `session:active_pid` set, transfer the flag to `session:was_interrupted`.
///
/// Returns the number of orphaned sessions found.
pub fn scan_orphans(wstore: &Arc<Store>) -> u32 {
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
        let wstore = Arc::new(Store::open(&tmp.path().join("objects.db")).unwrap());

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

    #[test]
    fn test_mark_resume_failed_sets_the_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let wstore = Arc::new(Store::open(&tmp.path().join("objects.db")).unwrap());

        let block_id = "44444444-4444-4444-4444-444444444444";
        let mut block = Block {
            oid: block_id.to_string(),
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
        wstore.insert(&mut block).unwrap();

        mark_resume_failed(&wstore, &None, block_id);

        let after: Block = wstore.get(block_id).unwrap().unwrap();
        assert_eq!(
            after.meta.get(META_SESSION_RESUME_FAILED).and_then(|v| v.as_bool()),
            Some(true),
        );
    }

    /// reagent P1 on the original PR: mark_resume_failed wrote the meta but
    /// never broadcast waveobj:update, so an already-open pane's blockAtom
    /// never saw the flag live — the disclosure banner wouldn't render until
    /// the pane was reloaded, defeating the point of a live-disclosure
    /// signal. This asserts the broadcast actually reaches a connected
    /// WebSocket client, not just that the DB write succeeded.
    #[tokio::test]
    async fn test_mark_resume_failed_broadcasts_waveobj_update() {
        let tmp = tempfile::tempdir().unwrap();
        let wstore = Arc::new(Store::open(&tmp.path().join("objects.db")).unwrap());
        let event_bus = Arc::new(EventBus::new());

        let block_id = "77777777-7777-7777-7777-777777777777";
        let mut block = Block {
            oid: block_id.to_string(),
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
        wstore.insert(&mut block).unwrap();

        let mut receivers = event_bus.register_ws("test-conn", "test-tab");

        mark_resume_failed(&wstore, &Some(event_bus.clone()), block_id);

        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), receivers.priority.recv())
            .await
            .expect("timed out waiting for waveobj:update broadcast")
            .expect("priority channel closed");
        assert_eq!(msg.get("eventtype").and_then(|v| v.as_str()), Some("waveobj:update"));
        assert_eq!(msg.get("oref").and_then(|v| v.as_str()), Some(format!("block:{}", block_id).as_str()));
        let obj = msg.get("data").and_then(|d| d.get("obj")).expect("data.obj present");
        assert_eq!(
            obj.get("meta")
                .and_then(|m| m.get(META_SESSION_RESUME_FAILED))
                .and_then(|v| v.as_bool()),
            Some(true),
            "broadcast payload must carry the flag, not just trigger a generic refetch",
        );
    }
}
