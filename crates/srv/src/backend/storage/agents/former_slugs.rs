// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The slugs an agent answered to before a rename: addresses that still reach
//! it, never names it can speak as (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md`
//! Phase 1).
//!
//! Stored twice, like the slug itself: in the shared definition's
//! `former_slugs` and in this `objects.db`'s `db_agent_former_slugs`. The two
//! are kept in step both ways: a mirror write carries this table's list to
//! the shared record (where `DefinitionStore::upsert` only appends), and a
//! backfill or startup copies the shared list into this table for every row
//! it has. Readers consult both, so a channel that hasn't synced yet still
//! answers. Lowercased, as every name-keyed table folds names.
//!
//! **Only routing reads these.** A message, a task or a cron job addressed to
//! a former slug reaches its agent; ownership checks, signing and key serving
//! compare the current slug only (`actor::uid_owns_name`, `self_keys_for`), so
//! nothing can sign or be served keys under one. And a current slug always
//! wins: a name that is some agent's slug or live name is never an alias.
//!
//! Every message asks whether its sender is a former slug, so the answer for
//! a name that isn't one costs nothing: [`FormerSlugSet`] holds every former
//! slug in memory, and only a name in it is looked up in full.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::*;
use crate::registry::FormerSlug;

/// `db_agent_former_slugs`, applied on every open from `run_object_schema`.
/// Additive, so no version bump. Deleted with the agent's row, like
/// `db_agent_former_names`: a deleted agent's former slug is free again, as
/// its current one is.
pub(crate) const FORMER_SLUGS_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_agent_former_slugs (
        agent_uid  TEXT NOT NULL,
        slug       TEXT NOT NULL,
        retired_at INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (agent_uid, slug),
        FOREIGN KEY (agent_uid) REFERENCES db_agents(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_agent_former_slugs_slug ON db_agent_former_slugs(slug);";

/// How long the known set is trusted before it is read again. The shared
/// definitions change under other processes, which only a re-read sees (the
/// slug index's stats, no parse unless a file changed); a write through this
/// store clears the set at once. Former slugs appear only by a migration,
/// which runs with the agents stopped, so the lag never meets a message.
/// Former display names appear on a rename, which clears the sets when it
/// goes through this store (`agent_def_update`, `instance_rename`).
pub(super) const KNOWN_FOR: Duration = Duration::from_secs(2);

/// Every former slug this channel can see, lowercased, from the shared slug
/// index and this table: a sender or target not in it is no former slug, and
/// costs no store access.
#[derive(Default)]
pub(in crate::backend::storage) struct FormerSlugSet {
    known: Mutex<Option<(Instant, HashSet<String>)>>,
    /// The same for former display names no agent holds now, by route key
    /// (`former_names`).
    pub(super) display: Mutex<Option<(Instant, HashSet<String>)>>,
    /// Current display names by route key, each to the one agent holding it
    /// (`None` when two do), less the keys some agent holds as its slug
    /// (`Store::display_route_owner`, #1459): a name not in it costs nothing.
    pub(super) routes: Mutex<Option<(Instant, HashMap<String, Option<FormerSlugOwner>>)>>,
    /// Store reads it caused (the set's re-reads and full lookups), for tests.
    #[cfg(test)]
    pub(super) reads: std::sync::atomic::AtomicUsize,
}

impl FormerSlugSet {
    /// Forget the set, after this process wrote a former slug.
    pub(in crate::backend::storage) fn invalidate(&self) {
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.display.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.routes.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(super) fn read(&self) {
        #[cfg(test)]
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Age the set past [`KNOWN_FOR`], as time passing would.
    #[cfg(test)]
    pub(super) fn expire(&self) {
        for set in [&self.known, &self.display] {
            if let Some((at, _)) = set.lock().unwrap().as_mut() {
                *at -= KNOWN_FOR * 2;
            }
        }
        if let Some((at, _)) = self.routes.lock().unwrap().as_mut() {
            *at -= KNOWN_FOR * 2;
        }
    }
}

/// The agent a former slug belongs to, and the slug it goes by now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormerSlugOwner {
    pub uid: String,
    pub slug: String,
}

fn fold(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The one uid `folded` is a former slug of, given this channel's rows and
/// the shared definitions, or `None`: not a former slug, a current slug
/// (which always wins), or claimed by several agents (logged; reservation
/// keeps that from happening).
pub(super) fn former_slug_owner_in(
    conn: &rusqlite::Connection,
    shared: &SharedSlugs,
    rows: &[AgentNameMatch],
    folded: &str,
) -> Result<Option<String>, StoreError> {
    if folded.is_empty()
        || rows.iter().any(|r| fold(&r.slug) == folded)
        || shared.entries().any(|(_, n)| fold(&n.slug) == folded)
    {
        return Ok(None);
    }
    let mut owners: BTreeSet<String> =
        shared.entries().filter(|(_, n)| n.former.iter().any(|f| fold(f) == folded)).map(|(id, _)| id.to_string()).collect();
    let mut stmt = conn.prepare("SELECT agent_uid FROM db_agent_former_slugs WHERE slug = ?1")?;
    for uid in stmt.query_map(params![folded], |r| r.get::<_, String>(0))? {
        owners.insert(uid?);
    }
    if owners.len() > 1 {
        tracing::warn!(name = folded, owners = ?owners, "former slug claimed by several agents; resolving none");
        return Ok(None);
    }
    Ok(owners.into_iter().next())
}

impl Store {
    /// Whether `name` may be a former slug: in [`FormerSlugSet`]. No store
    /// access unless the set is older than [`KNOWN_FOR`] or was cleared.
    pub fn former_slug_known(&self, name: &str) -> bool {
        let folded = fold(name);
        if folded.is_empty() {
            return false;
        }
        if let Some((at, names)) = self.former_slugs.known.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if at.elapsed() < KNOWN_FOR {
                return names.contains(&folded);
            }
        }
        self.former_slugs.read();
        let mut names: HashSet<String> =
            self.shared_slugs().entries().flat_map(|(_, n)| n.former.iter().map(|f| fold(f))).collect();
        let local: Result<Vec<String>, StoreError> = (|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT slug FROM db_agent_former_slugs")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })();
        match local {
            Ok(local) => names.extend(local.iter().map(|s| fold(s))),
            // Unknown, so not cached: the full lookup decides.
            Err(_) => return true,
        }
        let hit = names.contains(&folded);
        *self.former_slugs.known.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), names));
        hit
    }

    /// Record `formers` as former slugs of `uid`, lowercased. An existing
    /// entry keeps its `retired_at`; a uid with no row here records nothing
    /// (its shared record still holds them). Returns the rows added.
    pub fn agent_former_slugs_record(&self, uid: &str, formers: &[FormerSlug]) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let added = record_former_slugs(&conn, uid, formers);
        self.former_slugs.invalidate();
        added
    }

    /// This `objects.db`'s former slugs of `uid`, oldest first.
    pub fn agent_former_slugs_local(&self, uid: &str) -> Result<Vec<FormerSlug>, StoreError> {
        let conn = self.conn.lock().unwrap();
        local_former_slugs(&conn, uid)
    }

    /// Copy every active shared definition's former slugs into this table,
    /// for the agents that have a row here. Run at startup; idempotent.
    pub fn agent_former_slugs_sync_from_shared(&self) -> Result<usize, StoreError> {
        let Some(reg) = self.shared_def_registry() else {
            return Ok(0);
        };
        let records = reg.list_active().map_err(|e| StoreError::Other(format!("global definition registry: {e}")))?;
        let conn = self.conn.lock().unwrap();
        let mut added = 0;
        for rec in records.iter().filter(|r| !r.data.former_slugs.is_empty()) {
            added += record_former_slugs(&conn, &rec.data.id, &rec.data.former_slugs)?;
        }
        self.former_slugs.invalidate();
        if added > 0 {
            tracing::info!(added, "former slugs: copied from the shared definitions");
        }
        Ok(added)
    }

    /// Every former slug of every agent, from the shared definitions and this
    /// table, with when it was retired (the earliest either copy says). For
    /// the relay's grace subscription (`live_names`), which runs once a
    /// minute at most: this parses the shared definitions.
    pub fn agent_former_slugs_all(&self) -> Result<Vec<(String, FormerSlug)>, StoreError> {
        let mut all: std::collections::BTreeMap<(String, String), i64> = Default::default();
        let mut add = |uid: &str, f: &FormerSlug| {
            let at = all.entry((uid.to_string(), fold(&f.slug))).or_insert(f.retired_at);
            *at = (*at).min(f.retired_at);
        };
        if let Some(reg) = self.shared_def_registry() {
            match reg.list_active() {
                Ok(records) => records.iter().for_each(|r| r.data.former_slugs.iter().for_each(|f| add(&r.data.id, f))),
                Err(e) => tracing::warn!(error = %e, "former slugs: shared definitions unreadable; using this channel's"),
            }
        }
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT agent_uid, slug, retired_at FROM db_agent_former_slugs")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, FormerSlug { slug: r.get(1)?, retired_at: r.get(2)? })))?;
            for row in rows {
                let (uid, f) = row?;
                add(&uid, &f);
            }
        }
        Ok(all.into_iter().filter(|((_, slug), _)| !slug.is_empty()).map(|((uid, slug), retired_at)| (uid, FormerSlug { slug, retired_at })).collect())
    }

    /// The agent `name` is a former slug of, with its current slug: one
    /// definition, shared or local, lists it, and nothing has it as a current
    /// slug. For routing only (see the module doc). Reads the shared slug
    /// index (a stat per definition, no parse unless one changed) and this
    /// table, only for a name in [`FormerSlugSet`]: any other costs nothing.
    pub fn former_slug_owner(&self, name: &str) -> Result<Option<FormerSlugOwner>, StoreError> {
        let folded = fold(name);
        if !self.former_slug_known(&folded) {
            return Ok(None);
        }
        self.former_slugs.read();
        let shared = self.shared_slugs();
        let conn = self.conn.lock().unwrap();
        let rows = rows_with_canonical_slugs(&conn, "", &shared)?;
        let Some(uid) = former_slug_owner_in(&conn, &shared, &rows, &folded)? else {
            return Ok(None);
        };
        let slug = rows
            .iter()
            .find(|r| r.id == uid)
            .map(|r| r.slug.clone())
            .or_else(|| shared.slug(&uid).map(str::to_string))
            .filter(|s| !s.trim().is_empty());
        Ok(slug.map(|slug| FormerSlugOwner { uid, slug }))
    }

    /// Whether `name` is a former slug of any agent this channel can see —
    /// reserved, so nothing new takes it. Unlike [`Self::former_slug_owner`],
    /// true even when several agents list it. A read failure counts as
    /// reserved.
    pub fn former_slug_reserved(&self, name: &str) -> bool {
        let folded = fold(name);
        if !self.former_slug_known(&folded) {
            return false;
        }
        if self.shared_slugs().entries().any(|(_, n)| n.former.iter().any(|f| fold(f) == folded)) {
            return true;
        }
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT EXISTS(SELECT 1 FROM db_agent_former_slugs WHERE slug = ?1)", params![folded], |r| r.get(0))
            .unwrap_or(true)
    }
}

pub(super) fn record_former_slugs(conn: &rusqlite::Connection, uid: &str, formers: &[FormerSlug]) -> Result<usize, StoreError> {
    let mut added = 0;
    for f in formers {
        let slug = fold(&f.slug);
        if slug.is_empty() {
            continue;
        }
        added += conn.execute(
            "INSERT OR IGNORE INTO db_agent_former_slugs (agent_uid, slug, retired_at)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM db_agents WHERE id = ?1)",
            params![uid, slug, f.retired_at],
        )?;
    }
    Ok(added)
}

pub(super) fn local_former_slugs(conn: &rusqlite::Connection, uid: &str) -> Result<Vec<FormerSlug>, StoreError> {
    let mut stmt =
        conn.prepare("SELECT slug, retired_at FROM db_agent_former_slugs WHERE agent_uid = ?1 ORDER BY retired_at, slug")?;
    let rows = stmt.query_map(params![uid], |r| Ok(FormerSlug { slug: r.get(0)?, retired_at: r.get(1)? }))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
impl Store {
    /// Rename `uid`'s slug to `new` as Phase 2's S1 and S2 will: the shared
    /// record (straight to its file, since an update keeps the slug) and this
    /// channel's row, with the old slug recorded as a former one in both.
    /// Nothing renames in production yet. Returns the old slug.
    pub(crate) fn test_rename_slug(&self, uid: &str, new: &str, retired_at: i64) -> String {
        let old = self.agent_def_get(uid).unwrap().unwrap().slug;
        let former = FormerSlug { slug: old.clone(), retired_at };
        if let Some(reg) = self.shared_def_registry() {
            let mut rec = reg.get(uid).unwrap().unwrap();
            rec.data.slug = new.to_string();
            rec.data.former_slugs.push(former.clone());
            std::fs::write(reg.root().join(format!("{uid}.json")), serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
            self.former_slugs.invalidate();
        }
        self.test_force_slug(uid, new).unwrap();
        self.agent_former_slugs_record(uid, &[former]).unwrap();
        old
    }
}

#[cfg(test)]
#[path = "former_slugs_tests.rs"]
mod tests;
