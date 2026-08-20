// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Saved fleet-control target groups — see `db_agent_groups` in
//! migrations.rs (`OBJECT_SCHEMA_VERSION` v24) and
//! docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.
//!
//! A group is a durable, named list of block ids a user has saved for
//! reuse as a bulk-action target set — the Ansible-inventory-group lesson
//! from that spec's research: a saved group, not raw ephemeral selection,
//! is the abstraction that scales as a fleet grows. `member_ids` is stored
//! as a JSON array; the frontend resolves a group to concrete ids before
//! calling `fleet.broadcast`/`fleet.bulk-stop` so membership can't drift
//! between confirming a bulk action and running it.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentGroup {
    pub id: String,
    pub name: String,
    pub member_ids: Vec<String>,
    pub created_at: i64,
}

impl Store {
    pub fn agent_group_create(
        &self,
        id: &str,
        name: &str,
        member_ids: &[String],
        created_at: i64,
    ) -> Result<(), StoreError> {
        let member_ids_json = serde_json::to_string(member_ids)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_groups (id, name, member_ids, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, member_ids_json, created_at],
        )?;
        Ok(())
    }

    pub fn agent_group_list(&self) -> Result<Vec<AgentGroup>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, member_ids, created_at FROM db_agent_groups ORDER BY created_at ASC",
        )?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn agent_group_get(&self, id: &str) -> Result<Option<AgentGroup>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, member_ids, created_at FROM db_agent_groups WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], map_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Partial update: `None` fields are left unchanged. Returns `false` if
    /// no row matched `id` (caller should surface a not-found error).
    pub fn agent_group_update(
        &self,
        id: &str,
        name: Option<&str>,
        member_ids: Option<&[String]>,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        if let Some(name) = name {
            conn.execute(
                "UPDATE db_agent_groups SET name = ?2 WHERE id = ?1",
                params![id, name],
            )?;
        }
        if let Some(member_ids) = member_ids {
            let member_ids_json = serde_json::to_string(member_ids)?;
            conn.execute(
                "UPDATE db_agent_groups SET member_ids = ?2 WHERE id = ?1",
                params![id, member_ids_json],
            )?;
        }
        let exists: bool = conn
            .query_row("SELECT 1 FROM db_agent_groups WHERE id = ?1", params![id], |_| Ok(()))
            .is_ok();
        Ok(exists)
    }

    pub fn agent_group_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM db_agent_groups WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentGroup> {
    let member_ids_json: String = row.get(2)?;
    let member_ids: Vec<String> = serde_json::from_str(&member_ids_json).unwrap_or_default();
    Ok(AgentGroup {
        id: row.get(0)?,
        name: row.get(1)?,
        member_ids,
        created_at: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open(tmp.path()).unwrap()
    }

    #[test]
    fn create_and_list_roundtrips() {
        let store = object_store();
        store
            .agent_group_create("g1", "backend", &["block-a".to_string(), "block-b".to_string()], 1000)
            .unwrap();
        let groups = store.agent_group_list().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "g1");
        assert_eq!(groups[0].name, "backend");
        assert_eq!(groups[0].member_ids, vec!["block-a".to_string(), "block-b".to_string()]);
        assert_eq!(groups[0].created_at, 1000);
    }

    #[test]
    fn get_returns_none_for_missing_id() {
        let store = object_store();
        assert!(store.agent_group_get("nope").unwrap().is_none());
    }

    #[test]
    fn update_name_only_leaves_members_untouched() {
        let store = object_store();
        store.agent_group_create("g1", "old-name", &["block-a".to_string()], 1000).unwrap();
        let updated = store.agent_group_update("g1", Some("new-name"), None).unwrap();
        assert!(updated);
        let g = store.agent_group_get("g1").unwrap().unwrap();
        assert_eq!(g.name, "new-name");
        assert_eq!(g.member_ids, vec!["block-a".to_string()]);
    }

    #[test]
    fn update_members_only_leaves_name_untouched() {
        let store = object_store();
        store.agent_group_create("g1", "name", &["block-a".to_string()], 1000).unwrap();
        let updated = store
            .agent_group_update("g1", None, Some(&["block-b".to_string(), "block-c".to_string()]))
            .unwrap();
        assert!(updated);
        let g = store.agent_group_get("g1").unwrap().unwrap();
        assert_eq!(g.name, "name");
        assert_eq!(g.member_ids, vec!["block-b".to_string(), "block-c".to_string()]);
    }

    #[test]
    fn update_returns_false_for_missing_id() {
        let store = object_store();
        assert!(!store.agent_group_update("nope", Some("x"), None).unwrap());
    }

    #[test]
    fn delete_removes_the_row_and_returns_true() {
        let store = object_store();
        store.agent_group_create("g1", "name", &[], 1000).unwrap();
        assert!(store.agent_group_delete("g1").unwrap());
        assert!(store.agent_group_get("g1").unwrap().is_none());
    }

    #[test]
    fn delete_returns_false_for_missing_id() {
        let store = object_store();
        assert!(!store.agent_group_delete("nope").unwrap());
    }

    #[test]
    fn list_orders_by_created_at() {
        let store = object_store();
        store.agent_group_create("g2", "second", &[], 2000).unwrap();
        store.agent_group_create("g1", "first", &[], 1000).unwrap();
        let groups = store.agent_group_list().unwrap();
        assert_eq!(groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), vec!["g1", "g2"]);
    }
}
