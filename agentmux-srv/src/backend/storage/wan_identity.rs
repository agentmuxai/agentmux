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
/// v2 (D2): the receiver tables — peer-record cache, known instances, and
/// seen signatures.
const SCHEMA_VERSION: i64 = 2;

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
    -- v2 (D2, receiver side) --
    -- Directory records fetched for verification. Self-certifying and
    -- content-addressed by the key fingerprint, so a cached record never
    -- expires; re-checked against its chain on every use.
    CREATE TABLE IF NOT EXISTS wan_peer_records (
        instance_id TEXT    NOT NULL,
        agent_id    TEXT    NOT NULL,
        channel     TEXT    NOT NULL,
        key_fp      TEXT    NOT NULL,
        record      TEXT    NOT NULL,
        fetched_at  INTEGER NOT NULL,
        PRIMARY KEY (instance_id, agent_id, channel, key_fp)
    );
    -- Every verified instance, recorded on first sight (§2.6). approved_at is
    -- written only by the host-gated approval window, which ships disabled;
    -- revoked_at is sticky.
    CREATE TABLE IF NOT EXISTS wan_known_instances (
        instance_id           TEXT    PRIMARY KEY,
        host_hint             TEXT    NOT NULL,
        first_seen_at         INTEGER NOT NULL,
        approved_at           INTEGER,
        revoked_at            INTEGER,
        revocation_checked_at INTEGER
    );
    -- (instance, agent, msgid) already delivered (§2.5). Written after
    -- successful local delivery; pruned once the message could no longer
    -- pass freshness.
    CREATE TABLE IF NOT EXISTS wan_seen_sigs (
        instance_id TEXT    NOT NULL,
        agent_id    TEXT    NOT NULL,
        msg_id      TEXT    NOT NULL,
        expires_at  INTEGER NOT NULL,
        PRIMARY KEY (instance_id, agent_id, msg_id)
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
    /// When this store first held the key. Doubles as the certificate's
    /// `issued_at`, so re-certifying a key produces the identical record and
    /// a re-publish is a no-op at the directory.
    pub created_at: i64,
    /// The fingerprint of this key's certificate once the cloud directory
    /// accepted it; `None` until then. The relay carry gate (§2.1 condition
    /// 4) requires it to equal the key's own fingerprint.
    pub published_fp: Option<String>,
}

impl std::fmt::Debug for WanAgentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WanAgentKey")
            .field("public_key", &self.public_key)
            .field("imported", &self.imported)
            .field("created_at", &self.created_at)
            .field("published_fp", &self.published_fp)
            .finish_non_exhaustive()
    }
}

impl WanAgentKey {
    /// The raw public key, if the stored base64 is well-formed.
    pub fn public_key_bytes(&self) -> Option<[u8; 32]> {
        decode_32(&self.public_key)
    }

    /// Whether the directory is confirmed to hold this exact key.
    pub fn is_published(&self) -> bool {
        match (self.public_key_bytes(), &self.published_fp) {
            (Some(public), Some(fp)) => agentmux_common::jekt_sign::wan_key_fingerprint(&public) == *fp,
            _ => false,
        }
    }
}

/// What this install knows about another instance (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownInstance {
    pub approved_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub revocation_checked_at: Option<i64>,
}

pub struct WanIdentityStore {
    conn: Mutex<Connection>,
}

static GLOBAL: std::sync::OnceLock<std::sync::Arc<WanIdentityStore>> = std::sync::OnceLock::new();

/// Register the process's `wan.db` for code that has no channel object store
/// in hand — the cloud subscriber runs against `id_store`. Set once, at
/// bootstrap, alongside `Store::set_wan_identity`.
pub fn install_global(store: std::sync::Arc<WanIdentityStore>) {
    let _ = GLOBAL.set(store);
}

pub fn global() -> Option<std::sync::Arc<WanIdentityStore>> {
    GLOBAL.get().cloned()
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

    /// An agent's WAN key, if this store holds one.
    pub fn agent_key_load(&self, agent_id: &str) -> Result<Option<WanAgentKey>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Self::agent_key_load_locked(&conn, &agent_id.to_lowercase())
    }

    const AGENT_KEY_COLUMNS: &'static str = "public_key, private_key, imported, created_at, published_fp";

    fn agent_key_from_row(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<WanAgentKey> {
        Ok(WanAgentKey {
            public_key: r.get(offset)?,
            private_key: r.get(offset + 1)?,
            imported: r.get::<_, i64>(offset + 2)? != 0,
            created_at: r.get(offset + 3)?,
            published_fp: r.get(offset + 4)?,
        })
    }

    fn agent_key_load_locked(conn: &Connection, key: &str) -> Result<Option<WanAgentKey>, StoreError> {
        Ok(conn
            .query_row(
                &format!("SELECT {} FROM wan_agent_keys WHERE agent_id = ?1", Self::AGENT_KEY_COLUMNS),
                params![key],
                |r| Self::agent_key_from_row(r, 0),
            )
            .optional()?)
    }

    /// Every agent key the directory is not confirmed to hold, by agent id —
    /// what the publisher (`muxbus/wan_publish.rs`) works through.
    pub fn agent_keys_unpublished(&self) -> Result<Vec<(String, WanAgentKey)>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT agent_id, {} FROM wan_agent_keys ORDER BY agent_id",
            Self::AGENT_KEY_COLUMNS
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, Self::agent_key_from_row(r, 1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (agent_id, key) = row?;
            if !key.is_published() {
                out.push((agent_id, key));
            }
        }
        Ok(out)
    }

    /// Record that the directory accepted `public_key`'s certificate. Only
    /// marks the row if it still holds that key — an agent deleted and
    /// recreated while the PUT was in flight must not have its new key marked
    /// published on the strength of the old one's.
    pub fn agent_key_mark_published(&self, agent_id: &str, public_key: &str) -> Result<bool, StoreError> {
        let Some(public) = decode_32(public_key) else { return Ok(false) };
        let fp = agentmux_common::jekt_sign::wan_key_fingerprint(&public);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let updated = conn.execute(
            "UPDATE wan_agent_keys SET published_fp = ?1, published_at = ?2 WHERE agent_id = ?3 AND public_key = ?4",
            params![fp, agentmux_common::time::now_secs(), agent_id.to_lowercase(), public_key],
        )?;
        Ok(updated > 0)
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

    // ── Receiver side (D2, §2.3/§2.5/§2.6) ──

    /// A cached directory record for exactly this (instance, agent, channel,
    /// fingerprint). The caller re-checks its chain; a row that no longer
    /// parses is treated as absent.
    pub fn peer_record_get(
        &self,
        instance_id: &str,
        agent_id: &str,
        channel: &str,
        key_fp: &str,
    ) -> Result<Option<agentmux_common::jekt_sign::WanKeyRecord>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let json: Option<String> = conn
            .query_row(
                "SELECT record FROM wan_peer_records \
                 WHERE instance_id = ?1 AND agent_id = ?2 AND channel = ?3 AND key_fp = ?4",
                params![instance_id.to_lowercase(), agent_id.to_lowercase(), channel, key_fp],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Cache a record whose chain the caller has checked.
    pub fn peer_record_put(&self, record: &agentmux_common::jekt_sign::WanKeyRecord) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO wan_peer_records \
             (instance_id, agent_id, channel, key_fp, record, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.instance_id,
                record.agent_id,
                record.channel,
                record.key_fp,
                serde_json::to_string(record)?,
                agentmux_common::time::now_secs(),
            ],
        )?;
        Ok(())
    }

    /// Forget every cached peer record — on logout or account switch (§2.3).
    pub fn peer_cache_clear(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM wan_peer_records", [])?;
        Ok(())
    }

    /// Record `instance_id` as seen (first sight inserts it) and return what
    /// is known about it.
    pub fn known_instance_observe(&self, instance_id: &str, host_hint: &str, now: i64) -> Result<KnownInstance, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO wan_known_instances (instance_id, host_hint, first_seen_at) VALUES (?1, ?2, ?3)",
            params![instance_id, host_hint, now],
        )?;
        Ok(conn.query_row(
            "SELECT approved_at, revoked_at, revocation_checked_at FROM wan_known_instances WHERE instance_id = ?1",
            params![instance_id],
            |r| {
                Ok(KnownInstance {
                    approved_at: r.get(0)?,
                    revoked_at: r.get(1)?,
                    revocation_checked_at: r.get(2)?,
                })
            },
        )?)
    }

    /// Record a revocation-status check. `revoked` is sticky: a later check
    /// that says "not revoked" never clears it.
    pub fn known_instance_record_revocation_check(
        &self,
        instance_id: &str,
        revoked: bool,
        now: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE wan_known_instances SET revocation_checked_at = ?2, \
             revoked_at = CASE WHEN ?3 THEN COALESCE(revoked_at, ?2) ELSE revoked_at END \
             WHERE instance_id = ?1",
            params![instance_id, now, revoked],
        )?;
        Ok(())
    }

    /// Whether this (instance, agent, msgid) was already delivered.
    pub fn seen_sig_contains(&self, instance_id: &str, agent_id: &str, msg_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(conn
            .query_row(
                "SELECT 1 FROM wan_seen_sigs WHERE instance_id = ?1 AND agent_id = ?2 AND msg_id = ?3",
                params![instance_id.to_lowercase(), agent_id.to_lowercase(), msg_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Record a delivered signature until `expires_at`, and prune the rows
    /// that could no longer pass freshness at `now` — the same clock the
    /// verifier's freshness check used, so a row is never pruned while its
    /// message could still verify.
    pub fn seen_sig_record(
        &self,
        instance_id: &str,
        agent_id: &str,
        msg_id: &str,
        expires_at: i64,
        now: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO wan_seen_sigs (instance_id, agent_id, msg_id, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![instance_id.to_lowercase(), agent_id.to_lowercase(), msg_id, expires_at],
        )?;
        conn.execute("DELETE FROM wan_seen_sigs WHERE expires_at < ?1", params![now])?;
        Ok(())
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
