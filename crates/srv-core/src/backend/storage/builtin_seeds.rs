// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The record of what AgentMux has shipped into a store once (`backend::builtin_skills`): a key per seeded
//! item that stays after the user deletes the item, so a delete is final and the seed does not bring it
//! back at the next start. The table is created on first use, like `db_skill_reach`, with no schema bump.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_builtin_seeds (
        key       TEXT PRIMARY KEY,
        seeded_at INTEGER NOT NULL DEFAULT 0
    );";

impl Store {
    /// Whether the item `key` was seeded into this store before (and may since have been edited or deleted).
    pub fn builtin_seed_done(&self, key: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM db_builtin_seeds WHERE key = ?1", params![key], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// Records that `key` was seeded at `now_ms`. Marking twice keeps the first time.
    pub fn builtin_seed_mark(&self, key: &str, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute("INSERT OR IGNORE INTO db_builtin_seeds (key, seeded_at) VALUES (?1, ?2)", params![key, now_ms])?;
        Ok(())
    }
}
