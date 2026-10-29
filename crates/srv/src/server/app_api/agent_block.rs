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
#[derive(Debug, Clone)]
pub(crate) struct AgentBlock {
    pub block: Block,
    pub tab_id: Option<String>,
}

/// A block launched from a template keeps the template's id in `agentId` and binds its agent through
/// `agentInstanceId`, so the bound instance counts too.
fn is_agent_block_of(mstore: &Store, block: &Block, agent_uid: &str) -> bool {
    !obj::is_history_reader(&block.meta)
        && (["agentId", "agent:id"].iter().any(|key| obj::meta_get_string(&block.meta, key, "") == agent_uid)
            || (obj::meta_get_string(&block.meta, "view", "") == "agent"
                && mstore.instance_get_by_block_id(&block.oid).ok().flatten().is_some_and(|i| i.id == agent_uid)))
}

fn is_parked(block: &Block) -> bool {
    obj::meta_get_bool(&block.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false)
}

/// Whether the block failed to open because the agent's conversation is open in another pane
/// (`open_in_other_pane`): it is not the one that holds the conversation.
fn lost_the_conversation(block: &Block) -> bool {
    block.meta.get(blockcontroller::core::META_LAST_FAILURE).and_then(|f| f.get("code")).and_then(|c| c.as_str()) == Some("open_in_other_pane")
}

/// Whether the block runs its agent now: a controller that has not ended. A stopped one stays
/// registered, so presence alone would not tell it from a running one.
fn controller_is_live(block_id: &str) -> bool {
    blockcontroller::get_controller(block_id)
        .is_some_and(|c| c.get_runtime_status().shellprocstatus != blockcontroller::STATUS_DONE)
}

/// The block of the agent with this UID, in any tab or parked. `None` when it has none.
///
/// Only a block that is in a tab or marked parked counts: an unmarked block in no tab is an
/// orphan boot drops (`persist.rs`), which `AttachBlock` could not attach. An agent should have
/// one block. Where old layouts left several, one that was refused for a conversation open in
/// another pane comes last (another block holds it), then the one with a live controller wins, then
/// one in a tab over a parked one, then the lowest id (so the answer is stable).
pub(crate) fn find_agent_block_by_uid(mstore: &Store, agent_uid: &str) -> Result<Option<AgentBlock>, String> {
    let agent_uid = agent_uid.trim();
    if agent_uid.is_empty() {
        return Ok(None);
    }
    let mut found: Vec<Block> = mstore
        .get_all::<Block>()
        .map_err(|e| format!("find_agent_block_by_uid: list blocks: {e}"))?
        .into_iter()
        .filter(|b| is_agent_block_of(mstore, b, agent_uid))
        .collect();
    if found.is_empty() {
        return Ok(None);
    }
    let tabs: Vec<Tab> = mstore.get_all::<Tab>().map_err(|e| format!("find_agent_block_by_uid: list tabs: {e}"))?;
    let tab_of = |block: &Block| tabs.iter().find(|t| t.blockids.iter().any(|id| id == &block.oid)).map(|t| t.oid.clone());
    found.retain(|b| is_parked(b) || tab_of(b).is_some());
    found.sort_by_key(|b| (lost_the_conversation(b), !controller_is_live(&b.oid), tab_of(b).is_none(), b.oid.clone()));
    let Some(block) = found.into_iter().next() else {
        return Ok(None);
    };
    let tab_id = tab_of(&block);
    Ok(Some(AgentBlock { block, tab_id }))
}

/// Every block that is in a tab or parked, each with its tab (`None` when parked). The agent readers
/// that used to walk tabs' `blockids` use this, so a headless agent's parked block is not missing
/// from them. An unmarked block in no tab is an orphan and is left out.
pub(crate) fn blocks_in_tabs_or_parked(mstore: &Store) -> Result<Vec<(Block, Option<String>)>, String> {
    let tabs: Vec<Tab> = mstore.get_all::<Tab>().map_err(|e| format!("list tabs: {e}"))?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for tab in &tabs {
        for block_id in &tab.blockids {
            if let Ok(Some(block)) = mstore.get::<Block>(block_id) {
                if !obj::is_history_reader(&block.meta) && seen.insert(block.oid.clone()) {
                    out.push((block, Some(tab.oid.clone())));
                }
            }
        }
    }
    for block in mstore.get_all::<Block>().map_err(|e| format!("list blocks: {e}"))? {
        if is_parked(&block) && !obj::is_history_reader(&block.meta) && seen.insert(block.oid.clone()) {
            out.push((block, None));
        }
    }
    Ok(out)
}

/// [`resolve_tab_id_for_block`], and for a parked block, which no tab holds, its agent's home tab
/// ([`parked_agents_tab`]) and only then the active tab: a headless agent that spawns or opens panes
/// has no tab of its own to use.
pub(crate) fn resolve_tab_id_or_active(mstore: &Store, block_id: &str) -> Result<String, String> {
    match resolve_tab_id_for_block(mstore, block_id) {
        Ok(tab_id) => Ok(tab_id),
        Err(e) => match mstore.get::<Block>(block_id) {
            Ok(Some(block)) if is_parked(&block) => match parked_agents_tab(mstore, &block) {
                Some(tab_id) => Ok(tab_id),
                None => resolve_tab_id(mstore, None),
            },
            _ => Err(e),
        },
    }
}

/// The tab a parked agent works from, so its workers open beside it and not in whatever tab the user
/// is looking at: the tab its pane was parked from, else a tab one of its open workers is in.
fn parked_agents_tab(mstore: &Store, block: &Block) -> Option<String> {
    let from = obj::meta_get_string(&block.meta, crate::persist_subscriber::parked::PARKED_FROM_TAB_META_KEY, "");
    if !from.is_empty() && mstore.get::<Tab>(&from).ok().flatten().is_some() {
        return Some(from);
    }
    let agent = obj::meta_get_string(&block.meta, "agentId", "");
    super::agent_spawn::workers_of(mstore, &agent).iter().find_map(|w| resolve_tab_id_for_block(mstore, &w.block_id).ok())
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

    /// A block launched from a template keeps the template's id in `agentId`; its agent is the bound instance.
    #[test]
    fn a_template_launched_agents_block_is_found_through_its_binding() {
        use crate::backend::storage::agents::{AgentInstance, InstanceStatus};
        let store = Store::open_in_memory().unwrap();
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        meta.insert("agentId".to_string(), serde_json::json!("tmpl"));
        meta.insert("agentInstanceId".to_string(), serde_json::json!("inst"));
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() };
        store.insert(&mut block).unwrap();
        let tab = insert_tab(&store, &[&block.oid]);
        assert!(find_agent_block_by_uid(&store, "inst").unwrap().is_none(), "no agent is bound yet");

        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "inst", "slug": "inst", "name": "inst", "icon": "robot", "provider": "claude", "description": "", "created_at": 1,
        }))
        .unwrap();
        store.agent_def_insert(&mut def).unwrap();
        store
            .instance_create(&AgentInstance {
                id: "inst".to_string(),
                definition_id: "inst".to_string(),
                parent_instance_id: String::new(),
                block_id: block.oid.clone(),
                session_id: "sess".to_string(),
                status: InstanceStatus::Running.as_str().to_string(),
                github_context: String::new(),
                started_at: 1000,
                ended_at: 0,
                created_at: 1000,
                identity_id: String::new(),
                memory_id: String::new(),
                instance_name: String::new(),
                working_directory: String::new(),
                display_hidden: false,
            })
            .unwrap();
        let found = find_agent_block_by_uid(&store, "inst").unwrap().unwrap();
        assert_eq!((found.block.oid.as_str(), found.tab_id.as_deref()), (block.oid.as_str(), Some(tab.as_str())));
    }

    #[test]
    fn the_blocks_in_tabs_or_parked_leave_out_an_unmarked_orphan() {
        let store = Store::open_in_memory().unwrap();
        let (shown, parked, orphan) = (insert_block(&store, "ann", false), insert_block(&store, "bob", true), insert_block(&store, "cy", false));
        let tab = insert_tab(&store, &[&shown]);
        let mut all = blocks_in_tabs_or_parked(&store).unwrap();
        all.sort_by(|a, b| a.0.oid.cmp(&b.0.oid));
        let mut want = vec![(shown, Some(tab)), (parked, None)];
        want.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(all.iter().map(|(b, t)| (b.oid.clone(), t.clone())).collect::<Vec<_>>(), want);
        assert!(all.iter().all(|(b, _)| b.oid != orphan));
    }

    /// Two blocks of one agent (an old layout): the one refused for a conversation open in another pane does
    /// not hold it, so a start resumes the other. The refused block's id sorts first, so without the rule the
    /// last tie-break (the id) would pick it.
    #[test]
    fn a_block_refused_for_an_open_conversation_is_not_picked_over_the_holder() {
        let store = Store::open_in_memory().unwrap();
        let with_id = |oid: &str| {
            let mut meta = MetaMapType::new();
            meta.insert("agentId".to_string(), serde_json::json!("ann"));
            let mut block = Block { oid: oid.to_string(), meta, ..Default::default() };
            store.insert(&mut block).unwrap();
            block
        };
        let holder = with_id("b-holder").oid;
        let mut refused = with_id("a-refused");
        refused.meta.insert(blockcontroller::core::META_LAST_FAILURE.to_string(), serde_json::json!({ "code": "open_in_other_pane", "holderBlockId": holder }));
        store.update(&mut refused).unwrap();
        insert_tab(&store, &[&holder, &refused.oid]);
        let found = find_agent_block_by_uid(&store, "ann").unwrap().unwrap();
        assert_eq!(found.block.oid, holder);
    }

    /// The history reader carries the agent's id like its pane does, and is not the pane: with the agent
    /// parked, a reader in a tab must not be taken for it.
    #[test]
    fn a_history_reader_is_not_the_agents_block() {
        let store = Store::open_in_memory().unwrap();
        let parked = insert_block(&store, "ann", true);
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        meta.insert("agentId".to_string(), serde_json::json!("ann"));
        meta.insert(obj::HISTORY_READER_META_KEY.to_string(), serde_json::json!("ann"));
        let mut reader = Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() };
        store.insert(&mut reader).unwrap();
        insert_tab(&store, &[&reader.oid]);
        let found = find_agent_block_by_uid(&store, "ann").unwrap().unwrap();
        assert_eq!((found.block.oid.as_str(), found.tab_id), (parked.as_str(), None), "the parked block, not the reader in the tab");
        assert!(blocks_in_tabs_or_parked(&store).unwrap().iter().all(|(b, _)| b.oid != reader.oid));
        assert!(obj::is_history_reader(&reader.meta));
        assert!(!obj::is_history_reader(&MetaMapType::new()));
    }

    #[test]
    fn an_unmarked_block_in_no_tab_is_not_the_agents_block() {
        let store = Store::open_in_memory().unwrap();
        let orphan = insert_block(&store, "ann", false);
        assert!(find_agent_block_by_uid(&store, "ann").unwrap().is_none(), "{orphan} is an orphan boot drops");
        let parked = insert_block(&store, "ann", true);
        let found = find_agent_block_by_uid(&store, "ann").unwrap().unwrap();
        assert_eq!(found.block.oid, parked);
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
    fn a_parked_agent_works_from_the_tab_it_was_parked_from_not_the_active_one() {
        let store = Store::open_in_memory().unwrap();
        let (team, active) = (insert_tab(&store, &[]), insert_tab(&store, &[]));
        let mut ws = Workspace { oid: uuid::Uuid::new_v4().to_string(), tabids: vec![team.clone(), active.clone()], activetabid: active.clone(), ..Default::default() };
        store.insert(&mut ws).unwrap();
        let parked = insert_block(&store, "ann", true);
        assert_eq!(resolve_tab_id_or_active(&store, &parked).unwrap(), active, "no home tab recorded: the active one");
        let mut block = store.get::<Block>(&parked).unwrap().unwrap();
        block.meta.insert(crate::persist_subscriber::parked::PARKED_FROM_TAB_META_KEY.to_string(), serde_json::json!(team));
        store.update(&mut block).unwrap();
        assert_eq!(resolve_tab_id_or_active(&store, &parked).unwrap(), team);
        block.meta.insert(crate::persist_subscriber::parked::PARKED_FROM_TAB_META_KEY.to_string(), serde_json::json!("gone"));
        store.update(&mut block).unwrap();
        assert_eq!(resolve_tab_id_or_active(&store, &parked).unwrap(), active, "a tab that is gone is not a home");
    }

    #[test]
    fn a_parked_agent_with_no_recorded_tab_works_from_the_tab_of_its_open_worker() {
        let store = Store::open_in_memory().unwrap();
        let (team, active) = (insert_tab(&store, &[]), insert_tab(&store, &[]));
        let mut ws = Workspace { oid: uuid::Uuid::new_v4().to_string(), tabids: vec![team.clone(), active.clone()], activetabid: active, ..Default::default() };
        store.insert(&mut ws).unwrap();
        let parked = insert_block(&store, "lead", true);
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "wrk", "slug": "wrk", "name": "Wrk", "icon": "robot", "provider": "claude", "description": "", "created_at": 1,
        }))
        .unwrap();
        store.agent_def_insert(&mut def).unwrap();
        let mut worker = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        worker.meta.insert("agentId".to_string(), serde_json::json!("wrk"));
        worker.meta.insert(crate::server::app_api::agent_spawn::COACHED_BY_META_KEY.to_string(), serde_json::json!({ "id": "lead" }));
        store.insert(&mut worker).unwrap();
        let mut tab = store.get::<Tab>(&team).unwrap().unwrap();
        tab.blockids.push(worker.oid.clone());
        store.update(&mut tab).unwrap();
        assert_eq!(resolve_tab_id_or_active(&store, &parked).unwrap(), team);
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
