// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's entries as records in the sync layer's log (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! section 6): what a machine writes into a Vault's collection when its entries change.
//!
//! [`capture`] reads one Vault's entries from the store and writes whatever changed since the last capture into the
//! Vault's collection, one record per entry, named by the entry's link id (the id that is the same on every machine). A
//! record's body is the entry's [`EntryBody`](super::sync_record::EntryBody) sealed with the Vault's key at the epoch the
//! record carries, so the log, a LAN peer, the relay and the cloud see only the clear header. An entry that is gone, or
//! moved to another Vault, ends its record with a tombstone. It sends nothing and changes no entry: taking a record from
//! another machine into the store is the next step.
//!
//! What this machine keeps to itself does not go: a sign-in that [`stays_here`], the agents an entry is shared with, and
//! when it was last used (`EntryBody` leaves those out).

use ring::hmac;
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use super::seal::{self, Key};
use super::sync_capture_kinds as kinds;
use super::sync_links;
use super::sync_record::{stays_here, EntryBody, Profiles};
use super::sync_seal::{self, Place};
use super::sync_store::links_of;
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::backend::user_sync::log::{self, SealAt, Signer};
use crate::backend::user_sync::seal::KeyRing;

/// The kind of a Vault's collection.
pub const KIND: &str = "vault";

/// What a capture remembers: per entry its Vault, the record it was written as and a digest of what was written, so an
/// unchanged entry costs one lookup and no new version (a sealed body is different every time, so the log cannot say).
const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_sync_vault_seen (
        entry_id TEXT PRIMARY KEY,
        vault    TEXT NOT NULL,
        record   TEXT NOT NULL,
        hash     TEXT NOT NULL
    );
    -- Records this machine no longer writes an entry under (it follows another machine's, see `sync_take`): ended with a
    -- tombstone by the next capture.
    CREATE TABLE IF NOT EXISTS db_sync_vault_ended (
        vault  TEXT NOT NULL,
        record TEXT NOT NULL,
        PRIMARY KEY (vault, record)
    );";

/// Creates the log's tables and this module's.
pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    log::init(conn)?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(kinds::SCHEMA)
}

/// What a capture did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Captured {
    /// Records written: an entry's first version, or a changed one.
    pub written: usize,
    /// Records ended with a tombstone: an entry deleted, or moved to another Vault, or a grant revoked.
    pub tombstoned: usize,
    /// Activity lines written.
    pub lines: usize,
    /// Site grants written.
    pub grants: usize,
}

/// For `log::append_local_with`: seals `body` for the place the record will have, under the Vault's key at the epoch the
/// record carries. The associated data is the Vault's (`sync_seal::AAD_PREFIX`).
pub fn sealer<'a>(ring: &'a KeyRing, body: &'a [u8]) -> impl FnOnce(&SealAt) -> Result<Vec<u8>, String> + 'a {
    move |at| {
        let key = ring.get(at.epoch).ok_or_else(|| format!("no key for epoch {}", at.epoch))?;
        let place = Place { vault: &at.collection, record: &at.record, version: &at.version, base: at.base.as_ref(), epoch: at.epoch };
        sync_seal::seal_raw(key, &place, body)
    }
}

/// What is kept of a body to tell later whether it changed: an HMAC under this machine's own key, never a bare hash. A
/// hash of the body would let anyone holding the database test guesses at a low-entropy secret (everything else in the body
/// is in the tables beside it) without either key.
fn digest(key: &Key, body: &[u8]) -> String {
    let mut ctx = hmac::Context::with_key(&hmac::Key::new(hmac::HMAC_SHA256, &key[..]));
    ctx.update(b"agentmux-vault-sync-seen-v1\0");
    ctx.update(body);
    ctx.sign().as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The body a record carries for `row`: the entry as [`EntryBody`] writes it, with its link ids, as the bytes that are
/// sealed and compared. Wiped when dropped.
pub(super) fn body_of(store: &Store, row: &VaultRow, key: &Key, profiles: &dyn Profiles) -> Result<Zeroizing<Vec<u8>>, String> {
    let parts = seal::open(key, &row.id, &row.sealed)?;
    let body = EntryBody::of_row(row, &parts, profiles).with_link_ids(links_of(store, &row.id)?.ids());
    Ok(Zeroizing::new(serde_json::to_vec(&body).map_err(|e| e.to_string())?))
}

/// Notes that `row`, as it is stored now, is what the log holds for `record` in `vault_id`: a capture then finds nothing
/// changed. Taking a record into the store ends with this, or the next capture would write the taken entry back as a new
/// version made here.
pub(super) fn mark_seen(store: &Store, row: &VaultRow, record: &str, key: &Key, profiles: &dyn Profiles) -> Result<(), String> {
    let hash = digest(key, &body_of(store, row, key, profiles)?);
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    conn.execute("INSERT OR REPLACE INTO db_sync_vault_seen (entry_id, vault, record, hash) VALUES (?1, ?2, ?3, ?4)", params![row.id, row.vault_id, record, hash]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Every entry that has been written: the record it is written under.
fn records_written(store: &Store) -> Result<std::collections::HashMap<String, String>, String> {
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    let mut stmt = conn.prepare("SELECT entry_id, record FROM db_sync_vault_seen").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// The Vault and record `entry_id` is written under, if it has been written; otherwise the record it will be written
/// under, which is its own link id, with no Vault (nothing of it is in any log).
pub(super) fn identity_of(store: &Store, entry_id: &str) -> Result<Option<(String, String)>, String> {
    {
        let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = seen_of(&conn, entry_id).map_err(|e| e.to_string())? {
            return Ok(Some((s.vault, s.record)));
        }
    }
    Ok(store.vault_link_primary(entry_id).map_err(|e| e.to_string())?.map(|p| (String::new(), p)))
}

/// Notes that `record` in `vault` is no longer written: the next capture of that Vault ends it with a tombstone.
pub(super) fn end_record(store: &Store, vault: &str, record: &str) -> Result<(), String> {
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    conn.execute("INSERT OR IGNORE INTO db_sync_vault_ended (vault, record) VALUES (?1, ?2)", params![vault, record]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Forgets an entry that was deleted here because a record said so: its tombstone is already in the log.
pub(super) fn forget(store: &Store, entry_id: &str) -> Result<(), String> {
    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    conn.execute("DELETE FROM db_sync_vault_seen WHERE entry_id = ?1", params![entry_id]).map_err(|e| e.to_string())?;
    Ok(())
}

struct Seen {
    vault: String,
    record: String,
    hash: String,
}

fn seen_of(conn: &Connection, entry_id: &str) -> rusqlite::Result<Option<Seen>> {
    conn.query_row("SELECT vault, record, hash FROM db_sync_vault_seen WHERE entry_id = ?1", params![entry_id], |r| {
        Ok(Seen { vault: r.get(0)?, record: r.get(1)?, hash: r.get(2)? })
    })
    .optional()
}

/// Ends a record, if it is live.
pub(super) fn tombstone(conn: &Connection, signer: &mut Signer, collection: &str, record: &str, now_ms: i64) -> Result<bool, String> {
    match log::current(conn, collection, record).map_err(|e| e.to_string())? {
        Some(cur) if !cur.header.tombstone => {
            log::append_local(conn, signer, collection, record, Vec::new(), true, now_ms).map_err(|e| format!("{e:?}"))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// An entry that is to be recorded: the entry, the record it goes as, and the body.
struct Pending {
    entry_id: String,
    record: String,
    /// The body in the clear, wiped when dropped, on an error return too.
    body: Zeroizing<Vec<u8>>,
    hash: String,
}

/// Writes what changed in `vault_id`'s entries into its collection. `key` is this machine's own key, which opens the
/// entries' values; `ring` holds the Vault's keys; `profiles` names the person's browser profiles here.
pub fn capture(store: &Store, signer: &mut Signer, vault_id: &str, ring: &KeyRing, key: &Key, profiles: &dyn Profiles, now_ms: i64) -> Result<Captured, String> {
    // Everything that reads the store comes first: the connection is the store's own, and is taken only after. The lines and
    // grants are read before the entries, so a line is never read for an entry that was made after the entries were.
    let more = kinds::read(store, vault_id)?;
    let all: Vec<_> = store.vault_list().map_err(|e| e.to_string())?.into_iter().filter(|r| r.vault_id == vault_id).collect();
    let here: std::collections::HashSet<&str> = all.iter().map(|r| r.id.as_str()).collect();
    let rows: Vec<_> = all.iter().filter(|r| !stays_here(r, profiles)).cloned().collect();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let link_of = super::link_ids(store, &ids)?;
    let written_as = records_written(store)?;
    let mut pending = Vec::new();
    for row in &rows {
        // The record an entry is written under is the one it was first written (or taken) as, whatever link ids it has
        // learned since: one entry is one record (`identity_of`). Until then, its own link id.
        let Some(record) = written_as.get(&row.id).or_else(|| link_of.get(&row.id)) else { continue };
        if !sync_links::is_link_id(record) {
            continue;
        }
        let body = body_of(store, row, key, profiles)?;
        let hash = digest(key, &body);
        pending.push(Pending { entry_id: row.id.clone(), record: record.clone(), body, hash });
    }
    let now_here: std::collections::HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();

    let conn = store.conn().lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Captured::default();
    log::ensure_collection(&conn, vault_id, KIND, now_ms).map_err(|e| e.to_string())?;
    // Records that an entry has left for another machine's: ended once.
    let mut ended = conn.prepare("SELECT record FROM db_sync_vault_ended WHERE vault = ?1").map_err(|e| e.to_string())?;
    let ended: Vec<String> = ended.query_map(params![vault_id], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    for record in ended {
        if tombstone(&conn, signer, vault_id, &record, now_ms)? {
            out.tombstoned += 1;
        }
        conn.execute("DELETE FROM db_sync_vault_ended WHERE vault = ?1 AND record = ?2", params![vault_id, record]).map_err(|e| e.to_string())?;
    }
    for p in &pending {
        let seen = seen_of(&conn, &p.entry_id).map_err(|e| e.to_string())?;
        if seen.as_ref().is_some_and(|s| s.vault == vault_id && s.record == p.record && s.hash == p.hash) {
            continue;
        }
        log::append_local_with(&conn, signer, vault_id, &p.record, false, now_ms, sealer(ring, &p.body)).map_err(|e| format!("{e:?}"))?;
        out.written += 1;
        // An entry that came from another Vault ends its record there.
        if let Some(old) = seen.filter(|s| s.vault != vault_id) {
            if tombstone(&conn, signer, &old.vault, &old.record, now_ms)? {
                out.tombstoned += 1;
            }
        }
        conn.execute(
            "INSERT OR REPLACE INTO db_sync_vault_seen (entry_id, vault, record, hash) VALUES (?1, ?2, ?3, ?4)",
            params![p.entry_id, vault_id, p.record, p.hash],
        )
        .map_err(|e| e.to_string())?;
    }
    let records: std::collections::HashMap<String, String> = pending.iter().map(|p| (p.entry_id.clone(), p.record.clone())).collect();
    let wrote = kinds::write(&conn, signer, vault_id, ring, &more, &rows, &records, &here, now_ms)?;
    out.lines = wrote.lines;
    out.grants = wrote.grants;
    out.tombstoned += wrote.tombstoned;
    // Entries this Vault had recorded that no longer exist: deleted. One moved to another Vault is left to that Vault's
    // capture, which ends this record when it writes the entry; one that is only no longer kept here (its profile is
    // gone) is not deleted for the other machines.
    let mut stmt = conn.prepare("SELECT entry_id, record FROM db_sync_vault_seen WHERE vault = ?1").map_err(|e| e.to_string())?;
    let was: Vec<(String, String)> = stmt.query_map(params![vault_id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    drop(stmt);
    for (entry_id, record) in was.into_iter().filter(|(e, _)| !now_here.contains(e.as_str())) {
        let exists = conn.query_row("SELECT 1 FROM db_vault_entries WHERE id = ?1", params![entry_id], |_| Ok(())).optional().map_err(|e| e.to_string())?.is_some();
        if exists {
            continue;
        }
        if tombstone(&conn, signer, vault_id, &record, now_ms)? {
            out.tombstoned += 1;
        }
        conn.execute("DELETE FROM db_sync_vault_seen WHERE entry_id = ?1", params![entry_id]).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
#[path = "sync_capture_tests.rs"]
mod tests;
