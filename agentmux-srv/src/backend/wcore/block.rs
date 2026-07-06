#![allow(dead_code)]
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Block CRUD operations.

use uuid::Uuid;

use crate::backend::storage::store::Store;
use crate::backend::storage::StoreError;
use crate::backend::obj::*;

/// Create a new block in a tab.
pub fn create_block(
    store: &Store,
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

/// Delete a block's rows: remove it from its parent tab's `blockids` and
/// delete the Block row.
///
/// SPEC_864 site #6 — the layout-tree prune that used to live here
/// (wcore-direct `LayoutState` write, the last Path-B writer on the
/// block-delete path) moved to the reducer: the `delete_block` saga
/// dispatches `LayoutDeleteNodeByBlock` as its Step 2, and the persist
/// subscriber writes the reducer's resulting tree from the
/// `LayoutNodeDeleted{new_tree, tree_cleared}` event. This fn is row-ops
/// only, keeping `db_layout` single-writer. SPEC_864 Phase 5 deleted the
/// `prune_block_from_layout`/`heal_layout` backstop that used to live
/// below — no Path-B writer remains to produce the orphans it swept.
pub fn delete_block(
    store: &Store,
    tab_id: &str,
    block_id: &str,
) -> Result<(), StoreError> {
    let mut tab = store.must_get::<Tab>(tab_id)?;
    tab.blockids.retain(|id| id != block_id);
    store.update(&mut tab)?;
    store.delete::<Block>(block_id)?;
    Ok(())
}

/// Resolve a block ID from an 8-character prefix within a tab.
pub fn resolve_block_id_from_prefix(
    store: &Store,
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
