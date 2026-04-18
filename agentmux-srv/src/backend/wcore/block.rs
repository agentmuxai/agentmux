#![allow(dead_code)]
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Block CRUD operations.

use uuid::Uuid;

use crate::backend::storage::wstore::WaveStore;
use crate::backend::storage::StoreError;
use crate::backend::obj::*;

/// Create a new block in a tab.
pub fn create_block(
    store: &WaveStore,
    tab_id: &str,
    meta: MetaMapType,
) -> Result<Block, StoreError> {
    let mut tab = store.must_get::<Tab>(tab_id)?;

    let mut block = Block {
        oid: Uuid::new_v4().to_string(),
        parentoref: format!("tab:{}", tab_id),
        meta,
        ..Default::default()
    };
    store.insert(&mut block)?;

    tab.blockids.push(block.oid.clone());
    store.update(&mut tab)?;

    Ok(block)
}

/// Delete a block from its parent tab and prune it from the layout tree.
pub fn delete_block(
    store: &WaveStore,
    tab_id: &str,
    block_id: &str,
) -> Result<(), StoreError> {
    let mut tab = store.must_get::<Tab>(tab_id)?;
    tab.blockids.retain(|id| id != block_id);
    store.update(&mut tab)?;
    store.delete::<Block>(block_id)?;

    // Prune the deleted block's node from the layout tree so it doesn't
    // leave a blank pane. The frontend also removes the node, but if the
    // frontend update races with the delete or is lost, the orphaned node
    // persists in the database.
    if !tab.layoutstate.is_empty() {
        if let Ok(Some(mut layout)) = store.get::<LayoutState>(&tab.layoutstate) {
            tracing::info!(
                block_id = %block_id,
                layout_id = %tab.layoutstate,
                "pruning deleted block from layout tree"
            );
            prune_block_from_layout(&mut layout, block_id);
            let _ = store.update(&mut layout);
        }
    }
    Ok(())
}

/// Remove all references to `block_id` from a layout's rootnode tree and leaforder.
fn prune_block_from_layout(layout: &mut LayoutState, block_id: &str) {
    // Prune leaforder
    if let Some(ref mut leaves) = layout.leaforder {
        leaves.retain(|entry| entry.blockid != block_id);
    }

    // Rootnode: handle the single-pane case where rootnode IS the orphan
    // leaf (no `children` array, just `data.blockId`). `prune_node` only
    // touches the `children` array, so a rootnode-leaf orphan would
    // otherwise persist forever. See SPEC_LAYOUT_HEAL_ROOTNODE_ORPHAN.md.
    let root_is_orphan = layout.rootnode
        .as_ref()
        .and_then(|r| r.get("data"))
        .and_then(|d| d.get("blockId"))
        .and_then(|id| id.as_str())
        .map(|id| id == block_id)
        .unwrap_or(false);
    if root_is_orphan {
        layout.rootnode = None;
    } else if let Some(ref mut root) = layout.rootnode {
        prune_node(root, block_id);
    }
}

/// Recursively remove child nodes whose `data.blockId` matches `block_id`.
/// If removing a child leaves a parent with only one child, collapse by
/// promoting the sole child to replace the parent node.
fn prune_node(node: &mut serde_json::Value, block_id: &str) {
    if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
        // Remove children that are leaves matching block_id
        children.retain(|child| {
            child.get("data")
                .and_then(|d| d.get("blockId"))
                .and_then(|id| id.as_str())
                .map(|id| id != block_id)
                .unwrap_or(true) // keep non-leaf nodes
        });
        // Recurse into remaining children
        for child in children.iter_mut() {
            prune_node(child, block_id);
        }
    }
    // If only one child remains, promote it to replace this split node.
    // This avoids degenerate single-child splits in the layout tree.
    let should_collapse = node.get("children")
        .and_then(|c| c.as_array())
        .map(|c| c.len() == 1)
        .unwrap_or(false);
    if should_collapse {
        if let Some(sole_child) = node.get("children")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .cloned()
        {
            // Preserve the parent's id and size, replace everything else
            let parent_id = node.get("id").cloned();
            let parent_size = node.get("size").cloned();
            *node = sole_child;
            if let Some(id) = parent_id {
                node["id"] = id;
            }
            if let Some(size) = parent_size {
                node["size"] = size;
            }
        }
    }
}

/// Validate a layout against the set of existing block IDs, removing any
/// orphaned leaf nodes. Called on tab activation as a self-healing pass.
pub fn heal_layout(
    store: &WaveStore,
    tab_id: &str,
) -> Result<bool, StoreError> {
    let tab = store.must_get::<Tab>(tab_id)?;
    if tab.layoutstate.is_empty() {
        return Ok(false);
    }
    let mut layout = match store.get::<LayoutState>(&tab.layoutstate)? {
        Some(l) => l,
        None => return Ok(false),
    };

    let valid_blocks: std::collections::HashSet<&str> =
        tab.blockids.iter().map(|s| s.as_str()).collect();

    let (changed, orphans) = heal_layout_body(&mut layout, &valid_blocks);
    if !changed {
        return Ok(false);
    }
    tracing::warn!(
        tab_id = %tab_id,
        orphan_count = orphans.len(),
        orphans = ?orphans,
        "healing layout: removing orphaned block nodes"
    );
    store.update(&mut layout)?;
    Ok(true)
}

/// Pure heal pass on a LayoutState given the set of block IDs that should
/// still be present. Returns `(changed, orphans_removed)`.
///
/// Collects orphans from both `leaforder` AND any leaf still reachable in
/// `rootnode` — `leaforder` can drift out of sync with `rootnode` if a
/// write was interrupted, and the heal is the last defense. Prunes each
/// orphan via `prune_block_from_layout`, then clears `focusednodeid` if
/// the rootnode ended up empty.
fn heal_layout_body(
    layout: &mut LayoutState,
    valid_blocks: &std::collections::HashSet<&str>,
) -> (bool, Vec<String>) {
    // Orphans visible via leaforder.
    let mut orphans: Vec<String> = layout.leaforder
        .as_ref()
        .map(|leaves| {
            leaves.iter()
                .filter(|e| !valid_blocks.contains(e.blockid.as_str()))
                .map(|e| e.blockid.clone())
                .collect()
        })
        .unwrap_or_default();

    // Orphans reachable only via rootnode (leaforder might be clean while
    // rootnode retains a stale leaf — the inverse of the case that
    // originally motivated the heal).
    if let Some(ref root) = layout.rootnode {
        collect_leaf_block_ids(root, &mut |id| {
            if !valid_blocks.contains(id) && !orphans.iter().any(|o| o == id) {
                orphans.push(id.to_string());
            }
        });
    }

    if orphans.is_empty() {
        return (false, orphans);
    }

    for orphan_id in &orphans {
        prune_block_from_layout(layout, orphan_id);
    }

    // If rootnode dropped, the focused-node pointer is now dangling.
    if layout.rootnode.is_none() && !layout.focusednodeid.is_empty() {
        layout.focusednodeid = String::new();
    }

    (true, orphans)
}

/// Walk a layout-tree node, calling `sink` once per leaf `data.blockId`.
fn collect_leaf_block_ids(node: &serde_json::Value, sink: &mut dyn FnMut(&str)) {
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            collect_leaf_block_ids(child, sink);
        }
        return;
    }
    if let Some(id) = node
        .get("data")
        .and_then(|d| d.get("blockId"))
        .and_then(|id| id.as_str())
    {
        sink(id);
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_layout(oid: &str) -> LayoutState {
        LayoutState {
            oid: oid.to_string(),
            version: 1,
            rootnode: None,
            leaforder: None,
            focusednodeid: String::new(),
            magnifiednodeid: String::new(),
            pendingbackendactions: None,
            meta: None,
        }
    }

    fn single_leaf_layout(block_id: &str, node_id: &str) -> LayoutState {
        LayoutState {
            oid: "layout-1".into(),
            version: 1,
            rootnode: Some(json!({
                "data": { "blockId": block_id },
                "flexDirection": "row",
                "id": node_id,
                "size": 10,
            })),
            leaforder: Some(vec![LeafOrderEntry {
                blockid: block_id.into(),
                nodeid: node_id.into(),
            }]),
            focusednodeid: node_id.into(),
            magnifiednodeid: String::new(),
            pendingbackendactions: None,
            meta: None,
        }
    }

    fn two_leaf_split_layout(left_block: &str, right_block: &str) -> LayoutState {
        LayoutState {
            oid: "layout-1".into(),
            version: 1,
            rootnode: Some(json!({
                "id": "split-1",
                "flexDirection": "row",
                "size": 10,
                "children": [
                    {
                        "id": "leaf-left",
                        "flexDirection": "row",
                        "size": 5,
                        "data": { "blockId": left_block }
                    },
                    {
                        "id": "leaf-right",
                        "flexDirection": "row",
                        "size": 5,
                        "data": { "blockId": right_block }
                    }
                ]
            })),
            leaforder: Some(vec![
                LeafOrderEntry { blockid: left_block.into(), nodeid: "leaf-left".into() },
                LeafOrderEntry { blockid: right_block.into(), nodeid: "leaf-right".into() },
            ]),
            focusednodeid: "leaf-left".into(),
            magnifiednodeid: String::new(),
            pendingbackendactions: None,
            meta: None,
        }
    }

    fn set<'a>(ids: &[&'a str]) -> std::collections::HashSet<&'a str> {
        ids.iter().copied().collect()
    }

    #[test]
    fn prune_removes_rootnode_leaf_when_it_is_orphan() {
        // THE BUG: single-pane layout where rootnode IS the orphan leaf.
        // prune_node's old implementation only walked `children`, so rootnode
        // stayed. This test fails on pre-fix main.
        let mut layout = single_leaf_layout("orphan-block", "node-1");
        prune_block_from_layout(&mut layout, "orphan-block");

        assert!(layout.rootnode.is_none(),
            "rootnode must be cleared when it was the orphan leaf");
        assert_eq!(
            layout.leaforder.as_deref().map(|l| l.len()),
            Some(0),
            "leaforder entry must be removed too",
        );
    }

    #[test]
    fn prune_removes_child_leaf_and_collapses() {
        // Regression of existing multi-pane behavior.
        let mut layout = two_leaf_split_layout("keep", "drop");
        prune_block_from_layout(&mut layout, "drop");

        assert!(layout.rootnode.is_some(), "rootnode should survive");
        // The split should collapse to just the "keep" leaf.
        let root = layout.rootnode.as_ref().unwrap();
        let kept_block = root.get("data")
            .and_then(|d| d.get("blockId"))
            .and_then(|id| id.as_str());
        assert_eq!(kept_block, Some("keep"),
            "after pruning 'drop' and collapsing, rootnode should be the 'keep' leaf");
    }

    #[test]
    fn prune_noop_when_block_absent() {
        let mut layout = single_leaf_layout("other-block", "node-1");
        let before = serde_json::to_string(&layout.rootnode).unwrap();
        prune_block_from_layout(&mut layout, "not-present");
        let after = serde_json::to_string(&layout.rootnode).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn heal_body_clears_focused_nodeid_when_rootnode_drops() {
        let mut layout = single_leaf_layout("orphan-block", "node-1");
        assert_eq!(layout.focusednodeid, "node-1");
        let (changed, _orphans) = heal_layout_body(&mut layout, &set(&[]));
        assert!(changed);
        assert!(layout.rootnode.is_none());
        assert_eq!(layout.focusednodeid, "",
            "focusednodeid must be cleared when rootnode is empty");
    }

    #[test]
    fn heal_body_catches_rootnode_orphan_missing_from_leaforder() {
        // Malformed save: rootnode leaf points at an orphan but leaforder is
        // already clean (or absent). The healer must still prune rootnode.
        let mut layout = single_leaf_layout("orphan-block", "node-1");
        layout.leaforder = Some(vec![]); // leaforder doesn't mention it

        let (changed, orphans) = heal_layout_body(&mut layout, &set(&[]));
        assert!(changed, "healer must notice rootnode-only orphan");
        assert!(orphans.contains(&"orphan-block".to_string()));
        assert!(layout.rootnode.is_none());
    }

    #[test]
    fn heal_body_idempotent_on_clean_layout() {
        let mut layout = single_leaf_layout("live-block", "node-1");
        let (changed, _) = heal_layout_body(&mut layout, &set(&["live-block"]));
        assert!(!changed, "no orphans → no change");
        // Run again; still no change.
        let (changed_again, _) = heal_layout_body(&mut layout, &set(&["live-block"]));
        assert!(!changed_again);
    }

    #[test]
    fn heal_body_handles_empty_layout() {
        let mut layout = empty_layout("empty-layout");
        let (changed, orphans) = heal_layout_body(&mut layout, &set(&[]));
        assert!(!changed);
        assert!(orphans.is_empty());
    }
}

/// Resolve a block ID from an 8-character prefix within a tab.
pub fn resolve_block_id_from_prefix(
    store: &WaveStore,
    tab_id: &str,
    prefix: &str,
) -> Result<String, StoreError> {
    if prefix.len() != 8 {
        return Err(StoreError::Other(
            "block_id prefix must be 8 characters".to_string(),
        ));
    }
    let tab = store.must_get::<Tab>(tab_id)?;
    for block_id in &tab.blockids {
        if block_id.starts_with(prefix) {
            return Ok(block_id.clone());
        }
    }
    Err(StoreError::NotFound)
}
