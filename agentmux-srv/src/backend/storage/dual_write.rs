// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `db_agents` dual-write helpers.
//!
//! Every mutation on `db_agent_definitions` / `db_agent_instances`
//! mirrors into `db_agents`. Originally written as Phase 3a with the old
//! tables authoritative and dual-write failures logged + continued until
//! a later Phase 3b read-migration PR — that framing is now stale.
//! `agent_def_list()` (`agents.rs`) already reads `db_agents` (an
//! undocumented partial Phase 3b flip found 2026-08-15, see
//! `ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §3.1), and
//! `agents_dual_write_definition_upsert` below now hard-fails
//! (`Result<(), StoreError>`, propagated with `?` at its call site) rather
//! than logging and continuing. Other functions in this file may not have
//! made the same transition — check each one's own doc comment rather
//! than assuming a single global phase; see
//! `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
//! P4. See `docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md` for
//! the original design.
//!
//! Extracted from `store.rs` in Phase R.5 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). This
//! whole file goes away in Phase 3c when the legacy tables are
//! dropped — keeping it isolated makes that future deletion clean.

use rusqlite::{params, Connection};

use super::error::StoreError;
use super::store::{AgentDefinition, AgentInstance, Store};

impl Store {
    /// Mirror a `db_agent_definitions` row into `db_agents` as the
    /// canonical row for that definition.
    ///
    /// - `is_seeded = 1` rows become `is_template = 1` (canonical
    ///   template; bindings stay empty).
    /// - `is_seeded = 0` rows become `is_template = 0` with
    ///   `parent_template_id = parent_id` (user-cloned from a template;
    ///   bindings come from the matching instance, if any — handled by
    ///   `agents_dual_write_instance_create` updating in place).
    ///
    /// Idempotent: uses `INSERT … ON CONFLICT(id) DO UPDATE`. Existing
    /// bindings on the row (written previously by an instance dual-write)
    /// are preserved — only definition-side fields are overwritten.
    ///
    /// Phase 3b: returns `Err` on failure (previously logged + continued).
    /// Phase 3b readers see `db_agents`, so a silent dual-write failure
    /// would leak stale data into the picker.
    pub(crate) fn agents_dual_write_definition_upsert(
        &self,
        def: &AgentDefinition,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let is_template = if def.is_seeded == 1 { 1_i64 } else { 0_i64 };
        let parent_template_id = if def.is_seeded == 1 {
            String::new()
        } else {
            def.parent_id.clone()
        };
        // Phase 3b: carry the caller's `user_hidden` into the INSERT
        // (previously hardcoded to 0). The legacy `db_agent_definitions`
        // INSERT honours it, and now that Phase 3b readers look at
        // `db_agents`, the projection must agree from row creation —
        // not just after a subsequent `agent_def_set_hidden` flip.
        // ON CONFLICT deliberately does NOT update `user_hidden`: hide
        // is a per-user view-state flag that survives definition
        // payload edits.
        conn.execute(
            "INSERT INTO db_agents (
                id, name, icon, description,
                is_template, parent_template_id,
                provider, provider_flags, shell, environment,
                agent_type, agent_bus_id, accounts,
                auto_start, restart_on_crash, idle_timeout_minutes,
                slug, branch_label, working_directory,
                created_at, updated_at, is_seeded, user_hidden,
                container_image, container_volumes, container_name,
                use_ambient_login, model_vendor_base_url, auto_continue_enabled,
                default_memory_id, conversation_visibility
             ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6,
                ?7, ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15, ?16,
                ?17, ?18, ?19,
                ?20, ?21, ?22, ?23,
                ?24, ?25, ?26,
                ?27, ?28, ?29,
                ?30, ?31
             )
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                icon = excluded.icon,
                description = excluded.description,
                is_template = excluded.is_template,
                parent_template_id = excluded.parent_template_id,
                provider = excluded.provider,
                provider_flags = excluded.provider_flags,
                shell = excluded.shell,
                environment = excluded.environment,
                agent_type = excluded.agent_type,
                agent_bus_id = excluded.agent_bus_id,
                accounts = excluded.accounts,
                auto_start = excluded.auto_start,
                restart_on_crash = excluded.restart_on_crash,
                idle_timeout_minutes = excluded.idle_timeout_minutes,
                slug = excluded.slug,
                branch_label = excluded.branch_label,
                working_directory = excluded.working_directory,
                updated_at = excluded.updated_at,
                is_seeded = excluded.is_seeded,
                container_image = excluded.container_image,
                container_volumes = excluded.container_volumes,
                container_name = excluded.container_name,
                use_ambient_login = excluded.use_ambient_login,
                model_vendor_base_url = excluded.model_vendor_base_url,
                auto_continue_enabled = excluded.auto_continue_enabled,
                default_memory_id = excluded.default_memory_id,
                conversation_visibility = excluded.conversation_visibility",
            params![
                def.id,
                def.name,
                def.icon,
                def.description,
                is_template,
                parent_template_id,
                def.provider,
                def.provider_flags,
                def.shell,
                def.environment,
                def.agent_type,
                def.agent_bus_id,
                def.accounts,
                def.auto_start,
                def.restart_on_crash,
                def.idle_timeout_minutes,
                def.slug,
                def.branch_label,
                def.working_directory,
                def.created_at,
                def.updated_at,
                def.is_seeded,
                def.user_hidden,
                def.container_image,
                def.container_volumes,
                def.container_name,
                def.use_ambient_login,
                def.model_vendor_base_url,
                def.auto_continue_enabled,
                def.memory_id,
                def.conversation_visibility,
            ],
        )?;
        Ok(())
    }

    /// Mirror a `db_agent_definitions` DELETE into `db_agents`. The
    /// definition row itself is removed; any user-cloned children (rows
    /// with `parent_template_id = old_id`) are left intact because the
    /// FK cascade on the OLD schema only deletes instances, not other
    /// definitions.
    pub(crate) fn agents_dual_write_definition_delete(
        &self,
        def_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Reagent P2 on #1013 round 3: `id` is the PK so two DELETE
        // statements scoped by `id = ?1 AND is_template = N` add nothing
        // over a single PK delete (only one row can match either, and
        // an early return on the first error would skip the second
        // cleanup unnecessarily). Collapsed to a single direct PK
        // delete that handles both template and user-clone projections.
        conn.execute("DELETE FROM db_agents WHERE id = ?1", params![def_id])?;
        Ok(())
    }

    /// Bulk dual-write: mirror `agent_def_delete_seeded`. Deletes:
    ///   1. every `is_template = 1` row (the template projections), AND
    ///   2. every `db_agents` row whose `id` is in the
    ///      `cascaded_inst_ids` set (template-instance projections that
    ///      were just removed by the FK cascade on `db_agent_instances`).
    ///
    /// User-clone DEFINITION projections (`is_template = 0`, `id` is a
    /// def_id in `db_agent_definitions`) are NOT touched — those rows
    /// represent persistent user agents and live or die with their
    /// `db_agent_definitions` row, not with the seeded-template bulk
    /// delete. Reagent P1 round 4 on #1013: the previous version
    /// scoped by `parent_template_id` and over-deleted user-clone DEF
    /// projections too. Idempotent.
    pub(crate) fn agents_dual_write_seeded_delete(
        &self,
        cascaded_inst_ids: &[String],
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_agents WHERE is_template = 1", [])?;
        // Delete cascaded instance projections one by one. Could batch
        // via `id IN (?1, ?2, ...)` but the seeded delete is rare and
        // the typical set is small (< 100); per-id loop keeps the SQL
        // simple and avoids dynamic-parameter expansion.
        for inst_id in cascaded_inst_ids {
            conn.execute(
                "DELETE FROM db_agents WHERE id = ?1 AND is_template = 0",
                params![inst_id],
            )?;
        }
        Ok(())
    }

    /// Mirror a `db_agent_instances` INSERT into `db_agents`. Always
    /// creates a NEW row in `db_agents` whose id == instance id.
    ///
    /// The row's identity comes from the instance (id, name, bindings).
    /// Its template-config fields are copied from the parent definition;
    /// `parent_template_id` points at that definition.
    ///
    /// Continuations (`parent_instance_id` non-empty) mirror their
    /// new bindings into the chain's existing `db_agents` row rather
    /// than creating a separate one. The chain root is resolved via
    /// `agents_projection_key_for_inst`. Codex P2 on PR #1110 — without
    /// this, a named-agent rebind (continue with different identity,
    /// memory, cwd, or github context) never reaches the consolidated
    /// row and Phase 3b readers see stale data.
    pub(crate) fn agents_dual_write_instance_create(
        &self,
        inst: &AgentInstance,
    ) -> Result<(), StoreError> {
        let is_continuation = !inst.parent_instance_id.is_empty();
        let conn = self.conn.lock().unwrap();
        // Pull the parent definition for the cmd-config fields.
        let def = match Self::load_definition_for_dual_write(&conn, &inst.definition_id)? {
            Some(d) => d,
            None => {
                // Orphan instance — no matching definition. The legacy
                // table accepted the row (no FK in the old test stores),
                // but with nothing to project there's nothing this
                // helper can mirror. Surfacing this as Err would block
                // a write the legacy path tolerated; log at error level
                // and continue with no projection.
                tracing::error!(
                    instance_id = %inst.id,
                    definition_id = %inst.definition_id,
                    "db_agents dual-write: instance has no matching definition; skipping mirror",
                );
                return Ok(());
            }
        };
        let name = if inst.instance_name.is_empty() {
            def.name.clone()
        } else {
            inst.instance_name.clone()
        };
        // Reagent P1 on #1013 round 2: match the backfill rule
        // (`agents_consolidate.rs::backfill_instances`) exactly. When the
        // parent def is a user-clone (`is_seeded = 0`), the existing
        // `db_agents` row keyed by `def.id` already represents this
        // agent — FOLD the instance's bindings into it instead of
        // inserting a new row keyed by `inst.id` with
        // `parent_template_id = def.id` (which would point at a
        // non-template row and produce a shape inconsistent with what
        // backfill produced for identical data).
        // Monotonic-bump helper. See the original comment below for
        // the full reasoning; extracted into a closure so both the
        // user-clone path and the new continuation-mirror path can
        // reuse it. Successive fast-fire launches (e.g. test loops on
        // Windows millisecond-resolution clocks) need the strict
        // global ordering to survive ORDER BY updated_at.
        //
        // Reagent P2 round 3 on #1013: don't write `inst.created_at`
        // here — the user-clone def may already have a fresher
        // `updated_at` from a prior `agent_def_update`. Wall-clock
        // now() is the right monotonic stamp.
        let now_ms = {
            let wall_now: i64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(inst.created_at);
            let global_prior: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(updated_at), 0) FROM db_agents",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0);
            std::cmp::max(wall_now, global_prior.saturating_add(1))
        };

        let res = if def.is_seeded == 0 {
            // User-clone def — one db_agents row per def, keyed by
            // def.id. Continuations and head launches both UPDATE the
            // same row. (The chain's identity has nothing to do with
            // which row gets touched here; it's always def.id.)
            conn.execute(
                "UPDATE db_agents SET
                    name = ?2,
                    identity_id = ?3,
                    memory_id = ?4,
                    working_directory = ?5,
                    github_context = ?6,
                    instance_name = ?7,
                    updated_at = ?8,
                    user_hidden = ?9,
                    last_block_id = ?10
                 WHERE id = ?1",
                params![
                    def.id,
                    name,
                    inst.identity_id,
                    inst.memory_id,
                    def.working_directory,
                    inst.github_context,
                    inst.instance_name,
                    now_ms,
                    if inst.display_hidden { 1_i64 } else { 0_i64 },
                    inst.block_id,
                ],
            )
        } else if is_continuation {
            // Template-instance continuation — UPDATE the chain head's
            // row with the new bindings. The head's id is found via
            // `agents_projection_key_for_inst` (which walks parent
            // _instance_id up to the root). No new row is created;
            // there's exactly one db_agents row per logical agent.
            // Codex P2 on PR #1110.
            let root_id = match Self::agents_projection_key_for_inst(&conn, &inst.id) {
                Some((k, _)) => k,
                None => {
                    tracing::warn!(
                        instance_id = %inst.id,
                        parent_instance_id = %inst.parent_instance_id,
                        "db_agents dual-write: continuation chain has no resolvable root; skipping mirror",
                    );
                    return Ok(());
                }
            };
            conn.execute(
                "UPDATE db_agents SET
                    name = ?2,
                    identity_id = ?3,
                    memory_id = ?4,
                    working_directory = ?5,
                    github_context = ?6,
                    instance_name = ?7,
                    updated_at = ?8,
                    user_hidden = ?9,
                    last_block_id = ?10
                 WHERE id = ?1 AND is_template = 0",
                params![
                    root_id,
                    name,
                    inst.identity_id,
                    inst.memory_id,
                    def.working_directory,
                    inst.github_context,
                    inst.instance_name,
                    now_ms,
                    if inst.display_hidden { 1_i64 } else { 0_i64 },
                    inst.block_id,
                ],
            )
        } else {
            // Template-instance head — INSERT a new row keyed by inst.id.
            conn.execute(
                "INSERT INTO db_agents (
                    id, name, icon, description,
                    is_template, parent_template_id,
                    provider, provider_flags, shell, environment,
                    agent_type, agent_bus_id, accounts,
                    auto_start, restart_on_crash, idle_timeout_minutes,
                    slug, branch_label,
                    identity_id, memory_id, working_directory, github_context,
                    instance_name,
                    created_at, updated_at, is_seeded, user_hidden,
                    last_block_id,
                    container_image, container_volumes, container_name,
                    use_ambient_login
                 ) VALUES (
                    ?1, ?2, ?3, ?4,
                    0, ?5,
                    ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12,
                    ?13, ?14, ?15,
                    ?16, ?17,
                    ?18, ?19, ?20, ?21,
                    ?22,
                    ?23, ?24, 0, ?25,
                    ?26,
                    ?27, ?28, ?29,
                    ?30
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    identity_id = excluded.identity_id,
                    memory_id = excluded.memory_id,
                    working_directory = excluded.working_directory,
                    github_context = excluded.github_context,
                    instance_name = excluded.instance_name,
                    updated_at = excluded.updated_at,
                    user_hidden = excluded.user_hidden,
                    last_block_id = excluded.last_block_id",
                params![
                    inst.id,
                    name,
                    def.icon,
                    def.description,
                    def.id, // parent_template_id = definition_id
                    def.provider,
                    def.provider_flags,
                    def.shell,
                    def.environment,
                    def.agent_type,
                    def.agent_bus_id,
                    def.accounts,
                    def.auto_start,
                    def.restart_on_crash,
                    def.idle_timeout_minutes,
                    def.slug,
                    def.branch_label,
                    inst.identity_id,
                    inst.memory_id,
                    def.working_directory,
                    inst.github_context,
                    inst.instance_name,
                    inst.created_at,
                    inst.created_at, // updated_at = created_at on insert
                    if inst.display_hidden { 1_i64 } else { 0_i64 },
                    inst.block_id,
                    def.container_image,
                    def.container_volumes,
                    def.container_name,
                    def.use_ambient_login,
                ],
            )
        };
        res?;
        Ok(())
    }

    /// Mirror a `db_agent_instances` UPDATE into `db_agents`. Touches
    /// only the fields that `instance_update` writes (block + session +
    /// status + github_context + ended_at) — name/bindings come from
    /// the original create.
    ///
    /// Continuations flow through here too — `agents_projection_key_for_inst`
    /// walks up the chain to the head's id, so a continuation's
    /// `github_context` refresh lands on the canonical row (codex P2 on
    /// PR #1110, paired with the create-path fix in
    /// `agents_dual_write_instance_create`).
    pub(crate) fn agents_dual_write_instance_update(
        &self,
        inst: &AgentInstance,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // Reagent P1 round 4 on #1013: route by projection key so the
        // UPDATE actually hits the folded user-clone-def row when this
        // instance was folded at create. The previous version keyed by
        // `inst.id` always and silently no-op'd on every folded
        // instance's lifecycle event.
        let key = match Self::agents_projection_key_for_inst(&conn, &inst.id) {
            Some((k, _)) => k,
            None => return Ok(()),
        };
        // `instance_update` only touches block_id/session_id/status/
        // github_context/ended_at. Of those, only `github_context` lands
        // on db_agents (block/session/status/ended_at are not modelled
        // on the consolidated row — they're block/session-machine
        // concerns the consolidation deliberately drops). We DO refresh
        // updated_at so a Phase-3b reader can sort by recency. Apply
        // the same monotonic-floor trick as the fold branch in
        // `agents_dual_write_instance_create` — wall clock alone
        // collides under millisecond resolution on fast successive
        // mutations.
        let global_prior: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(updated_at), 0) FROM db_agents",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        let now_monotonic = std::cmp::max(now, global_prior.saturating_add(1));
        conn.execute(
            "UPDATE db_agents SET
                github_context = ?1,
                updated_at = ?2
             WHERE id = ?3 AND is_template = 0",
            params![inst.github_context, now_monotonic, key],
        )?;
        Ok(())
    }

    /// Mirror `instance_set_hidden` into `db_agents.user_hidden`.
    pub(crate) fn agents_dual_write_instance_set_hidden(
        &self,
        id: &str,
        hidden: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Reagent P1 round 4 on #1013: route by projection key (see
        // `agents_dual_write_instance_update` for context).
        let key = match Self::agents_projection_key_for_inst(&conn, id) {
            Some((k, _)) => k,
            None => return Ok(()),
        };
        conn.execute(
            "UPDATE db_agents SET user_hidden = ?1 WHERE id = ?2 AND is_template = 0",
            params![if hidden { 1_i64 } else { 0_i64 }, key],
        )?;
        Ok(())
    }

    /// Mirror `instance_repoint_definition` into `db_agents`. The
    /// `parent_template_id` of every user-clone row that pointed at
    /// `old_def_id` is updated to `new_def_id`.
    pub(crate) fn agents_dual_write_instance_repoint(
        &self,
        old_def_id: &str,
        new_def_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            // Reagent P1 round 3 on #1013: `instance_repoint_definition`
            // only rewrites `db_agent_instances.definition_id` — it
            // does NOT rewrite the parent of a sibling user-cloned
            // definition that happens to share the same parent template.
            // Restrict the projection update to rows whose `id` is an
            // ACTUALLY repointed instance id (i.e. the rows whose
            // `db_agent_instances.definition_id` was just rewritten to
            // `new_def_id`). User-clone definition projections, whose
            // `id` lives in `db_agent_definitions` not `db_agent_instances`,
            // are untouched.
            "UPDATE db_agents
             SET parent_template_id = ?1
             WHERE is_template = 0
               AND parent_template_id = ?2
               AND id IN (SELECT id FROM db_agent_instances WHERE definition_id = ?1)",
            params![new_def_id, old_def_id],
        )?;
        Ok(())
    }

    /// Mirror `instance_delete` into `db_agents`.
    pub(crate) fn agents_dual_write_instance_delete(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Reagent P1 round 4 on #1013: route by projection key. For a
        // template-instance projection (`is_folded = false`), the row
        // lives at `id` and goes when the instance goes. For a
        // user-clone-def fold (`is_folded = true`), the projection at
        // `def.id` represents the DEF — the def persists when its
        // instance ends, so NO-OP. `agents_dual_write_definition_delete`
        // is the right entry point to remove that row.
        //
        // Race fallback: `instance_delete` runs the DELETE on
        // `db_agent_instances` BEFORE calling this helper, so by now
        // the instance row is gone and `agents_projection_key_for_inst`
        // returns None. Fall back to checking `db_agents` directly: if
        // there's a row at `id` with `is_template = 0`, that's a
        // non-folded projection — delete it. (Folded projections live
        // at `def.id`, never at `inst.id`, so this is safe.)
        let key_info = Self::agents_projection_key_for_inst(&conn, id);
        let (key, is_folded) = match key_info {
            Some(info) => info,
            None => (id.to_string(), false),
        };
        if is_folded {
            return Ok(());
        }
        conn.execute(
            "DELETE FROM db_agents WHERE id = ?1 AND is_template = 0",
            params![key],
        )?;
        Ok(())
    }

    /// Reagent P1 round 4 on #1013 — fold-aware projection key lookup.
    /// Resolves "for an instance with this id, which `db_agents` row
    /// represents it post-create?". Returns `Some((key, is_folded))`:
    ///   - `is_folded = true`  → key is the parent definition id
    ///                            (the user-clone-def projection absorbs
    ///                            the instance's bindings).
    ///   - `is_folded = false` → key is the instance id (template-instance
    ///                            projection is its own row).
    /// Returns `None` if the instance no longer exists in
    /// `db_agent_instances` (e.g. already deleted by FK cascade).
    fn agents_projection_key_for_inst(
        conn: &Connection,
        inst_id: &str,
    ) -> Option<(String, bool)> {
        // Codex P2 on PR #1110: continuations of template-instances
        // must resolve to the chain HEAD's id, not the continuation's
        // own id — there's exactly one db_agents row per logical
        // agent, keyed by the head. Walk up `parent_instance_id` via
        // a recursive CTE; the row whose parent is `''` (or whose
        // parent no longer exists) is the root.
        //
        // For user-clone defs (`is_seeded = 0`) the projection key is
        // the def.id regardless of where in the chain we are — the
        // existing one-row-per-def behavior is unchanged. Only the
        // is_seeded=1 (template) branch needs the chain walk.
        conn.query_row(
            "WITH RECURSIVE chain(id, parent_instance_id, definition_id) AS (
                SELECT id, parent_instance_id, definition_id
                FROM db_agent_instances WHERE id = ?1
                UNION ALL
                SELECT p.id, p.parent_instance_id, p.definition_id
                FROM db_agent_instances p
                JOIN chain c ON p.id = c.parent_instance_id
             )
             SELECT c.id, c.definition_id, COALESCE(d.is_seeded, 1) AS is_seeded
             FROM chain c
             LEFT JOIN db_agent_definitions d ON c.definition_id = d.id
             WHERE c.parent_instance_id = ''
                OR NOT EXISTS (
                    SELECT 1 FROM db_agent_instances q
                    WHERE q.id = c.parent_instance_id
                )
             LIMIT 1",
            params![inst_id],
            |row| {
                let root_id: String = row.get(0)?;
                let def_id: String = row.get(1)?;
                let is_seeded: i64 = row.get(2)?;
                Ok(if is_seeded == 0 {
                    (def_id, true) // folded into def-projection
                } else {
                    (root_id, false) // chain head's row
                })
            },
        )
        .ok()
    }

    /// Helper: re-read a definition row from inside an active connection
    /// lock (used by `agents_dual_write_instance_create` to avoid
    /// re-locking the mutex recursively).
    fn load_definition_for_dual_write(
        conn: &Connection,
        id: &str,
    ) -> rusqlite::Result<Option<AgentDefinition>> {
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_id, branch_label, updated_at,
                    user_hidden, container_image, container_volumes, container_name,
                    use_ambient_login, model_vendor_base_url, auto_continue_enabled, memory_id,
                    conversation_visibility
             FROM db_agent_definitions WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            Ok(AgentDefinition {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                icon: row.get(3)?,
                provider: row.get(4)?,
                description: row.get(5)?,
                working_directory: row.get(6)?,
                shell: row.get(7)?,
                provider_flags: row.get(8)?,
                auto_start: row.get(9)?,
                restart_on_crash: row.get(10)?,
                idle_timeout_minutes: row.get(11)?,
                created_at: row.get(12)?,
                agent_type: row.get(13)?,
                environment: row.get(14)?,
                agent_bus_id: row.get(15)?,
                is_seeded: row.get(16)?,
                accounts: row.get(17)?,
                parent_id: row.get(18)?,
                branch_label: row.get(19)?,
                updated_at: row.get(20)?,
                user_hidden: row.get(21)?,
                container_image: row.get(22)?,
                container_volumes: row.get(23)?,
                container_name: row.get(24)?,
                use_ambient_login: row.get(25)?,
                model_vendor_base_url: row.get(26)?,
                auto_continue_enabled: row.get(27)?,
                memory_id: row.get(28)?,
                conversation_visibility: row.get(29)?,
            })
        });
        match result {
            Ok(d) => Ok(Some(d)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
