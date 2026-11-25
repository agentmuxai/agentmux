// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Vault keeps of changes agents made, for the person to look at (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md):
//! which entries hold a value an agent had the Vault read (an import or a capture), or a site an agent chose, that the person
//! has not yet seen; the details an agent's edit replaced, kept 7 days; and the changes of where a login is offered that an
//! agent proposed on a login it did not make. The rules are in `backend::vault::agent_set`, `details` and `proposals`;
//! nothing here holds a value.

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

    -- The details an agent's edit replaced (label, plain fields, tags: never a value), kept 7 days so the person can put them
    -- back. Written in the same transaction as the change.
    CREATE TABLE IF NOT EXISTS db_vault_details_kept (
        id               INTEGER PRIMARY KEY AUTOINCREMENT,
        entry_id         TEXT NOT NULL,
        entry_created_at INTEGER NOT NULL,
        label            TEXT NOT NULL,
        fields           TEXT NOT NULL,
        tags             TEXT NOT NULL,
        changed_at       INTEGER NOT NULL,
        changed_by       TEXT NOT NULL,
        what             TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS db_vault_details_kept_entry ON db_vault_details_kept (entry_id, entry_created_at);

    -- A change of where a login is offered (site, realm, proxy) that an agent asked for on a login it did not make. It is
    -- only a record until the person accepts it; one per entry and field, kept 7 days.
    CREATE TABLE IF NOT EXISTS db_vault_proposals (
        entry_id         TEXT NOT NULL,
        field            TEXT NOT NULL,
        entry_created_at INTEGER NOT NULL,
        entry_updated_at INTEGER NOT NULL,
        value            TEXT NOT NULL,
        agent            TEXT NOT NULL,
        reason           TEXT NOT NULL,
        at               INTEGER NOT NULL,
        PRIMARY KEY (entry_id, field)
    );

    -- Which changes by agents the person has marked seen, on this machine only (never synced): one by one, or every change up
    -- to a line (`Seen all`).
    CREATE TABLE IF NOT EXISTS db_vault_changes_seen (
        seq INTEGER PRIMARY KEY
    );
    CREATE TABLE IF NOT EXISTS db_vault_changes_seen_all (
        id          INTEGER PRIMARY KEY CHECK (id = 1),
        through_seq INTEGER NOT NULL
    );
";
/// Added to a table an older build made.
pub(super) const COLUMNS: [&str; 1] = ["ALTER TABLE db_vault_unseen ADD COLUMN what TEXT NOT NULL DEFAULT 'value'"];

/// An entry's value an agent set and the person has not seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Unseen {
    pub by_agent: String,
    pub at: i64,
    /// What the agent set: `value`, `site` or `value and site`.
    pub what: String,
}

/// The two marks an agent's changes can leave on an entry.
pub const VALUE: &str = "value";
pub const SITE: &str = "site";

/// `old` and `new` together: an entry whose value and site an agent each set is marked for both.
fn merged(old: &str, new: &str) -> String {
    let (value, site) = (old.contains(VALUE) || new.contains(VALUE), old.contains(SITE) || new.contains(SITE));
    match (value, site) {
        (true, true) => format!("{VALUE} and {SITE}"),
        (false, true) => SITE.to_string(),
        _ => VALUE.to_string(),
    }
}

/// The details an agent's edit replaced.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailsKept {
    pub entry_id: String,
    pub entry_created_at: i64,
    pub label: String,
    pub fields: String,
    pub tags: String,
    pub changed_at: i64,
    pub changed_by: String,
    /// What the edit changed, in names (`label`, `field site`).
    pub what: String,
}

/// An agent's request to change where a login is offered, waiting for the person.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub entry_id: String,
    pub field: String,
    pub entry_created_at: i64,
    pub entry_updated_at: i64,
    pub value: String,
    pub agent: String,
    pub reason: String,
    pub at: i64,
}

/// [`Store::vault_unseen_mark`] inside the caller's transaction.
fn mark_in(tx: &rusqlite::Transaction<'_>, entry_id: &str, entry_created_at: i64, by_agent: &str, at: i64, what: &str) -> Result<(), StoreError> {
    tx.execute("DELETE FROM db_vault_grants WHERE entry_id = ?1", params![entry_id])?;
    let before: Option<String> = tx.query_row("SELECT what FROM db_vault_unseen WHERE entry_id = ?1 AND entry_created_at = ?2", params![entry_id, entry_created_at], |r| r.get(0)).optional()?;
    let what = before.map_or_else(|| what.to_string(), |old| merged(&old, what));
    tx.execute(
        "INSERT OR REPLACE INTO db_vault_unseen (entry_id, entry_created_at, by_agent, at, what) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![entry_id, entry_created_at, by_agent, at, what],
    )?;
    Ok(())
}

impl Store {
    /// Records that `by_agent` set the `what` (value or site) of `entry_id` (made at `entry_created_at`) at `at`, unseen by the person. The
    /// entry's *Always* grants go in the same step: they were given for the value the person saw, and several uses skip the
    /// card on a grant, so a value an agent replaced must ask again.
    pub fn vault_unseen_mark(&self, entry_id: &str, entry_created_at: i64, by_agent: &str, at: i64, what: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        mark_in(&tx, entry_id, entry_created_at, by_agent, at, what)?;
        tx.commit()?;
        Ok(())
    }

    /// The unseen value of `entry_id`, if the entry that holds it is still the one it was set on.
    pub fn vault_unseen_get(&self, entry_id: &str) -> Result<Option<Unseen>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT u.by_agent, u.at, u.what FROM db_vault_unseen u JOIN db_vault_entries e ON e.id = u.entry_id AND e.created_at = u.entry_created_at WHERE u.entry_id = ?1",
                params![entry_id],
                |r| Ok(Unseen { by_agent: r.get(0)?, at: r.get(1)?, what: r.get(2)? }),
            )
            .optional()?)
    }

    /// The person has seen it, or answered a use of it: it is trusted as any other. True when there was a mark.
    pub fn vault_unseen_clear(&self, entry_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_vault_unseen WHERE entry_id = ?1", params![entry_id])? > 0)
    }

    /// An edit of an entry's details (label, plain fields, tags), only if it still holds the sealed value `was` and the fields
    /// `was_fields`, label and tags that were read (`kept` holds them: a change of only the label or the tags is not
    /// overwritten by an older read), and the details it replaces kept in the same transaction. False, with nothing written, if
    /// the entry changed or is gone. With `unseen_site_by` (the agent), the entry's place is marked unseen and its *Always*
    /// grants dropped in the same transaction: a change of where a login is offered is never made without the mark.
    pub fn vault_update_details_kept(&self, row: &super::vault::VaultRow, was: &[u8], was_fields: &str, kept: &DetailsKept, unseen_site_by: Option<&str>) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE db_vault_entries SET label = ?2, fields = ?3, tags = ?4, updated_at = ?5 WHERE id = ?1 AND created_at = ?6 AND sealed = ?7 AND fields = ?8 AND label = ?9 AND tags = ?10",
            params![row.id, row.label, row.fields, row.tags, row.updated_at, row.created_at, was, was_fields, kept.label, kept.tags],
        )?;
        if n == 0 {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO db_vault_details_kept (entry_id, entry_created_at, label, fields, tags, changed_at, changed_by, what) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![kept.entry_id, kept.entry_created_at, kept.label, kept.fields, kept.tags, kept.changed_at, kept.changed_by, kept.what],
        )?;
        if let Some(agent) = unseen_site_by {
            mark_in(&tx, &row.id, row.created_at, agent, row.updated_at, SITE)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// The details earlier edits of the entry replaced, newest first.
    pub fn vault_details_kept(&self, entry_id: &str, entry_created_at: i64) -> Result<Vec<DetailsKept>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT entry_id, entry_created_at, label, fields, tags, changed_at, changed_by, what FROM db_vault_details_kept WHERE entry_id = ?1 AND entry_created_at = ?2 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![entry_id, entry_created_at], |r| {
            Ok(DetailsKept { entry_id: r.get(0)?, entry_created_at: r.get(1)?, label: r.get(2)?, fields: r.get(3)?, tags: r.get(4)?, changed_at: r.get(5)?, changed_by: r.get(6)?, what: r.get(7)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Drops the kept details older than `before_ms`.
    pub fn vault_details_prune(&self, before_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_vault_details_kept WHERE changed_at < ?1", params![before_ms])?)
    }

    /// Records `p`, replacing the one for the same entry and field.
    pub fn vault_proposal_put(&self, p: &Proposal) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_vault_proposals (entry_id, field, entry_created_at, entry_updated_at, value, agent, reason, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![p.entry_id, p.field, p.entry_created_at, p.entry_updated_at, p.value, p.agent, p.reason, p.at],
        )?;
        Ok(())
    }

    /// The proposals not older than `since_ms`, for one entry or (`None`) every entry, oldest first.
    pub fn vault_proposals(&self, entry_id: Option<&str>, since_ms: i64) -> Result<Vec<Proposal>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT entry_id, field, entry_created_at, entry_updated_at, value, agent, reason, at FROM db_vault_proposals WHERE at >= ?1 AND (?2 IS NULL OR entry_id = ?2) ORDER BY at, entry_id, field",
        )?;
        let rows = stmt.query_map(params![since_ms, entry_id], |r| {
            Ok(Proposal { entry_id: r.get(0)?, field: r.get(1)?, entry_created_at: r.get(2)?, entry_updated_at: r.get(3)?, value: r.get(4)?, agent: r.get(5)?, reason: r.get(6)?, at: r.get(7)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Drops a proposal. True when there was one.
    pub fn vault_proposal_delete(&self, entry_id: &str, field: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_vault_proposals WHERE entry_id = ?1 AND field = ?2", params![entry_id, field])? > 0)
    }

    /// The person marks the change recorded by history line `seq` as seen.
    pub fn vault_change_seen(&self, seq: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR IGNORE INTO db_vault_changes_seen (seq) VALUES (?1)", params![seq])?;
        Ok(())
    }

    /// The person marks every change up to history line `through_seq` as seen. The one-by-one marks it covers go.
    pub fn vault_changes_seen_all(&self, through_seq: i64) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO db_vault_changes_seen_all (id, through_seq) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET through_seq = MAX(through_seq, excluded.through_seq)",
            params![through_seq],
        )?;
        tx.execute("DELETE FROM db_vault_changes_seen WHERE seq <= ?1", params![through_seq])?;
        tx.commit()?;
        Ok(())
    }

    /// What the person has marked seen: every change up to the first number, and the lines in the set.
    pub fn vault_changes_seen(&self) -> Result<(i64, std::collections::HashSet<i64>), StoreError> {
        let conn = self.conn.lock().unwrap();
        let through: i64 = conn.query_row("SELECT through_seq FROM db_vault_changes_seen_all WHERE id = 1", [], |r| r.get(0)).optional()?.unwrap_or(0);
        let mut stmt = conn.prepare("SELECT seq FROM db_vault_changes_seen")?;
        let seqs = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<Result<_, _>>()?;
        Ok((through, seqs))
    }

    /// Drops the proposals older than `before_ms`.
    pub fn vault_proposals_prune(&self, before_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_vault_proposals WHERE at < ?1", params![before_ms])?)
    }
}
