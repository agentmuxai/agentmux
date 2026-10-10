// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity M4d-2: each agent's LAN and WAN signing keypairs filed under its
//! UID (`db_agents.id`), beside the name-keyed tables they are copied from
//! (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10).
//!
//! A name-keyed key is copied to a UID only on evidence that the agent owns
//! it: the name is the row's slug, no other row holds that slug (folded),
//! the key is no older than the row, and the name is not a deleted agent's
//! (`db_agent_key_tombstones`). Otherwise the UID gets a fresh keypair. The
//! v1 signatures keep using the name-keyed keys until M5; a UID key signs
//! only the v2 signatures (M4d-6), so neither outcome changes what peers
//! verify today.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::{params, OptionalExtension};

use super::agent_lan_keys::random_seed_bytes;
use super::error::StoreError;
use super::store::Store;
use agentmux_common::time::now_secs;

/// One UID-keyed keypair, both halves base64.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UidKeypair {
    pub public_key: String,
    pub private_key: String,
}

/// An agent's UID-keyed LAN and WAN keypairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUidKeys {
    pub lan: UidKeypair,
    pub wan: UidKeypair,
}

/// How a UID key came to be, for the `m4d.key_*` counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UidKeyOrigin {
    /// It was already there.
    Existing,
    /// Copied from the name-keyed key, on ownership evidence.
    Copied,
    /// Minted, and why the name-keyed key wasn't copied.
    Fresh(FreshReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshReason {
    /// The row has no slug, or no name-keyed key exists under it.
    NoNameKey,
    /// Another row holds the same slug, folded.
    SlugShared,
    /// The row's `created_at` is 0 (a legacy or test row): no age to compare.
    RowUndated,
    /// The name-keyed key predates the row: another agent's.
    KeyOlderThanRow,
    /// The name is a deleted agent's.
    Tombstoned,
}

impl UidKeyOrigin {
    fn counter(self) -> Option<&'static str> {
        match self {
            UidKeyOrigin::Existing => None,
            UidKeyOrigin::Copied => Some("m4d.key_copied"),
            UidKeyOrigin::Fresh(FreshReason::NoNameKey) => Some("m4d.key_fresh.no_name_key"),
            UidKeyOrigin::Fresh(FreshReason::SlugShared) => Some("m4d.key_fresh.slug_shared"),
            UidKeyOrigin::Fresh(FreshReason::RowUndated) => Some("m4d.key_fresh.row_undated"),
            UidKeyOrigin::Fresh(FreshReason::KeyOlderThanRow) => Some("m4d.key_fresh.key_older_than_row"),
            UidKeyOrigin::Fresh(FreshReason::Tombstoned) => Some("m4d.key_fresh.tombstoned"),
        }
    }
}

/// The two key tiers, each with its legacy name-keyed table and its UID table.
#[derive(Clone, Copy)]
enum Tier {
    Lan,
    Wan,
}

impl Tier {
    fn name_table(self) -> &'static str {
        match self {
            Tier::Lan => "db_agent_lan_keys",
            Tier::Wan => "db_agent_wan_keys",
        }
    }
    fn uid_table(self) -> &'static str {
        match self {
            Tier::Lan => "db_agent_lan_keys_by_uid",
            Tier::Wan => "db_agent_wan_keys_by_uid",
        }
    }
    fn generate(self) -> ([u8; 32], [u8; 32]) {
        let seed = random_seed_bytes();
        match self {
            Tier::Lan => agentmux_common::jekt_sign::generate_lan_keypair(seed),
            Tier::Wan => agentmux_common::jekt_sign::generate_wan_keypair(seed),
        }
    }
}

/// The UID's row, as the evidence test needs it.
struct Row {
    slug: String,
    created_at_ms: i64,
}

impl Store {
    /// The UID's LAN and WAN keypairs, copied or minted on first use (§6.5.10
    /// M4d-2). One critical section: the row is re-read inside the lock, so a
    /// row purged since the caller read it gets nothing (`Ok(None)`) and no
    /// private key is left without an owner.
    pub fn agent_uid_keys_ensure(&self, uid: &str) -> Result<Option<(AgentUidKeys, [UidKeyOrigin; 2])>, StoreError> {
        let uid = uid.trim();
        if uid.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT slug, created_at FROM db_agents WHERE id = ?1",
                params![uid],
                |r| Ok(Row { slug: r.get::<_, Option<String>>(0)?.unwrap_or_default(), created_at_ms: r.get::<_, Option<i64>>(1)?.unwrap_or(0) }),
            )
            .optional()?;
        let Some(row) = row else { return Ok(None) };
        let (lan, lan_origin) = ensure_tier(&conn, Tier::Lan, uid, &row)?;
        let (wan, wan_origin) = ensure_tier(&conn, Tier::Wan, uid, &row)?;
        drop(conn);
        for origin in [lan_origin, wan_origin] {
            if let Some(counter) = origin.counter() {
                crate::backend::agent_resolve::record_uid_fallback(counter);
            }
        }
        Ok(Some((AgentUidKeys { lan, wan }, [lan_origin, wan_origin])))
    }

    /// The UID's LAN public key, if it has one; never creates one.
    pub fn agent_uid_lan_public_key_load(&self, uid: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT public_key FROM db_agent_lan_keys_by_uid WHERE uid = ?1", params![uid.trim()], |r| r.get(0))
            .optional()
            .map_err(Into::into)
    }
}

fn load_uid_key(conn: &rusqlite::Connection, tier: Tier, uid: &str) -> rusqlite::Result<Option<UidKeypair>> {
    conn.query_row(
        &format!("SELECT public_key, private_key FROM {} WHERE uid = ?1", tier.uid_table()),
        params![uid],
        |r| Ok(UidKeypair { public_key: r.get(0)?, private_key: r.get(1)? }),
    )
    .optional()
}

/// One tier: the existing UID key, else a copy of the name-keyed key when the
/// evidence holds, else a fresh one. Runs under the caller's lock.
fn ensure_tier(conn: &rusqlite::Connection, tier: Tier, uid: &str, row: &Row) -> Result<(UidKeypair, UidKeyOrigin), StoreError> {
    if let Some(existing) = load_uid_key(conn, tier, uid)? {
        return Ok((existing, UidKeyOrigin::Existing));
    }
    let (pair, copied_from, origin) = match copyable_name_key(conn, tier, row)? {
        Ok((pair, name)) => (pair, name, UidKeyOrigin::Copied),
        Err(reason) => {
            let (public, private) = tier.generate();
            (UidKeypair { public_key: BASE64.encode(public), private_key: BASE64.encode(private) }, String::new(), UidKeyOrigin::Fresh(reason))
        }
    };
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO {} (uid, public_key, private_key, created_at, copied_from) VALUES (?1, ?2, ?3, ?4, ?5)",
            tier.uid_table()
        ),
        params![uid, pair.public_key, pair.private_key, now_secs(), copied_from],
    )?;
    let stored = load_uid_key(conn, tier, uid)?.ok_or(StoreError::NotFound)?;
    Ok((stored, origin))
}

/// The name-keyed key the UID may take, and its name, or why not.
fn copyable_name_key(conn: &rusqlite::Connection, tier: Tier, row: &Row) -> Result<Result<(UidKeypair, String), FreshReason>, StoreError> {
    let slug = row.slug.trim().to_lowercase();
    if slug.is_empty() {
        return Ok(Err(FreshReason::NoNameKey));
    }
    let key = conn
        .query_row(
            &format!("SELECT public_key, private_key, created_at FROM {} WHERE agent_id = ?1", tier.name_table()),
            params![slug],
            |r| Ok((UidKeypair { public_key: r.get(0)?, private_key: r.get(1)? }, r.get::<_, i64>(2)?)),
        )
        .optional()?;
    let Some((pair, key_created_secs)) = key else { return Ok(Err(FreshReason::NoNameKey)) };
    // Slug uniqueness is case-sensitive, the key tables are not: "Aria" and
    // "aria" share one key row, so either holding it disqualifies both.
    let holders: i64 = conn.query_row("SELECT COUNT(*) FROM db_agents WHERE lower(trim(slug)) = ?1", params![slug], |r| r.get(0))?;
    if holders != 1 {
        return Ok(Err(FreshReason::SlugShared));
    }
    if row.created_at_ms <= 0 {
        return Ok(Err(FreshReason::RowUndated));
    }
    // Units differ: keys in seconds, rows in milliseconds. A key minted in the
    // same second as its row passes.
    if key_created_secs < row.created_at_ms / 1000 {
        return Ok(Err(FreshReason::KeyOlderThanRow));
    }
    let tombstoned: bool = conn
        .query_row("SELECT 1 FROM db_agent_key_tombstones WHERE name = ?1", params![slug], |_| Ok(true))
        .optional()?
        .unwrap_or(false);
    if tombstoned {
        return Ok(Err(FreshReason::Tombstoned));
    }
    Ok(Ok((pair, slug)))
}

#[cfg(test)]
mod tests;
