// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an imported catalog skill came from, so it can be checked for changes later. The source
//! lives in `db_skill_source`, in the identity store beside `db_skills`, and is created on first
//! use without a schema bump (like `db_skill_reach`). A skill written by hand has no row.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_skill_source (
        skill_id    TEXT PRIMARY KEY,
        kind        TEXT NOT NULL,
        location    TEXT NOT NULL,
        git_ref     TEXT NOT NULL DEFAULT '',
        item_key    TEXT NOT NULL DEFAULT '',
        imported_at INTEGER NOT NULL DEFAULT 0
    );";

/// The place a catalog skill was imported from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SkillSource {
    /// `folder`, `git` or `url`.
    pub kind: String,
    /// The folder path or the https URL.
    pub location: String,
    /// The branch or tag for `git`; empty for the default branch.
    #[serde(default)]
    pub git_ref: String,
    /// Which `SKILL.md` inside the source (`""` for the source's own).
    #[serde(default)]
    pub item_key: String,
}

impl Store {
    /// Remember where `skill_id` came from. Call on the catalog store.
    pub fn skill_source_set(&self, skill_id: &str, src: &SkillSource, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT INTO db_skill_source (skill_id, kind, location, git_ref, item_key, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(skill_id) DO UPDATE SET kind = excluded.kind, location = excluded.location,
                git_ref = excluded.git_ref, item_key = excluded.item_key, imported_at = excluded.imported_at",
            params![skill_id, src.kind, src.location, src.git_ref, src.item_key, now_ms],
        )?;
        Ok(())
    }

    /// The source of one skill, if it was imported.
    pub fn skill_source_get(&self, skill_id: &str) -> Result<Option<SkillSource>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        Ok(conn
            .query_row(
                "SELECT kind, location, git_ref, item_key FROM db_skill_source WHERE skill_id = ?1",
                params![skill_id],
                |r| Ok(SkillSource { kind: r.get(0)?, location: r.get(1)?, git_ref: r.get(2)?, item_key: r.get(3)? }),
            )
            .optional()?)
    }

    /// Every imported skill's source, by skill id.
    pub fn skill_source_map(&self) -> Result<std::collections::HashMap<String, SkillSource>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let mut stmt = conn.prepare("SELECT skill_id, kind, location, git_ref, item_key FROM db_skill_source")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, SkillSource { kind: r.get(1)?, location: r.get(2)?, git_ref: r.get(3)?, item_key: r.get(4)? }))
        })?;
        Ok(rows.collect::<Result<std::collections::HashMap<_, _>, _>>()?)
    }

    /// Drop a deleted skill's source. Call on the catalog store.
    pub fn skill_source_forget(&self, skill_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute("DELETE FROM db_skill_source WHERE skill_id = ?1", params![skill_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    fn catalog() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.apply_identity_schema_for_tests().unwrap();
        s
    }

    #[test]
    fn a_source_is_remembered_replaced_and_forgotten_per_skill() {
        let c = catalog();
        assert_eq!(c.skill_source_get("s1").unwrap(), None);
        let a = SkillSource { kind: "git".into(), location: "https://example.com/r.git".into(), git_ref: "main".into(), item_key: "skills/a".into() };
        c.skill_source_set("s1", &a, 1).unwrap();
        assert_eq!(c.skill_source_get("s1").unwrap(), Some(a.clone()));
        let b = SkillSource { kind: "url".into(), location: "https://example.com/SKILL.md".into(), git_ref: String::new(), item_key: String::new() };
        c.skill_source_set("s1", &b, 2).unwrap();
        c.skill_source_set("s2", &a, 2).unwrap();
        assert_eq!(c.skill_source_get("s1").unwrap(), Some(b));
        assert_eq!(c.skill_source_map().unwrap().len(), 2);
        c.skill_source_forget("s1").unwrap();
        assert_eq!(c.skill_source_get("s1").unwrap(), None);
        assert_eq!(c.skill_source_get("s2").unwrap(), Some(a));
    }
}
