// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The computer-wide sync lock (SPEC_USER_DATA_SYNC_LAYER §3.1). The identity store, and with it the Kanban and the
//! Vaults, is shared by every AgentMux on a computer, so only one install may act as the computer's member: sign
//! records as it, run the sync loop and answer `sync.*` ops. That install holds this lock, a single row in the shared
//! store taken by compare-and-set with a lease. An install that does not hold it never signs as the member and never
//! runs the loop; a crashed holder is replaced when its lease runs out. Each taking raises a generation, so a holder
//! that was replaced finds its renewal refused and stops (a fence, like a task claim's attempt).

use rusqlite::{params, Connection, OptionalExtension};

/// The table, created by `log::init` with the log's.
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_sync_lock (
        id         INTEGER PRIMARY KEY CHECK (id = 1),
        holder     TEXT NOT NULL,
        generation INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    );";

/// How long a lease lasts before a crashed holder can be replaced.
pub const LEASE_MS: i64 = 60_000;

/// The answer to a request for the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lock {
    /// This install holds it, at this generation. Renew before the lease ends.
    Held { generation: i64 },
    /// Another install holds it until `until`.
    Busy { by: String, until: i64 },
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

fn row(conn: &Connection) -> rusqlite::Result<Option<(String, i64, i64)>> {
    conn.query_row("SELECT holder, generation, expires_at FROM db_sync_lock WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()
}

/// Takes the lock for `install` if it is free, expired or already its own. One statement decides, so two installs
/// asking at once cannot both get it.
pub fn acquire(conn: &Connection, install: &str, now_ms: i64, lease_ms: i64) -> rusqlite::Result<Lock> {
    conn.execute(
        "INSERT INTO db_sync_lock (id, holder, generation, expires_at) VALUES (1, ?1, 1, ?2)
         ON CONFLICT (id) DO UPDATE SET
             holder = excluded.holder,
             generation = CASE WHEN db_sync_lock.holder = excluded.holder THEN db_sync_lock.generation ELSE db_sync_lock.generation + 1 END,
             expires_at = excluded.expires_at
         WHERE db_sync_lock.holder = excluded.holder OR db_sync_lock.expires_at <= ?3",
        params![install, now_ms + lease_ms, now_ms],
    )?;
    Ok(match row(conn)? {
        Some((holder, generation, _)) if holder == install => Lock::Held { generation },
        Some((by, _, until)) => Lock::Busy { by, until },
        None => Lock::Busy { by: String::new(), until: 0 },
    })
}

/// Extends the lease if `install` still holds the lock at `generation`; false means it was replaced and must stop.
pub fn renew(conn: &Connection, install: &str, generation: i64, now_ms: i64, lease_ms: i64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE db_sync_lock SET expires_at = ?3 WHERE id = 1 AND holder = ?1 AND generation = ?2 AND expires_at > ?4",
        params![install, generation, now_ms + lease_ms, now_ms],
    )?;
    Ok(n == 1)
}

/// Gives the lock up (an install shutting down), if it holds it.
pub fn release(conn: &Connection, install: &str, generation: i64) -> rusqlite::Result<()> {
    conn.execute("UPDATE db_sync_lock SET expires_at = 0 WHERE id = 1 AND holder = ?1 AND generation = ?2", params![install, generation])?;
    Ok(())
}

/// Whether `install` holds the lock at `generation` right now: the check made before signing as the member.
pub fn holds(conn: &Connection, install: &str, generation: i64, now_ms: i64) -> rusqlite::Result<bool> {
    Ok(row(conn)?.is_some_and(|(h, g, until)| h == install && g == generation && until > now_ms))
}
