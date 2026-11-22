// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The change log and the state tables beside it (SPEC_USER_DATA_SYNC_LAYER §3.2, §5). Every change, made here or
//! taken from another machine, is one row of `db_sync_log`, keyed by its origin machine and that machine's
//! sequence number, so a catch-up is "everything after these sequences" and applying a record twice changes
//! nothing. `db_sync_state` points at the log row that is each record's current state. `db_sync_mark` is the
//! high-water mark of the head last accepted from a store, so an older head is refused (rollback).
//!
//! Functions take a `&Connection` so the whole layer is tested against in-memory stores of fake machines.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::adapter::{Adapter, Merge};
use super::hlc::{Clock, SkewRefused, Version};
use super::record::{BadRecord, Header, Record};

/// The tables, created on every open of the identity store (no schema version bump).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_sync_collections (
        id         TEXT PRIMARY KEY,
        kind       TEXT NOT NULL,
        epoch      INTEGER NOT NULL DEFAULT 0,
        tier       TEXT NOT NULL DEFAULT 'local',
        created_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS db_sync_log (
        collection TEXT NOT NULL,
        origin     TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        record     TEXT NOT NULL,
        wall_ms    INTEGER NOT NULL,
        counter    INTEGER NOT NULL,
        machine    TEXT NOT NULL,
        epoch      INTEGER NOT NULL,
        tombstone  INTEGER NOT NULL DEFAULT 0,
        size       INTEGER NOT NULL,
        sig        TEXT NOT NULL,
        body       BLOB NOT NULL,
        PRIMARY KEY (collection, origin, seq)
    );
    CREATE INDEX IF NOT EXISTS idx_sync_log_record ON db_sync_log (collection, record);
    CREATE TABLE IF NOT EXISTS db_sync_state (
        collection TEXT NOT NULL,
        record     TEXT NOT NULL,
        origin     TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        PRIMARY KEY (collection, record)
    );
    CREATE TABLE IF NOT EXISTS db_sync_mark (
        collection TEXT PRIMARY KEY,
        generation INTEGER NOT NULL DEFAULT 0,
        vector     TEXT NOT NULL DEFAULT '{}'
    );";

/// What a machine holds of a collection: for each origin, the highest sequence it has.
pub type Vector = BTreeMap<String, u64>;
/// The public keys pinned for each origin machine.
pub type Pins = BTreeMap<String, [u8; 32]>;

/// Days a deleted record's tombstone is kept: a board, and a Vault.
pub const HORIZON_BOARD_DAYS: i64 = 30;
pub const HORIZON_VAULT_DAYS: i64 = 90;

/// Why a record was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    UnknownCollection,
    UnknownOrigin,
    Bad(BadRecord),
    /// The record is at another key epoch than the collection's here (a removed member, or news not yet taken in).
    Epoch { have: u32, got: u32 },
    Skew(SkewRefused),
    /// A sequence was skipped: ask the origin again from `have`.
    Gap { have: u64 },
    /// A head older than the one already accepted.
    Rollback,
}

#[derive(Debug)]
pub enum SyncError {
    Refused(Refused),
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for SyncError {
    fn from(e: rusqlite::Error) -> Self {
        SyncError::Db(e)
    }
}

impl From<Refused> for SyncError {
    fn from(r: Refused) -> Self {
        SyncError::Refused(r)
    }
}

/// What applying a record did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    KeptLocal,
    Conflict,
    /// Already held (same origin and sequence): nothing changed.
    Duplicate,
}

/// The machine writing: its identifier, its signing key's seed and its clock.
pub struct Signer<'a> {
    pub machine: &'a str,
    pub private: &'a [u8],
    pub clock: &'a mut Clock,
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

pub fn ensure_collection(conn: &Connection, id: &str, kind: &str, now_ms: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO db_sync_collections (id, kind, created_at) VALUES (?1, ?2, ?3)",
        params![id, kind, now_ms],
    )?;
    Ok(())
}

pub fn collection_epoch(conn: &Connection, id: &str) -> rusqlite::Result<Option<u32>> {
    conn.query_row("SELECT epoch FROM db_sync_collections WHERE id = ?1", params![id], |r| r.get(0)).optional()
}

/// A member left: the collection moves to the next key epoch, and records at the old one are refused.
pub fn set_epoch(conn: &Connection, id: &str, epoch: u32) -> rusqlite::Result<()> {
    conn.execute("UPDATE db_sync_collections SET epoch = ?2 WHERE id = ?1", params![id, epoch])?;
    Ok(())
}

pub fn horizon_days(kind: &str) -> i64 {
    if kind == "vault" {
        HORIZON_VAULT_DAYS
    } else {
        HORIZON_BOARD_DAYS
    }
}

const COLS: &str = "collection, record, wall_ms, counter, machine, origin, seq, epoch, tombstone, size";

fn header_of(r: &Row) -> rusqlite::Result<Header> {
    Ok(Header {
        collection: r.get(0)?,
        record: r.get(1)?,
        version: Version { wall_ms: r.get(2)?, counter: r.get(3)?, machine: r.get(4)? },
        origin: r.get(5)?,
        seq: r.get::<_, i64>(6)? as u64,
        epoch: r.get(7)?,
        tombstone: r.get::<_, i64>(8)? != 0,
        size: r.get::<_, i64>(9)? as u64,
    })
}

fn record_of(r: &Row) -> rusqlite::Result<Record> {
    Ok(Record { header: header_of(r)?, sig: r.get(10)?, body: r.get(11)? })
}

fn have(conn: &Connection, collection: &str, origin: &str) -> rusqlite::Result<u64> {
    let n: Option<i64> =
        conn.query_row("SELECT MAX(seq) FROM db_sync_log WHERE collection = ?1 AND origin = ?2", params![collection, origin], |r| r.get(0))?;
    Ok(n.unwrap_or(0) as u64)
}

/// What this machine holds: the highest sequence of each origin.
pub fn vector(conn: &Connection, collection: &str) -> rusqlite::Result<Vector> {
    let mut stmt = conn.prepare("SELECT origin, MAX(seq) FROM db_sync_log WHERE collection = ?1 GROUP BY origin")?;
    let rows = stmt.query_map(params![collection], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?;
    rows.collect()
}

fn insert_log(conn: &Connection, rec: &Record) -> rusqlite::Result<()> {
    let h = &rec.header;
    conn.execute(
        "INSERT INTO db_sync_log (collection, origin, seq, record, wall_ms, counter, machine, epoch, tombstone, size, sig, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            h.collection, h.origin, h.seq as i64, h.record, h.version.wall_ms, h.version.counter, h.version.machine, h.epoch,
            h.tombstone as i64, h.size as i64, rec.sig, rec.body
        ],
    )?;
    Ok(())
}

fn point_state(conn: &Connection, h: &Header) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO db_sync_state (collection, record, origin, seq) VALUES (?1, ?2, ?3, ?4)",
        params![h.collection, h.record, h.origin, h.seq as i64],
    )?;
    Ok(())
}

/// A record's current state, as a whole record.
pub fn current(conn: &Connection, collection: &str, record: &str) -> rusqlite::Result<Option<Record>> {
    conn.query_row(
        "SELECT l.collection, l.record, l.wall_ms, l.counter, l.machine, l.origin, l.seq, l.epoch, l.tombstone, l.size, l.sig, l.body
         FROM db_sync_state s JOIN db_sync_log l ON l.collection = s.collection AND l.origin = s.origin AND l.seq = s.seq
         WHERE s.collection = ?1 AND s.record = ?2",
        params![collection, record],
        record_of,
    )
    .optional()
}

/// A change made here: the next sequence of this machine's log, the next version of its clock, signed.
pub fn append_local(
    conn: &Connection,
    signer: &mut Signer,
    collection: &str,
    record: &str,
    body: Vec<u8>,
    tombstone: bool,
    now_ms: i64,
) -> Result<Record, SyncError> {
    let epoch = collection_epoch(conn, collection)?.ok_or(Refused::UnknownCollection)?;
    let header = Header {
        collection: collection.to_string(),
        record: record.to_string(),
        version: signer.clock.tick(now_ms),
        origin: signer.machine.to_string(),
        seq: have(conn, collection, signer.machine)? + 1,
        epoch,
        tombstone,
        size: body.len() as u64,
    };
    let rec = Record::sign(signer.private, header, body).ok_or(Refused::Bad(BadRecord::Malformed))?;
    let tx = conn.unchecked_transaction()?;
    insert_log(&tx, &rec)?;
    point_state(&tx, &rec.header)?;
    tx.commit()?;
    Ok(rec)
}

/// A record from another machine. Refused unless it is at this collection's key epoch, from a pinned origin,
/// correctly signed, not too far ahead of this clock and the next in its origin's sequence; applying one already
/// held changes nothing. The adapter says whether it becomes the record's state.
pub fn apply_remote(
    conn: &Connection,
    rec: &Record,
    pins: &Pins,
    clock: &mut Clock,
    now_ms: i64,
    adapter: &dyn Adapter,
) -> Result<Outcome, SyncError> {
    let h = &rec.header;
    let epoch = collection_epoch(conn, &h.collection)?.ok_or(Refused::UnknownCollection)?;
    if h.epoch != epoch {
        return Err(Refused::Epoch { have: epoch, got: h.epoch }.into());
    }
    let key = pins.get(&h.origin).ok_or(Refused::UnknownOrigin)?;
    rec.verify(key).map_err(Refused::Bad)?;
    let held = have(conn, &h.collection, &h.origin)?;
    if h.seq <= held {
        return Ok(Outcome::Duplicate);
    }
    if h.seq > held + 1 {
        return Err(Refused::Gap { have: held }.into());
    }
    clock.observe(&h.version, now_ms).map_err(Refused::Skew)?;
    let tx = conn.unchecked_transaction()?;
    insert_log(&tx, rec)?;
    let local = current(&tx, &h.collection, &h.record)?.map(|r| r.header);
    let merge = adapter.merge(local.as_ref(), h);
    if merge == Merge::Applied {
        point_state(&tx, h)?;
    }
    tx.commit()?;
    Ok(match merge {
        Merge::Applied => Outcome::Applied,
        Merge::KeptLocal => Outcome::KeptLocal,
        Merge::Conflict => Outcome::Conflict,
    })
}

/// What a peer lacks: the records after the sequences in `vector`, per origin, at most `limit`.
pub fn since(conn: &Connection, collection: &str, vector: &Vector, limit: usize) -> rusqlite::Result<Vec<Record>> {
    let mut origins = conn.prepare("SELECT DISTINCT origin FROM db_sync_log WHERE collection = ?1 ORDER BY origin")?;
    let names: Vec<String> = origins.query_map(params![collection], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for origin in names {
        let room = limit.saturating_sub(out.len());
        if room == 0 {
            break;
        }
        let after = vector.get(&origin).copied().unwrap_or(0) as i64;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS}, sig, body FROM db_sync_log WHERE collection = ?1 AND origin = ?2 AND seq > ?3 ORDER BY seq LIMIT ?4"
        ))?;
        let rows = stmt.query_map(params![collection, origin, after, room as i64], record_of)?;
        for r in rows {
            out.push(r?);
        }
    }
    Ok(out)
}

/// The newest head accepted from a store: a generation (it rises when a store compacts or re-seals) and the vector.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Head {
    pub generation: u64,
    pub vector: Vector,
}

pub fn mark(conn: &Connection, collection: &str) -> rusqlite::Result<Head> {
    let row: Option<(i64, String)> = conn
        .query_row("SELECT generation, vector FROM db_sync_mark WHERE collection = ?1", params![collection], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((g, v)) => Head { generation: g as u64, vector: serde_json::from_str(&v).unwrap_or_default() },
        None => Head::default(),
    })
}

/// Takes a head served by a store as the new high-water mark; refuses one lower than the mark (a lower generation,
/// or the same generation with any origin behind where it was).
pub fn accept_head(conn: &Connection, collection: &str, head: &Head) -> Result<(), SyncError> {
    let m = mark(conn, collection)?;
    let behind = head.generation < m.generation
        || (head.generation == m.generation && m.vector.iter().any(|(o, s)| head.vector.get(o).copied().unwrap_or(0) < *s));
    if behind {
        return Err(Refused::Rollback.into());
    }
    conn.execute(
        "INSERT OR REPLACE INTO db_sync_mark (collection, generation, vector) VALUES (?1, ?2, ?3)",
        params![collection, head.generation as i64, serde_json::to_string(&head.vector).unwrap_or_else(|_| "{}".into())],
    )?;
    Ok(())
}

/// Records whose current state is a tombstone older than the collection's horizon: safe to forget once every
/// member has acknowledged them.
pub fn expired_tombstones(conn: &Connection, collection: &str, now_ms: i64) -> rusqlite::Result<Vec<String>> {
    let kind: String = conn.query_row("SELECT kind FROM db_sync_collections WHERE id = ?1", params![collection], |r| r.get(0))?;
    let cutoff = now_ms - horizon_days(&kind) * 86_400_000;
    let mut stmt = conn.prepare(
        "SELECT l.record FROM db_sync_state s JOIN db_sync_log l ON l.collection = s.collection AND l.origin = s.origin AND l.seq = s.seq
         WHERE s.collection = ?1 AND l.tombstone = 1 AND l.wall_ms < ?2 ORDER BY l.record",
    )?;
    let rows = stmt.query_map(params![collection, cutoff], |r| r.get(0))?;
    rows.collect()
}

/// A member that was last in contact before the horizon resyncs from scratch; what it changed offline goes to the
/// conflict inbox, not silently lost.
pub fn needs_full_resync(last_contact_ms: i64, now_ms: i64, horizon_days: i64) -> bool {
    now_ms - last_contact_ms > horizon_days * 86_400_000
}

/// What kind of change a history line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Created,
    Updated,
    Deleted,
}

/// A line of a collection's change history: who (which machine), when, what kind of change; never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub machine: String,
    pub seq: u64,
    pub wall_ms: i64,
    pub record: String,
    pub change: Change,
}

/// The collection's change history, newest first, from its log.
pub fn history(conn: &Connection, collection: &str, limit: usize) -> rusqlite::Result<Vec<HistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT origin, seq, wall_ms, counter, record, tombstone FROM db_sync_log WHERE collection = ?1
         ORDER BY wall_ms DESC, counter DESC, machine DESC LIMIT ?2",
    )?;
    let rows: Vec<(String, i64, i64, i64, String, i64)> = stmt
        .query_map(params![collection, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (origin, seq, wall_ms, counter, record, tombstone) in rows {
        let change = if tombstone != 0 {
            Change::Deleted
        } else {
            let earlier: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM db_sync_log WHERE collection = ?1 AND record = ?2 AND tombstone = 0
                 AND (wall_ms < ?3 OR (wall_ms = ?3 AND counter < ?4)))",
                params![collection, record, wall_ms, counter],
                |r| r.get(0),
            )?;
            if earlier {
                Change::Updated
            } else {
                Change::Created
            }
        };
        out.push(HistoryEntry { machine: origin, seq: seq as u64, wall_ms, record, change });
    }
    Ok(out)
}
