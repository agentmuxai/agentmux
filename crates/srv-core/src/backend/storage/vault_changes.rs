// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Vault keeps of changes agents made, for the person to look at (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md):
//! which entries hold a value an agent had the Vault read (an import or a capture) that the person has not yet seen. The
//! rules are in `backend::vault::agent_set`; nothing here holds a value.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;

pub(super) const SCHEMA: &str = "
    -- An entry whose value an agent had the Vault read (`import`, `capture`) and the person has not seen yet: a use of it
    -- never goes ahead on silence until they have marked it seen or answered a use of it once. `entry_created_at` ties the
    -- row to one incarnation of a reused id. One row per entry; a later change by an agent replaces it.
    CREATE TABLE IF NOT EXISTS db_vault_unseen (
        entry_id         TEXT PRIMARY KEY,
        entry_created_at INTEGER NOT NULL,
        by_agent         TEXT NOT NULL,
        at               INTEGER NOT NULL
    );
";

/// An entry's value an agent set and the person has not seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Unseen {
    pub by_agent: String,
    pub at: i64,
}

impl Store {
    /// Records that `by_agent` set the value of `entry_id` (made at `entry_created_at`) at `at`, unseen by the person.
    pub fn vault_unseen_mark(&self, entry_id: &str, entry_created_at: i64, by_agent: &str, at: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_vault_unseen (entry_id, entry_created_at, by_agent, at) VALUES (?1, ?2, ?3, ?4)",
            params![entry_id, entry_created_at, by_agent, at],
        )?;
        Ok(())
    }

    /// The unseen value of `entry_id`, if the entry that holds it is still the one it was set on.
    pub fn vault_unseen_get(&self, entry_id: &str) -> Result<Option<Unseen>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT u.by_agent, u.at FROM db_vault_unseen u JOIN db_vault_entries e ON e.id = u.entry_id AND e.created_at = u.entry_created_at WHERE u.entry_id = ?1",
                params![entry_id],
                |r| Ok(Unseen { by_agent: r.get(0)?, at: r.get(1)? }),
            )
            .optional()?)
    }

    /// The person has seen it, or answered a use of it: it is trusted as any other. True when there was a mark.
    pub fn vault_unseen_clear(&self, entry_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_vault_unseen WHERE entry_id = ?1", params![entry_id])? > 0)
    }
}
