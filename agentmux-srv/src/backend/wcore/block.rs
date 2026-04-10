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
    // Prune rootnode tree (recursive JSON walk)
    if let Some(ref mut root) = layout.rootnode {
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

    // Collect orphaned block IDs from leaforder
    let orphans: Vec<String> = layout.leaforder
        .as_ref()
        .map(|leaves| {
            leaves.iter()
                .filter(|e| !valid_blocks.contains(e.blockid.as_str()))
                .map(|e| e.blockid.clone())
                .collect()
        })
        .unwrap_or_default();

    if orphans.is_empty() {
        return Ok(false);
    }

    tracing::warn!(
        tab_id = %tab_id,
        orphan_count = orphans.len(),
        orphans = ?orphans,
        "healing layout: removing orphaned block nodes"
    );

    for orphan_id in &orphans {
        prune_block_from_layout(&mut layout, orphan_id);
    }
    store.update(&mut layout)?;
    Ok(true)
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
