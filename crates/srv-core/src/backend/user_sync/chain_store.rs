// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A membership chain kept in the store, so a restart does not forget a collection's members, its key epoch's removals
//! or a fork not yet outgrown. What is read back is checked again, entry by entry, as a chain received from a peer is:
//! a stored chain that was altered is an error, never an empty or a shorter collection.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use super::members::{Chain, Entry, MemberError};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_sync_chain (
        collection TEXT NOT NULL,
        slot       TEXT NOT NULL CHECK (slot IN ('chain', 'fork')),
        idx        INTEGER NOT NULL,
        entry      TEXT NOT NULL,
        PRIMARY KEY (collection, slot, idx)
    );";

/// Why a stored chain was not given back.
#[derive(Debug)]
pub enum LoadError {
    Db(rusqlite::Error),
    /// A stored entry that is not an entry.
    Unreadable,
    /// The entries do not verify as a chain.
    Unsound(MemberError),
}

impl From<rusqlite::Error> for LoadError {
    fn from(e: rusqlite::Error) -> Self {
        LoadError::Db(e)
    }
}

/// Stores the chain and its unsettled forks, replacing what was stored for the collection.
pub fn save(conn: &Connection, collection: &str, chain: &Chain) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM db_sync_chain WHERE collection = ?1", params![collection])?;
    for (slot, entries) in [("chain", chain.entries.as_slice()), ("fork", chain.forks())] {
        for (i, e) in entries.iter().enumerate() {
            let text = serde_json::to_string(e).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            tx.execute("INSERT INTO db_sync_chain (collection, slot, idx, entry) VALUES (?1, ?2, ?3, ?4)", params![collection, slot, i as i64, text])?;
        }
    }
    tx.commit()
}

fn read(conn: &Connection, collection: &str, slot: &str) -> Result<Vec<Entry>, LoadError> {
    let mut stmt = conn.prepare("SELECT entry FROM db_sync_chain WHERE collection = ?1 AND slot = ?2 ORDER BY idx")?;
    let rows: Vec<String> = stmt.query_map(params![collection, slot], |r| r.get(0))?.collect::<Result<_, _>>()?;
    rows.iter().map(|t| serde_json::from_str(t).map_err(|_| LoadError::Unreadable)).collect()
}

/// The chain stored for a collection, checked again; none if nothing is stored.
pub fn load(conn: &Connection, collection: &str) -> Result<Option<Chain>, LoadError> {
    let entries = read(conn, collection, "chain")?;
    if entries.is_empty() {
        return Ok(None);
    }
    let forks = read(conn, collection, "fork")?;
    Chain::restore(collection, entries, forks).map(Some).map_err(LoadError::Unsound)
}

/// Every stored chain, by collection. One that fails its check is an error for the whole read: it is not skipped.
pub fn load_all(conn: &Connection) -> Result<BTreeMap<String, Chain>, LoadError> {
    let mut stmt = conn.prepare("SELECT DISTINCT collection FROM db_sync_chain ORDER BY collection")?;
    let names: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut out = BTreeMap::new();
    for name in names {
        if let Some(chain) = load(conn, &name)? {
            out.insert(name, chain);
        }
    }
    Ok(out)
}
