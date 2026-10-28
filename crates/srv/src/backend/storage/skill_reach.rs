// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who a catalog (global) skill reaches beyond the agents it is bound to: every agent, and/or every
//! agent created from a given time. The switches live in `db_skill_reach`, in the identity store
//! beside `db_skills`, so they survive an upgrade (like `db_mcp_server_reach`). Read when an
//! agent's files are written (`Store::effective_skills`), so no hook on agent creation is needed.

use rusqlite::params;
use std::collections::HashMap;

use super::error::StoreError;
use super::store::Store;

/// The table, created on first use in the store that holds the catalog: it is added without a
/// schema version bump, like `db_mcp_server_reach` (an older build ignores it, a newer one
/// creates it), and no migration file grows for it.
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_skill_reach (
        skill_id         TEXT PRIMARY KEY,
        all_agents       INTEGER NOT NULL DEFAULT 0,
        new_agents_since INTEGER NOT NULL DEFAULT 0,
        updated_at       INTEGER NOT NULL DEFAULT 0
    );";

/// Who a catalog (global) skill reaches beyond its binds, from `db_skill_reach`. The default, no
/// row, is "only the agents it is bound to". Same shape as an MCP server's reach
/// (`McpServerReach`), with a second switch for the agents to come.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkillReach {
    /// Every agent, the existing ones and those created later.
    pub all_agents: bool,
    /// Every agent created at or after this time (unix ms), when "new agents get it" was switched
    /// on. 0: off. Read when the agent's files are written, so no hook on agent creation is needed.
    pub new_agents_since: i64,
}

impl SkillReach {
    /// Whether an agent created at `agent_created_at` gets the skill without a bind.
    pub fn reaches(&self, agent_created_at: i64) -> bool {
        self.all_agents || (self.new_agents_since > 0 && agent_created_at >= self.new_agents_since)
    }
}

impl Store {
    /// Every catalog skill's reach, by skill id. Call on the catalog store (the identity store),
    /// where `db_skill_reach` sits beside `db_skills`.
    pub fn skill_reach_map(&self) -> Result<HashMap<String, SkillReach>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let mut stmt = conn.prepare("SELECT skill_id, all_agents, new_agents_since FROM db_skill_reach")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                SkillReach { all_agents: row.get::<_, i64>(1)? != 0, new_agents_since: row.get::<_, i64>(2)? },
            ))
        })?;
        Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
    }

    /// Set who a catalog skill reaches. Call on the catalog store; the caller checks the skill is a
    /// global row. Switching "new agents" on starts at `now_ms` and keeps its start when it is
    /// already on, so setting `all_agents` later does not make older agents count as new.
    pub fn skill_set_reach(&self, skill_id: &str, all_agents: bool, new_agents: bool, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT INTO db_skill_reach (skill_id, all_agents, new_agents_since, updated_at)
             VALUES (?1, ?2, CASE WHEN ?3 THEN ?4 ELSE 0 END, ?4)
             ON CONFLICT(skill_id) DO UPDATE SET
                all_agents = excluded.all_agents,
                new_agents_since = CASE WHEN ?3 = 0 THEN 0
                                        WHEN db_skill_reach.new_agents_since > 0 THEN db_skill_reach.new_agents_since
                                        ELSE ?4 END,
                updated_at = excluded.updated_at",
            params![skill_id, all_agents as i64, new_agents as i64, now_ms],
        )?;
        Ok(())
    }

    /// Drop a deleted skill's reach. Call on the catalog store.
    pub fn skill_reach_forget(&self, skill_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute("DELETE FROM db_skill_reach WHERE skill_id = ?1", params![skill_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod reach_tests {
    use super::*;
    use crate::backend::storage::skills::Skill;

    fn stores() -> (Store, Store) {
        let mstore = Store::open_in_memory().unwrap();
        let catalog = Store::open_in_memory().unwrap();
        catalog.apply_identity_schema_for_tests().unwrap();
        (mstore, catalog)
    }

    fn global_skill(id: &str) -> Skill {
        Skill {
            id: id.to_string(),
            name: format!("Skill {id}"),
            trigger: String::new(),
            skill_type: "agent-skill".to_string(),
            description: "d".to_string(),
            content: "c".to_string(),
            is_global: true,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn agent_created_at(store: &Store, id: &str, created_at: i64) {
        store
            .conn()
            .lock()
            .unwrap()
            .execute("INSERT INTO db_agents (id, name, provider, created_at) VALUES (?1, ?1, 'claude', ?2)", params![id, created_at])
            .unwrap();
    }

    fn names(m: &Store, c: &Store, agent: &str) -> Vec<String> {
        let mut v: Vec<String> = m.effective_skills(c, agent).into_iter().map(|s| s.name).collect();
        v.sort();
        v
    }

    #[test]
    fn a_catalog_skill_reaches_only_its_bound_agents_until_switched_on_for_all() {
        let (m, c) = stores();
        agent_created_at(&m, "a1", 100);
        agent_created_at(&m, "a2", 200);
        c.skill_upsert_unique_global(&global_skill("s1")).unwrap();
        m.skill_bind(&c, "a1", "s1").unwrap();
        assert_eq!(names(&m, &c, "a1").len(), 1);
        assert!(names(&m, &c, "a2").is_empty(), "being in the catalog reaches no one");
        c.skill_set_reach("s1", true, false, 1_000).unwrap();
        assert_eq!(names(&m, &c, "a2").len(), 1, "every agent, bound or not");
        c.skill_set_reach("s1", false, false, 2_000).unwrap();
        assert!(names(&m, &c, "a2").is_empty());
        assert_eq!(names(&m, &c, "a1").len(), 1, "a bind is kept when the reach goes off");
    }

    #[test]
    fn new_agents_get_a_skill_switched_on_for_them_and_older_agents_do_not() {
        let (m, c) = stores();
        agent_created_at(&m, "old", 100);
        c.skill_upsert_unique_global(&global_skill("s1")).unwrap();
        c.skill_set_reach("s1", false, true, 1_000).unwrap();
        agent_created_at(&m, "new", 1_500);
        assert!(names(&m, &c, "old").is_empty(), "an agent from before the switch is not new");
        assert_eq!(names(&m, &c, "new").len(), 1);
        // Switching "all agents" on later does not move the start; off again leaves the new ones.
        c.skill_set_reach("s1", true, true, 5_000).unwrap();
        c.skill_set_reach("s1", false, true, 6_000).unwrap();
        assert_eq!(c.skill_reach_map().unwrap()["s1"].new_agents_since, 1_000);
        assert!(names(&m, &c, "old").is_empty());
        assert_eq!(names(&m, &c, "new").len(), 1);
        // Opting out stops it for every agent that only had it through "new agents".
        c.skill_set_reach("s1", false, false, 7_000).unwrap();
        assert!(names(&m, &c, "new").is_empty());
    }

    #[test]
    fn the_catalog_and_the_agents_list_say_how_a_skill_reaches() {
        let (m, c) = stores();
        agent_created_at(&m, "a1", 100);
        c.skill_upsert_unique_global(&global_skill("s1")).unwrap();
        c.skill_set_reach("s1", true, true, 50).unwrap();
        let item = m.skill_list_global(&c).unwrap().into_iter().next().unwrap();
        assert!(item.all_agents && item.new_agents);
        let for_agent = m.skill_list_global_for_agent(&c, "a1").unwrap().into_iter().next().unwrap();
        assert!(for_agent.reached && !for_agent.bound_to_agent);
    }

    #[test]
    fn deleting_a_skill_drops_its_reach() {
        let (m, c) = stores();
        c.skill_upsert_unique_global(&global_skill("s1")).unwrap();
        c.skill_set_reach("s1", true, true, 50).unwrap();
        assert!(m.skill_delete(&c, "s1").unwrap());
        assert!(c.skill_reach_map().unwrap().is_empty());
    }
}
