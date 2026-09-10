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

fn aliased_cols<R: ManagedResource>(alias: &str) -> String {
    R::COLUMNS
        .iter()
        .map(|c| format!("{alias}.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn placeholders(n: usize) -> String {
    (1..=n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
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
    /// Rows visible to `owner_id` — its own referenced rows plus every global
    /// row — each paired with whether this owner holds the ref. Global rows
    /// first, newest first.
    pub(super) fn managed_list<R: ManagedResource>(
        &self,
        owner: Owner,
        owner_id: &str,
    ) -> Result<Vec<(R, bool)>, StoreError> {
        let sql = format!(
            "SELECT {cols},
                    EXISTS(SELECT 1 FROM {reft} r WHERE r.{refc} = s.id AND r.{key} = ?1) AS bound
             FROM {table} s
             WHERE s.is_global = 1
                OR s.id IN (SELECT {refc} FROM {reft} WHERE {key} = ?1)
             ORDER BY s.is_global DESC, s.updated_at DESC",
            cols = aliased_cols::<R>("s"),
            reft = owner.ref_table::<R>(),
            refc = R::REF_COL,
            key = owner.key_col(),
            table = R::TABLE,
        );
        self.managed_rows_with_flag::<R>(&sql, owner_id)
    }

    /// Global rows only, each paired with whether `agent_id` holds the
    /// agent-level ref. Newest first. Never returns private rows — the
    /// caller-supplied `agent_id` is unverified on the catalog RPCs that
    /// use this (reagentx P0 on PR #2329).
    pub(super) fn managed_list_global_for_agent<R: ManagedResource>(
        &self,
        agent_id: &str,
    ) -> Result<Vec<(R, bool)>, StoreError> {
        let sql = format!(
            "SELECT {cols},
                    EXISTS(SELECT 1 FROM {reft} r WHERE r.{refc} = s.id AND r.agent_id = ?1) AS bound
             FROM {table} s
             WHERE s.is_global = 1
             ORDER BY s.updated_at DESC",
            cols = aliased_cols::<R>("s"),
            reft = R::AGENT_REF_TABLE,
            refc = R::REF_COL,
            table = R::TABLE,
        );
        self.managed_rows_with_flag::<R>(&sql, agent_id)
    }

    fn managed_rows_with_flag<R: ManagedResource>(
        &self,
        sql: &str,
        key: &str,
    ) -> Result<Vec<(R, bool)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(sql)?;
        let flag_idx = R::COLUMNS.len();
        let rows = stmt.query_map(params![key], |row| {
            Ok((R::from_row(row)?, row.get::<_, i64>(flag_idx)? != 0))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Global rows with how many agents hold an agent-level ref to each —
    /// the Armory catalog's "used by N agents". Newest first.
    pub(super) fn managed_list_global<R: ManagedResource>(&self) -> Result<Vec<(R, i64)>, StoreError> {
        let sql = format!(
            "SELECT {cols},
                    (SELECT COUNT(*) FROM {reft} r WHERE r.{refc} = s.id) AS bound_count
             FROM {table} s
             WHERE s.is_global = 1
             ORDER BY s.updated_at DESC",
            cols = aliased_cols::<R>("s"),
            reft = R::AGENT_REF_TABLE,
            refc = R::REF_COL,
            table = R::TABLE,
        );
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;
        let count_idx = R::COLUMNS.len();
        let rows = stmt.query_map([], |row| Ok((R::from_row(row)?, row.get::<_, i64>(count_idx)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// One row by id.
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

    /// Delete a row and purge its agent- and bundle-level refs explicitly
    /// (FK cascades may be off on some builds). True if a row was deleted.
    pub(super) fn managed_delete<R: ManagedResource>(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!("DELETE FROM {} WHERE {} = ?1", R::AGENT_REF_TABLE, R::REF_COL),
            params![id],
        )?;
        conn.execute(
            &format!("DELETE FROM {} WHERE {} = ?1", R::BUNDLE_REF_TABLE, R::REF_COL),
            params![id],
        )?;
        let rows = conn.execute(&format!("DELETE FROM {} WHERE id = ?1", R::TABLE), params![id])?;
        Ok(rows > 0)
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
    /// a LOCAL agent: the ref table's FK would otherwise swallow
    /// the `INSERT OR IGNORE` silently for a cross-channel agent, which is
    /// indistinguishable by row count from "already bound" (reagentx P1 on
    /// PR #2315; REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md).
    ///
    /// Checks `db_agents`, not `db_agent_definitions`: `R::AGENT_REF_TABLE`'s
    /// FK targets `db_agents` as of Phase 3c (#3088), so checking the legacy
    /// table here would reject a bind for any agent that exists ONLY as a
    /// `db_agents` row (a template launch), even though the FK the INSERT
    /// below actually depends on would accept it.
    pub(super) fn managed_bind_agent<R: ManagedResource>(&self, agent_id: &str, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let agent_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_agents WHERE id = ?1)",
            params![agent_id],
            |row| row.get(0),
        )?;
        if !agent_exists {
            return Err(StoreError::Other(format!(
                "agent {agent_id} not found in this channel's local registry — cross-channel {} binding is not supported",
                R::KIND
            )));
        }
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
    pub(super) fn managed_bind_bundle<R: ManagedResource>(
        &self,
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

    /// Atomically upsert `row` enforcing owner-scoped name uniqueness (no
    /// other row visible to the owner — bound or global — may share the
    /// name), and bind it when `bind_new`, in one transaction so concurrent
    /// upserts of the same name can't both pass the check. `force_global`
    /// overrides the row's own `is_global` on INSERT: the bundle-scoped
    /// upsert always creates PRIVATE rows (`Some(false)`), the agent-scoped
    /// one keeps the row's value (`None`).
    pub(super) fn managed_upsert_unique<R: ManagedResource>(
        &self,
        owner: Owner,
        owner_id: &str,
        row: &R,
        bind_new: bool,
        force_global: Option<bool>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            &format!(
                "SELECT COUNT(*) FROM {table}
                 WHERE name = ?1 AND id <> ?2 AND (is_global = 1 OR id IN (
                   SELECT {refc} FROM {reft} WHERE {key} = ?3
                 ))",
                table = R::TABLE,
                refc = R::REF_COL,
                reft = owner.ref_table::<R>(),
                key = owner.key_col(),
            ),
            params![row.name(), row.id(), owner_id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "{} name '{}' already bound to this {}",
                R::NAME_NOUN,
                row.name(),
                owner.word()
            )));
        }
        tx.execute(&insert_sql::<R>(), params_from_iter(values_with_global(row, force_global)))?;
        if bind_new {
            tx.execute(
                &format!(
                    "INSERT OR IGNORE INTO {} ({}, {}) VALUES (?1, ?2)",
                    owner.ref_table::<R>(),
                    owner.key_col(),
                    R::REF_COL
                ),
                params![owner_id, row.id()],
            )?;
        }
        tx.commit()?;
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
        self.managed_upsert_unique(Owner::Bundle, bundle_id, row, bind_new, Some(false))
    }

    /// Atomically upsert a GLOBAL row enforcing catalog-wide name uniqueness
    /// among every global row (not just those visible to one owner). Forces
    /// `is_global = 1` on INSERT; the caller is expected to have set it too.
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

    /// True when the row is global or the owner holds a ref to it — the
    /// read/mutation access check.
    pub(super) fn managed_is_accessible_to<R: ManagedResource>(
        &self,
        owner: Owner,
        owner_id: &str,
        id: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM {table}
                 WHERE id = ?1 AND (is_global = 1 OR id IN (
                   SELECT {refc} FROM {reft} WHERE {key} = ?2
                 ))",
                table = R::TABLE,
                refc = R::REF_COL,
                reft = owner.ref_table::<R>(),
                key = owner.key_col(),
            ),
            params![id, owner_id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// True when the owner holds a direct ref to the row (global rows are
    /// not implied) — the ownership check for edit/delete.
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
    pub(super) fn managed_union_bundle_refs<R: ManagedResource>(&self, agent_id: &str, visible: &mut Vec<R>) {
        if let Ok(Some(def)) = self.agent_def_get(agent_id) {
            if !def.memory_id.is_empty() {
                for (row, _) in self
                    .managed_list::<R>(Owner::Bundle, &def.memory_id)
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
}
