// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may fire a drone (`docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` §6.5): the agent that created it
//! (its owner) and the agents it was given to. The table is `db_drone_access` (`m0052_drone_access`).
//!
//! An agent is named by its id (its slug, lowercase), the name every other record of an agent's action uses.
//! A drone no agent created has no owner: a person made it, or it was there before the table was.

use rusqlite::params;

use crate::backend::storage::error::StoreError;
use crate::backend::storage::store::Store;

const OWNER: &str = "owner";
const GRANTED: &str = "granted";

/// What an agent may do with a drone, as far as firing goes.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DroneAccess {
    /// The agent that created the drone, or empty.
    pub owner: String,
    /// The agents it was given to.
    pub agents: Vec<String>,
}

impl DroneAccess {
    /// Whether `agent` owns the drone or was given it.
    pub fn allows(&self, agent: &str) -> bool {
        let agent = key(agent);
        !agent.is_empty() && (self.owner == agent || self.agents.iter().any(|a| *a == agent))
    }
}

/// The form an agent's name is kept in, so `Opaz` and `opaz` are one agent.
pub fn key(agent: &str) -> String {
    agent.trim().to_ascii_lowercase()
}

pub trait DroneAccessStore {
    /// The owner and the given agents of `drone_id`.
    fn access_get(&self, drone_id: &str) -> Result<DroneAccess, StoreError>;
    /// Make `agent` the owner of a drone that has none. Does nothing when it has one.
    fn access_set_owner(&self, drone_id: &str, agent: &str, now_ms: i64) -> Result<(), StoreError>;
    /// Replace the agents the drone was given to with `agents`, recorded as set by `by`.
    fn access_set_agents(&self, drone_id: &str, agents: &[String], by: &str, now_ms: i64) -> Result<(), StoreError>;
    /// Forget everything about `drone_id` (it was deleted).
    fn access_delete_for(&self, drone_id: &str) -> Result<usize, StoreError>;
}

impl DroneAccessStore for Store {
    fn access_get(&self, drone_id: &str) -> Result<DroneAccess, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare("SELECT agent, kind FROM db_drone_access WHERE drone_id = ?1 ORDER BY agent")?;
        let mut rows = stmt.query(params![drone_id])?;
        let mut access = DroneAccess::default();
        while let Some(row) = rows.next()? {
            let (agent, kind): (String, String) = (row.get(0)?, row.get(1)?);
            match kind.as_str() {
                OWNER => access.owner = agent,
                GRANTED => access.agents.push(agent),
                _ => {}
            }
        }
        Ok(access)
    }

    fn access_set_owner(&self, drone_id: &str, agent: &str, now_ms: i64) -> Result<(), StoreError> {
        let agent = key(agent);
        if agent.is_empty() {
            return Ok(());
        }
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_drone_access (drone_id, agent, kind, set_at, set_by)
             SELECT ?1, ?2, ?3, ?4, ?2
             WHERE NOT EXISTS (SELECT 1 FROM db_drone_access WHERE drone_id = ?1 AND kind = ?3)",
            params![drone_id, agent, OWNER, now_ms],
        )?;
        Ok(())
    }

    fn access_set_agents(&self, drone_id: &str, agents: &[String], by: &str, now_ms: i64) -> Result<(), StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM db_drone_access WHERE drone_id = ?1 AND kind = ?2", params![drone_id, GRANTED])?;
        for agent in agents.iter().map(|a| key(a)).filter(|a| !a.is_empty()) {
            tx.execute(
                "INSERT OR IGNORE INTO db_drone_access (drone_id, agent, kind, set_at, set_by) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![drone_id, agent, GRANTED, now_ms, by],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn access_delete_for(&self, drone_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute("DELETE FROM db_drone_access WHERE drone_id = ?1", params![drone_id])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> std::sync::Arc<Store> {
        crate::server::tests::test_state().id_store.clone()
    }

    #[tokio::test]
    async fn a_drone_nobody_made_has_no_owner_and_allows_no_agent() {
        let access = store().access_get("d-none").unwrap();
        assert_eq!(access, DroneAccess::default());
        assert!(!access.allows("opaz"));
        assert!(!access.allows(""), "an empty name is no one");
    }

    #[tokio::test]
    async fn the_first_owner_stays_and_a_name_is_one_whatever_its_case() {
        let s = store();
        s.access_set_owner("d1", "Opaz", 1).unwrap();
        s.access_set_owner("d1", "korp", 2).unwrap();
        let access = s.access_get("d1").unwrap();
        assert_eq!(access.owner, "opaz", "the owner is the first one and is kept in lower case");
        assert!(access.allows("OPAZ"));
        assert!(!access.allows("korp"));
    }

    #[tokio::test]
    async fn given_agents_are_replaced_whole_and_do_not_touch_the_owner() {
        let s = store();
        s.access_set_owner("d2", "opaz", 1).unwrap();
        s.access_set_agents("d2", &["Korp".to_string(), "korp".to_string(), " ".to_string()], "operator", 2).unwrap();
        assert_eq!(s.access_get("d2").unwrap().agents, ["korp"]);
        s.access_set_agents("d2", &["clamk".to_string()], "agent:opaz", 3).unwrap();
        let access = s.access_get("d2").unwrap();
        assert_eq!((access.owner.as_str(), access.agents), ("opaz", vec!["clamk".to_string()]));
    }

    #[tokio::test]
    async fn deleting_a_drone_forgets_who_had_it() {
        let s = store();
        s.access_set_owner("d3", "opaz", 1).unwrap();
        s.access_set_agents("d3", &["korp".to_string()], "operator", 2).unwrap();
        assert_eq!(s.access_delete_for("d3").unwrap(), 2);
        assert_eq!(s.access_get("d3").unwrap(), DroneAccess::default());
    }
}
