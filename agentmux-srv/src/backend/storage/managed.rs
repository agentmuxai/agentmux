// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The "managed primitive" storage shape shared by standalone Skills and MCP
//! Servers (v1 composable model, SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md;
//! bundle-level refs per SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md).
//!
//! Both primitives are the same pattern: one catalog table whose rows are
//! either global (visible to everyone) or private, an agent-level ref table
//! binding rows to agents, and a bundle-level ref table binding rows to
//! bundles. Before this module `skills.rs` and `mcp_servers.rs` each carried
//! their own copy of the eighteen list / get / delete / bind / unbind /
//! upsert / access-check methods — identical except for table and column
//! names and the noun in error messages
//! (REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md §2.4). The per-primitive
//! modules keep their public methods, response structs, doc comments and
//! tests; only the bodies delegate here. Every SQL statement below is the
//! one the hand-written methods used, with the names substituted.
//!
//! SQL is assembled with `format!` from the trait's `const` names only —
//! never from runtime input — so the generated statements are exactly as
//! fixed as the literals they replace. Values still go through `?N`
//! parameters.
//!
//! ## `self` vs `catalog` (Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md)
//!
//! As of Phase 2's application-layer redirect (§5.4), `self` and the
//! catalog table's store are no longer guaranteed to be the same physical
//! SQLite file: `db_skills`/`db_mcp_servers` are authoritatively
//! `identity_store` now, while `db_agent_skills_ref`/`db_bundle_skills_ref`
//! (and the MCP equivalents) stay on `self` (`wstore`) until Phase 3/6
//! promotes them too. Every method that used to join the catalog table
//! against a ref table in one SQL statement now takes an explicit
//! `catalog: &Store` parameter and does two queries composed in Rust
//! instead — fetch the relevant ref ids from `self`, fetch the relevant
//! catalog rows from `catalog`, combine and sort in memory. This is not a
//! new pattern — `managed_bind_bundle`/`managed_upsert_unique_for_bundle`
//! already took an `id_store: &Store` parameter for bundle-existence checks,
//! for the identical reason. `managed_get` needs no such change — it is
//! genuinely single-table, so callers invoke it directly on whichever store
//! actually owns the table (`catalog` for a Skill/McpServer row).
//!
//! `managed_upsert_unique` additionally loses single-transaction atomicity
//! by this same split: the catalog insert and the ref-table bind are now two
//! separate connections, so they can't share a `rusqlite::Transaction`. This
//! is a documented, accepted interim gap (§5.4) — narrowed by a best-effort
//! compensating delete on the catalog row if the ref-bind fails, so a failed
//! bind cannot leave a permanent, undiscoverable orphan catalog row behind.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, params_from_iter, types::Value, Row};

use super::error::StoreError;
use super::store::Store;

/// One managed primitive: its catalog row type plus the names that differ
/// between primitives.
pub trait ManagedResource: Sized {
    /// Catalog table, e.g. `db_skills`.
    const TABLE: &'static str;
    /// The ref tables' column that points at `TABLE.id`, e.g. `skill_id`.
    const REF_COL: &'static str;
    /// Agent-level ref table `(agent_id, REF_COL)`, e.g. `db_agent_skills_ref`.
    const AGENT_REF_TABLE: &'static str;
    /// Bundle-level ref table `(bundle_id, REF_COL)`, e.g. `db_bundle_skills_ref`.
    const BUNDLE_REF_TABLE: &'static str;
    /// Every column of `TABLE`, in the order `from_row` reads them and
    /// `values` produces them. Must include `id`, `name`, `is_global`,
    /// `created_at` and `updated_at`.
    const COLUMNS: &'static [&'static str];
    /// Columns rewritten by `ON CONFLICT(id) DO UPDATE` — everything a caller
    /// may edit; never `id`, `is_global` or `created_at`.
    const UPDATE_COLUMNS: &'static [&'static str];
    /// Noun in name-uniqueness errors: "`skill` name 'x' already bound…".
    const NAME_NOUN: &'static str;
    /// Noun with article in bundle errors: "cannot bind `an MCP server` to…".
    const ARTICLE_NOUN: &'static str;
    /// Noun in the cross-channel bind error: "cross-channel `MCP server` binding…".
    const KIND: &'static str;

    /// Read one row whose leading columns are `COLUMNS`, in order, from 0.
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self>;
    /// The row's values in `COLUMNS` order (`is_global` as 0/1).
    fn values(&self) -> Vec<Value>;
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    /// Whether this row is a global (catalog-wide) row. Needed by
    /// `managed_list`'s in-memory sort, now that combining a global set with
    /// a private set can no longer happen inside one `ORDER BY` (Phase 2 of
    /// SPEC_DURABLE_BINDINGS_2026_09_10.md).
    fn is_global(&self) -> bool;
    /// Last-updated timestamp, for the same in-memory sort as `is_global`.
    fn updated_at(&self) -> i64;
}

/// Which ref table a scoped query is keyed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Agent,
    Bundle,
}

impl Owner {
    fn ref_table<R: ManagedResource>(self) -> &'static str {
        match self {
            Owner::Agent => R::AGENT_REF_TABLE,
            Owner::Bundle => R::BUNDLE_REF_TABLE,
        }
    }
    fn key_col(self) -> &'static str {
        match self {
            Owner::Agent => "agent_id",
            Owner::Bundle => "bundle_id",
        }
    }
    fn word(self) -> &'static str {
        match self {
            Owner::Agent => "agent",
            Owner::Bundle => "bundle",
        }
    }
}

/// `?1, ?2, ... ?n`.
fn placeholders(n: usize) -> String {
    (1..=n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
}

/// `?{start}, ?{start+1}, ... ?{start+n-1}` — for a `?N` list that must
/// continue numbering after some earlier bound parameters (`managed_upsert_unique`'s
/// dup-check binds `name`/`id` at `?1`/`?2` before any owner-bound ids).
fn placeholders_from(start: usize, n: usize) -> String {
    (start..start + n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
}

fn update_set<R: ManagedResource>() -> String {
    R::UPDATE_COLUMNS
        .iter()
        .map(|c| format!("{c}=excluded.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `row.values()` with `is_global` replaced when a method hard-codes it
/// (the global upsert forces 1, the bundle-scoped upsert forces 0).
fn values_with_global<R: ManagedResource>(row: &R, force_global: Option<bool>) -> Vec<Value> {
    let mut v = row.values();
    if let Some(g) = force_global {
        let idx = R::COLUMNS
            .iter()
            .position(|c| *c == "is_global")
            .expect("ManagedResource::COLUMNS must contain is_global");
        v[idx] = Value::Integer(if g { 1 } else { 0 });
    }
    v
}

impl Store {
    // ── Cross-store helpers (Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md) ──
    // Small building blocks the rewritten methods below compose. Split out so
    // each one locks exactly one connection for exactly as long as it needs
    // to — important because `self` and `catalog` (or `id_store`) may be the
    // SAME `Store` (test_state() aliases them; even in production `id_store`
    // and `identity_store` can coincide with `wstore` in a degraded-mode
    // fallback), and `Mutex<Connection>` is not reentrant. A method that
    // locked `self` and then called into `catalog` while still holding that
    // lock would deadlock the moment they aliased.

    /// The ref ids `owner_id` holds for one primitive, on `self`.
    fn managed_bound_ids<R: ManagedResource>(&self, owner: Owner, owner_id: &str) -> Result<HashSet<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {refc} FROM {reft} WHERE {key} = ?1",
            refc = R::REF_COL,
            reft = owner.ref_table::<R>(),
            key = owner.key_col(),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![owner_id], |row| row.get::<_, String>(0))?;
        let mut out = HashSet::new();
        for r in rows {
            out.insert(r?);
        }
        Ok(out)
    }

    /// `REF_COL -> count` across every agent-level ref row, on `self`.
    fn managed_bound_counts<R: ManagedResource>(&self) -> Result<HashMap<String, i64>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {refc}, COUNT(*) FROM {reft} GROUP BY {refc}",
            refc = R::REF_COL,
            reft = R::AGENT_REF_TABLE,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
        let mut out = HashMap::new();
        for r in rows {
            let (id, count) = r?;
            out.insert(id, count);
        }
        Ok(out)
    }

    /// Every global row of a primitive, on `self` acting as the CATALOG
    /// store — newest first.
    fn managed_catalog_global<R: ManagedResource>(&self) -> Result<Vec<R>, StoreError> {
        let sql = format!(
            "SELECT {cols} FROM {table} WHERE is_global = 1 ORDER BY updated_at DESC",
            cols = R::COLUMNS.join(", "),
            table = R::TABLE,
        );
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| R::from_row(row))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Every global row, plus (if `ids` is non-empty) every row in `ids` —
    /// global or private — on `self` acting as the CATALOG store. Unordered;
    /// callers sort as needed.
    fn managed_catalog_global_and_ids<R: ManagedResource>(&self, ids: &HashSet<String>) -> Result<Vec<R>, StoreError> {
        let sql = if ids.is_empty() {
            format!(
                "SELECT {cols} FROM {table} WHERE is_global = 1",
                cols = R::COLUMNS.join(", "),
                table = R::TABLE,
            )
        } else {
            format!(
                "SELECT {cols} FROM {table} WHERE is_global = 1 OR id IN ({ph})",
                cols = R::COLUMNS.join(", "),
                table = R::TABLE,
                ph = placeholders(ids.len()),
            )
        };
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;
        let bind: Vec<Value> = ids.iter().map(|s| Value::Text(s.clone())).collect();
        let rows = stmt.query_map(params_from_iter(bind), |row| R::from_row(row))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── Public-to-crate managed-primitive operations ──────────────────────

    /// Rows visible to `owner_id` — its own referenced rows plus every global
    /// row — each paired with whether this owner holds the ref. Global rows
    /// first, newest first.
    pub(super) fn managed_list<R: ManagedResource>(
        &self,
        catalog: &Store,
        owner: Owner,
        owner_id: &str,
    ) -> Result<Vec<(R, bool)>, StoreError> {
        let bound_ids = self.managed_bound_ids::<R>(owner, owner_id)?;
        let mut rows = catalog.managed_catalog_global_and_ids::<R>(&bound_ids)?;
        // A global row this owner ALSO directly refs is still "bound" — the
        // flag reflects ownership of the ref row, not visibility.
        rows.sort_by(|a, b| b.is_global().cmp(&a.is_global()).then_with(|| b.updated_at().cmp(&a.updated_at())));
        Ok(rows
            .into_iter()
            .map(|row| {
                let bound = bound_ids.contains(row.id());
                (row, bound)
            })
            .collect())
    }

    /// Global rows only, each paired with whether `agent_id` holds the
    /// agent-level ref. Newest first. Never returns private rows — the
    /// caller-supplied `agent_id` is unverified on the catalog RPCs that
    /// use this (reagentx P0 on PR #2329).
    pub(super) fn managed_list_global_for_agent<R: ManagedResource>(
        &self,
        catalog: &Store,
        agent_id: &str,
    ) -> Result<Vec<(R, bool)>, StoreError> {
        let globals = catalog.managed_catalog_global::<R>()?;
        let bound_ids = self.managed_bound_ids::<R>(Owner::Agent, agent_id)?;
        Ok(globals
            .into_iter()
            .map(|row| {
                let bound = bound_ids.contains(row.id());
                (row, bound)
            })
            .collect())
    }

    /// Global rows with how many agents hold an agent-level ref to each —
    /// the Armory catalog's "used by N agents". Newest first.
    pub(super) fn managed_list_global<R: ManagedResource>(&self, catalog: &Store) -> Result<Vec<(R, i64)>, StoreError> {
        let globals = catalog.managed_catalog_global::<R>()?;
        let counts = self.managed_bound_counts::<R>()?;
        Ok(globals
            .into_iter()
            .map(|row| {
                let count = counts.get(row.id()).copied().unwrap_or(0);
                (row, count)
            })
            .collect())
    }

    /// One row by id. Genuinely single-table — callers invoke this directly
    /// on whichever store owns `R::TABLE` (`catalog` for Skill/McpServer).
    pub(super) fn managed_get<R: ManagedResource>(&self, id: &str) -> Result<Option<R>, StoreError> {
        let sql = format!(
            "SELECT {cols} FROM {table} WHERE id = ?1",
            cols = R::COLUMNS.join(", "),
            table = R::TABLE,
        );
        let conn = self.conn.lock().unwrap();
        match conn.query_row(&sql, params![id], |row| R::from_row(row)) {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Delete the catalog row on `catalog` FIRST, then purge `id`'s ref rows
    /// on `self`. True if a catalog row was deleted.
    ///
    /// Order matters (Codex P1, PR #3183): with the ref-purge running first,
    /// a catalog-delete failure AFTER the ref purge already committed (disk
    /// full, the identity store locked, any I/O error) stranded the row —
    /// still present in `catalog`, but no ref points at it and it is not
    /// necessarily global, so no access-check path can find it again, and
    /// the ordinary delete RPC's own pre-check (`skill_is_bound_to`) now
    /// rejects a retry too, since the ref that check depends on is already
    /// gone. Catalog-first fixes the failure mode entirely: if the catalog
    /// delete itself errors, `?` returns before the ref purge ever runs, so
    /// the row keeps its ref(s) and stays exactly as accessible/retryable as
    /// before the call. If the catalog delete SUCCEEDS (including "0 rows,
    /// already gone") and the SUBSEQUENT ref purge then fails or a process
    /// crashes between the two, the worst case is a dangling ref pointing at
    /// an id the identity store no longer holds — the same already-accepted,
    /// gracefully-degrading gap `SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.4
    /// documents for cross-channel dangling refs (`skill_get` on a missing
    /// id returns `None`, which every read path already handles), not a new
    /// failure mode.
    pub(super) fn managed_delete<R: ManagedResource>(&self, catalog: &Store, id: &str) -> Result<bool, StoreError> {
        let deleted = {
            let conn = catalog.conn.lock().unwrap();
            let rows = conn.execute(&format!("DELETE FROM {} WHERE id = ?1", R::TABLE), params![id])?;
            rows > 0
        };
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(&format!("DELETE FROM {} WHERE {} = ?1", R::AGENT_REF_TABLE, R::REF_COL), params![id])?;
            conn.execute(&format!("DELETE FROM {} WHERE {} = ?1", R::BUNDLE_REF_TABLE, R::REF_COL), params![id])?;
        }
        Ok(deleted)
    }

    /// Purge every bundle-level ref for `bundle_id`, for one resource kind.
    ///
    /// The mirror of `managed_delete`'s ref purge, from the other side: that
    /// one runs when a catalog row goes away, this one when a bundle does.
    /// Neither can rely on the declared foreign keys, which only cascade when
    /// `PRAGMA foreign_keys` is ON — and it is only ever set in tests. The
    /// bundle side additionally *cannot* have an enforcing FK at all, since
    /// `db_bundles` lives in a different physical database from the ref tables
    /// (`migrations.rs:721-741`).
    ///
    /// Returns the number of refs removed.
    pub(super) fn managed_unbind_all_for_bundle<R: ManagedResource>(
        &self,
        bundle_id: &str,
    ) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            &format!(
                "DELETE FROM {} WHERE {} = ?1",
                R::BUNDLE_REF_TABLE,
                Owner::Bundle.key_col()
            ),
            params![bundle_id],
        )?;
        Ok(rows)
    }

    /// Insert the agent-level ref (idempotent). Errors if `agent_id` is not
    /// a LOCAL agent: the ref table's `agent_id → db_agents` FK would
    /// otherwise swallow the `INSERT OR IGNORE` silently for a cross-channel
    /// agent, which is indistinguishable by row count from "already bound"
    /// (reagentx P1 on PR #2315; REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md).
    ///
    /// Checks `db_agents`, not `db_agent_definitions`: `R::AGENT_REF_TABLE`'s
    /// FK targets `db_agents` as of Phase 3c (#3088), so checking the legacy
    /// table here would reject a bind for any agent that exists ONLY as a
    /// `db_agents` row (a template launch), even though the FK the INSERT
    /// below actually depends on would accept it.
    ///
    /// Additionally errors if `id` has no row in `catalog` — Part A of
    /// SPEC_DURABLE_BINDINGS_2026_09_10.md §5.4 drops the ref table's
    /// `skill_id`/`mcp_id → db_skills`/`db_mcp_servers` FK entirely (it used
    /// to target THIS channel's local, now-stale catalog copy), so the
    /// existence check that FK used to provide is now explicit here instead
    /// — mirroring `managed_bind_bundle`'s existing `id_store` check.
    pub(super) fn managed_bind_agent<R: ManagedResource>(
        &self,
        catalog: &Store,
        agent_id: &str,
        id: &str,
    ) -> Result<(), StoreError> {
        let agent_exists: bool = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM db_agents WHERE id = ?1)",
                params![agent_id],
                |row| row.get(0),
            )?
        };
        if !agent_exists {
            return Err(StoreError::Other(format!(
                "agent {agent_id} not found in this channel's local registry — cross-channel {} binding is not supported",
                R::KIND
            )));
        }
        if catalog.managed_get::<R>(id)?.is_none() {
            return Err(StoreError::Other(format!(
                "{} {id} not found — cannot bind it to agent {agent_id}",
                R::NAME_NOUN
            )));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO {} (agent_id, {}) VALUES (?1, ?2)",
                R::AGENT_REF_TABLE,
                R::REF_COL
            ),
            params![agent_id, id],
        )?;
        Ok(())
    }

    /// Insert the bundle-level ref (idempotent). Bundle existence is checked
    /// in `id_store`, not `self` — bundles are authoritatively written through
    /// the shared store, and `self`'s local `db_bundles` copy is essentially
    /// always empty in production (reagentx P0 on PR #2639).
    ///
    /// Also checks `id` exists in `catalog` — same Part A FK-removal reason
    /// as `managed_bind_agent`'s new check (`db_bundle_skills_ref`/
    /// `db_bundle_mcp_ref` also lost their `skill_id`/`mcp_id` FK, and this
    /// table never had a `bundle_id` FK at all — see `migrations.rs`'s own
    /// comment on these tables). Added now for the same reason as the
    /// agent-level check, even though `bundle_skill_bind`/`bundle_mcp_bind`'s
    /// current call sites already pre-check existence via `skill_get`/
    /// `mcp_server_get` before calling — see this crate's report for why
    /// this is still worth doing as a second line of defense at the store
    /// layer rather than trusting every call site forever.
    pub(super) fn managed_bind_bundle<R: ManagedResource>(
        &self,
        catalog: &Store,
        id_store: &Store,
        bundle_id: &str,
        id: &str,
    ) -> Result<(), StoreError> {
        if !id_store.bundle_exists(bundle_id)? {
            return Err(StoreError::Other(format!(
                "bundle {bundle_id} not found — cannot bind {} to a nonexistent bundle",
                R::ARTICLE_NOUN
            )));
        }
        if catalog.managed_get::<R>(id)?.is_none() {
            return Err(StoreError::Other(format!(
                "{} {id} not found — cannot bind it to bundle {bundle_id}",
                R::NAME_NOUN
            )));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO {} (bundle_id, {}) VALUES (?1, ?2)",
                R::BUNDLE_REF_TABLE,
                R::REF_COL
            ),
            params![bundle_id, id],
        )?;
        Ok(())
    }

    fn bundle_exists(&self, bundle_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_bundles WHERE id = ?1)",
            params![bundle_id],
            |row| row.get(0),
        )?)
    }

    /// Remove the owner-level ref. True if a row was removed.
    pub(super) fn managed_unbind<R: ManagedResource>(
        &self,
        owner: Owner,
        owner_id: &str,
        id: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            &format!(
                "DELETE FROM {} WHERE {} = ?1 AND {} = ?2",
                owner.ref_table::<R>(),
                owner.key_col(),
                R::REF_COL
            ),
            params![owner_id, id],
        )?;
        Ok(rows > 0)
    }

    /// Upsert `row` into `catalog` enforcing owner-scoped name uniqueness (no
    /// other row visible to the owner — bound or global — may share the
    /// name), and bind it into `self` when `bind_new`.
    ///
    /// **No longer atomic** (Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md
    /// §5.4, accepted interim gap): the catalog insert and the ref-table bind
    /// are two separate connections now, so a narrow same-name race between
    /// two concurrent upserts for a brand-new name is possible (single-owner,
    /// single-submit UI flows are not a realistic multi-writer scenario — not
    /// closed until Phase 3/6 restores single-connection atomicity). What
    /// this DOES still guarantee: if the ref-bind fails after the catalog
    /// insert already committed, a best-effort COMPENSATING DELETE removes
    /// the just-inserted catalog row before the bind error propagates, so the
    /// caller's failure and the durable state agree ("nothing was created")
    /// instead of leaving a permanent, undiscoverable orphan catalog row (no
    /// ordinary list/access/delete path can find a row that is neither bound
    /// nor global). The compensating delete's own failure is logged, never
    /// allowed to mask the original bind error.
    pub(super) fn managed_upsert_unique<R: ManagedResource>(
        &self,
        catalog: &Store,
        owner: Owner,
        owner_id: &str,
        row: &R,
        bind_new: bool,
        force_global: Option<bool>,
    ) -> Result<(), StoreError> {
        let bound_ids = self.managed_bound_ids::<R>(owner, owner_id)?;

        let dup_sql = if bound_ids.is_empty() {
            format!(
                "SELECT COUNT(*) FROM {table} WHERE name = ?1 AND id <> ?2 AND is_global = 1",
                table = R::TABLE,
            )
        } else {
            format!(
                "SELECT COUNT(*) FROM {table} WHERE name = ?1 AND id <> ?2 AND (is_global = 1 OR id IN ({ph}))",
                table = R::TABLE,
                ph = placeholders_from(3, bound_ids.len()),
            )
        };
        let dup: i64 = {
            let conn = catalog.conn.lock().unwrap();
            let mut bind: Vec<Value> = vec![Value::Text(row.name().to_string()), Value::Text(row.id().to_string())];
            bind.extend(bound_ids.iter().map(|s| Value::Text(s.clone())));
            conn.query_row(&dup_sql, params_from_iter(bind), |r| r.get(0))?
        };
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "{} name '{}' already bound to this {}",
                R::NAME_NOUN,
                row.name(),
                owner.word()
            )));
        }

        {
            let conn = catalog.conn.lock().unwrap();
            conn.execute(&insert_sql::<R>(), params_from_iter(values_with_global(row, force_global)))?;
        }

        if bind_new {
            let bind_result = {
                let conn = self.conn.lock().unwrap();
                conn.execute(
                    &format!(
                        "INSERT OR IGNORE INTO {} ({}, {}) VALUES (?1, ?2)",
                        owner.ref_table::<R>(),
                        owner.key_col(),
                        R::REF_COL
                    ),
                    params![owner_id, row.id()],
                )
            };
            if let Err(bind_err) = bind_result {
                let conn = catalog.conn.lock().unwrap();
                if let Err(compensate_err) =
                    conn.execute(&format!("DELETE FROM {} WHERE id = ?1", R::TABLE), params![row.id()])
                {
                    tracing::warn!(
                        id = row.id(),
                        bind_error = %bind_err,
                        compensate_error = %compensate_err,
                        "managed_upsert_unique: compensating delete failed after a bind error — a durable orphan catalog row may remain"
                    );
                }
                return Err(bind_err.into());
            }
        }
        Ok(())
    }

    /// Bundle-scoped upsert: create a NEW, PRIVATE (never global) row bound
    /// directly to a bundle, enforcing bundle-scoped name uniqueness. Bundle
    /// existence is checked in `id_store` (see `managed_bind_bundle`).
    /// `managed_bind_bundle` may only bind EXISTING global rows — binding an
    /// existing private row would let one bundle "steal" read access to
    /// whatever entity that row belongs to — but globals already reach every
    /// agent, so this is how a bundle gets a genuinely bundle-specific row
    /// that has never belonged to anyone else (reagentx P1 on PR #2639).
    pub(super) fn managed_upsert_unique_for_bundle<R: ManagedResource>(
        &self,
        catalog: &Store,
        id_store: &Store,
        bundle_id: &str,
        row: &R,
        bind_new: bool,
    ) -> Result<(), StoreError> {
        if !id_store.bundle_exists(bundle_id)? {
            return Err(StoreError::Other(format!(
                "bundle {bundle_id} not found — cannot create {} for a nonexistent bundle",
                R::ARTICLE_NOUN
            )));
        }
        self.managed_upsert_unique(catalog, Owner::Bundle, bundle_id, row, bind_new, Some(false))
    }

    /// Atomically upsert a GLOBAL row enforcing catalog-wide name uniqueness
    /// among every global row (not just those visible to one owner). Forces
    /// `is_global = 1` on INSERT; the caller is expected to have set it too.
    ///
    /// Pure single-table operation — callers invoke this directly on
    /// whichever store owns `R::TABLE` (`identity_store` in production).
    pub(super) fn managed_upsert_unique_global<R: ManagedResource>(&self, row: &R) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            &format!(
                "SELECT COUNT(*) FROM {} WHERE name = ?1 AND id <> ?2 AND is_global = 1",
                R::TABLE
            ),
            params![row.name(), row.id()],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "a global {} named '{}' already exists",
                R::NAME_NOUN,
                row.name()
            )));
        }
        tx.execute(&insert_sql::<R>(), params_from_iter(values_with_global(row, Some(true))))?;
        tx.commit()?;
        Ok(())
    }

    /// True when the row is global (in `catalog`) or the owner holds a ref
    /// to it (in `self`) — the read/mutation access check.
    pub(super) fn managed_is_accessible_to<R: ManagedResource>(
        &self,
        catalog: &Store,
        owner: Owner,
        owner_id: &str,
        id: &str,
    ) -> Result<bool, StoreError> {
        let is_global: Option<bool> = {
            let conn = catalog.conn.lock().unwrap();
            match conn.query_row(
                &format!("SELECT is_global FROM {} WHERE id = ?1", R::TABLE),
                params![id],
                |r| r.get::<_, i64>(0),
            ) {
                Ok(v) => Some(v != 0),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            }
        };
        match is_global {
            None => Ok(false),
            Some(true) => Ok(true),
            Some(false) => {
                let conn = self.conn.lock().unwrap();
                let count: i64 = conn.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE {} = ?1 AND {} = ?2",
                        owner.ref_table::<R>(),
                        owner.key_col(),
                        R::REF_COL
                    ),
                    params![owner_id, id],
                    |r| r.get(0),
                )?;
                Ok(count > 0)
            }
        }
    }

    /// True when the owner holds a direct ref to the row (global rows are
    /// not implied) — the ownership check for edit/delete. Pure ref-table
    /// operation on `self`.
    pub(super) fn managed_is_bound_to<R: ManagedResource>(
        &self,
        owner: Owner,
        owner_id: &str,
        id: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM {} WHERE {} = ?1 AND {} = ?2",
                owner.ref_table::<R>(),
                owner.key_col(),
                R::REF_COL
            ),
            params![owner_id, id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Union the rows referenced by `agent_id`'s bound bundle into
    /// `visible`, deduped by id (a global row is already in `visible` and
    /// would otherwise appear twice). Composable model v2 — without this,
    /// bundle-level refs would be exactly as inert at launch as the bundle's
    /// old inline JSON columns were (GH issue #2024 item 3). Silent on
    /// lookup failure, like the two callers it was lifted from.
    pub(super) fn managed_union_bundle_refs<R: ManagedResource>(
        &self,
        catalog: &Store,
        agent_id: &str,
        visible: &mut Vec<R>,
    ) {
        if let Ok(Some(def)) = self.agent_def_get(agent_id) {
            if !def.memory_id.is_empty() {
                for (row, _) in self
                    .managed_list::<R>(catalog, Owner::Bundle, &def.memory_id)
                    .unwrap_or_default()
                {
                    if !visible.iter().any(|s| s.id() == row.id()) {
                        visible.push(row);
                    }
                }
            }
        }
    }
}

fn insert_sql<R: ManagedResource>() -> String {
    format!(
        "INSERT INTO {table} ({cols})
         VALUES ({vals})
         ON CONFLICT(id) DO UPDATE SET {set}",
        table = R::TABLE,
        cols = R::COLUMNS.join(", "),
        vals = placeholders(R::COLUMNS.len()),
        set = update_set::<R>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe;
    impl ManagedResource for Probe {
        const TABLE: &'static str = "db_probe";
        const REF_COL: &'static str = "probe_id";
        const AGENT_REF_TABLE: &'static str = "db_agent_probe_ref";
        const BUNDLE_REF_TABLE: &'static str = "db_bundle_probe_ref";
        const COLUMNS: &'static [&'static str] = &["id", "name", "is_global", "created_at", "updated_at"];
        const UPDATE_COLUMNS: &'static [&'static str] = &["name", "updated_at"];
        const NAME_NOUN: &'static str = "probe";
        const ARTICLE_NOUN: &'static str = "a probe";
        const KIND: &'static str = "probe";
        fn from_row(_row: &Row<'_>) -> rusqlite::Result<Self> {
            Ok(Probe)
        }
        fn values(&self) -> Vec<Value> {
            vec![
                Value::Text("id".into()),
                Value::Text("n".into()),
                Value::Integer(0),
                Value::Integer(1),
                Value::Integer(2),
            ]
        }
        fn id(&self) -> &str {
            "id"
        }
        fn name(&self) -> &str {
            "n"
        }
        fn is_global(&self) -> bool {
            false
        }
        fn updated_at(&self) -> i64 {
            2
        }
    }

    #[test]
    fn insert_sql_lists_every_column_once_and_updates_only_the_editable_ones() {
        let sql = insert_sql::<Probe>();
        assert!(sql.contains("INSERT INTO db_probe (id, name, is_global, created_at, updated_at)"));
        assert!(sql.contains("VALUES (?1, ?2, ?3, ?4, ?5)"));
        assert!(sql.contains("ON CONFLICT(id) DO UPDATE SET name=excluded.name, updated_at=excluded.updated_at"));
        assert!(!sql.contains("is_global=excluded"), "is_global must never be rewritten on conflict");
    }

    #[test]
    fn force_global_replaces_only_the_is_global_value() {
        let forced = values_with_global(&Probe, Some(true));
        assert_eq!(forced[2], Value::Integer(1));
        assert_eq!(forced[0], Value::Text("id".into()));
        let cleared = values_with_global(&Probe, Some(false));
        assert_eq!(cleared[2], Value::Integer(0));
        assert_eq!(values_with_global(&Probe, None), Probe.values());
    }

    #[test]
    fn owner_selects_the_matching_ref_table_and_key() {
        assert_eq!(Owner::Agent.ref_table::<Probe>(), "db_agent_probe_ref");
        assert_eq!(Owner::Bundle.ref_table::<Probe>(), "db_bundle_probe_ref");
        assert_eq!(Owner::Agent.key_col(), "agent_id");
        assert_eq!(Owner::Bundle.key_col(), "bundle_id");
    }

    #[test]
    fn placeholders_from_continues_numbering_past_earlier_bound_params() {
        assert_eq!(placeholders_from(3, 0), "");
        assert_eq!(placeholders_from(3, 1), "?3");
        assert_eq!(placeholders_from(3, 3), "?3, ?4, ?5");
    }

    // ── Cross-store integration tests (Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md) ──
    //
    // Use the REAL `Skill` primitive against two genuinely separate
    // `Store::open_in_memory()` instances — one acting as the ref-owning
    // `wstore`, one as `catalog` (with the identity schema applied) — so a
    // bug that only shows up when the two stores are NOT the same connection
    // (e.g. accidentally querying `self` for a catalog row, or vice versa)
    // cannot hide the way it would if both roles resolved to one aliased
    // store (as `test_state()` does elsewhere in this codebase).

    use crate::backend::storage::skills::Skill;

    fn wstore() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn catalog_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        store
    }

    fn skill(id: &str, name: &str, is_global: bool, updated_at: i64) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            trigger: String::new(),
            skill_type: "prompt".to_string(),
            description: String::new(),
            content: "c".to_string(),
            is_global,
            created_at: updated_at,
            updated_at,
        }
    }

    fn insert_agent(store: &Store, id: &str) {
        store
            .conn()
            .lock()
            .unwrap()
            .execute("INSERT INTO db_agents (id, name, provider) VALUES (?1, 'A', 'claude')", params![id])
            .unwrap();
    }

    fn insert_bundle(store: &Store, id: &str) {
        store
            .conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO db_bundles (id, name, created_at, updated_at) VALUES (?1, ?1, 0, 0)",
                params![id],
            )
            .unwrap();
    }

    #[test]
    fn managed_list_combines_global_and_bound_private_rows_from_two_distinct_stores() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        catalog.managed_upsert_unique_global(&skill("global-1", "Global", true, 200)).unwrap();
        catalog.managed_upsert_unique_global(&skill("global-2", "Unrelated Global", true, 100)).unwrap();
        // A private row that exists in the catalog but is NOT referenced by
        // agent-1 — must not appear.
        {
            let conn = catalog.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_skills (id, name, is_global, created_at, updated_at) VALUES ('private-other', 'Someone Else', 0, 50, 50)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO db_skills (id, name, is_global, created_at, updated_at) VALUES ('private-mine', 'Mine', 0, 150, 150)",
                [],
            )
            .unwrap();
        }
        wstore.skill_bind(&catalog, "agent-1", "private-mine").unwrap();

        let list = wstore.managed_list::<Skill>(&catalog, Owner::Agent, "agent-1").unwrap();
        let ids: Vec<&str> = list.iter().map(|(s, _)| s.id.as_str()).collect();
        assert_eq!(ids, vec!["global-1", "global-2", "private-mine"], "globals first (newest first), then the bound private row");
        assert!(!list.iter().any(|(s, _)| s.id == "private-other"), "an unbound private row from another owner must not appear");

        let bound: std::collections::HashMap<&str, bool> = list.iter().map(|(s, b)| (s.id.as_str(), *b)).collect();
        assert!(!bound["global-1"], "a global the owner has not directly bound is visible but not 'bound'");
        assert!(bound["private-mine"]);
    }

    #[test]
    fn managed_list_global_reports_bound_count_from_the_wstore_side() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        insert_agent(&wstore, "agent-2");
        catalog.managed_upsert_unique_global(&skill("global-1", "Global", true, 100)).unwrap();
        wstore.skill_bind(&catalog, "agent-1", "global-1").unwrap();
        wstore.skill_bind(&catalog, "agent-2", "global-1").unwrap();

        let list = wstore.managed_list_global::<Skill>(&catalog).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1, 2, "bound_count must come from wstore's ref table, not catalog");
    }

    #[test]
    fn managed_list_global_for_agent_never_returns_private_rows() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        catalog.managed_upsert_unique_global(&skill("global-1", "Global", true, 100)).unwrap();
        {
            let conn = catalog.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_skills (id, name, is_global, created_at, updated_at) VALUES ('private-1', 'Private', 0, 50, 50)",
                [],
            )
            .unwrap();
        }
        wstore.skill_bind(&catalog, "agent-1", "global-1").unwrap();

        let list = wstore.managed_list_global_for_agent::<Skill>(&catalog, "agent-1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0.id, "global-1");
        assert!(list[0].1);
    }

    #[test]
    fn managed_is_accessible_to_checks_globality_in_catalog_and_binding_in_wstore() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        catalog.managed_upsert_unique_global(&skill("global-1", "Global", true, 100)).unwrap();
        {
            let conn = catalog.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_skills (id, name, is_global, created_at, updated_at) VALUES ('private-1', 'Private', 0, 50, 50)",
                [],
            )
            .unwrap();
        }

        assert!(wstore.managed_is_accessible_to::<Skill>(&catalog, Owner::Agent, "agent-1", "global-1").unwrap());
        assert!(!wstore.managed_is_accessible_to::<Skill>(&catalog, Owner::Agent, "agent-1", "private-1").unwrap());
        wstore.skill_bind(&catalog, "agent-1", "private-1").unwrap();
        assert!(wstore.managed_is_accessible_to::<Skill>(&catalog, Owner::Agent, "agent-1", "private-1").unwrap());
        assert!(!wstore.managed_is_accessible_to::<Skill>(&catalog, Owner::Agent, "agent-1", "no-such-id").unwrap());
    }

    #[test]
    fn managed_upsert_unique_detects_a_duplicate_name_in_the_catalog_store() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        catalog.managed_upsert_unique_global(&skill("global-1", "Deploy", true, 100)).unwrap();

        let result = wstore.managed_upsert_unique(&catalog, Owner::Agent, "agent-1", &skill("new-1", "Deploy", false, 200), true, None);
        assert!(result.is_err(), "a name already used by a visible global row must be rejected");
        // The catalog must not have gained a row from the rejected attempt.
        assert!(catalog.managed_get::<Skill>("new-1").unwrap().is_none());
    }

    #[test]
    fn managed_upsert_unique_compensates_by_deleting_the_catalog_row_when_the_bind_fails() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        // Force the ref-insert to fail deterministically: drop the ref table
        // out from under it (a stand-in for the "lock contention / disk
        // full / a channel-local constraint" failures the doc comment
        // describes — `INSERT OR IGNORE` swallows an agent_id FK violation
        // silently rather than erroring, so a missing db_agents row can't be
        // used to exercise this path; a missing TABLE is a real, unignored
        // SQL error).
        wstore.conn().lock().unwrap().execute_batch("DROP TABLE db_agent_skills_ref;").unwrap();

        let result = wstore.managed_upsert_unique(&catalog, Owner::Agent, "agent-1", &skill("new-1", "Deploy", false, 200), true, None);
        assert!(result.is_err(), "the bind must fail now that its table is gone");
        assert!(
            catalog.managed_get::<Skill>("new-1").unwrap().is_none(),
            "the catalog insert must be compensated away, leaving no orphan row"
        );
    }

    #[test]
    fn managed_upsert_unique_leaves_the_catalog_row_when_bind_new_is_false() {
        let wstore = wstore();
        let catalog = catalog_store();
        wstore.managed_upsert_unique(&catalog, Owner::Agent, "agent-1", &skill("new-1", "Deploy", false, 200), false, None).unwrap();
        assert!(catalog.managed_get::<Skill>("new-1").unwrap().is_some());
    }

    #[test]
    fn managed_delete_purges_wstore_refs_and_removes_the_catalog_row() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        insert_bundle(&wstore, "bundle-1");
        catalog.managed_upsert_unique_global(&skill("global-1", "Global", true, 100)).unwrap();
        wstore.skill_bind(&catalog, "agent-1", "global-1").unwrap();
        wstore.bundle_skill_bind(&catalog, &wstore, "bundle-1", "global-1").unwrap();

        let deleted = wstore.managed_delete::<Skill>(&catalog, "global-1").unwrap();
        assert!(deleted);
        assert!(catalog.managed_get::<Skill>("global-1").unwrap().is_none());
        assert!(!wstore.skill_is_bound_to("agent-1", "global-1").unwrap());
        assert!(!wstore.bundle_skill_is_bound_to("bundle-1", "global-1").unwrap());
    }

    #[test]
    fn managed_bind_agent_succeeds_when_the_catalog_row_exists_in_the_separate_store() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");
        catalog.managed_upsert_unique_global(&skill("global-1", "Global", true, 100)).unwrap();

        wstore.skill_bind(&catalog, "agent-1", "global-1").unwrap();
        assert!(wstore.skill_is_bound_to("agent-1", "global-1").unwrap());
    }

    #[test]
    fn managed_bind_agent_rejects_a_catalog_row_that_does_not_exist_anywhere() {
        let wstore = wstore();
        let catalog = catalog_store();
        insert_agent(&wstore, "agent-1");

        let result = wstore.skill_bind(&catalog, "agent-1", "no-such-skill");
        assert!(result.is_err(), "binding an id with no row in catalog must error, not silently no-op");
        assert!(!wstore.skill_is_bound_to("agent-1", "no-such-skill").unwrap());
    }
}
