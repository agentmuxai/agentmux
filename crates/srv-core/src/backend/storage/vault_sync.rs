// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Vault keeps for a Vault shared across machines, beside its own tables (`vault.rs`): an entry's link-id
//! aliases and the ids of removed entries, and the activity lines other machines made. The rules that use them are in
//! `backend::vault::sync_links` and `sync_activity`; nothing here holds a value.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;
use super::vault::VaultActivityRow;

pub(super) const SCHEMA: &str = "
    -- The other link ids of an entry (`db_vault_link_ids` holds the one this machine gave it): a link made on another
    -- machine of a shared Vault opens the entry here too. An id names one entry.
    CREATE TABLE IF NOT EXISTS db_vault_link_aliases (
        link_id  TEXT PRIMARY KEY,
        entry_id TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_vault_link_aliases_entry ON db_vault_link_aliases (entry_id);
    -- Ids that named an entry that was removed: never given to another.
    CREATE TABLE IF NOT EXISTS db_vault_link_retired (
        link_id TEXT PRIMARY KEY
    );
    -- Activity lines that other machines made, named by machine and the sequence number there. They hold no value, and
    -- they have no place in this machine's own hash chain.
    CREATE TABLE IF NOT EXISTS db_vault_activity_remote (
        machine     TEXT NOT NULL,
        seq         INTEGER NOT NULL,
        at          INTEGER NOT NULL,
        entry_id    TEXT NOT NULL,
        entry_label TEXT NOT NULL,
        actor       TEXT NOT NULL,
        action      TEXT NOT NULL,
        target      TEXT NOT NULL,
        outcome     TEXT NOT NULL,
        version     INTEGER NOT NULL,
        actor_kind  TEXT NOT NULL,
        host        TEXT NOT NULL,
        reason      TEXT NOT NULL,
        refs        TEXT NOT NULL,
        PRIMARY KEY (machine, seq)
    );
    CREATE INDEX IF NOT EXISTS idx_vault_activity_remote_at ON db_vault_activity_remote (at);
";

const REMOTE_COLUMNS: &str = "seq, at, entry_id, entry_label, actor, action, target, outcome, version, actor_kind, host, reason, refs";

impl Store {
    /// The link id this machine gave `entry_id` as its own, if it has one.
    pub fn vault_link_primary(&self, entry_id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT link_id FROM db_vault_link_ids WHERE entry_id = ?1", params![entry_id], |r| r.get(0)).optional()?)
    }

    /// The other ids that open `entry_id`, in order.
    pub fn vault_link_aliases(&self, entry_id: &str) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT link_id FROM db_vault_link_aliases WHERE entry_id = ?1 ORDER BY link_id")?;
        let rows = stmt.query_map(params![entry_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The entry that holds `link_id` now, as its own id or as an alias, whether or not the entry still exists.
    pub fn vault_link_holder(&self, link_id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT entry_id FROM db_vault_link_ids WHERE link_id = ?1 UNION ALL SELECT entry_id FROM db_vault_link_aliases WHERE link_id = ?1",
                params![link_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Whether `link_id` named an entry that was removed.
    pub fn vault_link_retired(&self, link_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM db_vault_link_retired WHERE link_id = ?1", params![link_id], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// Sets the ids of `entry_id`: `primary` is kept only if the entry has none yet, and its aliases are replaced by
    /// `aliases`. An id another entry holds, or a retired one, is not taken: the table holds each id once.
    pub fn vault_links_set(&self, entry_id: &str, primary: Option<&str>, aliases: &[String]) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let free = "NOT EXISTS (SELECT 1 FROM db_vault_link_retired WHERE link_id = ?2) AND NOT EXISTS (SELECT 1 FROM db_vault_link_ids WHERE link_id = ?2 AND entry_id != ?1) AND NOT EXISTS (SELECT 1 FROM db_vault_link_aliases WHERE link_id = ?2 AND entry_id != ?1)";
        if let Some(p) = primary {
            tx.execute(&format!("INSERT OR IGNORE INTO db_vault_link_ids (entry_id, link_id) SELECT ?1, ?2 WHERE {free}"), params![entry_id, p])?;
        }
        tx.execute("DELETE FROM db_vault_link_aliases WHERE entry_id = ?1", params![entry_id])?;
        for a in aliases {
            tx.execute(&format!("INSERT OR IGNORE INTO db_vault_link_aliases (link_id, entry_id) SELECT ?2, ?1 WHERE {free}"), params![entry_id, a])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Adds an activity line that machine `machine` made as its number `seq`. `false` if this machine already has it.
    pub fn vault_activity_remote_add(&self, machine: &str, seq: i64, r: &VaultActivityRow) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "INSERT OR IGNORE INTO db_vault_activity_remote (machine, seq, at, entry_id, entry_label, actor, action, target, outcome, version, actor_kind, host, reason, refs) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![machine, seq, r.at, r.entry_id, r.entry_label, r.actor, r.action, r.target, r.outcome, r.version, r.actor_kind, r.host, r.reason, r.refs],
        )?;
        Ok(n > 0)
    }

    pub fn vault_activity_remote_has(&self, machine: &str, seq: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM db_vault_activity_remote WHERE machine = ?1 AND seq = ?2", params![machine, seq], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// The latest `limit` lines other machines made, newest first, each with the machine that made it.
    pub fn vault_activity_remote_list(&self, limit: i64) -> Result<Vec<(String, VaultActivityRow)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT machine, {REMOTE_COLUMNS} FROM db_vault_activity_remote ORDER BY at DESC, machine, seq DESC LIMIT ?1"))?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                VaultActivityRow {
                    seq: r.get(1)?,
                    at: r.get(2)?,
                    entry_id: r.get(3)?,
                    entry_label: r.get(4)?,
                    actor: r.get(5)?,
                    action: r.get(6)?,
                    target: r.get(7)?,
                    outcome: r.get(8)?,
                    version: r.get(9)?,
                    actor_kind: r.get(10)?,
                    host: r.get(11)?,
                    reason: r.get(12)?,
                    refs: r.get(13)?,
                    prev_hash: String::new(),
                    hash: String::new(),
                },
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
