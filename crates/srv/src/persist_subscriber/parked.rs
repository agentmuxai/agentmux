// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Persisting a parked block: one in no tab (`Command::ParkBlock`, `Command::AttachBlock`).

use crate::backend::obj::{Block, Tab};
use crate::backend::wcore;
use crate::backend::storage::store::Store;

/// Block meta that marks a parked block, one in no tab (`Command::ParkBlock`): boot loads such a
/// block into the reducer's state (`persist::bootstrap`) instead of dropping it as an orphan.
pub(crate) const PARKED_META_KEY: &str = "agent:parked";

/// A parked block (`Command::ParkBlock`): out of the tab's `blockids`, its row and meta kept,
/// its parent cleared and `agent:parked` set, which boot reads to load it (`persist::bootstrap`).
pub(super) fn apply_block_parked(mstore: &Store, tab_id: &str, block_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    mstore.with_tx(|tx| {
        if let Some(mut tab) = tx.get::<Tab>(tab_id)? {
            let before = tab.blockids.len();
            tab.blockids.retain(|id| id != block_id);
            if tab.blockids.len() != before {
                tx.update(&mut tab)?;
            }
        }
        if let Some(mut block) = tx.get::<Block>(block_id)? {
            block.parentoref = String::new();
            block.meta.insert(PARKED_META_KEY.to_string(), serde_json::json!(true));
            tx.update(&mut block)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// A parked block put into a tab (`Command::AttachBlock`): in the tab's `blockids`, its parent
/// set, the parked mark gone. Idempotent on re-delivery.
pub(super) fn apply_block_attached(mstore: &Store, block_id: &str, dst_tab_id: &str, dst_index: u32) -> Result<(), Box<dyn std::error::Error>> {
    mstore.with_tx(|tx| {
        if let Some(mut tab) = tx.get::<Tab>(dst_tab_id)? {
            if !tab.blockids.iter().any(|id| id == block_id) {
                let at = (dst_index as usize).min(tab.blockids.len());
                tab.blockids.insert(at, block_id.to_string());
                tx.update(&mut tab)?;
            }
        }
        if let Some(mut block) = tx.get::<Block>(block_id)? {
            block.parentoref = format!("tab:{}", dst_tab_id);
            block.meta.remove(PARKED_META_KEY);
            tx.update(&mut block)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// A deleted block: its row, and its place in its tab. A parked block was in no tab (`tab_id` is
/// empty), so there is only the row. Idempotent.
pub(super) fn apply_block_deleted(mstore: &Store, tab_id: &str, block_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    if mstore.get::<Block>(block_id)?.is_none() {
        return Ok(()); // already gone
    }
    if tab_id.is_empty() {
        return Ok(wcore::delete_parked_block(mstore, block_id)?);
    }
    match wcore::delete_block(mstore, tab_id, block_id) {
        Ok(()) | Err(crate::backend::storage::StoreError::NotFound) => Ok(()),
        Err(e) => Err(Box::new(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::wcore;
    use agentmux_common::ipc::Event;

    fn block_created(tab_id: &str, block_id: &str) -> Event {
        Event::BlockCreated { tab_id: tab_id.into(), block_id: block_id.into(), meta: serde_json::json!({ "agentId": "ann" }), version: 1 }
    }

    /// Park keeps the row and its meta and only leaves the tab; attach puts it back in any tab.
    #[test]
    fn park_then_attach_moves_the_block_between_no_tab_and_a_tab_keeping_its_meta() {
        let s = Store::open_in_memory().unwrap();
        let ws = wcore::create_workspace(&s, "W").unwrap();
        let (a, b) = (wcore::create_tab_with_opts(&s, &ws.oid, "a", false).unwrap(), wcore::create_tab_with_opts(&s, &ws.oid, "b", false).unwrap());
        crate::persist_subscriber::apply_event_to_mstore(&block_created(&a.oid, "blk"), &s).unwrap();

        apply_block_parked(&s, &a.oid, "blk").unwrap();
        let block = s.get::<Block>("blk").unwrap().unwrap();
        assert!(s.get::<Tab>(&a.oid).unwrap().unwrap().blockids.is_empty());
        assert_eq!(block.parentoref, "");
        assert_eq!(block.meta.get(PARKED_META_KEY), Some(&serde_json::json!(true)));
        assert_eq!(block.meta.get("agentId"), Some(&serde_json::json!("ann")), "its meta is kept");

        apply_block_attached(&s, "blk", &b.oid, 0).unwrap();
        apply_block_attached(&s, "blk", &b.oid, 0).unwrap();
        let block = s.get::<Block>("blk").unwrap().unwrap();
        assert_eq!(s.get::<Tab>(&b.oid).unwrap().unwrap().blockids, vec!["blk".to_string()], "once, however often delivered");
        assert_eq!(block.parentoref, format!("tab:{}", b.oid));
        assert!(!block.meta.contains_key(PARKED_META_KEY));
    }

    /// A parked block was in no tab: its delete removes the row and touches no tab.
    #[test]
    fn deleting_a_parked_block_removes_its_row_and_leaves_the_tabs_alone() {
        let s = Store::open_in_memory().unwrap();
        let ws = wcore::create_workspace(&s, "W").unwrap();
        let a = wcore::create_tab_with_opts(&s, &ws.oid, "a", false).unwrap();
        crate::persist_subscriber::apply_event_to_mstore(&block_created(&a.oid, "blk"), &s).unwrap();
        apply_block_parked(&s, &a.oid, "blk").unwrap();

        let deleted = Event::BlockDeleted { tab_id: String::new(), block_id: "blk".into(), version: 3 };
        crate::persist_subscriber::apply_event_to_mstore(&deleted, &s).unwrap();
        crate::persist_subscriber::apply_event_to_mstore(&deleted, &s).unwrap();
        assert!(s.get::<Block>("blk").unwrap().is_none());
        assert!(s.get::<Tab>(&a.oid).unwrap().is_some());
    }
}
