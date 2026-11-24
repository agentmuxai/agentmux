// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Vault keeps for a Vault shared across machines, beside its own tables (`vault.rs`): an entry's link-id
//! aliases and the ids of removed entries, and the activity lines other machines made. The rules that use them are in
//! `backend::vault::sync_links` and `sync_activity`; nothing here holds a value.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;
use super::vault::{VaultActivityRow, VaultRow};
use super::vault_versions::VaultVersionRow;

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

/// What [`Store::vault_take_write`] writes.
pub struct TakeWrite<'a> {
    /// The entry's whole row, its sealed parts under this machine's own key. A new entry's `created_at` is the record's;
    /// an existing entry keeps its own.
    pub row: &'a VaultRow,
    /// The value the entry holds now, if it exists: it is written only if it still does.
    pub was: Option<&'a [u8]>,
    pub primary: Option<&'a str>,
    pub aliases: &'a [String],
    pub revive: Option<&'a str>,
    /// Versions the record's conflict keeps (`entry_created_at` is the entry's incarnation).
    pub kept: &'a [VaultVersionRow],
}

/// Sets the ids of `entry_id` inside a transaction: see [`Store::vault_links_set`].
fn links_set_in(tx: &rusqlite::Connection, entry_id: &str, primary: Option<&str>, aliases: &[String]) -> rusqlite::Result<()> {
    let free = "NOT EXISTS (SELECT 1 FROM db_vault_link_retired WHERE link_id = ?2) AND NOT EXISTS (SELECT 1 FROM db_vault_link_ids WHERE link_id = ?2 AND entry_id != ?1) AND NOT EXISTS (SELECT 1 FROM db_vault_link_aliases WHERE link_id = ?2 AND entry_id != ?1)";
    if let Some(p) = primary {
        tx.execute(&format!("INSERT OR IGNORE INTO db_vault_link_ids (entry_id, link_id) SELECT ?1, ?2 WHERE {free}"), params![entry_id, p])?;
    }
    tx.execute("DELETE FROM db_vault_link_aliases WHERE entry_id = ?1", params![entry_id])?;
    for a in aliases {
        tx.execute(&format!("INSERT OR IGNORE INTO db_vault_link_aliases (link_id, entry_id) SELECT ?2, ?1 WHERE {free}"), params![entry_id, a])?;
    }
    Ok(())
}

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
        links_set_in(&tx, entry_id, primary, aliases)?;
        tx.commit()?;
        Ok(())
    }

    /// What taking a record from another machine writes, in one transaction (`backend::vault::sync_take`): the entry's row
    /// (made if there is none, else its label, kind, fields, tags, value, Vault and update time replaced), its link ids, and
    /// the versions of it that the record's conflict keeps. If `was` is `Some`, the entry must still hold that sealed value:
    /// a change made here since it was read is never overwritten, and `false` is returned with nothing written. `revive` is
    /// an id the record itself names: if an entry that had it was deleted here, the record is a later change to that entry
    /// and takes the id back (every other retired id stays retired).
    pub fn vault_take_write(&self, w: &TakeWrite<'_>) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let row = w.row;
        match w.was {
            Some(was) => {
                // The incarnation (`created_at`) stays: the entry's earlier values are filed under it.
                let n = tx.execute(
                    "UPDATE db_vault_entries SET kind = ?2, label = ?3, fields = ?4, tags = ?5, sealed = ?6, updated_at = ?7, vault_id = ?8 WHERE id = ?1 AND sealed = ?9",
                    params![row.id, row.kind, row.label, row.fields, row.tags, row.sealed, row.updated_at, row.vault_id, was],
                )?;
                if n == 0 {
                    return Ok(false);
                }
            }
            None => {
                if let Some(id) = w.revive {
                    tx.execute("DELETE FROM db_vault_link_retired WHERE link_id = ?1", params![id])?;
                }
                tx.execute(
                    "INSERT INTO db_vault_entries (id, kind, label, fields, tags, sealed, created_at, updated_at, last_used_at, vault_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![row.id, row.kind, row.label, row.fields, row.tags, row.sealed, row.created_at, row.updated_at, row.last_used_at, row.vault_id],
                )?;
            }
        }
        links_set_in(&tx, &row.id, w.primary, w.aliases)?;
        for v in w.kept {
            tx.execute(
                "INSERT INTO db_vault_versions (entry_id, entry_created_at, sealed, replaced_at, replaced_by, how, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![v.entry_id, v.entry_created_at, v.sealed, v.replaced_at, v.replaced_by, v.how, v.state],
            )?;
        }
        tx.commit()?;
        Ok(true)
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
