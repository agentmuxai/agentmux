// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A membership chain kept in the store, so a restart does not forget a collection's members, its key epoch's removals
//! or a fork not yet outgrown. What is read back is checked again, entry by entry, as a chain received from a peer is,
//! and against the length and head stored beside it: a stored chain that was altered (an entry gone from its middle or
//! its end) is an error, never an empty or a shorter collection.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};

use super::members::{Chain, Entry, MemberError};
use super::ops::PendingJoin;

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_sync_chain (
        collection TEXT NOT NULL,
        slot       TEXT NOT NULL CHECK (slot IN ('chain', 'fork')),
        idx        INTEGER NOT NULL,
        entry      TEXT NOT NULL,
        PRIMARY KEY (collection, slot, idx)
    );
    CREATE TABLE IF NOT EXISTS db_sync_pending (
        collection TEXT NOT NULL,
        machine    TEXT NOT NULL,
        subject    TEXT NOT NULL,
        asked_at   INTEGER NOT NULL,
        PRIMARY KEY (collection, machine)
    );
    CREATE TABLE IF NOT EXISTS db_sync_chain_head (
        collection TEXT PRIMARY KEY,
        n          INTEGER NOT NULL,
        head       TEXT NOT NULL
    );";

/// Why a stored chain was not given back.
#[derive(Debug)]
pub enum LoadError {
    Db(rusqlite::Error),
    /// A stored entry that is not an entry.
    Unreadable,
    /// The entries do not verify as a chain.
    Unsound(MemberError),
    /// The stored entries are not the ones that were saved: fork entries with no chain, or a chain whose length or head
    /// is not the one stored beside it (an entry gone from its end, or a head row gone).
    Altered,
}

impl From<rusqlite::Error> for LoadError {
    fn from(e: rusqlite::Error) -> Self {
        LoadError::Db(e)
    }
}

/// Stores the chain and its unsettled forks, replacing what was stored for the collection.
pub fn save(conn: &Connection, collection: &str, chain: &Chain) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    save_in(&tx, collection, chain)?;
    tx.commit()
}

/// [`save`] without a transaction of its own, for a caller that makes it one with other writes.
pub(super) fn save_in(tx: &Connection, collection: &str, chain: &Chain) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM db_sync_chain WHERE collection = ?1", params![collection])?;
    tx.execute("DELETE FROM db_sync_chain_head WHERE collection = ?1", params![collection])?;
    if let Some(last) = chain.entries.last() {
        tx.execute("INSERT INTO db_sync_chain_head (collection, n, head) VALUES (?1, ?2, ?3)", params![collection, chain.entries.len() as i64, last.hash()])?;
    }
    for (slot, entries) in [("chain", chain.entries.iter().collect::<Vec<_>>()), ("fork", chain.side().collect::<Vec<_>>())] {
        for (i, e) in entries.into_iter().enumerate() {
            let text = serde_json::to_string(e).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            tx.execute("INSERT INTO db_sync_chain (collection, slot, idx, entry) VALUES (?1, ?2, ?3, ?4)", params![collection, slot, i as i64, text])?;
        }
    }
    Ok(())
}

fn read(conn: &Connection, collection: &str, slot: &str) -> Result<Vec<Entry>, LoadError> {
    let mut stmt = conn.prepare("SELECT entry FROM db_sync_chain WHERE collection = ?1 AND slot = ?2 ORDER BY idx")?;
    let rows: Vec<String> = stmt.query_map(params![collection, slot], |r| r.get(0))?.collect::<Result<_, _>>()?;
    rows.iter().map(|t| serde_json::from_str(t).map_err(|_| LoadError::Unreadable)).collect()
}

/// The chain stored for a collection, checked again; none if nothing is stored.
pub fn load(conn: &Connection, collection: &str) -> Result<Option<Chain>, LoadError> {
    let entries = read(conn, collection, "chain")?;
    let forks = read(conn, collection, "fork")?;
    let stored: Option<(i64, String)> =
        conn.query_row("SELECT n, head FROM db_sync_chain_head WHERE collection = ?1", params![collection], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    if entries.is_empty() {
        return if forks.is_empty() && stored.is_none() { Ok(None) } else { Err(LoadError::Altered) };
    }
    let chain = Chain::restore(collection, entries, forks).map_err(LoadError::Unsound)?;
    let last = chain.entries.last().map(|e| e.hash());
    match stored {
        Some((n, head)) if n == chain.entries.len() as i64 && last.as_deref() == Some(head.as_str()) => Ok(Some(chain)),
        _ => Err(LoadError::Altered),
    }
}

/// Every stored chain, by collection. One that fails its check is an error for the whole read: it is not skipped. A
/// collection is named by its entry rows or by its head row, so one whose entries are all gone but whose head row is
/// left is read, and reported as altered, not dropped as if it had never been stored.
pub fn load_all(conn: &Connection) -> Result<BTreeMap<String, Chain>, LoadError> {
    let mut stmt = conn.prepare("SELECT collection FROM db_sync_chain UNION SELECT collection FROM db_sync_chain_head ORDER BY collection")?;
    let names: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut out = BTreeMap::new();
    for name in names {
        if let Some(chain) = load(conn, &name)? {
            out.insert(name, chain);
        }
    }
    Ok(out)
}

/// Stores the joins waiting for a person, replacing what was stored: a join asked of one call is still there for the call
/// in which a person confirms it.
pub fn save_pending(conn: &Connection, pending: &[PendingJoin]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM db_sync_pending", [])?;
    for p in pending {
        let subject = serde_json::to_string(&p.subject).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        tx.execute(
            "INSERT INTO db_sync_pending (collection, machine, subject, asked_at) VALUES (?1, ?2, ?3, ?4)",
            params![p.collection, p.subject.id, subject, p.asked_at],
        )?;
    }
    tx.commit()
}

pub fn load_pending(conn: &Connection) -> Result<Vec<PendingJoin>, LoadError> {
    let mut stmt = conn.prepare("SELECT collection, subject, asked_at FROM db_sync_pending ORDER BY asked_at, collection, machine")?;
    let rows: Vec<(String, String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
    rows.into_iter()
        .map(|(collection, subject, asked_at)| Ok(PendingJoin { collection, subject: serde_json::from_str(&subject).map_err(|_| LoadError::Unreadable)?, asked_at }))
        .collect()
}
