// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cross-channel transcript singleton + block-meta zone resolution.

use std::sync::Arc;

use crate::backend::storage::filestore::FileStore;

use super::zone_naming::{agent_current_zone, is_valid_definition_id};

/// Process-global handle to the GLOBAL transcript FileStore (the one rooted at
/// `<shared>/agents/transcripts`, opened once in `main.rs`). Backs the
/// `agent:<defId>:current` zone so a conversation loads when the agent is
/// opened from *any* build/channel — finishing the cross-channel arc
/// (#1387–#1396). `None` until `set_global_transcript_store` runs (or never, in
/// unit tests / when the shared root can't be resolved), in which case the
/// hot-path mirror is a no-op and reads fall back to the per-channel store.
///
/// It's a process-global rather than a threaded parameter because the store is
/// genuinely process-wide (one per srv instance) and the alternative would be
/// plumbing an `Option<Arc<FileStore>>` through `resync_controller` and every
/// block-controller constructor purely to reach the stdout-reader hot path.
static GLOBAL_TRANSCRIPT_STORE: std::sync::OnceLock<Arc<FileStore>> = std::sync::OnceLock::new();

/// Install the global transcript store. Called once from `main.rs` startup.
/// Idempotent — a second call is ignored (the first store wins).
pub fn set_global_transcript_store(store: Arc<FileStore>) {
    let _ = GLOBAL_TRANSCRIPT_STORE.set(store);
}

/// Borrow the global transcript store, if installed.
pub fn global_transcript_store() -> Option<&'static Arc<FileStore>> {
    GLOBAL_TRANSCRIPT_STORE.get()
}

/// Resolve the agent's GLOBAL `agent:<defId>:current` zone from a block's meta.
///
/// The block's `agentId` meta IS the agent `definition_id` (the same value the
/// snapshot RPCs and `blockfile:read_range` fallback key on — see
/// `app_api.rs`), so the zone the hot-path mirror *writes* and the zone the
/// read fallback *reads* are identical by construction. Returns `None` when the
/// block isn't agent-anchored or carries an invalid id (no mirror/fallback).
pub fn agent_zone_for_block_meta(meta: &crate::backend::obj::MetaMapType) -> Option<String> {
    let def_id = crate::backend::obj::meta_get_string(meta, "agentId", "");
    if is_valid_definition_id(&def_id) {
        Some(agent_current_zone(&def_id))
    } else {
        None
    }
}
