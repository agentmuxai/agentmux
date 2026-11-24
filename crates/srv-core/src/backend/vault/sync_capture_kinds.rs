// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The other two kinds of record in a Vault's collection, written by [`super::sync_capture::capture`] beside its entries:
//! this machine's activity lines (`sync_activity`) and the site grants of its entries (`sync_grant`).
//!
//! Both name their entry by the record it is written under (its link id), never by the id it has here: an entry taken from
//! another machine has an id of its own there, and the record's id is the one every machine has.
//!
//! - A line is written once, as `a.<machine>.<seq>`, sealed. It goes to the Vault its entry is in, and a line with no entry,
//!   or of an entry that stays on this machine, stays here too. What was written is remembered as the highest sequence
//!   number looked at, per Vault, so a capture reads only the lines made since.
//! - A grant is written as the record named for its entry and its site, and written again only when the grant or the entry
//!   it was made for is not what was written. A grant that is revoked, or whose entry is deleted or moved to another Vault,
//!   ends with a tombstone. The table this reads holds other kinds of grant too (an agent's "always", a command's first use,
//!   a setting), which name an agent or a setting of one machine: only a grant whose key is a site (`https://` or `http://`) is
//!   written, and no other ever leaves the machine.

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
    );
    -- Grants taken from another machine whose entry is not here yet: the entry's link id, to apply them when it arrives.
    CREATE TABLE IF NOT EXISTS db_sync_vault_grants_waiting (
        vault      TEXT NOT NULL,
        record     TEXT NOT NULL,
        entry_link TEXT NOT NULL,
        PRIMARY KEY (vault, record)
    );";

/// Notes that the grant taken into the store as `entry_id` is what the log holds for `record`: a capture then finds nothing
/// changed, and writes no version of its own. It is not waiting any more.
pub(super) fn mark_grant(store: &Store, vault: &str, record: &str, entry_id: &str, granted_at: i64, entry_created_at: i64) -> Result<(), String> {
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    conn.execute(
        "INSERT OR REPLACE INTO db_sync_vault_grants_seen (vault, record, entry_id, granted_at, entry_created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![vault, record, entry_id, granted_at, entry_created_at],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM db_sync_vault_grants_waiting WHERE vault = ?1 AND record = ?2", params![vault, record]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Forgets a grant taken or ended here: it is not written again, and not waiting.
pub(super) fn forget_grant(store: &Store, vault: &str, record: &str) -> Result<(), String> {
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    conn.execute("DELETE FROM db_sync_vault_grants_seen WHERE vault = ?1 AND record = ?2", params![vault, record]).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM db_sync_vault_grants_waiting WHERE vault = ?1 AND record = ?2", params![vault, record]).map_err(|e| e.to_string())?;
    Ok(())
}

/// A grant record whose entry (named by `entry_link`) is not here yet.
pub(super) fn wait_grant(store: &Store, vault: &str, record: &str, entry_link: &str) -> Result<(), String> {
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    conn.execute("INSERT OR REPLACE INTO db_sync_vault_grants_waiting (vault, record, entry_link) VALUES (?1, ?2, ?3)", params![vault, record, entry_link]).map_err(|e| e.to_string())?;
    Ok(())
}

/// The grant records waiting for an entry that has any of `links`.
pub(super) fn waiting_for(store: &Store, vault: &str, links: &[String]) -> Result<Vec<String>, String> {
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    let mut stmt = conn.prepare("SELECT record, entry_link FROM db_sync_vault_grants_waiting WHERE vault = ?1").map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![vault], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (record, link) = row.map_err(|e| e.to_string())?;
        if links.contains(&link) {
            out.push(record);
        }
    }
    Ok(out)
}

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

/// Writes the lines and grants of `vault_id`. `rows` are its entries that are captured, and `records` the record each is
/// written under; `here` are all of its entries, including the ones that stay on this machine.
#[allow(clippy::too_many_arguments)]
pub(super) fn write(conn: &Connection, signer: &mut Signer, vault_id: &str, ring: &KeyRing, reads: &Reads, rows: &[VaultRow], records: &HashMap<String, String>, here: &HashSet<&str>, now_ms: i64) -> Result<Wrote, String> {
    let mut out = Wrote::default();
    let captured: HashMap<&str, i64> = rows.iter().map(|r| (r.id.as_str(), r.created_at)).collect();
    out.lines = write_lines(conn, signer, vault_id, ring, reads, &entries_of(conn, vault_id, records)?, now_ms)?;
    let (grants, tombstoned) = write_grants(conn, signer, vault_id, ring, reads, &captured, records, here, now_ms)?;
    out.grants = grants;
    out.tombstoned = tombstoned;
    Ok(out)
}

/// The entries whose lines go to `vault_id`, each with its record: the ones captured now, and the ones it has written that are
/// gone since (their last lines, the delete's included, are still to go). An entry that still exists is not one of the second
/// kind: one that moved to another Vault, or stays on this machine, has its lines go where it is, not here.
fn entries_of(conn: &Connection, vault_id: &str, records: &HashMap<String, String>) -> Result<HashMap<String, String>, String> {
    let mut ids = records.clone();
    let mut stmt = conn
        .prepare("SELECT entry_id, record FROM db_sync_vault_seen s WHERE vault = ?1 AND NOT EXISTS (SELECT 1 FROM db_vault_entries e WHERE e.id = s.entry_id)")
        .map_err(|e| e.to_string())?;
    let seen = stmt.query_map(params![vault_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
    for pair in seen {
        let (entry, record) = pair.map_err(|e| e.to_string())?;
        ids.entry(entry).or_insert(record);
    }
    Ok(ids)
}

fn write_lines(conn: &Connection, signer: &mut Signer, vault_id: &str, ring: &KeyRing, reads: &Reads, entries: &HashMap<String, String>, now_ms: i64) -> Result<usize, String> {
    let Some(last) = reads.lines.last().map(|l| l.seq) else { return Ok(0) };
    let mut written = 0;
    for (line, entry_record) in reads.lines.iter().filter_map(|l| entries.get(&l.entry_id).map(|r| (l, r))) {
        let record = sync_activity::record_id(signer.machine, line.seq);
        // A line is never written twice: its record is the same every time.
        if log::current(conn, vault_id, &record).map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let mut body = ActivityBody::of_row(line);
        body.entry_id = entry_record.clone();
        let body = Zeroizing::new(serde_json::to_vec(&body).map_err(|e| e.to_string())?);
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

/// Whether a grant's key is a site. The same table holds an agent's grants and a setting's, whose keys name an agent, a
/// profile or a setting of this machine: those are not written.
fn is_site(key: &str) -> bool {
    key.starts_with("https://") || key.starts_with("http://")
}

#[allow(clippy::too_many_arguments)]
fn write_grants(conn: &Connection, signer: &mut Signer, vault_id: &str, ring: &KeyRing, reads: &Reads, captured: &HashMap<&str, i64>, records: &HashMap<String, String>, here: &HashSet<&str>, now_ms: i64) -> Result<(usize, usize), String> {
    let seen = seen_grants(conn, vault_id)?;
    let (mut written, mut ended) = (0, 0);
    let mut current = HashSet::new();
    for (entry_id, origin, granted_at) in &reads.grants {
        let (Some(&created_at), Some(entry_record)) = (captured.get(entry_id.as_str()), records.get(entry_id)) else { continue };
        if !is_site(origin) {
            continue;
        }
        let body = GrantBody::new(entry_record, origin, *granted_at, created_at);
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
