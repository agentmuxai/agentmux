// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Workspaces as the user means them: a named set of windows, saved as they
//! change (docs/specs/SPEC_WORKSPACES_2026_10_23.md §2, §4.2). Not the srv
//! `Workspace` object, which is one window's contents.
//!
//! One row per workspace in this instance's `objects.db`. `record` is the
//! whole set of windows in the session record's shape
//! (`session_windows::snapshot_session`, plus `saved_at_ms`), so the restore
//! that reads a session record reads it too. `previous` is the last record
//! that loaded cleanly, replaced only after a clean load; `pre_upgrade` is the
//! record a new version copied from the one before it. All three are JSON
//! text, kept as text: one that doesn't parse is a fallback case, not an
//! error reading the row.
//!
//! Also here: [`LayoutWrites`], the counter of writes to the objects a
//! workspace is saved from, which says when it has changed since its last save.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::store::{Store, StoreTx};
use super::StoreError;

/// The table, created on every open of `objects.db`, with no schema version
/// bump (see `work_claims::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_workspaces (
        id          TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        record      TEXT,
        previous    TEXT,
        pre_upgrade TEXT,
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        opened_at   INTEGER
    );";

const COLS: &str = "id, name, record, previous, pre_upgrade, created_at, updated_at, opened_at";

/// One workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserWorkspaceRow {
    pub id: String,
    pub name: String,
    /// Its windows as last saved; `None` until the first save.
    pub record: Option<String>,
    /// The last record that loaded cleanly.
    pub previous: Option<String>,
    /// The record the previous version had, copied at this version's first launch.
    pub pre_upgrade: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// When it was last opened; `None` if never.
    pub opened_at: Option<i64>,
}

fn row_to_workspace(row: &Row) -> rusqlite::Result<UserWorkspaceRow> {
    Ok(UserWorkspaceRow {
        id: row.get("id")?,
        name: row.get("name")?,
        record: row.get("record")?,
        previous: row.get("previous")?,
        pre_upgrade: row.get("pre_upgrade")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        opened_at: row.get("opened_at")?,
    })
}

/// Every row of `db_workspaces` in `conn`, the most recently opened first. A
/// database without the table (one an older version wrote) errors.
pub(crate) fn list_in(conn: &Connection) -> Result<Vec<UserWorkspaceRow>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM db_workspaces ORDER BY COALESCE(opened_at, 0) DESC, updated_at DESC, rowid"
    ))?;
    let rows = stmt.query_map([], row_to_workspace)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn insert_in(conn: &Connection, ws: &UserWorkspaceRow) -> Result<(), StoreError> {
    conn.execute(
        &format!("INSERT INTO db_workspaces ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"),
        params![
            ws.id,
            ws.name,
            ws.record,
            ws.previous,
            ws.pre_upgrade,
            ws.created_at,
            ws.updated_at,
            ws.opened_at
        ],
    )?;
    Ok(())
}

fn rename_in(conn: &Connection, id: &str, name: &str, now_ms: i64) -> Result<bool, StoreError> {
    let n = conn.execute(
        "UPDATE db_workspaces SET name = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, name, now_ms],
    )?;
    Ok(n > 0)
}

impl Store {
    pub fn user_workspace_get(&self, id: &str) -> Result<Option<UserWorkspaceRow>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(&format!("SELECT {COLS} FROM db_workspaces WHERE id = ?1"), params![id], row_to_workspace)
            .optional()?)
    }

    /// Every workspace, the most recently opened first.
    pub fn user_workspace_list(&self) -> Result<Vec<UserWorkspaceRow>, StoreError> {
        list_in(&self.conn().lock().unwrap())
    }

    pub fn user_workspace_insert(&self, ws: &UserWorkspaceRow) -> Result<(), StoreError> {
        insert_in(&self.conn().lock().unwrap(), ws)
    }

    /// Store `record` as workspace `id`'s windows. Returns whether the row exists.
    pub fn user_workspace_save_record(&self, id: &str, record: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_workspaces SET record = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, record, now_ms],
        )?;
        Ok(n > 0)
    }

    /// Keep `record` as workspace `id`'s last clean load.
    pub fn user_workspace_set_previous(&self, id: &str, record: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute("UPDATE db_workspaces SET previous = ?2 WHERE id = ?1", params![id, record])?;
        Ok(n > 0)
    }

    pub fn user_workspace_mark_opened(&self, id: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute("UPDATE db_workspaces SET opened_at = ?2 WHERE id = ?1", params![id, now_ms])?;
        Ok(n > 0)
    }

    /// Delete workspace `id`'s row. Returns whether it existed.
    pub fn user_workspace_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute("DELETE FROM db_workspaces WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// The writes to windows, window contents, tabs, layouts and blocks.
    pub fn layout_writes(&self) -> &LayoutWrites {
        &self.layout_writes
    }
}

impl StoreTx<'_> {
    /// [`Store::user_workspace_insert`], inside a transaction.
    pub fn user_workspace_insert(&self, ws: &UserWorkspaceRow) -> Result<(), StoreError> {
        insert_in(self.raw_conn(), ws)
    }

    /// Rename workspace `id`, inside a transaction. Returns whether it exists.
    pub fn user_workspace_rename(&self, id: &str, name: &str, now_ms: i64) -> Result<bool, StoreError> {
        rename_in(self.raw_conn(), id, name, now_ms)
    }
}

/// How many times the objects a workspace is saved from (window, window
/// contents, tab, layout, block) were written, and how many of those writes
/// the last save covered. Bumped by the store's own writes, so no writer can
/// miss it; a relaxed atomic add, no lock and no allocation.
#[derive(Debug, Default)]
pub struct LayoutWrites {
    changed: AtomicU64,
    /// The writes that change the arrangement: everything but a block, whose
    /// settings an agent pane writes all the time as it works.
    arranged: AtomicU64,
    saved: AtomicU64,
    /// Set while a workspace switch moves the windows from one workspace to
    /// the other: a save then would store a mix of the two.
    held: AtomicBool,
}

impl LayoutWrites {
    /// Whether `otype` is one of the objects a workspace is saved from.
    pub(crate) fn tracks(otype: &str) -> bool {
        matches!(otype, "window" | "workspace" | "tab" | "layout" | "block")
    }

    /// Count a write of an object of `otype`.
    pub(crate) fn wrote(&self, otype: &str) {
        if Self::tracks(otype) {
            self.changed.fetch_add(1, Ordering::Relaxed);
            if otype != "block" {
                self.arranged.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// The count of writes that changed the arrangement so far.
    pub fn arranged(&self) -> u64 {
        self.arranged.load(Ordering::Relaxed)
    }

    /// The count of writes so far.
    pub fn changed(&self) -> u64 {
        self.changed.load(Ordering::Relaxed)
    }

    /// A save covered every write up to `generation` (a [`Self::changed`]
    /// read before it took its snapshot). Never moves backwards.
    pub fn mark_saved(&self, generation: u64) {
        self.saved.fetch_max(generation, Ordering::Relaxed);
    }

    /// Whether anything was written since the last save covered it.
    pub fn dirty(&self) -> bool {
        self.changed() > self.saved.load(Ordering::Relaxed)
    }

    /// Hold saves (`true`) or let them run again (`false`).
    pub fn hold_saves(&self, held: bool) {
        self.held.store(held, Ordering::SeqCst);
    }

    /// Whether saves are held.
    pub fn saves_held(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000_000_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("objects.db")).unwrap();
        (dir, s)
    }

    fn ws(id: &str, name: &str, opened_at: Option<i64>) -> UserWorkspaceRow {
        UserWorkspaceRow {
            id: id.into(),
            name: name.into(),
            created_at: NOW,
            updated_at: NOW,
            opened_at,
            ..Default::default()
        }
    }

    #[test]
    fn the_table_is_created_on_open() {
        let (_d, s) = store();
        assert!(s.user_workspace_list().unwrap().is_empty());
        assert!(s.user_workspace_get("nope").unwrap().is_none());
    }

    #[test]
    fn insert_then_get_round_trips() {
        let (_d, s) = store();
        let mut w = ws("a", "Default Workspace", Some(NOW));
        w.record = Some(r#"{"windows":[]}"#.into());
        w.pre_upgrade = Some("{}".into());
        s.user_workspace_insert(&w).unwrap();
        assert_eq!(s.user_workspace_get("a").unwrap(), Some(w));
    }

    #[test]
    fn list_puts_the_most_recently_opened_first() {
        let (_d, s) = store();
        s.user_workspace_insert(&ws("never", "Never opened", None)).unwrap();
        s.user_workspace_insert(&ws("old", "Old", Some(NOW - 10))).unwrap();
        s.user_workspace_insert(&ws("new", "New", Some(NOW))).unwrap();
        let ids: Vec<String> = s.user_workspace_list().unwrap().into_iter().map(|w| w.id).collect();
        assert_eq!(ids, vec!["new", "old", "never"]);
    }

    #[test]
    fn save_record_and_previous_update_only_their_row() {
        let (_d, s) = store();
        s.user_workspace_insert(&ws("a", "A", None)).unwrap();
        s.user_workspace_insert(&ws("b", "B", None)).unwrap();
        assert!(s.user_workspace_save_record("a", "{\"x\":1}", NOW + 5).unwrap());
        assert!(s.user_workspace_set_previous("a", "{\"x\":0}").unwrap());
        assert!(s.user_workspace_mark_opened("a", NOW + 6).unwrap());
        let a = s.user_workspace_get("a").unwrap().unwrap();
        assert_eq!(a.record.as_deref(), Some("{\"x\":1}"));
        assert_eq!(a.previous.as_deref(), Some("{\"x\":0}"));
        assert_eq!(a.updated_at, NOW + 5);
        assert_eq!(a.opened_at, Some(NOW + 6));
        assert_eq!(s.user_workspace_get("b").unwrap().unwrap(), ws("b", "B", None));
        // An unknown id writes nothing, and says so.
        assert!(!s.user_workspace_save_record("zzz", "{}", NOW).unwrap());
    }

    #[test]
    fn delete_removes_only_its_row() {
        let (_d, s) = store();
        s.user_workspace_insert(&ws("a", "A", None)).unwrap();
        s.user_workspace_insert(&ws("b", "B", None)).unwrap();
        assert!(s.user_workspace_delete("a").unwrap());
        assert!(!s.user_workspace_delete("a").unwrap(), "already gone");
        let ids: Vec<String> = s.user_workspace_list().unwrap().into_iter().map(|w| w.id).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn rename_inside_a_transaction() {
        let (_d, s) = store();
        s.user_workspace_insert(&ws("a", "A", None)).unwrap();
        let renamed = s.with_tx(|tx| tx.user_workspace_rename("a", "Reviews", NOW + 1)).unwrap();
        assert!(renamed);
        let a = s.user_workspace_get("a").unwrap().unwrap();
        assert_eq!((a.name.as_str(), a.updated_at), ("Reviews", NOW + 1));
        // A failed transaction leaves the name as it was.
        let failed: Result<(), StoreError> = s.with_tx(|tx| {
            tx.user_workspace_rename("a", "Lost", NOW + 2)?;
            Err(StoreError::Other("abort".into()))
        });
        assert!(failed.is_err());
        assert_eq!(s.user_workspace_get("a").unwrap().unwrap().name, "Reviews");
    }

    #[test]
    fn layout_writes_count_only_the_objects_a_workspace_is_saved_from() {
        let w = LayoutWrites::default();
        assert!(!w.dirty());
        w.wrote("client");
        w.wrote("temp");
        assert!(!w.dirty(), "the client row is not part of a workspace");
        for otype in ["window", "workspace", "tab", "layout", "block"] {
            w.wrote(otype);
        }
        assert_eq!(w.changed(), 5);
        assert!(w.dirty());
        w.mark_saved(3);
        assert!(w.dirty(), "two writes came after what the save covered");
        w.mark_saved(5);
        assert!(!w.dirty());
        w.mark_saved(1);
        assert!(!w.dirty(), "an older save never marks newer writes unsaved");
    }

    #[test]
    fn the_store_counts_its_own_object_writes() {
        use crate::backend::obj::{Client, Tab};
        let (_d, s) = store();
        let before = s.layout_writes().changed();
        let mut client = Client { oid: "c".into(), ..Default::default() };
        s.insert(&mut client).unwrap();
        s.update(&mut client).unwrap();
        assert_eq!(s.layout_writes().changed(), before, "client writes aren't counted");
        let mut tab = Tab { oid: "t".into(), ..Default::default() };
        s.insert(&mut tab).unwrap();
        s.update(&mut tab).unwrap();
        s.with_tx(|tx| tx.update(&mut tab)).unwrap();
        s.delete::<Tab>("t").unwrap();
        assert_eq!(s.layout_writes().changed(), before + 4);
    }
}
