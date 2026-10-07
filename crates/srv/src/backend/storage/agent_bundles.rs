// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's Bundles list: the bundles picked for it, in order
//! (`SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md` §3.6).
//!
//! The agent's own bundle (`AgentDefinition::memory_id`) is not stored here:
//! it is always first and can't be removed, so [`Store::agent_bundle_chain`]
//! puts it in front of the picks. The table is per channel, like the agent's
//! own bundle (#3148); bundles themselves live in the identity store, so a
//! listed id can name a bundle deleted elsewhere, and readers skip it.
//!
//! The Stash's old Startup tab stored one bundle as `startup_bundle_id`
//! agent content. Reading the list folds that pick in and removes it, which
//! covers existing agents and one an older build writes later.

use rusqlite::params;

use super::bundles::{Bundle, GLOBAL_SECTION_SEPARATOR};
use super::error::StoreError;
use super::store::Store;

/// The agent content type the old Startup tab stored its one bundle under.
pub const STARTUP_BUNDLE_CONTENT_TYPE: &str = "startup_bundle_id";

impl Store {
    /// The bundles picked for `agent_id`, in order. Not the agent's own.
    pub fn agent_bundle_ids(&self, agent_id: &str) -> Result<Vec<String>, StoreError> {
        self.agent_bundles_fold_startup_pick(agent_id);
        self.agent_bundle_ids_stored(agent_id)
    }

    /// Append an old Startup-tab pick to the list, then remove it. Best
    /// effort: on a failure the pick stays, to be folded on a later read.
    fn agent_bundles_fold_startup_pick(&self, agent_id: &str) {
        let Ok(Some(content)) = self.agent_content_get(agent_id, STARTUP_BUNDLE_CONTENT_TYPE) else {
            return;
        };
        let pick = content.content.trim().to_string();
        if !pick.is_empty() {
            let mut ids = self.agent_bundle_ids_stored(agent_id).unwrap_or_default();
            if !ids.contains(&pick) {
                ids.push(pick);
                if let Err(e) = self.agent_bundles_set(agent_id, &ids) {
                    tracing::warn!(agent_id, error = %e, "agent bundles: Startup pick not folded in");
                    return;
                }
            }
        }
        let _ = self.agent_content_delete(agent_id, STARTUP_BUNDLE_CONTENT_TYPE);
    }

    fn agent_bundle_ids_stored(&self, agent_id: &str) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT bundle_id FROM db_agent_bundles WHERE agent_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Replace `agent_id`'s picks with `bundle_ids`, in that order. Blank ids,
    /// repeats and the agent's own bundle are dropped. Returns what was stored.
    pub fn agent_bundles_set(
        &self,
        agent_id: &str,
        bundle_ids: &[String],
    ) -> Result<Vec<String>, StoreError> {
        let own = self
            .agent_def_get(agent_id)?
            .map(|def| def.memory_id)
            .unwrap_or_default();
        let mut picks: Vec<String> = Vec::new();
        for id in bundle_ids.iter().map(|id| id.trim()) {
            if !id.is_empty() && id != own && !picks.iter().any(|p| p == id) {
                picks.push(id.to_string());
            }
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM db_agent_bundles WHERE agent_id = ?1", params![agent_id])?;
        for (position, id) in picks.iter().enumerate() {
            tx.execute(
                "INSERT INTO db_agent_bundles (agent_id, bundle_id, position) VALUES (?1, ?2, ?3)",
                params![agent_id, id, position as i64],
            )?;
        }
        tx.commit()?;
        Ok(picks)
    }

    /// Take a deleted bundle out of every agent's list. Returns the rows removed.
    pub fn agent_bundles_forget_bundle(&self, bundle_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_agent_bundles WHERE bundle_id = ?1", params![bundle_id])?)
    }

    /// The agent's whole list: its own bundle first (when it has one), then
    /// its picks. What launch unions skills and MCP servers over.
    pub fn agent_bundle_chain(&self, agent_id: &str) -> Vec<String> {
        let own = match self.agent_def_get(agent_id) {
            Ok(Some(def)) => def.memory_id,
            _ => String::new(),
        };
        let mut chain: Vec<String> = Vec::new();
        if !own.is_empty() {
            chain.push(own);
        }
        for id in self.agent_bundle_ids(agent_id).unwrap_or_default() {
            if !chain.contains(&id) {
                chain.push(id);
            }
        }
        chain
    }
}

/// The picked bundles' part of the startup file: one `# [Bundle] <name>`
/// section per bundle with instructions, in list order, joined by the rule
/// Global Memory uses. `bundles` are the picks as found (a deleted one is
/// already missing). Empty when none has instructions.
pub fn format_agent_bundle_block(bundles: &[Bundle]) -> String {
    bundles
        .iter()
        .filter(|b| !b.instructions.trim().is_empty())
        .map(|b| format!("# [Bundle] {}\n\n{}", b.name, b.instructions))
        .collect::<Vec<_>>()
        .join(GLOBAL_SECTION_SEPARATOR)
}

/// The bundles picked for `agent_id` (from `mstore`), looked up in
/// `bundle_store`, in list order. A bundle that no longer exists is skipped.
pub fn agent_picked_bundles(mstore: &Store, bundle_store: &Store, agent_id: &str) -> Vec<Bundle> {
    mstore
        .agent_bundle_ids(agent_id)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| match bundle_store.bundle_get(id) {
            Ok(Some(bundle)) => Some(bundle),
            Ok(None) => {
                tracing::debug!(agent_id, bundle_id = %id, "agent bundles: skipping a bundle that no longer exists");
                None
            }
            Err(e) => {
                tracing::warn!(agent_id, bundle_id = %id, error = %e, "agent bundles: bundle lookup failed");
                None
            }
        })
        .collect()
}

/// The text launch puts after `# Memory` in the startup file: Global Memory,
/// then the agent's picked bundles, each part only when it has content.
pub fn join_startup_blocks(global_block: &str, agent_block: &str) -> String {
    [global_block, agent_block]
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect::<Vec<_>>()
        .join(GLOBAL_SECTION_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;

    fn bundle(id: &str, name: &str, instructions: &str) -> Bundle {
        Bundle {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: instructions.into(),
            instructions_by_provider: "{}".into(),
            context_files: "[]".into(),
            mcp_servers: "[]".into(),
            skills: "[]".into(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        }
    }

    fn store_with_agent(own_bundle: &str) -> Store {
        let s = Store::open_in_memory().unwrap();
        let mut def = test_agent_def("a1", "a1", "claude", "agent", 1, "");
        def.memory_id = own_bundle.into();
        s.agent_def_insert(&mut def).unwrap();
        s
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn set_keeps_order_and_drops_blanks_repeats_and_the_own_bundle() {
        let s = store_with_agent("own");
        let stored = s.agent_bundles_set("a1", &ids(&["b2", "", "own", "b1", "b2", " b3 "])).unwrap();
        assert_eq!(stored, ids(&["b2", "b1", "b3"]));
        assert_eq!(s.agent_bundle_ids("a1").unwrap(), stored);
        // Replacing, not appending.
        s.agent_bundles_set("a1", &ids(&["b1"])).unwrap();
        assert_eq!(s.agent_bundle_ids("a1").unwrap(), ids(&["b1"]));
        assert!(s.agent_bundle_ids("nobody").unwrap().is_empty());
    }

    fn set_startup_pick(s: &Store, id: &str) {
        s.agent_content_set(&crate::backend::storage::content::AgentContent {
            agent_id: "a1".into(),
            content_type: STARTUP_BUNDLE_CONTENT_TYPE.into(),
            content: id.into(),
            updated_at: 1,
        })
        .unwrap();
    }

    #[test]
    fn an_old_startup_pick_is_folded_into_the_list_once() {
        let s = store_with_agent("own");
        s.agent_bundles_set("a1", &ids(&["b1"])).unwrap();
        set_startup_pick(&s, "b2");
        assert_eq!(s.agent_bundle_ids("a1").unwrap(), ids(&["b1", "b2"]));
        assert!(s.agent_content_get("a1", STARTUP_BUNDLE_CONTENT_TYPE).unwrap().is_none());
        // Removing it afterwards sticks: the pick is gone, not re-folded.
        s.agent_bundles_set("a1", &ids(&["b1"])).unwrap();
        assert_eq!(s.agent_bundle_ids("a1").unwrap(), ids(&["b1"]));
        // Already listed, or the own bundle, or blank: nothing added.
        for pick in ["b1", "own", " "] {
            set_startup_pick(&s, pick);
            assert_eq!(s.agent_bundle_ids("a1").unwrap(), ids(&["b1"]), "pick {pick:?}");
            assert!(s.agent_content_get("a1", STARTUP_BUNDLE_CONTENT_TYPE).unwrap().is_none());
        }
        // The launch path reads through the fold too.
        set_startup_pick(&s, "b3");
        assert_eq!(s.agent_bundle_chain("a1"), ids(&["own", "b1", "b3"]));
    }

    #[test]
    fn the_chain_puts_the_own_bundle_first() {
        let s = store_with_agent("own");
        assert_eq!(s.agent_bundle_chain("a1"), ids(&["own"]));
        s.agent_bundles_set("a1", &ids(&["b1", "b2"])).unwrap();
        assert_eq!(s.agent_bundle_chain("a1"), ids(&["own", "b1", "b2"]));
        let no_own = store_with_agent("");
        no_own.agent_bundles_set("a1", &ids(&["b1"])).unwrap();
        assert_eq!(no_own.agent_bundle_chain("a1"), ids(&["b1"]));
    }

    #[test]
    fn a_deleted_bundle_leaves_every_list() {
        let s = store_with_agent("own");
        let mut other = test_agent_def("a2", "a2", "claude", "agent", 1, "");
        s.agent_def_insert(&mut other).unwrap();
        s.agent_bundles_set("a1", &ids(&["gone", "b1"])).unwrap();
        s.agent_bundles_set("a2", &ids(&["gone"])).unwrap();
        assert_eq!(s.agent_bundles_forget_bundle("gone").unwrap(), 2);
        assert_eq!(s.agent_bundle_ids("a1").unwrap(), ids(&["b1"]));
        assert!(s.agent_bundle_ids("a2").unwrap().is_empty());
    }

    #[test]
    fn deleting_the_agent_deletes_its_list() {
        let s = store_with_agent("own");
        s.agent_bundles_set("a1", &ids(&["b1"])).unwrap();
        assert!(s.agent_def_delete("a1").unwrap());
        assert!(s.agent_bundle_ids("a1").unwrap().is_empty());
    }

    #[test]
    fn picked_bundles_skip_a_missing_one_and_keep_order() {
        let s = store_with_agent("own");
        s.bundle_upsert(&bundle("b1", "One", "first")).unwrap();
        s.bundle_upsert(&bundle("b2", "Two", "second")).unwrap();
        s.agent_bundles_set("a1", &ids(&["b2", "gone", "b1"])).unwrap();
        let names: Vec<String> = agent_picked_bundles(&s, &s, "a1").into_iter().map(|b| b.name).collect();
        assert_eq!(names, ids(&["Two", "One"]));
    }

    #[test]
    fn the_block_has_one_section_per_bundle_with_instructions() {
        let block = format_agent_bundle_block(&[
            bundle("b1", "One", "first"),
            bundle("b2", "Empty", "  "),
            bundle("b3", "Three", "third"),
        ]);
        assert_eq!(block, "# [Bundle] One\n\nfirst\n\n---\n\n# [Bundle] Three\n\nthird");
        assert!(format_agent_bundle_block(&[bundle("b2", "Empty", "")]).is_empty());
    }

    #[test]
    fn startup_blocks_join_only_what_has_content() {
        assert_eq!(join_startup_blocks("G", "B"), "G\n\n---\n\nB");
        assert_eq!(join_startup_blocks("", "B"), "B");
        assert_eq!(join_startup_blocks("G", ""), "G");
        assert_eq!(join_startup_blocks("", ""), "");
    }
}
