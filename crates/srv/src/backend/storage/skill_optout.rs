// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent that opted out of a catalog skill that reaches everyone ("every agent" or "new
//! agents"). The opt-out only cancels the reach; an explicit bind to that agent still gives it the
//! skill. Rows live in `db_skill_optout`, in the identity store beside `db_skills`, created on first
//! use without a schema bump (like `db_skill_reach`).

use rusqlite::params;
use std::collections::HashSet;

use super::error::StoreError;
use super::store::Store;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_skill_optout (
        skill_id TEXT NOT NULL,
        agent_id TEXT NOT NULL,
        PRIMARY KEY (skill_id, agent_id)
    );";

impl Store {
    /// Every `(skill_id, agent_id)` pair that opted out. Call on the catalog store.
    pub fn skill_optout_set_all(&self) -> Result<HashSet<(String, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let mut stmt = conn.prepare("SELECT skill_id, agent_id FROM db_skill_optout")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<Result<HashSet<_>, _>>()?)
    }

    /// Opt an agent out of a skill's reach, or back in. Idempotent. Call on the catalog store.
    pub fn skill_optout_put(&self, skill_id: &str, agent_id: &str, opted_out: bool) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        if opted_out {
            conn.execute("INSERT OR IGNORE INTO db_skill_optout (skill_id, agent_id) VALUES (?1, ?2)", params![skill_id, agent_id])?;
        } else {
            conn.execute("DELETE FROM db_skill_optout WHERE skill_id = ?1 AND agent_id = ?2", params![skill_id, agent_id])?;
        }
        Ok(())
    }

    /// Drop a deleted skill's opt-outs. Call on the catalog store.
    pub fn skill_optout_forget(&self, skill_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute("DELETE FROM db_skill_optout WHERE skill_id = ?1", params![skill_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod optout_tests {
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

    fn agent(store: &Store, id: &str) {
        store
            .conn()
            .lock()
            .unwrap()
            .execute("INSERT INTO db_agents (id, name, provider, created_at) VALUES (?1, ?1, 'claude', 1)", params![id])
            .unwrap();
    }

    fn names(m: &Store, c: &Store, agent: &str) -> Vec<String> {
        let mut v: Vec<String> = m.effective_skills(c, agent).into_iter().map(|s| s.name).collect();
        v.sort();
        v
    }

    #[test]
    fn an_opt_out_cancels_the_reach_for_that_agent_only_and_a_bind_still_wins() {
        let (m, c) = stores();
        c.skill_upsert_unique_global(&global_skill("s1")).unwrap();
        agent(&m, "a1");
        agent(&m, "a2");
        c.skill_set_reach("s1", true, false, 5).unwrap();
        assert_eq!(names(&m, &c, "a1"), ["Skill s1"]);
        assert_eq!(names(&m, &c, "a2"), ["Skill s1"]);

        c.skill_optout_put("s1", "a1", true).unwrap();
        assert!(names(&m, &c, "a1").is_empty(), "opted out of what reaches everyone");
        assert_eq!(names(&m, &c, "a2"), ["Skill s1"], "the others still get it");
        // The list the matrix reads says why.
        let item = m.skill_list_global_for_agent(&c, "a1").unwrap().into_iter().find(|i| i.skill.id == "s1").unwrap();
        assert!(!item.reached && item.opted_out && !item.bound_to_agent);

        // An explicit bind is the person asking for it, so it wins over the opt-out.
        m.skill_bind(&c, "a1", "s1").unwrap();
        assert_eq!(names(&m, &c, "a1"), ["Skill s1"]);

        c.skill_optout_put("s1", "a1", false).unwrap();
        c.skill_optout_put("s1", "a1", false).unwrap(); // idempotent
        assert!(c.skill_optout_set_all().unwrap().is_empty());
    }

    #[test]
    fn deleting_a_skill_drops_its_opt_outs() {
        let (m, c) = stores();
        c.skill_upsert_unique_global(&global_skill("s1")).unwrap();
        c.skill_optout_put("s1", "a1", true).unwrap();
        c.skill_optout_put("s2", "a1", true).unwrap();
        m.skill_delete(&c, "s1").unwrap();
        let left = c.skill_optout_set_all().unwrap();
        assert_eq!(left.len(), 1);
        assert!(left.contains(&("s2".to_string(), "a1".to_string())));
    }
}
