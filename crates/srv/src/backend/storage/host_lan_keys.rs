// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host's LAN signing keys: one per agent on this machine, whatever channel or build runs it.
//!
//! An agent's LAN key used to live only in the channel's own `objects.db`, so every channel store that
//! first ran the agent minted a key of its own. Moving an agent from `stable` to a local build, or the
//! other way, then changed the key LAN peers had pinned, and they refused its jekts (Pinned senders).
//! The key belongs to the agent on the host, so it is kept beside the sign-ins every channel shares:
//! in `~/.agentmux/shared/identity-store.db`, in a table the older builds do not know and need not
//! (no schema version bump, so none of them is locked out).
//!
//! A channel store still keeps its own rows (`db_agent_lan_keys`, `db_agent_lan_keys_by_uid`), which
//! everything else reads. They follow the host table:
//! - the host has a key for the agent: the channel adopts it, replacing any key of its own;
//! - the host has none: the channel's own key, if it has one, becomes the host's (first one wins, so
//!   the key the peers pin today survives on the channel that runs first), else a fresh key is minted
//!   and published;
//! - the host table cannot be reached: the channel's own key is used, as before.
//!
//! Only LAN keys are here. WAN, jekt and token keys are still the channel's.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use super::agent_lan_keys::AgentLanKeypair;
use super::error::StoreError;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_host_lan_keys (
    kind        TEXT NOT NULL,
    subject     TEXT NOT NULL,
    public_key  TEXT NOT NULL,
    private_key TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (kind, subject)
);";

/// Which key a host row is: the name-keyed one (`v1` signatures) or the UID-keyed one (`v2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyKind {
    Name,
    Uid,
}

impl HostKeyKind {
    fn as_str(self) -> &'static str {
        match self {
            HostKeyKind::Name => "name",
            HostKeyKind::Uid => "uid",
        }
    }
}

/// A host row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    pub pair: AgentLanKeypair,
    /// Seconds.
    pub created_at: i64,
}

/// The host table, in the shared identity store. Opens its own connection, and again after a failure.
pub struct HostLanKeys {
    path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl std::fmt::Debug for HostLanKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostLanKeys").field("path", &self.path).finish_non_exhaustive()
    }
}

fn connect(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

impl HostLanKeys {
    /// The table in the database at `path` (made if missing).
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Ok(Self { path: path.to_path_buf(), conn: Mutex::new(Some(connect(path)?)) })
    }

    fn with<R>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<R>) -> Result<R, StoreError> {
        let mut slot = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(connect(&self.path)?);
        }
        let result = f(slot.as_ref().expect("opened above"));
        if result.is_err() {
            *slot = None; // the next call opens afresh
        }
        result.map_err(Into::into)
    }

    /// The host's key for `subject`. When it has none and `candidate` is given, that becomes the host's
    /// (the first writer wins; a racing channel gets the winner back).
    pub fn claim(
        &self,
        kind: HostKeyKind,
        subject: &str,
        candidate: Option<&HostKey>,
    ) -> Result<Option<HostKey>, StoreError> {
        self.with(|conn| {
            if let Some(c) = candidate {
                conn.execute(
                    "INSERT OR IGNORE INTO db_host_lan_keys (kind, subject, public_key, private_key, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![kind.as_str(), subject, c.pair.public_key, c.pair.private_key, c.created_at],
                )?;
            }
            conn.query_row(
                "SELECT public_key, private_key, created_at FROM db_host_lan_keys WHERE kind = ?1 AND subject = ?2",
                params![kind.as_str(), subject],
                |r| {
                    Ok(HostKey {
                        pair: AgentLanKeypair { public_key: r.get(0)?, private_key: r.get(1)? },
                        created_at: r.get(2)?,
                    })
                },
            )
            .optional()
        })
    }

    /// Make `key` the host's key for `subject`, replacing any: a rotation.
    pub fn replace(&self, kind: HostKeyKind, subject: &str, key: &HostKey) -> Result<(), StoreError> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO db_host_lan_keys (kind, subject, public_key, private_key, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT (kind, subject) DO UPDATE SET public_key = ?3, private_key = ?4, created_at = ?5",
                params![kind.as_str(), subject, key.pair.public_key, key.pair.private_key, key.created_at],
            )
            .map(|_| ())
        })
    }

    /// Drop the host's keys for `subjects`: an agent that was deleted. A channel that still has the agent
    /// publishes its own key again the next time it asks.
    pub fn forget(&self, kind: HostKeyKind, subjects: &[String]) -> Result<usize, StoreError> {
        self.with(|conn| {
            let mut removed = 0;
            for subject in subjects {
                removed += conn.execute(
                    "DELETE FROM db_host_lan_keys WHERE kind = ?1 AND subject = ?2",
                    params![kind.as_str(), subject],
                )?;
            }
            Ok(removed)
        })
    }
}

/// The first 8 characters of a public key, as the Pinned senders screen shows it.
pub(super) fn short(public_key: &str) -> &str {
    public_key.get(..8).unwrap_or(public_key)
}

#[cfg(test)]
mod tests;
