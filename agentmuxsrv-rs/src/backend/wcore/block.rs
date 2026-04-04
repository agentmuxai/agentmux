// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Block CRUD operations.

use uuid::Uuid;

use crate::backend::storage::wstore::WaveStore;
use crate::backend::storage::StoreError;
use crate::backend::waveobj::*;

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

/// Delete a block from its parent tab.
pub fn delete_block(
    store: &WaveStore,
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
