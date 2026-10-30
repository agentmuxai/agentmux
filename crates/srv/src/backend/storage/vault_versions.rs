// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Vault entry's earlier values: `db_vault_versions` (schema in `vault.rs`).
//! Each row is a sealed blob as the entry held it, never a value.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct VaultVersionRow {
    pub seq: i64,
    pub entry_id: String,
    pub entry_created_at: i64,
    pub sealed: Vec<u8>,
    pub replaced_at: i64,
    /// Who put in the value that replaced it: "you" or an agent.
    pub replaced_by: String,
    /// How it was replaced: "replaced", "captured", "restored", …
    pub how: String,
    pub state: String,
}

const COLUMNS: &str = "seq, entry_id, entry_created_at, sealed, replaced_at, replaced_by, how, state";

/// One write of a command's capture (`backend::vault::capture`), committed with
/// the others it made or not at all.
pub enum CaptureWrite {
    /// A new entry.
    Insert(super::vault::VaultRow),
    /// A new sealed value for an entry, only if it is still the incarnation
    /// (`row.created_at`) and value (`was`) that were read; `kept` is the value it
    /// replaces, kept as a version.
    Replace { row: super::vault::VaultRow, was: Vec<u8>, kept: Option<VaultVersionRow> },
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<VaultVersionRow> {
    Ok(VaultVersionRow {
        seq: r.get(0)?,
        entry_id: r.get(1)?,
        entry_created_at: r.get(2)?,
        sealed: r.get(3)?,
        replaced_at: r.get(4)?,
        replaced_by: r.get(5)?,
        how: r.get(6)?,
        state: r.get(7)?,
    })
}

impl Store {
    /// Keeps an earlier value; returns its number.
    pub fn vault_version_add(&self, v: &VaultVersionRow) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_vault_versions (entry_id, entry_created_at, sealed, replaced_at, replaced_by, how, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![v.entry_id, v.entry_created_at, v.sealed, v.replaced_at, v.replaced_by, v.how, v.state],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// One incarnation's earlier values, newest first.
    pub fn vault_versions(&self, entry_id: &str, entry_created_at: i64) -> Result<Vec<VaultVersionRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM db_vault_versions WHERE entry_id = ?1 AND entry_created_at = ?2 ORDER BY seq DESC"
        ))?;
        let rows = stmt.query_map(params![entry_id, entry_created_at], row_of)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn vault_version_get(&self, seq: i64) -> Result<Option<VaultVersionRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row(&format!("SELECT {COLUMNS} FROM db_vault_versions WHERE seq = ?1"), params![seq], row_of).optional()?)
    }

    /// Makes version `seq` the entry's value, and keeps the value it replaces as
    /// `previous`, in one transaction. Only if the entry still holds `previous`'s
    /// value (same incarnation, same sealed blob): a value written since is never
    /// overwritten unkept. False if anything changed, or `seq` isn't the entry's.
    pub fn vault_version_swap(&self, entry_id: &str, seq: i64, previous: &VaultVersionRow, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let chosen: Option<Vec<u8>> = tx
            .query_row("SELECT sealed FROM db_vault_versions WHERE seq = ?1 AND entry_id = ?2", params![seq, entry_id], |r| r.get(0))
            .optional()?;
        let Some(sealed) = chosen else { return Ok(false) };
        if tx.execute(
            "UPDATE db_vault_entries SET sealed = ?2, updated_at = ?3 WHERE id = ?1 AND created_at = ?4 AND sealed = ?5",
            params![entry_id, sealed, now_ms, previous.entry_created_at, previous.sealed],
        )? == 0
        {
            return Ok(false);
        }
        tx.execute("DELETE FROM db_vault_versions WHERE seq = ?1", params![seq])?;
        tx.execute(
            "INSERT INTO db_vault_versions (entry_id, entry_created_at, sealed, replaced_at, replaced_by, how, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                previous.entry_id,
                previous.entry_created_at,
                previous.sealed,
                previous.replaced_at,
                previous.replaced_by,
                previous.how,
                previous.state
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Commits a capture's writes in one transaction. False, with nothing written,
    /// if an entry it replaces changed or was made again since it was read.
    pub fn vault_capture_commit(&self, writes: &[CaptureWrite]) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        for w in writes {
            match w {
                CaptureWrite::Insert(row) => {
                    tx.execute(
                        "INSERT INTO db_vault_entries (id, kind, label, fields, tags, sealed, created_at, updated_at, last_used_at)                          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![row.id, row.kind, row.label, row.fields, row.tags, row.sealed, row.created_at, row.updated_at, row.last_used_at],
                    )?;
                }
                CaptureWrite::Replace { row, was, kept } => {
                    let n = tx.execute(
                        "UPDATE db_vault_entries SET sealed = ?2, updated_at = ?3 WHERE id = ?1 AND created_at = ?4 AND sealed = ?5",
                        params![row.id, row.sealed, row.updated_at, row.created_at, was],
                    )?;
                    if n == 0 {
                        // Dropping `tx` rolls back what this capture wrote so far.
                        return Ok(false);
                    }
                    if let Some(v) = kept {
                        tx.execute(
                            "INSERT INTO db_vault_versions (entry_id, entry_created_at, sealed, replaced_at, replaced_by, how, state)                              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![v.entry_id, v.entry_created_at, v.sealed, v.replaced_at, v.replaced_by, v.how, v.state],
                        )?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(true)
    }

    /// Deletes the versions replaced before `before_ms`; returns how many.
    pub fn vault_versions_prune(&self, before_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_vault_versions WHERE replaced_at < ?1", params![before_ms])?)
    }

    /// How many earlier values each entry has, for the pane's list.
    pub fn vault_version_counts(&self) -> Result<std::collections::HashMap<String, (i64, u32)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT entry_id, entry_created_at, COUNT(*) FROM db_vault_versions GROUP BY entry_id, entry_created_at")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)?, r.get::<_, u32>(2)?))))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
