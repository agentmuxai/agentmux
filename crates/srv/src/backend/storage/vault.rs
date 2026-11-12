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
        last_used_at INTEGER NOT NULL DEFAULT 0,
        vault_id     TEXT NOT NULL DEFAULT 'personal'
    );
    -- The person's Vaults on this machine: each entry belongs to exactly one. `agents` is who may
    -- use its entries: `all`, `none`, or `named` with `agent_ids` a JSON array of agent ids.
    -- `personal` is today's Vault, always there, and can't be deleted.
    CREATE TABLE IF NOT EXISTS db_vaults (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        agents     TEXT NOT NULL DEFAULT 'all',
        agent_ids  TEXT NOT NULL DEFAULT '[]',
        position   INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL DEFAULT 0
    );
    INSERT OR IGNORE INTO db_vaults (id, name, agents, position) VALUES ('personal', 'Personal', 'all', 0);
    -- The Vault each browser profile (`personal` or `profile:<id>`) saves its sign-ins to and offers
    -- them from. A profile with no row uses Personal.
    CREATE TABLE IF NOT EXISTS db_vault_signin_vaults (
        profile  TEXT PRIMARY KEY,
        vault_id TEXT NOT NULL
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
    -- An entry's own lines: its expiry reminders, and who made its value.
    CREATE INDEX IF NOT EXISTS idx_vault_activity_entry ON db_vault_activity (entry_id, seq);
    -- Where the person let an agent use an entry without asking again: the
    -- origin of a site, for one entry (spec section 6.4).
    CREATE TABLE IF NOT EXISTS db_vault_grants (
        entry_id   TEXT NOT NULL,
        origin     TEXT NOT NULL,
        granted_at INTEGER NOT NULL,
        PRIMARY KEY (entry_id, origin)
    );
    -- The sign-ins a browser pane saved for a site that asked for a user name and password (HTTP
    -- Basic Auth). The password is in the keychain under a hash, which can't be listed or turned
    -- back into a site; this row is what the Vault lists. It holds no secret: the site, the realm
    -- the site named, and the user name masked as the approval window shows it.
    CREATE TABLE IF NOT EXISTS db_browser_signins (
        identity_id TEXT NOT NULL,
        origin      TEXT NOT NULL,
        realm       TEXT NOT NULL,
        is_proxy    INTEGER NOT NULL,
        username    TEXT NOT NULL DEFAULT '',
        saved_at    INTEGER NOT NULL,
        PRIMARY KEY (identity_id, origin, realm, is_proxy)
    );
    -- Sites the person said never to offer saving a sign-in for, per browser profile
    -- (`personal` or `profile:<id>`).
    CREATE TABLE IF NOT EXISTS db_browser_signin_never (
        profile TEXT NOT NULL,
        origin  TEXT NOT NULL,
        at      INTEGER NOT NULL,
        PRIMARY KEY (profile, origin)
    );
    -- An entry's earlier values, sealed as the entry is (its id is the seal's
    -- associated data, so a blob moves between the two tables unchanged), kept 7
    -- days after they were replaced (`backend::vault::versions`). `entry_created_at`
    -- ties each to one incarnation of a reused id; `state` is `previous`, or
    -- `pending` for a rotation's value not yet in use.
    CREATE TABLE IF NOT EXISTS db_vault_versions (
        seq              INTEGER PRIMARY KEY AUTOINCREMENT,
        entry_id         TEXT NOT NULL,
        entry_created_at INTEGER NOT NULL,
        sealed           BLOB NOT NULL,
        replaced_at      INTEGER NOT NULL,
        replaced_by      TEXT NOT NULL DEFAULT '',
        how              TEXT NOT NULL DEFAULT '',
        state            TEXT NOT NULL DEFAULT 'previous'
    );
    CREATE INDEX IF NOT EXISTS idx_vault_versions_entry ON db_vault_versions (entry_id, seq);
    -- Which AgentMux instances show a Vault pane now, until when: agents'
    -- whole-window captures of AgentMux are refused while any does.
    CREATE TABLE IF NOT EXISTS db_vault_on_screen (
        instance TEXT PRIMARY KEY,
        until_ms INTEGER NOT NULL
    );
";

/// Columns added after the Vault's tables first shipped, applied like the identity store's others.
pub(super) const VAULT_ALTERS: [&str; 8] = [
    // Several Vaults: every entry made before belongs to `personal`.
    "ALTER TABLE db_vault_entries ADD COLUMN vault_id TEXT NOT NULL DEFAULT 'personal'",
    // The entry's history (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §5): rows made
    // before these columns keep the defaults, and have no hash.
    "ALTER TABLE db_vault_activity ADD COLUMN version INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE db_vault_activity ADD COLUMN actor_kind TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_vault_activity ADD COLUMN host TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_vault_activity ADD COLUMN reason TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_vault_activity ADD COLUMN refs TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_vault_activity ADD COLUMN prev_hash TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_vault_activity ADD COLUMN hash TEXT NOT NULL DEFAULT ''",
];

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
    /// The Vault the entry belongs to (`db_vaults.id`).
    pub vault_id: String,
}

/// A Vault of the person's on this machine.
#[derive(Debug, Clone, PartialEq)]
pub struct VaultInfoRow {
    pub id: String,
    pub name: String,
    /// `all`, `none` or `named`.
    pub agents: String,
    /// With `named`: the agents' ids, a JSON array.
    pub agent_ids: String,
    pub position: i64,
    pub created_at: i64,
}

/// The Vault every entry made before Vaults existed belongs to.
pub const PERSONAL_VAULT: &str = "personal";

/// How many additive changes the Vault's tables have had since they were first made: the revision `Vault health` names.
pub fn schema_revision() -> usize {
    VAULT_ALTERS.len()
}

/// The actions that give an entry a new value: each one is the next version of it.
pub const VALUE_SETTING: [&str; 7] = ["generated", "captured", "rotated", "replaced", "restored", "created", "imported"];

/// The actions that hand an entry's value to a process, a form or a sign-in: each is made under the version then current.
pub const VALUE_USES: [&str; 7] = ["filled a web form", "signed in", "ran a command", "revealed", "copied", "sent on sign-up", "filled on sign-up"];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VaultActivityRow {
    /// Order on this machine; the store gives it.
    pub seq: i64,
    pub at: i64,
    pub entry_id: String,
    pub entry_label: String,
    /// "you" for the person, else the agent's name.
    pub actor: String,
    pub action: String,
    /// Where: a site, a command, a file name.
    pub target: String,
    pub outcome: String,
    /// The entry's value version this event made or used; 0 for one that touches no value. The store gives it.
    pub version: i64,
    /// `agent`, `person` or `srv`.
    pub actor_kind: String,
    /// The machine's name.
    pub host: String,
    /// The agent's reason, one line.
    pub reason: String,
    /// Short references (a task, a PR), a JSON array, or empty.
    pub refs: String,
    /// The hash of the row before it in the chain, empty for the first chained row.
    pub prev_hash: String,
    /// This row's place in the chain; empty for a row made before the chain.
    pub hash: String,
}

/// A row's hash in the chain: SHA-256 over the previous row's hash and the row's own fields, each with its length in
/// front so two rows can't be cut into each other. No value goes in: a row holds none.
pub fn activity_hash(prev: &str, r: &VaultActivityRow) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let mut put = |bytes: &[u8]| {
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    };
    put(prev.as_bytes());
    for n in [r.seq, r.at, r.version] {
        put(&n.to_le_bytes());
    }
    for t in [&r.entry_id, &r.entry_label, &r.actor_kind, &r.actor, &r.host, &r.action, &r.target, &r.outcome, &r.reason, &r.refs] {
        put(t.as_bytes());
    }
    hex::encode(h.finalize())
}

const ACTIVITY_COLUMNS: &str = "seq, at, entry_id, entry_label, actor, action, target, outcome, version, actor_kind, host, reason, refs, prev_hash, hash";

fn activity_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<VaultActivityRow> {
    Ok(VaultActivityRow {
        seq: r.get(0)?,
        at: r.get(1)?,
        entry_id: r.get(2)?,
        entry_label: r.get(3)?,
        actor: r.get(4)?,
        action: r.get(5)?,
        target: r.get(6)?,
        outcome: r.get(7)?,
        version: r.get(8)?,
        actor_kind: r.get(9)?,
        host: r.get(10)?,
        reason: r.get(11)?,
        refs: r.get(12)?,
        prev_hash: r.get(13)?,
        hash: r.get(14)?,
    })
}

const COLUMNS: &str = "id, kind, label, fields, tags, sealed, created_at, updated_at, last_used_at, vault_id";

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
        vault_id: r.get(9)?,
    })
}

fn vault_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<VaultInfoRow> {
    Ok(VaultInfoRow { id: r.get(0)?, name: r.get(1)?, agents: r.get(2)?, agent_ids: r.get(3)?, position: r.get(4)?, created_at: r.get(5)? })
}

impl Store {
    pub fn vault_insert(&self, row: &VaultRow) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!("INSERT INTO db_vault_entries ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
            params![row.id, row.kind, row.label, row.fields, row.tags, row.sealed, row.created_at, row.updated_at, row.last_used_at, row.vault_id],
        )?;
        Ok(())
    }

    /// Replaces an entry's label, fields, tags and sealed parts. False if
    /// there is no such entry. Tests only: an edit goes through
    /// `vault_update_kept`, a capture through `vault_capture_commit`.
    #[cfg(test)]
    pub fn vault_update(&self, row: &VaultRow) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE db_vault_entries SET label = ?2, fields = ?3, tags = ?4, sealed = ?5, updated_at = ?6 WHERE id = ?1",
            params![row.id, row.label, row.fields, row.tags, row.sealed, row.updated_at],
        )?;
        Ok(n > 0)
    }

    /// Replaces only an entry's plain fields (a card's default flag moving to
    /// another card), leaving its value and its update time alone. False if
    /// there is no such entry.
    pub fn vault_set_fields(&self, id: &str, fields: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("UPDATE db_vault_entries SET fields = ?2 WHERE id = ?1", params![id, fields])?;
        Ok(n > 0)
    }

    /// Deletes an entry and the grants and earlier values it had.
    pub fn vault_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM db_vault_grants WHERE entry_id = ?1", params![id])?;
        tx.execute("DELETE FROM db_vault_versions WHERE entry_id = ?1", params![id])?;
        let n = tx.execute("DELETE FROM db_vault_entries WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// Notes that a browser pane saved a sign-in in the keychain (never its password):
    /// how sign-ins were kept before the Vault held them, for tests of the move.
    #[cfg(test)]
    pub fn browser_signin_note(&self, identity_id: &str, origin: &str, realm: &str, is_proxy: bool, masked_username: &str, at_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_browser_signins (identity_id, origin, realm, is_proxy, username, saved_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![identity_id, origin, realm, is_proxy, masked_username, at_ms],
        )?;
        Ok(())
    }

    /// Forgets the note of a sign-in that was deleted.
    pub fn browser_signin_forget(&self, identity_id: &str, origin: &str, realm: &str, is_proxy: bool) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_browser_signins WHERE identity_id = ?1 AND origin = ?2 AND realm = ?3 AND is_proxy = ?4", params![identity_id, origin, realm, is_proxy])?;
        Ok(())
    }

    /// Never offer saving a sign-in for `origin` in `profile`.
    pub fn browser_signin_never_add(&self, profile: &str, origin: &str, at_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR REPLACE INTO db_browser_signin_never (profile, origin, at) VALUES (?1, ?2, ?3)", params![profile, origin, at_ms])?;
        Ok(())
    }

    pub fn browser_signin_never_has(&self, profile: &str, origin: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM db_browser_signin_never WHERE profile = ?1 AND origin = ?2", params![profile, origin], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// Every noted sign-in's keychain address `(identity_id, origin, realm, is_proxy)`:
    /// what the Vault moves in from the keychain.
    pub fn browser_signins_saved(&self) -> Result<Vec<(String, String, String, bool)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT identity_id, origin, realm, is_proxy FROM db_browser_signins ORDER BY saved_at, origin, realm")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The sign-ins browser panes saved that the Vault hasn't moved in yet, newest
    /// first: `(origin, realm, is_proxy, masked username)`.
    pub fn browser_signins(&self) -> Result<Vec<(String, String, bool, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT origin, realm, is_proxy, username FROM db_browser_signins ORDER BY saved_at DESC, origin, realm")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn vault_grant_add(&self, entry_id: &str, origin: &str, at_ms: i64, entry_created_at: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Only for the entry the person was asked about: they have minutes to answer,
        // in which time it can be deleted, and a re-created one of the same name (its
        // id comes from its kind and label, its `created_at` is new) must not inherit
        // the site.
        conn.execute(
            "INSERT OR REPLACE INTO db_vault_grants (entry_id, origin, granted_at) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM db_vault_entries WHERE id = ?1 AND created_at = ?4)",
            params![entry_id, origin, at_ms, entry_created_at],
        )?;
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

    /// The person's Vaults, in their order.
    pub fn vaults(&self) -> Result<Vec<VaultInfoRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, name, agents, agent_ids, position, created_at FROM db_vaults ORDER BY position, created_at, id")?;
        let rows = stmt.query_map([], vault_of)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn vault_info(&self, id: &str) -> Result<Option<VaultInfoRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT id, name, agents, agent_ids, position, created_at FROM db_vaults WHERE id = ?1", params![id], vault_of)
            .optional()?)
    }

    /// Adds a Vault after the others.
    pub fn vault_add(&self, v: &VaultInfoRow) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_vaults (id, name, agents, agent_ids, position, created_at) \
             VALUES (?1, ?2, ?3, ?4, COALESCE((SELECT MAX(position) + 1 FROM db_vaults), 0), ?5)",
            params![v.id, v.name, v.agents, v.agent_ids, v.created_at],
        )?;
        Ok(())
    }

    /// Renames a Vault and sets who may use it. False if there is no such Vault.
    pub fn vault_set(&self, id: &str, name: &str, agents: &str, agent_ids: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("UPDATE db_vaults SET name = ?2, agents = ?3, agent_ids = ?4 WHERE id = ?1", params![id, name, agents, agent_ids])? > 0)
    }

    /// Removes an empty Vault. False if it still holds entries or there is no such Vault.
    pub fn vault_remove(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let held: i64 = tx.query_row("SELECT COUNT(*) FROM db_vault_entries WHERE vault_id = ?1", params![id], |r| r.get(0))?;
        if held > 0 {
            return Ok(false);
        }
        let n = tx.execute("DELETE FROM db_vaults WHERE id = ?1", params![id])?;
        // A profile that saved its sign-ins there goes back to Personal.
        tx.execute("DELETE FROM db_vault_signin_vaults WHERE vault_id = ?1", params![id])?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// Each browser profile's sign-in Vault, for the profiles that have one other than Personal.
    pub fn signin_vaults(&self) -> Result<Vec<(String, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT profile, vault_id FROM db_vault_signin_vaults ORDER BY profile")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The Vault `profile` keeps its sign-ins in, if it is not Personal.
    pub fn signin_vault(&self, profile: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT vault_id FROM db_vault_signin_vaults WHERE profile = ?1", params![profile], |r| r.get(0)).optional()?)
    }

    /// Sets the Vault `profile` keeps its sign-ins in; Personal clears the row.
    pub fn signin_vault_set(&self, profile: &str, vault_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        if vault_id == PERSONAL_VAULT {
            conn.execute("DELETE FROM db_vault_signin_vaults WHERE profile = ?1", params![profile])?;
        } else {
            conn.execute(
                "INSERT INTO db_vault_signin_vaults (profile, vault_id) VALUES (?1, ?2) ON CONFLICT(profile) DO UPDATE SET vault_id = ?2",
                params![profile, vault_id],
            )?;
        }
        Ok(())
    }

    /// Puts an entry in another Vault, keeping its id, versions and grants. False if there is no such entry.
    pub fn vault_move_entry(&self, entry_id: &str, vault_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("UPDATE db_vault_entries SET vault_id = ?2 WHERE id = ?1", params![entry_id, vault_id])? > 0)
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

    /// Inserts and updates many entries in one transaction: all of them or none.
    /// An update replaces the row's kind, label, fields, tags and sealed parts.
    pub fn vault_write_many(&self, inserts: &[VaultRow], updates: &[VaultRow]) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        for row in inserts {
            tx.execute(
                &format!("INSERT INTO db_vault_entries ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
                params![row.id, row.kind, row.label, row.fields, row.tags, row.sealed, row.created_at, row.updated_at, row.last_used_at, row.vault_id],
            )?;
        }
        for row in updates {
            let n = tx.execute(
                "UPDATE db_vault_entries SET kind = ?2, label = ?3, fields = ?4, tags = ?5, sealed = ?6, updated_at = ?7 WHERE id = ?1",
                params![row.id, row.kind, row.label, row.fields, row.tags, row.sealed, row.updated_at],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound);
            }
        }
        tx.commit()?;
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

    /// Appends a line to the activity log and chains it: the store gives it its `seq`, its `version` (the entry's
    /// count of value-setting lines that went through, plus this one if it is one) and its place in the hash
    /// chain, all in one transaction. Returns the line as stored.
    pub fn vault_activity_add(&self, row: &VaultActivityRow) -> Result<VaultActivityRow, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        // Immediate: the store is shared by every AgentMux process on the computer, and the chain's tip is read and then
        // written, so a second process must wait (busy_timeout) rather than read the same tip and lose its line.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut row = row.clone();
        row.seq = tx
            .query_row("SELECT seq FROM sqlite_sequence WHERE name = 'db_vault_activity'", [], |r| r.get::<_, i64>(0))
            .optional()?
            .unwrap_or(0)
            + 1;
        row.version = if row.entry_id.is_empty() { 0 } else { activity_version(&tx, &row)? };
        row.prev_hash = tx
            .query_row("SELECT hash FROM db_vault_activity WHERE hash != '' ORDER BY seq DESC LIMIT 1", [], |r| r.get::<_, String>(0))
            .optional()?
            .unwrap_or_default();
        row.hash = activity_hash(&row.prev_hash, &row);
        tx.execute(
            "INSERT INTO db_vault_activity (seq, at, entry_id, entry_label, actor, action, target, outcome, version, actor_kind, host, reason, refs, prev_hash, hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                row.seq, row.at, row.entry_id, row.entry_label, row.actor, row.action, row.target, row.outcome, row.version, row.actor_kind, row.host,
                row.reason, row.refs, row.prev_hash, row.hash
            ],
        )?;
        tx.commit()?;
        Ok(row)
    }

    /// The latest `limit` activity rows, newest first.
    pub fn vault_activity_list(&self, limit: i64) -> Result<Vec<VaultActivityRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {ACTIVITY_COLUMNS} FROM db_vault_activity ORDER BY seq DESC LIMIT ?1"))?;
        let rows = stmt.query_map(params![limit], activity_of)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every activity row, oldest first: what the hash chain is checked over.
    pub fn vault_activity_all(&self) -> Result<Vec<VaultActivityRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {ACTIVITY_COLUMNS} FROM db_vault_activity ORDER BY seq"))?;
        let rows = stmt.query_map([], activity_of)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One entry's latest `limit` activity rows of every action, from `since_ms` on (so a deleted entry whose id was
    /// reused lends the new one none of its lines), newest first.
    pub fn vault_activity_history(&self, entry_id: &str, since_ms: i64, limit: i64) -> Result<Vec<VaultActivityRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {ACTIVITY_COLUMNS} FROM db_vault_activity WHERE entry_id = ?1 AND at >= ?2 ORDER BY seq DESC LIMIT ?3"))?;
        let rows = stmt.query_map(params![entry_id, since_ms, limit], activity_of)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One entry's latest `limit` activity rows with one of `actions`, from
    /// `since_ms` on, newest first. Filtered by action, so its many uses never
    /// push these out; bounded by time, so a deleted entry whose id was reused
    /// lends the new one none of its lines.
    pub fn vault_activity_for_entry(&self, entry_id: &str, actions: &[&str], since_ms: i64, limit: i64) -> Result<Vec<VaultActivityRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let marks = vec!["?"; actions.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT {ACTIVITY_COLUMNS} FROM db_vault_activity WHERE entry_id = ? AND action IN ({marks}) AND at >= ? ORDER BY seq DESC LIMIT ?"
        ))?;
        let mut args: Vec<rusqlite::types::Value> = vec![entry_id.to_string().into()];
        args.extend(actions.iter().map(|a| rusqlite::types::Value::from(a.to_string())));
        args.push(since_ms.into());
        args.push(limit.into());
        let rows = stmt.query_map(rusqlite::params_from_iter(args), activity_of)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// The version `row` is made under: the number of the entry's value-setting lines that went through, one more when this
/// is one. A line that touches no value is 0. Counts the lines since the entry was last deleted, so one whose id came
/// round again starts at 1.
fn activity_version(tx: &rusqlite::Transaction<'_>, row: &VaultActivityRow) -> Result<i64, StoreError> {
    let sets = VALUE_SETTING.contains(&row.action.as_str());
    if !sets && !VALUE_USES.contains(&row.action.as_str()) {
        return Ok(0);
    }
    let since: i64 =
        tx.query_row("SELECT COALESCE(MAX(seq), 0) FROM db_vault_activity WHERE entry_id = ?1 AND action = 'deleted'", params![row.entry_id], |r| r.get(0))?;
    let marks = vec!["?"; VALUE_SETTING.len()].join(", ");
    let mut args: Vec<rusqlite::types::Value> = vec![row.entry_id.clone().into(), since.into()];
    args.extend(VALUE_SETTING.iter().map(|a| rusqlite::types::Value::from(a.to_string())));
    let made: i64 = tx.query_row(
        &format!("SELECT COUNT(*) FROM db_vault_activity WHERE entry_id = ? AND seq > ? AND outcome = 'done' AND action IN ({marks})"),
        rusqlite::params_from_iter(args),
        |r| r.get(0),
    )?;
    Ok(if sets && row.outcome == "done" { made + 1 } else { made })
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
            vault_id: PERSONAL_VAULT.into(),
        }
    }

    #[test]
    fn a_saved_browser_sign_in_is_listed_without_a_password_and_forgotten_when_deleted() {
        let s = store();
        s.browser_signin_note("id-a", "https://intranet.example.com", "Admin", false, "ab***", 10).unwrap();
        s.browser_signin_note("id-a", "https://proxy.example.com:8080", "", true, "*", 20).unwrap();
        // The same site saved again replaces its row, not adds one.
        s.browser_signin_note("id-a", "https://intranet.example.com", "Admin", false, "cd***", 30).unwrap();
        assert_eq!(
            s.browser_signins().unwrap(),
            vec![("https://intranet.example.com".to_string(), "Admin".to_string(), false, "cd***".to_string()), ("https://proxy.example.com:8080".to_string(), String::new(), true, "*".to_string())],
            "newest first"
        );
        // Another identity's sign-in for the same site is its own row, and deleting one leaves the other.
        s.browser_signin_note("id-b", "https://intranet.example.com", "Admin", false, "ef***", 40).unwrap();
        s.browser_signin_forget("id-a", "https://intranet.example.com", "Admin", false).unwrap();
        let left: Vec<String> = s.browser_signins().unwrap().into_iter().map(|r| r.3).collect();
        assert_eq!(left, vec!["ef***".to_string(), "*".to_string()]);
        // Forgetting one that isn't there is fine.
        s.browser_signin_forget("id-z", "https://nowhere.example", "", false).unwrap();
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
        // An entry deleted while its prompt was open: the late "Always" writes nothing.
        s.vault_grant_add("gone", "https://irs.gov", 1, 1).unwrap();
        assert!(s.vault_grants().unwrap().is_empty());
        s.vault_grant_add("a", "https://irs.gov", 1, 1).unwrap();
        s.vault_grant_add("a", "https://irs.gov", 2, 1).unwrap();
        s.vault_grant_add("a", "https://ssa.gov", 3, 1).unwrap();
        // The same id with another `created_at` is a different entry.
        s.vault_grant_add("b", "https://irs.gov", 4, 99).unwrap();
        assert!(!s.vault_grant_check("b", "https://irs.gov").unwrap());
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
                ..Default::default()
            })
            .unwrap();
        }
        let got = s.vault_activity_list(2).unwrap();
        assert_eq!(got.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(), ["deleted", "changed"]);
    }
}

impl Store {
    /// Runs `sql` on the identity store: how a test stands in for someone editing the database by hand.
    #[cfg(test)]
    pub fn vault_run_sql_for_tests(&self, sql: &str) {
        self.conn.lock().unwrap().execute_batch(sql).unwrap();
    }
}
