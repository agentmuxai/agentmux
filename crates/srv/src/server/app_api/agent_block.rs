// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's block, found by the agent and not by a tab
//! (`docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` §3.1, §3.3 rule 1): one block per
//! agent, in a tab or parked in none. Everything that wants "the agent's pane" asks here
//! instead of scanning one tab, so a pane in another window, or none, is still found.

use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block, Tab};
use crate::backend::storage::store::Store;

use super::{resolve_tab_id, resolve_tab_id_for_block};

/// An agent's block and the tab it is in. `tab_id` is `None` for a parked block.
#[cfg_attr(not(test), allow(dead_code))] // until place_agent (#5838 PR C) calls it
#[derive(Debug, Clone)]
pub(crate) struct AgentBlock {
    pub block: Block,
    pub tab_id: Option<String>,
}

#[cfg_attr(not(test), allow(dead_code))] // until place_agent (#5838 PR C) calls it
fn is_agent_block_of(block: &Block, agent_uid: &str) -> bool {
    ["agentId", "agent:id"].iter().any(|key| obj::meta_get_string(&block.meta, key, "") == agent_uid)
}

fn is_parked(block: &Block) -> bool {
    obj::meta_get_bool(&block.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false)
}

/// The block of the agent with this UID, in any tab or parked. `None` when it has none.
///
/// An agent should have one block. Where old layouts left several, the one with a live controller
/// wins, then one in a tab over a parked one, then the lowest id (so the answer is stable).
#[cfg_attr(not(test), allow(dead_code))] // until place_agent (#5838 PR C) calls it
pub(crate) fn find_agent_block_by_uid(mstore: &Store, agent_uid: &str) -> Result<Option<AgentBlock>, String> {
    let agent_uid = agent_uid.trim();
    if agent_uid.is_empty() {
        return Ok(None);
    }
    let mut found: Vec<Block> = mstore
        .get_all::<Block>()
        .map_err(|e| format!("find_agent_block_by_uid: list blocks: {e}"))?
        .into_iter()
        .filter(|b| is_agent_block_of(b, agent_uid))
        .collect();
    if found.is_empty() {
        return Ok(None);
    }
    let tabs: Vec<Tab> = mstore.get_all::<Tab>().map_err(|e| format!("find_agent_block_by_uid: list tabs: {e}"))?;
    let tab_of = |block: &Block| tabs.iter().find(|t| t.blockids.iter().any(|id| id == &block.oid)).map(|t| t.oid.clone());
    found.sort_by_key(|b| (blockcontroller::get_controller(&b.oid).is_none(), tab_of(b).is_none(), b.oid.clone()));
    let block = found.swap_remove(0);
    let tab_id = tab_of(&block);
    Ok(Some(AgentBlock { block, tab_id }))
}

/// [`resolve_tab_id_for_block`], and for a parked block, which no tab holds, the active tab: a
/// headless agent that spawns or opens panes has no tab of its own to use.
pub(crate) fn resolve_tab_id_or_active(mstore: &Store, block_id: &str) -> Result<String, String> {
    match resolve_tab_id_for_block(mstore, block_id) {
        Ok(tab_id) => Ok(tab_id),
        Err(e) => match mstore.get::<Block>(block_id) {
            Ok(Some(block)) if is_parked(&block) => resolve_tab_id(mstore, None),
            _ => Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{MetaMapType, Workspace};

    fn insert_tab(store: &Store, blocks: &[&str]) -> String {
        let mut tab = Tab { oid: uuid::Uuid::new_v4().to_string(), blockids: blocks.iter().map(|b| b.to_string()).collect(), ..Default::default() };
        store.insert(&mut tab).unwrap();
        tab.oid
    }

    fn insert_block(store: &Store, agent: &str, parked: bool) -> String {
        let mut meta = MetaMapType::new();
        meta.insert("agentId".to_string(), serde_json::json!(agent));
        if parked {
            meta.insert(crate::persist_subscriber::parked::PARKED_META_KEY.to_string(), serde_json::json!(true));
        }
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() };
        store.insert(&mut block).unwrap();
        block.oid
    }

    #[test]
    fn finds_the_agents_block_in_any_tab_or_parked_and_only_its_own() {
        let store = Store::open_in_memory().unwrap();
        let (shown, parked, other) = (insert_block(&store, "ann", false), insert_block(&store, "bob", true), insert_block(&store, "cy", false));
        let tab = insert_tab(&store, &[&shown, &other]);
        let ann = find_agent_block_by_uid(&store, "ann").unwrap().unwrap();
        assert_eq!((ann.block.oid.as_str(), ann.tab_id.as_deref()), (shown.as_str(), Some(tab.as_str())));
        let bob = find_agent_block_by_uid(&store, "bob").unwrap().unwrap();
        assert_eq!((bob.block.oid.as_str(), bob.tab_id), (parked.as_str(), None));
        assert!(find_agent_block_by_uid(&store, "nobody").unwrap().is_none());
        assert!(find_agent_block_by_uid(&store, "  ").unwrap().is_none());
    }

    #[test]
    fn of_two_blocks_for_one_agent_the_one_in_a_tab_wins_over_a_parked_one() {
        let store = Store::open_in_memory().unwrap();
        let (parked, shown) = (insert_block(&store, "ann", true), insert_block(&store, "ann", false));
        insert_tab(&store, &[&shown]);
        let found = find_agent_block_by_uid(&store, "ann").unwrap().unwrap();
        assert_eq!(found.block.oid, shown);
        assert_ne!(found.block.oid, parked);
    }

    #[test]
    fn a_parked_block_resolves_to_the_active_tab_and_an_unmarked_orphan_does_not() {
        let store = Store::open_in_memory().unwrap();
        let tab = insert_tab(&store, &[]);
        let mut ws = Workspace { oid: uuid::Uuid::new_v4().to_string(), tabids: vec![tab.clone()], activetabid: tab.clone(), ..Default::default() };
        store.insert(&mut ws).unwrap();
        let (parked, orphan) = (insert_block(&store, "ann", true), insert_block(&store, "bob", false));
        assert_eq!(resolve_tab_id_or_active(&store, &parked).unwrap(), tab);
        assert!(resolve_tab_id_or_active(&store, &orphan).is_err());
    }
}
