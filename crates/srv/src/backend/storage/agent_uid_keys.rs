// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity M4d-2: each agent's LAN and WAN signing keypairs filed under its
//! UID (`db_agents.id`), beside the name-keyed tables they are copied from
//! (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10).
//!
//! A name-keyed key is copied to a UID only on evidence that the agent owns
//! it: the name is the row's slug, no other row holds that slug (folded),
//! the key is no older than the row, and the name is not a deleted agent's
//! (`db_agent_key_tombstones`) nor any agent's former slug. The slug, and the
//! slugs other agents hold, are the canonical ones (`agents::canonical_slug`,
//! `SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` P0.1): a row whose shared record
//! was renamed takes nothing filed under its old name. Otherwise the UID gets a fresh keypair. The
//! v1 signatures keep using the name-keyed keys until M5; a UID key signs
//! only the v2 signatures (M4d-6), so neither outcome changes what peers
//! verify today.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::{params, OptionalExtension};

use super::agent_lan_keys::{aligned_name_key, random_seed_bytes, AgentLanKeypair};
use super::agents::SharedSlugs;
use super::error::StoreError;
use super::host_lan_keys::{short, HostKey, HostKeyKind, HostLanKeys};
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
    /// The name is a slug some agent was renamed away from: its keys are the
    /// old identity's, rotated rather than carried (spec §9 decision 2).
    FormerSlug,
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
            UidKeyOrigin::Fresh(FreshReason::FormerSlug) => Some("m4d.key_fresh.former_slug"),
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
        // Read before the lock: file I/O (the cached shared slug index).
        let shared = self.shared_slugs();
        let host = self.host_lan_keys();
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT slug, created_at FROM db_agents WHERE id = ?1",
                params![uid],
                |r| {
                    let local = r.get::<_, Option<String>>(0)?.unwrap_or_default();
                    Ok(Row {
                        slug: super::agents::canonical_slug(uid, &local, shared.slug(uid)),
                        created_at_ms: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    })
                },
            )
            .optional()?;
        let Some(row) = row else { return Ok(None) };
        let (lan, lan_origin) = ensure_tier(&conn, Tier::Lan, uid, &row, &shared, host.as_deref())?;
        let (wan, wan_origin) = ensure_tier(&conn, Tier::Wan, uid, &row, &shared, None)?;
        drop(conn);
        for origin in [lan_origin, wan_origin] {
            if let Some(counter) = origin.counter() {
                crate::backend::agent_resolve::record_uid_fallback(counter);
            }
        }
        Ok(Some((AgentUidKeys { lan, wan }, [lan_origin, wan_origin])))
    }

    /// Remove the UID's LAN and WAN keypairs, so the next `agent_uid_keys_ensure` takes the (current)
    /// name-keyed ones or mints new: for a UID whose copied keys were exposed. `true` when a row was removed.
    pub fn agent_uid_keys_delete(&self, uid: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut removed = 0;
        // The host's key first: one left there would be adopted again by the next `agent_uid_keys_ensure`.
        if let Some(host) = self.host_lan_keys() {
            removed += host.forget(HostKeyKind::Uid, &[uid.trim().to_string()])?;
        }
        for tier in [Tier::Lan, Tier::Wan] {
            removed += conn.execute(&format!("DELETE FROM {} WHERE uid = ?1", tier.uid_table()), params![uid.trim()])?;
        }
        Ok(removed > 0)
    }

    /// The UID's LAN public key, if it has one; never creates one. The host's key when it has one for a
    /// UID this channel knows (a channel row that lags it is brought level by `agent_uid_keys_ensure`).
    pub fn agent_uid_lan_public_key_load(&self, uid: &str) -> Result<Option<String>, StoreError> {
        let uid = uid.trim();
        let host = self.host_lan_keys();
        let conn = self.conn.lock().unwrap();
        let local: Option<String> = conn
            .query_row("SELECT public_key FROM db_agent_lan_keys_by_uid WHERE uid = ?1", params![uid], |r| r.get(0))
            .optional()?;
        if local.is_none() {
            return Ok(None);
        }
        Ok(match host.and_then(|h| h.claim(HostKeyKind::Uid, uid, None).ok().flatten()) {
            Some(h) => Some(h.pair.public_key),
            None => local,
        })
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

/// This channel's UID row, set to `pair` (the host's key for the UID).
fn write_uid_key(conn: &rusqlite::Connection, tier: Tier, uid: &str, pair: &UidKeypair, created_at: i64) -> rusqlite::Result<()> {
    conn.execute(
        &format!("INSERT OR REPLACE INTO {} (uid, public_key, private_key, created_at, copied_from) VALUES (?1, ?2, ?3, ?4, '')", tier.uid_table()),
        params![uid, pair.public_key, pair.private_key, created_at],
    )
    .map(|_| ())
}

fn as_host_key(pair: &UidKeypair) -> HostKey {
    HostKey { pair: AgentLanKeypair { public_key: pair.public_key.clone(), private_key: pair.private_key.clone() }, created_at: now_secs() }
}

/// The host table's LAN key for the UID when it has one, which this channel's row then follows; the
/// channel's own row, if it has one, when the host has none (it becomes the host's). `None` when neither
/// has a key. A host table that cannot be reached leaves the channel's own row.
fn host_aligned_uid_key(
    conn: &rusqlite::Connection,
    host: &HostLanKeys,
    uid: &str,
    local: Option<UidKeypair>,
) -> Result<Option<UidKeypair>, StoreError> {
    let candidate = local.as_ref().map(as_host_key);
    match host.claim(HostKeyKind::Uid, uid, candidate.as_ref()) {
        Ok(Some(h)) => {
            let pair = UidKeypair { public_key: h.pair.public_key, private_key: h.pair.private_key };
            if local.as_ref() != Some(&pair) {
                write_uid_key(conn, Tier::Lan, uid, &pair, h.created_at)?;
                tracing::info!(uid, key = short(&pair.public_key), "LAN key: adopted the host key for the agent's UID");
            }
            Ok(Some(pair))
        }
        Ok(None) => Ok(local),
        Err(e) => {
            tracing::warn!(uid, error = %e, "LAN key: the host key table is unreachable, using this channel's own");
            Ok(local)
        }
    }
}

/// One tier: the existing UID key, else a copy of the name-keyed key when the
/// evidence holds, else a fresh one. Runs under the caller's lock. `host` is the host's LAN key table
/// (LAN tier only): its key for the UID wins over anything this channel holds.
fn ensure_tier(
    conn: &rusqlite::Connection,
    tier: Tier,
    uid: &str,
    row: &Row,
    shared: &SharedSlugs,
    host: Option<&HostLanKeys>,
) -> Result<(UidKeypair, UidKeyOrigin), StoreError> {
    let local = load_uid_key(conn, tier, uid)?;
    if let Some(host) = host {
        if let Some(pair) = host_aligned_uid_key(conn, host, uid, local.clone())? {
            return Ok((pair, UidKeyOrigin::Existing));
        }
        // Nothing yet on the host or here. The name-keyed key it may be copied from follows the host too.
        let slug = row.slug.trim().to_lowercase();
        if !slug.is_empty() {
            aligned_name_key(conn, Some(host), &slug, false)?;
        }
    } else if let Some(existing) = local {
        return Ok((existing, UidKeyOrigin::Existing));
    }
    let (pair, copied_from, origin) = match copyable_name_key(conn, tier, row, shared)? {
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
    let mut stored = load_uid_key(conn, tier, uid)?.ok_or(StoreError::NotFound)?;
    if let Some(host) = host {
        // Publish it; a channel that published first has its key back instead.
        stored = host_aligned_uid_key(conn, host, uid, Some(stored.clone()))?.unwrap_or(stored);
    }
    Ok((stored, origin))
}

/// The name-keyed key the UID may take, and its name, or why not.
fn copyable_name_key(
    conn: &rusqlite::Connection,
    tier: Tier,
    row: &Row,
    shared: &SharedSlugs,
) -> Result<Result<(UidKeypair, String), FreshReason>, StoreError> {
    let slug = row.slug.trim().to_lowercase();
    if slug.is_empty() {
        return Ok(Err(FreshReason::NoNameKey));
    }
    if shared.is_former(&slug) {
        return Ok(Err(FreshReason::FormerSlug));
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
    // Folded in Rust as the key tables fold, never with SQLite's ASCII-only
    // lower(): "Ä" and "ä" would each count as the only holder and both
    // take one private key.
    let holders = slug_holders_folded(conn, &slug, shared)?;
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

/// How many agents hold `folded` as their slug, trimmed and folded with
/// Rust's Unicode-aware `to_lowercase` (the key tables' folding): this
/// channel's rows by their canonical slug, and the shared definitions, which
/// include agents this channel has no row for.
pub(super) fn slug_holders_folded(conn: &rusqlite::Connection, folded: &str, shared: &SharedSlugs) -> Result<usize, StoreError> {
    let mut ids: std::collections::HashSet<String> = super::agents::rows_with_canonical_slugs(conn, "", shared)?
        .into_iter()
        .filter(|r| !r.slug.trim().is_empty() && r.slug.trim().to_lowercase() == folded)
        .map(|r| r.id)
        .collect();
    ids.extend(shared.ids_with_slug(folded).map(str::to_string));
    Ok(ids.len())
}

#[cfg(test)]
mod tests;
