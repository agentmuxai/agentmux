// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The channel-wide WAN identity store, `<channel dir>/wan-identity/wan.db`
//! (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.2, step D1).
//!
//! Everything WAN verification needs to survive an upgrade lives here rather
//! than in `objects.db`, which is per *version* for installed builds
//! (`channels/<ch>/versions/<v>/data/`): with the agent keys in `objects.db`,
//! every upgrade re-minted every agent's WAN key. This file holds:
//!  - the **instance keypair** — the install's long-lived identity. Its id
//!    (`jekt_sign::wan_instance_id`) is what agents sign as their
//!    `AGENTMUX_HOST_LABEL`, and its key certifies each agent key;
//!  - the **agent WAN keys**, imported once from the version's
//!    `db_agent_wan_keys` (or minted fresh) and kept here from then on;
//!  - (D1b/D2) published fingerprints, the peer cache, replay and
//!    known-instance tables.
//!
//! **Concurrency.** Two versions of one channel may run at once
//! (`data_paths.rs`), so this file is shared between processes: WAL, a 5 s
//! busy timeout, and every mint is `INSERT OR IGNORE` then re-read, so two
//! processes minting at once converge on one row.
//!
//! **Schema is additive only.** `wan_meta.schema_version` records the newest
//! schema any binary has written; an older binary opening a newer file uses
//! explicit column lists and ignores what it doesn't know. A binary that
//! cannot open the file gets no store at all — WAN signing and verification
//! are then off for that process (every message `None`), and nothing is ever
//! minted into a fallback location.
//!
//! **Lock order.** The spawn path reads an agent's old `objects.db` key
//! *before* calling in here, and the agent-delete purge calls in here while
//! holding the `objects.db` lock. So the order is always `objects.db` →
//! `wan.db`; nothing here ever takes the `objects.db` lock.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};

use super::error::StoreError;

/// Bumped only by additive changes (new tables, new nullable columns).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS wan_meta (
        key   TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    );
    -- One row, ever. `singleton` pins it so a concurrent first mint
    -- collides instead of creating a second instance.
    CREATE TABLE IF NOT EXISTS wan_instance (
        singleton   INTEGER PRIMARY KEY CHECK (singleton = 1),
        public_key  TEXT    NOT NULL,
        private_key TEXT    NOT NULL,
        instance_id TEXT    NOT NULL,
        host_hint   TEXT    NOT NULL,
        created_at  INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS wan_agent_keys (
        agent_id     TEXT    PRIMARY KEY,
        public_key   TEXT    NOT NULL,
        private_key  TEXT    NOT NULL,
        created_at   INTEGER NOT NULL,
        -- 1 when the key was carried over from objects.db's
        -- db_agent_wan_keys rather than minted here.
        imported     INTEGER NOT NULL DEFAULT 0,
        -- D1b: the fingerprint of the certificate confirmed published to the
        -- cloud directory, and when. NULL until then.
        published_fp TEXT,
        published_at INTEGER
    );
";

/// This install's instance identity. The private key never leaves this
/// process except as a certificate signature.
#[derive(Clone, PartialEq, Eq)]
pub struct WanInstance {
    pub instance_id: String,
    pub public_key: [u8; 32],
    pub private_key: [u8; 32],
    /// The hostname when the instance was minted, sanitised to
    /// `[a-z0-9.-]{1,48}`. Never re-read from the live hostname: renaming
    /// the machine must not change what peers see.
    pub host_hint: String,
    pub created_at: i64,
}

impl std::fmt::Debug for WanInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WanInstance")
            .field("instance_id", &self.instance_id)
            .field("host_hint", &self.host_hint)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

/// An agent's WAN keypair, both halves standard base64 — the same encoding
/// `agent_wan_keys::AgentWanKeypair` uses, so an import is a straight copy
/// and `AGENTMUX_WAN_KEY` is unchanged for the MCP.
#[derive(Clone, PartialEq, Eq)]
pub struct WanAgentKey {
    pub public_key: String,
    pub private_key: String,
    pub imported: bool,
}

impl std::fmt::Debug for WanAgentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WanAgentKey")
            .field("public_key", &self.public_key)
            .field("imported", &self.imported)
            .finish_non_exhaustive()
    }
}

pub struct WanIdentityStore {
    conn: Mutex<Connection>,
}

/// `<channel dir>/wan-identity/wan.db`, from the launcher-provided paths.
/// `None` outside a launcher-started srv (tests, odd envs): WAN identity is
/// then off, never redirected somewhere else.
pub fn resolve_wan_identity_path() -> Option<PathBuf> {
    agentmux_common::DataPaths::from_env().map(|paths| paths.wan_identity_dir().join("wan.db"))
}

/// A hostname as a host hint: lowercased, anything outside `[a-z0-9.-]`
/// replaced by `-`, cut to 48 chars; `unknown` if nothing is left. The
/// receiver validates the same alphabet (`jekt_sign::is_valid_wan_host_hint`)
/// and renders anything else as `?`.
pub fn sanitize_host_hint(hostname: &str) -> String {
    let hint: String = hostname
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' { c } else { '-' })
        .take(48)
        .collect();
    if hint.is_empty() {
        "unknown".to_string()
    } else {
        hint
    }
}

/// 32 bytes from two v4 UUIDs — the CSPRNG source every key table here uses
/// (`agent_wan_keys::random_seed_bytes`).
fn random_seed_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes
}

fn decode_32(b64: &str) -> Option<[u8; 32]> {
    BASE64.decode(b64).ok()?.try_into().ok()
}

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(f.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Run `op` until it stops failing with SQLITE_BUSY, for at most
/// [`BUSY_TIMEOUT`]. Any other error, or BUSY past the deadline, is returned.
fn retry_while_busy<T>(mut op: impl FnMut() -> Result<T, rusqlite::Error>) -> Result<T, rusqlite::Error> {
    let deadline = std::time::Instant::now() + BUSY_TIMEOUT;
    let mut backoff = Duration::from_millis(5);
    loop {
        match op() {
            Err(e) if is_busy(&e) && std::time::Instant::now() < deadline => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}

#[cfg(unix)]
fn restrict_permissions(dir: &Path, file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
}

// Windows: the file inherits the per-user ACL of the profile directory it
// lives under; there is no mode bit to set.
#[cfg(not(unix))]
fn restrict_permissions(_dir: &Path, _file: &Path) {}

impl WanIdentityStore {
    /// Open (creating if needed) the store at `path`. Errors if the file
    /// can't be opened or its schema can't be ensured; the caller then runs
    /// without WAN identity.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Other(format!("wan-identity dir: {e}")))?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // Several processes opening a brand-new file at once (two versions of
        // a channel starting together) can get SQLITE_BUSY straight back from
        // the journal-mode switch or the first schema write: SQLite skips the
        // busy handler where waiting could deadlock. Measured — 8 concurrent
        // opens failed this way within a few runs. So the one-time setup is
        // retried as a whole, within the same bound as the busy timeout.
        retry_while_busy(|| {
            conn.pragma_update(None, "journal_mode", "WAL")?;
            // IMMEDIATE so two processes creating the schema at once
            // serialise on the write lock instead of one failing mid-way.
            conn.execute_batch(&format!("BEGIN IMMEDIATE; {SCHEMA} COMMIT;")).inspect_err(|_| {
                let _ = conn.execute_batch("ROLLBACK;");
            })?;
            conn.execute(
                "INSERT INTO wan_meta (key, value) VALUES ('schema_version', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
                params![SCHEMA_VERSION],
            )?;
            Ok(())
        })?;
        if let Some(dir) = path.parent() {
            restrict_permissions(dir, path);
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// The newest schema version any binary has written to this file.
    #[cfg(test)]
    pub fn schema_version(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(conn.query_row("SELECT value FROM wan_meta WHERE key = 'schema_version'", [], |r| r.get(0))?)
    }

    /// This install's instance, minted on first call. `hostname` is read
    /// only at mint time and becomes the permanent host hint.
    pub fn instance_ensure(&self, hostname: &str) -> Result<WanInstance, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = Self::instance_load(&conn)? {
            return Ok(existing);
        }
        let (public_key, private_key) = agentmux_common::jekt_sign::generate_wan_keypair(random_seed_bytes());
        conn.execute(
            "INSERT OR IGNORE INTO wan_instance \
             (singleton, public_key, private_key, instance_id, host_hint, created_at) \
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![
                BASE64.encode(public_key),
                BASE64.encode(private_key),
                agentmux_common::jekt_sign::wan_instance_id(&public_key),
                sanitize_host_hint(hostname),
                agentmux_common::time::now_secs(),
            ],
        )?;
        // Re-read whether or not this insert won: another process may have
        // minted first, and its instance is the one.
        Self::instance_load(&conn)?.ok_or_else(|| StoreError::Other("wan_instance row missing after mint".into()))
    }

    fn instance_load(conn: &Connection) -> Result<Option<WanInstance>, StoreError> {
        let row = conn
            .query_row(
                "SELECT public_key, private_key, instance_id, host_hint, created_at FROM wan_instance WHERE singleton = 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((public_b64, private_b64, instance_id, host_hint, created_at)) = row else {
            return Ok(None);
        };
        let (Some(public_key), Some(private_key)) = (decode_32(&public_b64), decode_32(&private_b64)) else {
            return Err(StoreError::Other("wan_instance holds a malformed key".into()));
        };
        // The id is derived, but stored for readability; a row whose id no
        // longer matches its key is corrupt, not an instance.
        if agentmux_common::jekt_sign::wan_instance_id(&public_key) != instance_id {
            return Err(StoreError::Other("wan_instance id does not match its key".into()));
        }
        Ok(Some(WanInstance { instance_id, public_key, private_key, host_hint, created_at }))
    }

    /// An agent's WAN key, if this store holds one. First production caller
    /// is D1b's publish (which certifies loaded keys); tests only until then.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn agent_key_load(&self, agent_id: &str) -> Result<Option<WanAgentKey>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Self::agent_key_load_locked(&conn, &agent_id.to_lowercase())
    }

    fn agent_key_load_locked(conn: &Connection, key: &str) -> Result<Option<WanAgentKey>, StoreError> {
        Ok(conn
            .query_row(
                "SELECT public_key, private_key, imported FROM wan_agent_keys WHERE agent_id = ?1",
                params![key],
                |r| {
                    Ok(WanAgentKey {
                        public_key: r.get(0)?,
                        private_key: r.get(1)?,
                        imported: r.get::<_, i64>(2)? != 0,
                    })
                },
            )
            .optional()?)
    }

    /// An agent's WAN key, created on first call: `import` (the agent's key
    /// from this version's `db_agent_wan_keys`, read by the caller) is
    /// carried over if given, otherwise a fresh key is minted. Only the first
    /// call for an agent decides — after that, `import` is ignored, so an
    /// agent keeps one key across upgrades. Case-insensitive, like every key
    /// table.
    pub fn agent_key_ensure(&self, agent_id: &str, import: Option<(&str, &str)>) -> Result<WanAgentKey, StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = Self::agent_key_load_locked(&conn, &key)? {
            return Ok(existing);
        }
        // An import must be a real keypair; anything else is minted over.
        let valid_import = import.filter(|(public_b64, private_b64)| {
            match (decode_32(public_b64), decode_32(private_b64)) {
                (Some(public), Some(private)) => agentmux_common::jekt_sign::generate_wan_keypair(private).0 == public,
                _ => false,
            }
        });
        let (public_key, private_key, imported) = match valid_import {
            Some((public_b64, private_b64)) => (public_b64.to_string(), private_b64.to_string(), true),
            None => {
                let (public, private) = agentmux_common::jekt_sign::generate_wan_keypair(random_seed_bytes());
                (BASE64.encode(public), BASE64.encode(private), false)
            }
        };
        conn.execute(
            "INSERT OR IGNORE INTO wan_agent_keys (agent_id, public_key, private_key, created_at, imported) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key, public_key, private_key, agentmux_common::time::now_secs(), imported as i64],
        )?;
        Self::agent_key_load_locked(&conn, &key)?
            .ok_or_else(|| StoreError::Other("wan_agent_keys row missing after mint".into()))
    }

    /// Delete the keys filed under each name (folded as the key tables fold).
    /// The agent-delete purge (`storage/agents.rs`) passes exactly the names
    /// it decided no other agent signs under, so a later agent that takes a
    /// name never inherits the dead agent's key. Returns rows removed.
    pub fn agent_keys_delete(&self, names: &[String]) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = 0;
        for name in names {
            removed += conn.execute("DELETE FROM wan_agent_keys WHERE agent_id = ?1", params![name.to_lowercase()])?;
        }
        Ok(removed)
    }
}

/// Test helper: attach a fresh `wan.db` in a temp dir to `store`, as
/// bootstrap does for the runtime object store. Keep the returned dir alive
/// for the test's duration.
#[cfg(test)]
pub fn attach_temp_wan_identity(store: &super::store::Store) -> (tempfile::TempDir, std::sync::Arc<WanIdentityStore>) {
    let dir = tempfile::tempdir().unwrap();
    let wan = std::sync::Arc::new(WanIdentityStore::open(&dir.path().join("wan.db")).unwrap());
    store.set_wan_identity(wan.clone());
    (dir, wan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_store() -> (tempfile::TempDir, WanIdentityStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = WanIdentityStore::open(&dir.path().join("wan-identity").join("wan.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn an_instance_is_minted_once_and_its_id_is_its_key_hash() {
        let (_dir, store) = temp_store();
        let first = store.instance_ensure("narko").unwrap();
        assert_eq!(first.instance_id, agentmux_common::jekt_sign::wan_instance_id(&first.public_key));
        assert_eq!(first.instance_id.len(), agentmux_common::jekt_sign::WAN_INSTANCE_ID_LEN);
        assert_eq!(
            agentmux_common::jekt_sign::generate_wan_keypair(first.private_key).0,
            first.public_key,
            "the stored private key must be the public key's seed"
        );
        assert_eq!(store.instance_ensure("narko").unwrap(), first);
    }

    #[test]
    fn the_instance_survives_reopening_as_after_an_upgrade() {
        // An upgrade starts a new objects.db but opens this same file.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wan.db");
        let before = WanIdentityStore::open(&path).unwrap().instance_ensure("narko").unwrap();
        let after = WanIdentityStore::open(&path).unwrap().instance_ensure("narko").unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn the_host_hint_is_fixed_at_mint_and_does_not_follow_a_rename() {
        let (_dir, store) = temp_store();
        let minted = store.instance_ensure("Narko").unwrap();
        assert_eq!(minted.host_hint, "narko");
        assert_eq!(store.instance_ensure("renamed-box").unwrap().host_hint, "narko");
    }

    #[test]
    fn host_hints_are_sanitised_to_what_receivers_accept() {
        assert_eq!(sanitize_host_hint("Narko"), "narko");
        assert_eq!(sanitize_host_hint("DESKTOP_AB12 (2)"), "desktop-ab12--2-");
        assert_eq!(sanitize_host_hint(""), "unknown");
        assert_eq!(sanitize_host_hint(&"x".repeat(60)).len(), 48);
        for host in ["Narko", "DESKTOP_AB12 (2)", "", "münchen", &"x".repeat(60)] {
            assert!(agentmux_common::jekt_sign::is_valid_wan_host_hint(&sanitize_host_hint(host)), "{host:?}");
        }
    }

    #[test]
    fn two_processes_minting_at_once_converge_on_one_instance() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("wan.db"));
        // Separate connections, as two srv processes would have.
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    WanIdentityStore::open(&path).unwrap().instance_ensure(&format!("host{i}")).unwrap().instance_id
                })
            })
            .collect();
        let ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(ids.windows(2).all(|w| w[0] == w[1]), "instances diverged: {ids:?}");
    }

    #[test]
    fn two_processes_minting_one_agent_at_once_converge_on_one_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("wan.db"));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    WanIdentityStore::open(&path).unwrap().agent_key_ensure("camper", None).unwrap().public_key
                })
            })
            .collect();
        let keys: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(keys.windows(2).all(|w| w[0] == w[1]), "agent keys diverged");
    }

    #[test]
    fn an_older_binary_opens_a_newer_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wan.db");
        {
            // A future schema: a newer version stamp, an extra table and an
            // extra column on an existing one.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute_batch(
                "INSERT INTO wan_meta (key, value) VALUES ('schema_version', 7);
                 CREATE TABLE wan_future (x INTEGER);
                 ALTER TABLE wan_agent_keys ADD COLUMN future_col TEXT;",
            )
            .unwrap();
        }
        let store = WanIdentityStore::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 7, "an older binary must not lower the stamp");
        store.instance_ensure("narko").unwrap();
        store.agent_key_ensure("camper", None).unwrap();
    }

    #[test]
    fn a_store_that_cannot_open_is_an_error_not_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // The db path is a directory: SQLite can't open it.
        let path = dir.path().join("wan.db");
        std::fs::create_dir_all(&path).unwrap();
        assert!(WanIdentityStore::open(&path).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "nothing minted elsewhere");
    }

    #[test]
    fn an_agent_key_is_imported_once_then_kept() {
        let (_dir, store) = temp_store();
        let (public, private) = agentmux_common::jekt_sign::generate_wan_keypair([7; 32]);
        let (public_b64, private_b64) = (BASE64.encode(public), BASE64.encode(private));
        let imported = store.agent_key_ensure("Camper", Some((&public_b64, &private_b64))).unwrap();
        assert!(imported.imported);
        assert_eq!(imported.public_key, public_b64);

        // A later import (another version's objects.db) doesn't replace it.
        let (other_public, other_private) = agentmux_common::jekt_sign::generate_wan_keypair([8; 32]);
        let again = store
            .agent_key_ensure("camper", Some((&BASE64.encode(other_public), &BASE64.encode(other_private))))
            .unwrap();
        assert_eq!(again, imported);
    }

    #[test]
    fn a_malformed_or_mismatched_import_is_minted_over() {
        let (_dir, store) = temp_store();
        let (public, _) = agentmux_common::jekt_sign::generate_wan_keypair([7; 32]);
        let (_, other_private) = agentmux_common::jekt_sign::generate_wan_keypair([8; 32]);
        let key = store
            .agent_key_ensure("camper", Some((&BASE64.encode(public), &BASE64.encode(other_private))))
            .unwrap();
        assert!(!key.imported);
        assert_ne!(key.public_key, BASE64.encode(public));
        let key2 = store.agent_key_ensure("lark", Some(("not base64", "x"))).unwrap();
        assert!(!key2.imported);
    }

    #[test]
    fn deleting_an_agents_keys_lets_a_new_agent_of_that_name_get_a_new_key() {
        let (_dir, store) = temp_store();
        let old = store.agent_key_ensure("camper", None).unwrap();
        store.agent_key_ensure("lark", None).unwrap();
        assert_eq!(store.agent_keys_delete(&["Camper".to_string(), "nobody".to_string()]).unwrap(), 1);
        assert!(store.agent_key_load("camper").unwrap().is_none());
        assert!(store.agent_key_load("lark").unwrap().is_some(), "other agents are untouched");
        assert_ne!(store.agent_key_ensure("camper", None).unwrap().public_key, old.public_key);
    }

    #[test]
    fn debug_output_never_contains_a_private_key() {
        let (_dir, store) = temp_store();
        let instance = store.instance_ensure("narko").unwrap();
        let agent = store.agent_key_ensure("camper", None).unwrap();
        assert!(!format!("{instance:?}").contains(&BASE64.encode(instance.private_key)));
        assert!(!format!("{agent:?}").contains(&agent.private_key));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, _store) = temp_store();
        let file = dir.path().join("wan-identity").join("wan.db");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
