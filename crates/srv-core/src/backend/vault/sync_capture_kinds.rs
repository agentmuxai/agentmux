// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The other two kinds of record in a Vault's collection, written by [`super::sync_capture::capture`] beside its entries:
//! this machine's activity lines (`sync_activity`) and the site grants of its entries (`sync_grant`).
//!
//! - A line is written once, as `a.<machine>.<seq>`, sealed. It goes to the Vault its entry is in, and a line with no entry,
//!   or of an entry that stays on this machine, stays here too. What was written is remembered as the highest sequence
//!   number looked at, per Vault, so a capture reads only the lines made since.
//! - A grant is written as the record named for its entry and its site, and written again only when the grant or the entry
//!   it was made for is not what was written. A grant that is revoked, or whose entry is deleted or moved to another Vault,
//!   ends with a tombstone. Grants to an agent never come here: the store holds only site grants in the table this reads.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use zeroize::Zeroizing;

use super::sync_activity::{self, ActivityBody};
use super::sync_capture::{sealer, tombstone};
use super::sync_grant::GrantBody;
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{VaultActivityRow, VaultRow};
use crate::backend::user_sync::log::{self, Signer};
use crate::backend::user_sync::seal::KeyRing;

pub(super) const SCHEMA: &str = "
    -- Per Vault, the highest activity sequence number a capture has looked at.
    CREATE TABLE IF NOT EXISTS db_sync_vault_lines_sent (
        vault TEXT PRIMARY KEY,
        seq   INTEGER NOT NULL
    );
    -- The grants written, and what they were written for: a grant is written again only when one of these changed.
    CREATE TABLE IF NOT EXISTS db_sync_vault_grants_seen (
        vault            TEXT NOT NULL,
        record           TEXT NOT NULL,
        entry_id         TEXT NOT NULL,
        granted_at       INTEGER NOT NULL,
        entry_created_at INTEGER NOT NULL,
        PRIMARY KEY (vault, record)
    );";

/// What a capture reads from the store before it takes the connection.
pub(super) struct Reads {
    lines: Vec<VaultActivityRow>,
    grants: Vec<(String, String, i64)>,
}

/// What [`write`] did.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Wrote {
    pub lines: usize,
    pub grants: usize,
    pub tombstoned: usize,
}

/// The activity lines made since `vault_id` was last captured, and the grants.
pub(super) fn read(store: &Store, vault_id: &str) -> Result<Reads, String> {
    let sent = {
        let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row("SELECT seq FROM db_sync_vault_lines_sent WHERE vault = ?1", params![vault_id], |r| r.get::<_, i64>(0)).unwrap_or(0)
    };
    Ok(Reads { lines: store.vault_activity_after(sent).map_err(|e| e.to_string())?, grants: store.vault_grant_rows().map_err(|e| e.to_string())? })
}

/// Writes the lines and grants of `vault_id`. `rows` are its entries that are captured; `here` are all of its entries,
/// including the ones that stay on this machine.
pub(super) fn write(conn: &Connection, signer: &mut Signer, vault_id: &str, ring: &KeyRing, reads: &Reads, rows: &[VaultRow], here: &HashSet<&str>, now_ms: i64) -> Result<Wrote, String> {
    let mut out = Wrote::default();
    let captured: HashMap<&str, i64> = rows.iter().map(|r| (r.id.as_str(), r.created_at)).collect();
    out.lines = write_lines(conn, signer, vault_id, ring, reads, &captured, now_ms)?;
    let (grants, tombstoned) = write_grants(conn, signer, vault_id, ring, reads, &captured, here, now_ms)?;
    out.grants = grants;
    out.tombstoned = tombstoned;
    Ok(out)
}

/// The entries whose lines go to `vault_id`: the ones captured now, and the ones it has written that are gone since (their
/// last lines, the delete's included, are still to go).
fn entries_of(conn: &Connection, vault_id: &str, captured: &HashMap<&str, i64>) -> Result<HashSet<String>, String> {
    let mut ids: HashSet<String> = captured.keys().map(|k| k.to_string()).collect();
    let mut stmt = conn.prepare("SELECT entry_id FROM db_sync_vault_seen WHERE vault = ?1").map_err(|e| e.to_string())?;
    let seen = stmt.query_map(params![vault_id], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
    for id in seen {
        ids.insert(id.map_err(|e| e.to_string())?);
    }
    Ok(ids)
}

fn write_lines(conn: &Connection, signer: &mut Signer, vault_id: &str, ring: &KeyRing, reads: &Reads, captured: &HashMap<&str, i64>, now_ms: i64) -> Result<usize, String> {
    let Some(last) = reads.lines.last().map(|l| l.seq) else { return Ok(0) };
    let entries = entries_of(conn, vault_id, captured)?;
    let mut written = 0;
    for line in reads.lines.iter().filter(|l| !l.entry_id.is_empty() && entries.contains(&l.entry_id)) {
        let record = sync_activity::record_id(signer.machine, line.seq);
        // A line is never written twice: its record is the same every time.
        if log::current(conn, vault_id, &record).map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let body = Zeroizing::new(serde_json::to_vec(&ActivityBody::of_row(line)).map_err(|e| e.to_string())?);
        log::append_local_with(conn, signer, vault_id, &record, false, now_ms, sealer(ring, &body)).map_err(|e| format!("{e:?}"))?;
        written += 1;
    }
    conn.execute("INSERT OR REPLACE INTO db_sync_vault_lines_sent (vault, seq) VALUES (?1, ?2)", params![vault_id, last]).map_err(|e| e.to_string())?;
    Ok(written)
}

/// One written grant, as remembered.
struct Seen {
    record: String,
    entry_id: String,
    granted_at: i64,
    entry_created_at: i64,
}

fn seen_grants(conn: &Connection, vault_id: &str) -> Result<Vec<Seen>, String> {
    let mut stmt = conn.prepare("SELECT record, entry_id, granted_at, entry_created_at FROM db_sync_vault_grants_seen WHERE vault = ?1").map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![vault_id], |r| Ok(Seen { record: r.get(0)?, entry_id: r.get(1)?, granted_at: r.get(2)?, entry_created_at: r.get(3)? })).map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn write_grants(conn: &Connection, signer: &mut Signer, vault_id: &str, ring: &KeyRing, reads: &Reads, captured: &HashMap<&str, i64>, here: &HashSet<&str>, now_ms: i64) -> Result<(usize, usize), String> {
    let seen = seen_grants(conn, vault_id)?;
    let (mut written, mut ended) = (0, 0);
    let mut current = HashSet::new();
    for (entry_id, origin, granted_at) in &reads.grants {
        let Some(&created_at) = captured.get(entry_id.as_str()) else { continue };
        if origin.is_empty() {
            continue;
        }
        let body = GrantBody::new(entry_id, origin, *granted_at, created_at);
        let record = body.record_id();
        current.insert(record.clone());
        if seen.iter().any(|s| s.record == record && s.granted_at == *granted_at && s.entry_created_at == created_at) {
            continue;
        }
        let bytes = Zeroizing::new(serde_json::to_vec(&body).map_err(|e| e.to_string())?);
        log::append_local_with(conn, signer, vault_id, &record, false, now_ms, sealer(ring, &bytes)).map_err(|e| format!("{e:?}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO db_sync_vault_grants_seen (vault, record, entry_id, granted_at, entry_created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![vault_id, record, entry_id, granted_at, created_at],
        )
        .map_err(|e| e.to_string())?;
        written += 1;
    }
    for s in seen.iter().filter(|s| !current.contains(&s.record)) {
        // An entry that is only no longer kept here (its profile is gone) keeps its grants for the other machines.
        if here.contains(s.entry_id.as_str()) && !captured.contains_key(s.entry_id.as_str()) {
            continue;
        }
        if tombstone(conn, signer, vault_id, &s.record, now_ms)? {
            ended += 1;
        }
        conn.execute("DELETE FROM db_sync_vault_grants_seen WHERE vault = ?1 AND record = ?2", params![vault_id, s.record]).map_err(|e| e.to_string())?;
    }
    Ok((written, ended))
}
