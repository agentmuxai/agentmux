// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's rows in the identity store: `db_vault_entries` (each entry,
//! its secret parts sealed by `backend::vault::seal`) and
//! `db_vault_activity` (what was done with them, never a value).
//! docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.5.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;

/// The Vault's tables, in the identity store (`run_identity_store_schema`
/// runs it): one store for every AgentMux on the computer, like the Vault key
/// in the keychain that seals the entries. `sealed` holds an entry's secret
/// parts, encrypted; no column holds a value.
pub(super) const VAULT_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_vault_entries (
        id           TEXT PRIMARY KEY,
        kind         TEXT NOT NULL,
        label        TEXT NOT NULL,
        fields       TEXT NOT NULL DEFAULT '{}',
        tags         TEXT NOT NULL DEFAULT '[]',
        sealed       BLOB NOT NULL,
        created_at   INTEGER NOT NULL DEFAULT 0,
        updated_at   INTEGER NOT NULL DEFAULT 0,
        last_used_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS db_vault_activity (
        seq         INTEGER PRIMARY KEY AUTOINCREMENT,
        at          INTEGER NOT NULL,
        entry_id    TEXT NOT NULL DEFAULT '',
        entry_label TEXT NOT NULL DEFAULT '',
        actor       TEXT NOT NULL DEFAULT '',
        action      TEXT NOT NULL,
        target      TEXT NOT NULL DEFAULT '',
        outcome     TEXT NOT NULL DEFAULT ''
    );
    -- Where the person let an agent use an entry without asking again: the
    -- origin of a site, for one entry (spec section 6.4).
    CREATE TABLE IF NOT EXISTS db_vault_grants (
        entry_id   TEXT NOT NULL,
        origin     TEXT NOT NULL,
        granted_at INTEGER NOT NULL,
        PRIMARY KEY (entry_id, origin)
    );
    -- Which AgentMux instances show a Vault pane now, until when: agents'
    -- whole-window captures of AgentMux are refused while any does.
    CREATE TABLE IF NOT EXISTS db_vault_on_screen (
        instance TEXT PRIMARY KEY,
        until_ms INTEGER NOT NULL
    );
";

#[derive(Debug, Clone, PartialEq)]
pub struct VaultRow {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// The plain fields, a JSON object.
    pub fields: String,
    /// Tags, a JSON array.
    pub tags: String,
    pub sealed: Vec<u8>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VaultActivityRow {
    pub at: i64,
    pub entry_id: String,
    pub entry_label: String,
    /// "you" for the person, else the agent's name.
    pub actor: String,
    pub action: String,
    /// Where: a site, a command, a file name.
    pub target: String,
    pub outcome: String,
}

const COLUMNS: &str = "id, kind, label, fields, tags, sealed, created_at, updated_at, last_used_at";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<VaultRow> {
    Ok(VaultRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        label: r.get(2)?,
        fields: r.get(3)?,
        tags: r.get(4)?,
        sealed: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
        last_used_at: r.get(8)?,
    })
}

impl Store {
    pub fn vault_insert(&self, row: &VaultRow) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!("INSERT INTO db_vault_entries ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
            params![row.id, row.kind, row.label, row.fields, row.tags, row.sealed, row.created_at, row.updated_at, row.last_used_at],
        )?;
        Ok(())
    }

    /// Replaces an entry's label, fields, tags and sealed parts. False if
    /// there is no such entry.
    pub fn vault_update(&self, row: &VaultRow) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE db_vault_entries SET label = ?2, fields = ?3, tags = ?4, sealed = ?5, updated_at = ?6 WHERE id = ?1",
            params![row.id, row.label, row.fields, row.tags, row.sealed, row.updated_at],
        )?;
        Ok(n > 0)
    }

    /// Deletes an entry and the grants it had.
    pub fn vault_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM db_vault_grants WHERE entry_id = ?1", params![id])?;
        let n = tx.execute("DELETE FROM db_vault_entries WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(n > 0)
    }

    pub fn vault_grant_add(&self, entry_id: &str, origin: &str, at_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR REPLACE INTO db_vault_grants (entry_id, origin, granted_at) VALUES (?1, ?2, ?3)", params![entry_id, origin, at_ms])?;
        Ok(())
    }

    pub fn vault_grant_revoke(&self, entry_id: &str, origin: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_vault_grants WHERE entry_id = ?1 AND origin = ?2", params![entry_id, origin])? > 0)
    }

    pub fn vault_grant_check(&self, entry_id: &str, origin: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM db_vault_grants WHERE entry_id = ?1 AND origin = ?2", params![entry_id, origin], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// Every entry's grants: entry id to its origins, oldest first.
    pub fn vault_grants(&self) -> Result<std::collections::HashMap<String, Vec<String>>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT entry_id, origin FROM db_vault_grants ORDER BY granted_at, origin")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for row in rows {
            let (id, origin) = row?;
            out.entry(id).or_default().push(origin);
        }
        Ok(out)
    }

    pub fn vault_get(&self, id: &str) -> Result<Option<VaultRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(&format!("SELECT {COLUMNS} FROM db_vault_entries WHERE id = ?1"), params![id], row_of)
            .optional()?)
    }

    /// Every entry, by label.
    pub fn vault_list(&self) -> Result<Vec<VaultRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM db_vault_entries ORDER BY label COLLATE NOCASE, id"))?;
        let rows = stmt.query_map([], row_of)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn vault_count(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT COUNT(*) FROM db_vault_entries", [], |r| r.get(0))?)
    }

    /// Notes that an entry was used at `at_ms`.
    pub fn vault_touch(&self, id: &str, at_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE db_vault_entries SET last_used_at = ?2 WHERE id = ?1", params![id, at_ms])?;
        Ok(())
    }

    /// Whether a MuxBus sign-in is kept, and whose, read from its row alone:
    /// its tokens are in the keychain, and reading them could ask the person.
    pub fn vault_kept_muxbus_user(&self) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT user_email FROM db_muxbus_credentials WHERE id = 'global'", [], |r| r.get(0))
            .optional()?)
    }

    /// Notes that `instance` shows a Vault pane until `until_ms`, or (0)
    /// that it no longer does. Notes expired by `now_ms` go too: a pane in a
    /// window that crashed never clears its own.
    pub fn vault_on_screen_set(&self, instance: &str, until_ms: i64, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_vault_on_screen WHERE until_ms <= ?1", params![now_ms])?;
        if until_ms <= 0 {
            conn.execute("DELETE FROM db_vault_on_screen WHERE instance = ?1", params![instance])?;
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO db_vault_on_screen (instance, until_ms) VALUES (?1, ?2)",
                params![instance, until_ms],
            )?;
        }
        Ok(())
    }

    /// Whether any AgentMux instance shows a Vault pane at `now_ms`.
    pub fn vault_on_screen(&self, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM db_vault_on_screen WHERE until_ms > ?1", params![now_ms], |r| r.get(0))?;
        Ok(n > 0)
    }

    pub fn vault_activity_add(&self, row: &VaultActivityRow) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_vault_activity (at, entry_id, entry_label, actor, action, target, outcome) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.at, row.entry_id, row.entry_label, row.actor, row.action, row.target, row.outcome],
        )?;
        Ok(())
    }

    /// The latest `limit` activity rows, newest first.
    pub fn vault_activity_list(&self, limit: i64) -> Result<Vec<VaultActivityRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT at, entry_id, entry_label, actor, action, target, outcome FROM db_vault_activity \
             ORDER BY seq DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(VaultActivityRow {
                at: r.get(0)?,
                entry_id: r.get(1)?,
                entry_label: r.get(2)?,
                actor: r.get(3)?,
                action: r.get(4)?,
                target: r.get(5)?,
                outcome: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.apply_identity_schema_for_tests().unwrap();
        s
    }

    fn row(id: &str, label: &str) -> VaultRow {
        VaultRow {
            id: id.into(),
            kind: "ssn".into(),
            label: label.into(),
            fields: "{}".into(),
            tags: "[]".into(),
            sealed: vec![1, 2, 3],
            created_at: 1,
            updated_at: 1,
            last_used_at: 0,
        }
    }

    #[test]
    fn entries_insert_update_list_and_delete() {
        let s = store();
        s.vault_insert(&row("b", "beta")).unwrap();
        s.vault_insert(&row("a", "Alpha")).unwrap();
        assert_eq!(s.vault_list().unwrap().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a", "b"], "by label");
        let mut changed = row("a", "Alpha 2");
        changed.sealed = vec![9];
        changed.updated_at = 5;
        assert!(s.vault_update(&changed).unwrap());
        let got = s.vault_get("a").unwrap().unwrap();
        assert_eq!((got.label.as_str(), got.sealed.clone(), got.created_at, got.updated_at), ("Alpha 2", vec![9], 1, 5));
        assert!(!s.vault_update(&row("missing", "x")).unwrap());
        assert!(s.vault_delete("a").unwrap());
        assert!(!s.vault_delete("a").unwrap());
        assert_eq!(s.vault_count().unwrap(), 1);
    }

    #[test]
    fn a_grant_is_for_one_entry_and_one_site_and_goes_with_the_entry() {
        let s = store();
        s.vault_insert(&row("a", "A")).unwrap();
        s.vault_insert(&row("b", "B")).unwrap();
        s.vault_grant_add("a", "https://irs.gov", 1).unwrap();
        s.vault_grant_add("a", "https://irs.gov", 2).unwrap();
        s.vault_grant_add("a", "https://ssa.gov", 3).unwrap();
        assert!(s.vault_grant_check("a", "https://irs.gov").unwrap());
        assert!(!s.vault_grant_check("a", "https://evil.example").unwrap(), "another site");
        assert!(!s.vault_grant_check("b", "https://irs.gov").unwrap(), "another entry");
        assert_eq!(s.vault_grants().unwrap()["a"], ["https://irs.gov", "https://ssa.gov"]);
        assert!(s.vault_grant_revoke("a", "https://irs.gov").unwrap());
        assert!(!s.vault_grant_revoke("a", "https://irs.gov").unwrap());
        assert!(!s.vault_grant_check("a", "https://irs.gov").unwrap(), "a revoked grant asks again");
        s.vault_delete("a").unwrap();
        assert!(s.vault_grants().unwrap().is_empty(), "deleting an entry deletes its grants");
    }

    #[test]
    fn on_screen_lasts_until_it_expires_or_is_cleared() {
        let s = store();
        assert!(!s.vault_on_screen(100).unwrap());
        s.vault_on_screen_set("a", 200, 100).unwrap();
        assert!(s.vault_on_screen(100).unwrap());
        assert!(!s.vault_on_screen(200).unwrap(), "expired");
        s.vault_on_screen_set("a", 300, 100).unwrap();
        s.vault_on_screen_set("a", 0, 100).unwrap();
        assert!(!s.vault_on_screen(100).unwrap(), "cleared");
        // A note that expired is deleted by the next write, not only ignored.
        s.vault_on_screen_set("crashed", 200, 100).unwrap();
        s.vault_on_screen_set("b", 900, 500).unwrap();
        let rows: i64 = s.conn.lock().unwrap().query_row("SELECT COUNT(*) FROM db_vault_on_screen", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn activity_lists_newest_first() {
        let s = store();
        for (i, action) in ["created", "changed", "deleted"].iter().enumerate() {
            s.vault_activity_add(&VaultActivityRow {
                at: i as i64,
                entry_id: "a".into(),
                entry_label: "Alpha".into(),
                actor: "you".into(),
                action: action.to_string(),
                target: String::new(),
                outcome: "done".into(),
            })
            .unwrap();
        }
        let got = s.vault_activity_list(2).unwrap();
        assert_eq!(got.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(), ["deleted", "changed"]);
    }
}
