// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-memory, latest-per-block cache of Activity Dock `ToolNode` status
//! deltas, pushed from the frontend as they happen. Backs `muxspect dock`
//! (read) and `muxspect dock clear` (write).
//!
//! See `docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md`
//! §3.1. Never persisted — this mirrors the dock's own nature: if no
//! renderer is currently attached to a block, there is nothing live to
//! report, and losing this cache on restart is correct, not a bug.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde::Serialize;

/// One `ToolNode`'s last-known status, as reported by the renderer that
/// owns it. `observed_at` is stamped on receipt by the server (not trusted
/// from the client) so `muxspect dock`'s age computation is consistent
/// even if the renderer's clock is skewed.
#[derive(Debug, Clone, Serialize)]
pub struct DockNodeSnapshot {
    pub node_id: String,
    pub tool_name: String,
    pub status: String,
    /// `ToolNode.timestamp` (ms) — when the tool call was initiated,
    /// per the renderer. `None` if the pushing client didn't have one.
    pub timestamp: Option<i64>,
    /// Server-side receipt time (ms) — used for age, not `timestamp`,
    /// so a slow/backed-up push doesn't understate how stale the
    /// snapshot itself is.
    pub observed_at: i64,
}

/// One block's tracked nodes, keyed by node id so a later delta for the
/// same node overwrites rather than accumulates.
type BlockNodes = HashMap<String, DockNodeSnapshot>;

#[derive(Default)]
pub struct DockSnapshotCache {
    inner: Mutex<HashMap<String, BlockNodes>>,
}

impl DockSnapshotCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or overwrite) one node's latest status for `block_id`.
    pub fn push_delta(&self, block_id: &str, node: DockNodeSnapshot) {
        let mut guard = self.inner.lock();
        guard.entry(block_id.to_string()).or_default().insert(node.node_id.clone(), node);
    }

    /// All tracked nodes for `block_id`, or empty if none/unknown block.
    pub fn get(&self, block_id: &str) -> Vec<DockNodeSnapshot> {
        let guard = self.inner.lock();
        guard.get(block_id).map(|m| m.values().cloned().collect()).unwrap_or_default()
    }

    /// Whether `node_id` is currently tracked for `block_id` — used to
    /// validate a `dock clear` request before publishing the clear event,
    /// so muxspect can distinguish "cleared" from "no such node."
    pub fn has_node(&self, block_id: &str, node_id: &str) -> bool {
        let guard = self.inner.lock();
        guard.get(block_id).is_some_and(|m| m.contains_key(node_id))
    }

    /// Drop a node from the cache — called once a `dock clear` has been
    /// published, so a subsequent `dock` read doesn't keep showing a node
    /// that was just cleared (the clearing renderer flips it to
    /// `"canceled"` locally, but this cache has no way to observe that
    /// without another push; removing it here is simpler and correct
    /// either way — a canceled node isn't "stuck" and doesn't need to
    /// keep appearing in diagnosis output).
    pub fn remove_node(&self, block_id: &str, node_id: &str) {
        let mut guard = self.inner.lock();
        if let Some(nodes) = guard.get_mut(block_id) {
            nodes.remove(node_id);
        }
    }

    /// Drop every tracked node for `block_id` — for wiring into block
    /// deletion, so the cache doesn't accumulate entries for blocks that
    /// no longer exist (open question §6.3 of the spec). Not wired to a
    /// call site yet — deleting a block is a small footprint (a handful
    /// of node entries) relative to the cache's overall size, so this is
    /// a nice-to-have bound, not a correctness requirement; wiring it up
    /// is a follow-up once the block-deletion path is touched for other
    /// reasons, same as `ScrubOrphanedInProgress`'s own standalone command
    /// existing with zero live dispatch call sites today.
    #[allow(dead_code)]
    pub fn evict_block(&self, block_id: &str) {
        self.inner.lock().remove(block_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, status: &str) -> DockNodeSnapshot {
        DockNodeSnapshot {
            node_id: id.to_string(),
            tool_name: "Bash".to_string(),
            status: status.to_string(),
            timestamp: Some(1000),
            observed_at: 2000,
        }
    }

    #[test]
    fn get_on_unknown_block_is_empty() {
        let cache = DockSnapshotCache::new();
        assert!(cache.get("no-such-block").is_empty());
    }

    #[test]
    fn push_then_get_round_trips() {
        let cache = DockSnapshotCache::new();
        cache.push_delta("block-1", node("n1", "running"));
        let got = cache.get("block-1");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].node_id, "n1");
        assert_eq!(got[0].status, "running");
    }

    #[test]
    fn later_delta_for_same_node_overwrites() {
        let cache = DockSnapshotCache::new();
        cache.push_delta("block-1", node("n1", "running"));
        cache.push_delta("block-1", node("n1", "success"));
        let got = cache.get("block-1");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].status, "success");
    }

    #[test]
    fn different_nodes_coexist() {
        let cache = DockSnapshotCache::new();
        cache.push_delta("block-1", node("n1", "running"));
        cache.push_delta("block-1", node("n2", "success"));
        assert_eq!(cache.get("block-1").len(), 2);
    }

    #[test]
    fn has_node_reflects_current_state() {
        let cache = DockSnapshotCache::new();
        assert!(!cache.has_node("block-1", "n1"));
        cache.push_delta("block-1", node("n1", "running"));
        assert!(cache.has_node("block-1", "n1"));
        assert!(!cache.has_node("block-1", "n2"));
        assert!(!cache.has_node("block-2", "n1"));
    }

    #[test]
    fn remove_node_drops_only_that_node() {
        let cache = DockSnapshotCache::new();
        cache.push_delta("block-1", node("n1", "running"));
        cache.push_delta("block-1", node("n2", "running"));
        cache.remove_node("block-1", "n1");
        assert!(!cache.has_node("block-1", "n1"));
        assert!(cache.has_node("block-1", "n2"));
    }

    #[test]
    fn remove_node_on_unknown_block_is_a_no_op() {
        let cache = DockSnapshotCache::new();
        cache.remove_node("no-such-block", "n1"); // must not panic
    }

    #[test]
    fn evict_block_drops_everything_for_that_block_only() {
        let cache = DockSnapshotCache::new();
        cache.push_delta("block-1", node("n1", "running"));
        cache.push_delta("block-2", node("n1", "running"));
        cache.evict_block("block-1");
        assert!(cache.get("block-1").is_empty());
        assert_eq!(cache.get("block-2").len(), 1);
    }
}
