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

use std::collections::BTreeSet;

use super::*;
use crate::registry::FormerSlug;

/// `db_agent_former_slugs`, applied on every open from `run_object_schema`.
/// Additive, so no version bump. Deleted with the agent's row, like
/// `db_agent_former_names`: a deleted agent's former slug is free again, as
/// its current one is.
pub(in crate::backend::storage) const FORMER_SLUGS_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_agent_former_slugs (
        agent_uid  TEXT NOT NULL,
        slug       TEXT NOT NULL,
        retired_at INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (agent_uid, slug),
        FOREIGN KEY (agent_uid) REFERENCES db_agents(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_agent_former_slugs_slug ON db_agent_former_slugs(slug);";

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
    /// Record `formers` as former slugs of `uid`, lowercased. An existing
    /// entry keeps its `retired_at`; a uid with no row here records nothing
    /// (its shared record still holds them). Returns the rows added.
    pub fn agent_former_slugs_record(&self, uid: &str, formers: &[FormerSlug]) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        record_former_slugs(&conn, uid, formers)
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
    /// table, so callers ask only once a live name has missed.
    pub fn former_slug_owner(&self, name: &str) -> Result<Option<FormerSlugOwner>, StoreError> {
        let folded = fold(name);
        if folded.is_empty() {
            return Ok(None);
        }
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
        if folded.is_empty() {
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
        }
        self.test_force_slug(uid, new).unwrap();
        self.agent_former_slugs_record(uid, &[former]).unwrap();
        old
    }
}

#[cfg(test)]
#[path = "former_slugs_tests.rs"]
mod tests;
