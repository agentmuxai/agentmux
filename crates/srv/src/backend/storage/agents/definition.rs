// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent_def_*`: the agent-definition catalog's CRUD on `db_agents`.

use super::*;

impl Store {
    /// Fetch a single agent definition by primary key. Reads `db_agents`
    /// directly (the same source `agent_def_list` reads, via the shared
    /// private `agent_row_get`) — as of the definition flip this is no longer
    /// scoped to definitions/templates only: any `db_agents` row, including
    /// a bare template launch with no `db_agent_definitions` counterpart,
    /// resolves here, matching what `agent_def_list` already returns for
    /// it. Falls back to the global cross-channel registry when the row
    /// doesn't exist locally at all.
    ///
    /// Used by the `template_promote` migration's deterministic-id
    /// idempotency check (see
    /// `migrate_promote_template_sessions_v1`): every retry asks
    /// "does the promote-target clone for this template already
    /// exist?" and either reuses it or inserts it.
    pub fn agent_def_get(&self, id: &str) -> Result<Option<AgentDefinition>, StoreError> {
        let local = self.agent_row_get(id)?;
        if local.is_some() {
            return Ok(local);
        }
        // Not in local SQLite — check the global cross-channel registry.
        // agent_def_list() already overlays this; keep agent_def_get consistent.
        match self.shared_def_get(id) {
            Ok(def) => Ok(def),
            Err(e) => {
                tracing::warn!(agent_id = %id, error = %e, "agent_def_get: global registry lookup failed; returning not-found");
                Ok(None)
            }
        }
    }

    /// `agent_def_get`, but a failed read of the global definition registry
    /// is an error rather than "not found". For callers that must refuse
    /// instead of acting on a partial answer — the name resolver treats "no
    /// such UID" as "send the bare name", which a hidden fault must not
    /// trigger (Codex P2 on #3568).
    pub(crate) fn agent_def_get_strict(&self, id: &str) -> Result<Option<AgentDefinition>, String> {
        if let Some(local) = self.agent_row_get(id).map_err(|e| e.to_string())? {
            return Ok(Some(local));
        }
        self.shared_def_get(id)
            .map_err(|e| format!("global definition registry: {e}"))
    }

    /// The global cross-channel definition for `id`, errors preserved.
    fn shared_def_get(&self, id: &str) -> Result<Option<AgentDefinition>, crate::registry::DefStoreError> {
        let Some(reg) = self.shared_def_registry() else {
            return Ok(None);
        };
        Ok(reg
            .get(id)?
            .map(|record| super::super::def_registry_mirror::record_to_agent_definition(&record)))
    }

    /// List all agent definitions, **most-recently-used first**.
    ///
    /// Reads from the consolidated `db_agents` table, ordered by
    /// `updated_at DESC` then `created_at ASC`. Every definition mutation
    /// AND every instance lifecycle touch bumps `db_agents.updated_at`, so
    /// recency on a row tracks the last time the agent was either edited or
    /// launched.
    ///
    /// Result-set shape: every `db_agents` row is returned — templates
    /// (`is_template = 1`) and user-clone projections (`is_template = 0`)
    /// each appear once. `parent_id` is sourced from
    /// `db_agents.parent_template_id`.
    pub fn agent_def_list(&self) -> Result<Vec<AgentDefinition>, StoreError> {
        // Local SQLite: this channel's templates (seeded) + its own user agents.
        let local: Vec<AgentDefinition> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(&format!("{AGENT_DEFINITION_SELECT} ORDER BY updated_at DESC, created_at ASC"))?;
            let rows = stmt.query_map([], map_agent_definition_row)?;
            let mut agents = Vec::new();
            for row in rows {
                agents.push(row?);
            }
            agents
        };
        // conn dropped. Overlay the GLOBAL cross-channel user-agent roster so
        // an agent created in another channel appears here too. The global
        // store wins for user rows (it's authoritative and holds cross-channel
        // agents); templates (seeded — never in the global store) come from
        // local SQLite. Falls back to SQLite-only when the global store is
        // absent or unreadable. (P0.2c.)
        let Some(reg) = self.shared_def_registry() else {
            return Ok(local);
        };
        let global = match reg.list_active() {
            Ok(recs) => recs,
            Err(e) => {
                tracing::warn!(error = %e, "def registry: global list failed, using SQLite only");
                return Ok(local);
            }
        };
        let mut by_id: std::collections::HashMap<String, AgentDefinition> =
            local.into_iter().map(|d| (d.id.clone(), d)).collect();
        for rec in &global {
            let mut def = super::super::def_registry_mirror::record_to_agent_definition(rec);
            // model_vendor_base_url is deliberately channel-local only —
            // DefinitionRecordV1 doesn't carry it, so record_to_agent_definition
            // always returns "" for it. Without this, the global overlay
            // silently wiped a same-channel agent's override on every read
            // (including agent.open's spawn-time resolution), making the
            // whole feature a no-op in default single-instance operation —
            // not just the genuinely-cross-channel case this limitation is
            // documented for. Preserve the local row's value when one
            // exists; only a truly cross-channel agent (no local row) sees
            // the empty default. (reagent P0 on PR #2505.)
            //
            // memory_id needs the IDENTICAL treatment, for the identical
            // reason (P0 fix, ReAgent review on PR #2587 round 6):
            // DefinitionRecordV1 doesn't carry memory_id either (same
            // documented gap, def_registry_mirror.rs), so
            // record_to_agent_definition always returns "" for it too.
            // Every local write auto-mirrors into the global registry
            // (agent_def_insert -> registry_def_upsert), so this overlay
            // fires for virtually every agent whenever a shared store is
            // configured — the "normal case," not an edge case. Without
            // this fix, agent_def_list() would silently zero memory_id on
            // every read, defeating the bundle lookups that read it
            // (bundle-scoped skills, the empty-provider
            // fallback in resolve_effective_provider_id) and m0021's own
            // memory_id-empty backfill filter (which would re-process
            // every already-bound agent on every migration run, since it
            // reads through this same function).
            if let Some(existing) = by_id.get(&def.id) {
                def.model_vendor_base_url = existing.model_vendor_base_url.clone();
                def.memory_id = existing.memory_id.clone();
                // conversation_visibility is the identical channel-local-only
                // case (SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md
                // Phase B/C, jekt rules SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md)
                // — DefinitionRecordV1 doesn't carry it, so without this the
                // global overlay would silently reset every agent's
                // disclosure policy back to "private" on every list read,
                // even though "private" is a safe fail-closed default (this
                // preserves the ADMINISTRATOR'S actual configured choice,
                // not just avoiding a crash).
                def.conversation_visibility = existing.conversation_visibility.clone();
            }
            by_id.insert(def.id.clone(), def);
        }
        let mut result: Vec<AgentDefinition> = by_id.into_values().collect();
        // Match the SQL ORDER BY: updated_at DESC, then created_at ASC.
        result.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then(a.created_at.cmp(&b.created_at))
        });
        Ok(result)
    }

    /// Count agent rows (used by seed engine to check if seeding is needed).
    /// Reads from the consolidated `db_agents` table. The seed engine only
    /// cares about `== 0` to decide "fresh database, seed templates".
    pub fn agent_def_count(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_agents",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Whether a definition with `id` exists in the LOCAL channel's SQLite
    /// (`db_agents`). Gates the cross-channel content/skills fallback: a
    /// locally-known agent with genuinely empty content/skills must NOT
    /// resurrect them from the global record. (reagent P1 on #1385.)
    pub(in crate::backend::storage) fn agent_def_exists_local(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_agents WHERE id = ?1)",
            params![id],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    /// Delete all seeded agents (is_seeded=1). Used by reseed to clear built-in agents.
    ///
    /// No project-instruction cleanup here, unlike `agent_def_delete`: this
    /// removes template rows, and a template is never launched — observations
    /// are recorded by `agent.open`, so a template has none to forget (Codex,
    /// PR #3162).
    pub fn agent_def_delete_seeded(&self) -> Result<usize, StoreError> {
        // Consolidation Phase 3b (PR 2): only the template rows go. Every
        // `is_template = 0` row — a user clone OR an agent launched from a
        // template — is an agent in its own right and persists across a
        // reseed, keeping its `parent_template_id` (templates have stable
        // ids, so the lineage survives the re-insert).
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_agents WHERE is_template=1", [])?)
    }

    /// Insert a new agent definition. Auto-derives slug from name if empty,
    /// resolves collisions by appending `-2`, `-3`, etc., and mutates
    /// `agent.slug` so the caller sees the resolved value (important
    /// for handlers that serialize the struct back to the frontend
    /// after insert).
    ///
    /// The collision check + insert run under a single mutex lock,
    /// so this is race-safe against concurrent inserts on the same
    /// connection.
    pub fn agent_def_insert(&self, agent: &mut AgentDefinition) -> Result<(), StoreError> {
        self.agent_def_insert_local_only(agent, None)?;
        // Mirror into the global cross-channel definition store. Content +
        // skills are inserted after the definition (separate calls), so this
        // initial record is content-less; agent_content_set / agent_skill_*
        // re-mirror with the full payload. (P0.2b.)
        self.registry_def_upsert(&agent.id);
        Ok(())
    }

    /// Insert into `db_agents` only — does NOT mirror to the global
    /// registry. Used directly by `agent_def_insert` (which mirrors right
    /// after) and by `agent_def_backfill_local_from_registry` (which must
    /// NOT mirror — see that function's doc comment for why re-mirroring
    /// immediately after this call would wipe the registry's real
    /// content/skills).
    ///
    /// `updated_at_override`: `None` stamps `updated_at = created_at`
    /// (the original, unchanged behavior for genuinely new definitions).
    /// `Some(ts)` stamps `updated_at = ts` instead — used by the backfill
    /// path to preserve the registry record's real `updated_at` rather
    /// than resetting it.
    ///
    /// `is_seeded = 1` rows become `is_template = 1` (a template; bindings
    /// stay empty). `is_seeded = 0` rows become `is_template = 0` with
    /// `parent_template_id = parent_id` — same is_template/parent_template_id
    /// derivation the old dual-write mirror used (frozen intent, no longer
    /// a second write).
    pub(in crate::backend::storage) fn agent_def_insert_local_only(
        &self,
        agent: &mut AgentDefinition,
        updated_at_override: Option<i64>,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let base = if agent.slug.is_empty() {
            derive_slug(&agent.name)
        } else {
            agent.slug.clone()
        };
        // Collision-resolve against every `db_agents` slug — the consolidated
        // table surfaces definition slugs and template-instance projections
        // alike, so a collision against an instance-derived row is caught here
        // too. Shared with `instance_create` so the two cannot drift.
        agent.slug = resolve_slug_collision(&conn, &base)?;
        let stamped_updated_at = updated_at_override.unwrap_or(agent.created_at);
        let is_template = if agent.is_seeded == 1 { 1_i64 } else { 0_i64 };
        let parent_template_id = if agent.is_seeded == 1 { String::new() } else { agent.parent_id.clone() };
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
             )",
            params![
                agent.id,
                agent.name,
                agent.icon,
                agent.description,
                is_template,
                parent_template_id,
                agent.provider,
                agent.provider_flags,
                agent.shell,
                agent.environment,
                agent.agent_type,
                agent.agent_bus_id,
                agent.accounts,
                agent.auto_start,
                agent.restart_on_crash,
                agent.idle_timeout_minutes,
                agent.slug,
                agent.branch_label,
                agent.working_directory,
                agent.created_at,
                // New definitions: updated_at == created_at, unless the
                // caller supplied an override (backfill path preserving
                // the registry record's real updated_at).
                stamped_updated_at,
                agent.is_seeded,
                // Phase 2 (hide templates): new rows start visible. The
                // user only hides via the explicit `agentdefhide` RPC,
                // and the agent-seed re-sync forces user_hidden = 0 on
                // any newly-added template id anyway, so honouring the
                // caller-supplied value here is safe even when a stray
                // 1 sneaks through.
                agent.user_hidden,
                agent.container_image,
                agent.container_volumes,
                agent.container_name,
                agent.use_ambient_login,
                agent.model_vendor_base_url,
                agent.auto_continue_enabled,
                agent.memory_id,
                agent.conversation_visibility,
            ],
        )?;
        // Reagent P1 on #1013 round 2: do NOT mutate the caller's
        // `&mut AgentDefinition` here. The PR is supposed to be
        // zero-behaviour-change; the previous version reflected
        // `stamped_updated_at` back onto the caller, which is an
        // observable mutation downstream callers may rely on remaining
        // untouched. Callers that need the freshly-stamped value can
        // re-fetch the row via the normal read path.
        Ok(())
    }

    /// Materialize a local shadow of a registry-only definition — this
    /// channel's `db_agents` row plus a best-effort local copy of
    /// content/skills — so `instance_create`'s FK can succeed for an agent
    /// whose definition exists cross-channel but was never created in THIS
    /// channel's SQLite.
    ///
    /// Deliberately does NOT call `registry_def_upsert` (unlike
    /// `agent_def_insert`, which does): the registry is already
    /// authoritative for this record. `registry_def_upsert` rebuilds
    /// the registry record from `agent_content_get_all_local`/
    /// `agent_skill_list_local` — LOCAL-only reads, by design (see that
    /// function's own comment) — so calling it immediately after this
    /// insert-with-no-content-yet would overwrite the registry's real
    /// content/skills with empty arrays. Content/skill rows are instead
    /// copied here via direct INSERTs (bypassing `agent_content_set`/
    /// `agent_skill_insert`, which each call `registry_def_refresh_if_mirrored`
    /// themselves) so nothing re-mirrors mid-backfill.
    ///
    /// Copy failures for content/skills are logged and otherwise
    /// ignored — the definition row alone satisfies the FK, and reads
    /// still resolve content/skills correctly via the existing
    /// cross-channel fallback even if this best-effort local copy is
    /// incomplete. A slug collision against an unrelated local agent
    /// will rename the local slug (existing `agent_def_insert_local_only`
    /// behavior) — low-probability and non-fatal, since slug isn't the
    /// FK target.
    pub(in crate::backend::storage) fn agent_def_backfill_local_from_registry(
        &self,
        record: &crate::registry::DefinitionRecord,
    ) -> Result<(), StoreError> {
        let mut def = super::super::def_registry_mirror::record_to_agent_definition(record);
        self.agent_def_insert_local_only(&mut def, Some(record.data.updated_at))?;

        let conn = self.conn.lock().unwrap();
        for c in &record.data.content {
            if let Err(e) = conn.execute(
                "INSERT INTO db_agent_content (agent_id, content_type, content, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(agent_id, content_type) DO UPDATE SET content=?3, updated_at=?4",
                params![def.id, c.content_type, c.content, record.data.updated_at],
            ) {
                tracing::warn!(
                    def_id = %def.id,
                    content_type = %c.content_type,
                    error = %e,
                    "instance_create backfill: local content copy failed (non-fatal)"
                );
            }
        }
        for s in &record.data.skills {
            if let Err(e) = conn.execute(
                "INSERT INTO db_agent_skills (id, agent_id, name, trigger, skill_type, description, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    s.id,
                    def.id,
                    s.name,
                    s.trigger,
                    s.skill_type,
                    s.description,
                    s.content,
                    record.data.updated_at,
                ],
            ) {
                tracing::warn!(
                    def_id = %def.id,
                    skill = %s.name,
                    error = %e,
                    "instance_create backfill: local skill copy failed (non-fatal)"
                );
            }
        }
        Ok(())
    }

    /// Atomic check-then-insert for `agent.define`.
    ///
    /// Looks up an existing user-owned definition by name (case-insensitive)
    /// or derived slug under the SAME mutex guard that protects the INSERT —
    /// preventing TOCTOU when two concurrent `agent.define` calls arrive for
    /// the same name.
    ///
    /// Returns:
    /// - `Ok(Some(def))` — an existing row matched; `agent` was NOT inserted.
    /// - `Ok(None)` — no match; `agent` was inserted and `agent.slug` now
    ///   holds the collision-resolved slug.
    pub fn agent_def_find_or_insert(
        &self,
        agent: &mut AgentDefinition,
    ) -> Result<Option<AgentDefinition>, StoreError> {
        let name_lower = agent.name.trim().to_lowercase();
        let derived_slug = derive_slug(agent.name.trim());

        {
            let conn = self.conn.lock().unwrap();

            // Check under the same lock to close the TOCTOU window.
            // `is_seeded = 0` already excludes templates; it does NOT
            // exclude a template-launch row (also `is_seeded = 0`, no
            // `db_agent_definitions` counterpart) — same as before this
            // read moved to `db_agents`, since the legacy table never held
            // launch rows to exclude in the first place.
            let mut stmt = conn.prepare(&format!(
                "{AGENT_DEFINITION_SELECT}
                 WHERE (lower(trim(name)) = ?1 OR slug = ?2)
                   AND is_seeded = 0
                 ORDER BY CASE WHEN lower(trim(name)) = ?1 THEN 0 ELSE 1 END
                 LIMIT 1"
            ))?;
            let mut rows = stmt.query_map(
                params![name_lower, derived_slug],
                map_agent_definition_row,
            )?;
            if let Some(row) = rows.next() {
                return Ok(Some(row?));
            }
            // Drop borrows on `conn` before proceeding to the insert.
            drop(rows);
            drop(stmt);

            // Not found — insert under the same lock.
            let base = if agent.slug.is_empty() {
                derive_slug(&agent.name)
            } else {
                agent.slug.clone()
            };
            let mut candidate = base.clone();
            let mut n: u32 = 2;
            loop {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM db_agents WHERE slug = ?1",
                    params![candidate],
                    |row| row.get(0),
                )?;
                if count == 0 {
                    break;
                }
                candidate = format!("{}-{}", base, n);
                n += 1;
            }
            agent.slug = candidate;
            let is_template = if agent.is_seeded == 1 { 1_i64 } else { 0_i64 };
            let parent_template_id = if agent.is_seeded == 1 { String::new() } else { agent.parent_id.clone() };
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
                 )",
                params![
                    agent.id,
                    agent.name,
                    agent.icon,
                    agent.description,
                    is_template,
                    parent_template_id,
                    agent.provider,
                    agent.provider_flags,
                    agent.shell,
                    agent.environment,
                    agent.agent_type,
                    agent.agent_bus_id,
                    agent.accounts,
                    agent.auto_start,
                    agent.restart_on_crash,
                    agent.idle_timeout_minutes,
                    agent.slug,
                    agent.branch_label,
                    agent.working_directory,
                    agent.created_at,
                    agent.created_at, // updated_at = created_at for new rows
                    agent.is_seeded,
                    agent.user_hidden,
                    agent.container_image,
                    agent.container_volumes,
                    agent.container_name,
                    agent.use_ambient_login,
                    agent.model_vendor_base_url,
                    agent.auto_continue_enabled,
                    agent.memory_id,
                    agent.conversation_visibility,
                ],
            )?;
        }
        // Do NOT mutate `agent.updated_at` here — matches
        // `agent_def_insert_local_only`'s own "zero-behaviour-change"
        // posture (reagent P1 on #1013 round 2): callers that need the
        // freshly-stamped value re-fetch the row via the normal read path.
        // Mirror the freshly-defined agent into the global store (agent.define
        // path). Without this a define-created agent stays channel-local until
        // a later edit. (codex P2 on #1385.)
        self.registry_def_upsert(&agent.id);

        Ok(None)
    }

    /// Set the `user_hidden` flag on a single agent definition. Phase 2
    /// of the two-tier picker (Q2 Decision Y). Returns:
    ///   `Ok(true)`  — row updated.
    ///   `Ok(false)` — no row with that id exists.
    ///   `Err(...)`  — the row exists but is NOT a seeded template
    ///                 (`is_seeded != 1`). User-owned definitions go
    ///                 through `agent_def_delete`, not hide.
    ///
    /// Does NOT bump `updated_at`: hide is a per-user view-state flag,
    /// not a definition-content edit. Keeps `updated_at` faithful to the
    /// agent's payload (the manifest re-sync compares `description` etc.
    /// against the canonical row).
    pub fn agent_def_set_hidden(&self, id: &str, hidden: bool) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        // Only seeded templates (`is_template = 1`) may flip the hide flag.
        // Templates carry `is_template = 1` and are the only rows allowed
        // to flip the hide flag; folded user-clone-def projections and
        // template-instance projections (both `is_template = 0`) reject.
        let is_template: i64 = match conn.query_row(
            "SELECT is_template FROM db_agents WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ) {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        if is_template != 1 {
            return Err(StoreError::Other(format!(
                "agent_def_set_hidden: {id} is not a seeded template (is_template={is_template}); \
                 user-owned definitions must use delete/archive paths, not hide"
            )));
        }
        // Templates are seeded; they do NOT go to the global cross-channel
        // def store, so no registry mirror here.
        let rows = conn.execute(
            "UPDATE db_agents SET user_hidden = ?1 WHERE id = ?2 AND is_template = 1",
            params![if hidden { 1_i64 } else { 0_i64 }, id],
        )?;
        Ok(rows > 0)
    }

    /// Whether this channel's store has its own row for definition `id`, as
    /// opposed to the definition only resolving through the global registry
    /// overlay. Only a local row can have its `default_memory_id` set.
    pub fn agent_def_has_local_row(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let found: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_agents WHERE id = ?1)",
            params![id],
            |r| r.get(0),
        )?;
        Ok(found)
    }

    /// Set `db_agents.default_memory_id` on a LOCAL agent — but only if
    /// it's currently empty. Exists because `agent_def_update`'s SET clause
    /// deliberately never touches this column (readonly-after-creation, see
    /// its own field doc comment) — this is the one narrow, explicit path
    /// allowed to set it, and only the FIRST time, for `m0021`'s backfill
    /// and definition-time provisioning to use. Returns `Ok(true)` if the
    /// row existed and had an empty `default_memory_id` (write applied),
    /// `Ok(false)` if the row doesn't exist locally OR already has a
    /// non-empty value (no-op, not an error — matches the "set once"
    /// semantic silently rather than requiring every caller to pre-check).
    ///
    /// LOCAL ONLY, deliberately — a cross-channel (global-registry-only)
    /// definition can't be reached this way today because
    /// `DefinitionRecordV1` doesn't carry `memory_id` yet (same accepted gap
    /// as `model_vendor_base_url`, see `def_registry_mirror.rs`). Callers
    /// backfilling across the whole agent population must check
    /// `agent_def_get` first and skip (with a log) anything that only
    /// resolves via the global registry.
    pub fn agent_def_set_memory_id_if_empty(
        &self,
        id: &str,
        memory_id: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_agents SET default_memory_id = ?1 WHERE id = ?2 AND default_memory_id = ''",
            params![memory_id, id],
        )?;
        Ok(rows > 0)
    }

    /// The provider's default vendor, or `"custom"` when the agent has a
    /// non-empty `model_vendor_base_url` override — same rule the
    /// frontend's `resolveEffectiveVendor`
    /// (`frontend/app/view/agent/providers/catalog.ts`) already uses for
    /// the dual-icon vendor badge. Falls back to the provider id itself
    /// if the provider isn't in the registry (e.g. a stale/custom
    /// provider string), same fallback the frontend uses.
    ///
    /// Derived wherever it's needed rather than stored: a Claude agent
    /// pointed at a custom endpoint is `"custom"`, not `"anthropic"`
    /// (#2587).
    pub(crate) fn resolve_effective_vendor(provider: &str, model_vendor_base_url: &str) -> String {
        if !model_vendor_base_url.trim().is_empty() {
            return "custom".to_string();
        }
        crate::backend::providers::get_provider(provider)
            .and_then(|p| p.supported_vendors.first().copied())
            .unwrap_or(provider)
            .to_string()
    }

    /// The harness/provider id `agent` runs: its own `provider`. A bundle
    /// carries no harness (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.1);
    /// the agent owns it, read-only after creation (`updateagent` and
    /// `agent.define` refuse a change). The bound bundle's `provider` is a
    /// hint and is consulted only when the agent has none at all, a
    /// definition the §3.3 migration couldn't reach; there it is the value
    /// the agent has always run with.
    ///
    /// **Call this on the EFFECTIVE identity/memory store
    /// (`AppState.id_store`), never on the per-channel `mstore`** — that
    /// fallback reads the bundle, which lives in the shared store when one is
    /// configured. One shared implementation for spawn (`agent_open.rs`),
    /// the credential gate (`identity/resolver/inject.rs`), clone, fork and
    /// export, so they can't disagree.
    pub fn resolve_effective_provider_id(&self, agent: &AgentDefinition) -> String {
        if !agent.provider.is_empty() || agent.memory_id.is_empty() {
            return agent.provider.clone();
        }
        match self.bundle_get(&agent.memory_id) {
            Ok(Some(b)) => b.provider,
            _ => String::new(),
        }
    }

    /// Provision a fresh, dedicated ABF bundle for a NEW agent definition —
    /// the definition-time half of
    /// `docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §3.2
    /// (m0021 is the backfill half, for agents that already existed before
    /// this shipped). The bundle carries no harness: the agent owns its
    /// provider (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.2); the agent
    /// is passed for the bundle's name.
    ///
    /// **Call this on the EFFECTIVE identity/memory store
    /// (`AppState.id_store`), never on the per-channel `mstore` directly.**
    /// Bundles live in the shared store when one is configured (the normal
    /// case) — every real bundle-read path (`listmemories`/`getmemory`/
    /// the Armory editor/the bundle-summary panel) reads through
    /// `id_store`, so a bundle created via `mstore.bundle_upsert`
    /// directly would be written to a different SQLite file and be
    /// invisible everywhere else (P1 finding, Codex review on PR #2587,
    /// live in the shipped code this fixes: `agent_def_provision_and_
    /// bind_bundle`'s callers all used to invoke this method ON `mstore`).
    ///
    /// Does NOT mutate `agent` or touch `db_agent_definitions` — returns
    /// the new bundle's id; callers set `agent.memory_id` themselves before
    /// their own insert (so the existing `agent_def_insert*` INSERT
    /// statements, which already include `memory_id`, pick it up with no
    /// separate write).
    pub fn bundle_provision_for_new_agent(
        &self,
        agent: &AgentDefinition,
        now: i64,
    ) -> Result<String, StoreError> {
        let bundle_id = uuid::Uuid::new_v4().to_string();
        let name = self.resolve_unique_bundle_name(&format!("{} — ABF", agent.name))?;
        let bundle = super::super::bundles::Bundle {
            id: bundle_id.clone(),
            name,
            description: String::new(),
            is_blank: false,
            is_global: false,
            // No harness: the agent owns it (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.2).
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: now,
            updated_at: now,
            is_system: false,
        };
        self.bundle_upsert(&bundle)?;
        Ok(bundle_id)
    }

    /// Resolve a `db_bundles.name` guaranteed not to collide with an
    /// existing row — that column is `TEXT NOT NULL UNIQUE`, but only
    /// `AgentDefinition.slug` is guaranteed unique, not the display
    /// `name`, so a naive `"{agent.name} — ABF"` collides whenever two
    /// agents share a display name. Appends a numeric suffix on
    /// collision, same shape `agent_def_insert_local_only` already uses
    /// for slug collisions.
    ///
    /// P1 fix (2026-08-15, ReAgent review on PR #2587 round 4): before
    /// this, a collision here silently left a runtime-provisioned agent
    /// unbound (the caller's best-effort error handling swallowed it) and
    /// — worse — unconditionally ABORTED `m0021`'s entire backfill loop
    /// via `?` on the very first collision, permanently stalling backfill
    /// for every agent processed after it, on every subsequent boot,
    /// until the underlying name collision was manually resolved.
    pub(crate) fn resolve_unique_bundle_name(&self, base_name: &str) -> Result<String, StoreError> {
        let existing_names: std::collections::HashSet<String> =
            self.bundle_list()?.into_iter().map(|b| b.name).collect();
        if !existing_names.contains(base_name) {
            return Ok(base_name.to_string());
        }
        let mut n: u32 = 2;
        loop {
            let candidate = format!("{base_name} ({n})");
            if !existing_names.contains(&candidate) {
                return Ok(candidate);
            }
            n += 1;
        }
    }

    /// Provision + bind a fresh bundle to an ALREADY-INSERTED agent
    /// definition — the two-step, post-insert counterpart to
    /// `bundle_provision_for_new_agent` above. Deliberately separate from
    /// the insert itself (rather than setting `memory_id` on the struct
    /// before calling `agent_def_insert*`/`agent_def_find_or_insert`):
    /// `agent_def_find_or_insert` in particular uses its `AgentDefinition`
    /// argument as BOTH the lookup key and the conditional-insert payload,
    /// so a bundle built up-front would go to waste (leaked, unbound) on
    /// every `if_exists=skip`/`update` call against an already-existing
    /// name — and `agent.define` is meant to be called repeatedly/
    /// idempotently. Binding after the fact, only once the caller has
    /// confirmed a row was genuinely freshly inserted, avoids that leak.
    ///
    /// Best-effort by design (matches `createagent`'s own color-assignment
    /// comment: a failure here shouldn't fail agent creation) — logs and
    /// returns on either step's failure rather than propagating, since the
    /// agent row itself is already durably committed by the time this
    /// runs, and a still-unbound agent is exactly the pre-existing
    /// `memory_id=''` state `m0021` already knows how to backfill later.
    ///
    /// Takes `&mut AgentDefinition` and writes the new bundle id back into
    /// `agent.memory_id` on success — same "caller's struct reflects what
    /// landed" convention `agent_def_update` documents just below — so RPC
    /// handlers that serialize `agent` straight back to the caller (e.g.
    /// `createagent`, `importagentfromclaw`) return the real value instead
    /// of the empty string the struct held before this call.
    ///
    /// `self` (the definition store, i.e. `mstore`) and `bundle_store`
    /// (the effective identity/memory store, i.e. `AppState.id_store`) are
    /// deliberately two SEPARATE parameters, not the same store used for
    /// both writes — see `bundle_provision_for_new_agent`'s own doc
    /// comment for why (P1 fix, Codex review on PR #2587: they used to be
    /// the same store, silently writing every provisioned bundle
    /// somewhere the rest of the app can't see it).
    pub fn agent_def_provision_and_bind_bundle(
        &self,
        bundle_store: &Store,
        agent: &mut AgentDefinition,
        now: i64,
    ) {
        let bundle_id = match bundle_store.bundle_provision_for_new_agent(agent, now) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(agent_id = %agent.id, error = %e, "agent_def_provision_and_bind_bundle: bundle create failed (non-fatal)");
                return;
            }
        };
        match self.agent_def_set_memory_id_if_empty(&agent.id, &bundle_id) {
            Ok(true) => {
                // So the agent's other channels bind this bundle instead of
                // minting their own (m0021).
                crate::backend::agent_bundle_sidecar::record(&agent.id, &bundle_id);
                agent.memory_id = bundle_id
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(agent_id = %agent.id, bundle_id = %bundle_id, error = %e, "agent_def_provision_and_bind_bundle: bind failed (non-fatal)");
            }
        }
    }

    /// Update an existing agent definition (all fields except id, created_at, is_seeded, `parent_id`).
    /// `parent_id` is NOT updatable post-insert — it describes the agent's
    /// lineage; re-parenting is done by creating a new fork, not mutating the
    /// original.
    ///
    /// `branch_label` IS written here (unlike `parent_id`) — this storage-
    /// layer function persists whatever is on `agent`. Immutability for most
    /// callers is enforced one layer up, at the RPC handler: `updateagent`
    /// always passes back `old.branch_label.clone()` unchanged, so it's a
    /// no-op for that caller. `renameagentdefinitiontitle` is the one
    /// deliberate exception that supplies a real change — see
    /// `crates/srv/src/server/agent_handlers/template.rs` and
    /// docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §4.
    ///
    /// Self-stamps `updated_at` with the current time and writes it back into
    /// `agent.updated_at`, so the caller's struct (e.g. an RPC response body)
    /// reflects exactly what landed in the database.
    pub fn agent_def_update(&self, agent: &mut AgentDefinition) -> Result<bool, StoreError> {
        let now = agentmux_common::time::now_ms();
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE db_agents SET name=?1, icon=?2, provider=?3, description=?4,
                 working_directory=?5, shell=?6, provider_flags=?7, auto_start=?8,
                 restart_on_crash=?9, idle_timeout_minutes=?10,
                 agent_type=?11, environment=?12, agent_bus_id=?13, accounts=?14, updated_at=?15,
                 container_image=?17, container_volumes=?18, container_name=?19,
                 use_ambient_login=?20, branch_label=?21, model_vendor_base_url=?22,
                 auto_continue_enabled=?23, conversation_visibility=?24
                 WHERE id=?16",
                params![
                    agent.name,
                    agent.icon,
                    agent.provider,
                    agent.description,
                    agent.working_directory,
                    agent.shell,
                    agent.provider_flags,
                    agent.auto_start,
                    agent.restart_on_crash,
                    agent.idle_timeout_minutes,
                    agent.agent_type,
                    agent.environment,
                    agent.agent_bus_id,
                    agent.accounts,
                    now,
                    agent.id,
                    agent.container_image,
                    agent.container_volumes,
                    agent.container_name,
                    agent.use_ambient_login,
                    agent.branch_label,
                    agent.model_vendor_base_url,
                    agent.auto_continue_enabled,
                    agent.conversation_visibility,
                ],
            )?
        };
        // Reflect the persisted timestamp back to the caller's struct so an
        // RPC response carries the fresh value, not the pre-update one.
        agent.updated_at = now;
        if rows > 0 {
            // Mirror the updated definition into the global store. (P0.2b.)
            self.registry_def_upsert(&agent.id);
        }
        // Cross-channel edit: an agent surfaced only via the global overlay has
        // no local SQLite row, so the UPDATE affected 0 rows. Apply the edit to
        // the global record directly (preserving its content/skills) so editing
        // a cross-channel agent isn't silently dropped with a "not found" error
        // — symmetric with the unconditional cross-channel delete. (reagent P1
        // on #1385.)
        let updated_global = if rows == 0 {
            self.registry_def_update_definition_fields(agent)
        } else {
            false
        };
        Ok(rows > 0 || updated_global)
    }

    /// Delete a agent definition by id. Returns true if a row was deleted.
    pub fn agent_def_delete(&self, id: &str) -> Result<bool, StoreError> {
        // Consolidation Phase 3b (PR 2): a user agent launched from this
        // template keeps its own `db_agents` row (it is an agent, not a
        // launch of the template), the same way user clones always survived
        // deleting their template — deleting `id` here removes only its own
        // row. The JSON registry mirror for the deleted row is covered by
        // `registry_def_retire` below.
        let (rows, scope) = {
            let conn = self.conn.lock().unwrap();
            // Read BEFORE the DELETE — afterwards there is no row to ask.
            // See `scope_for_row` for what this decides and why.
            let scope = Self::scope_for_row(&conn, id);
            // Identity M4a: also read before the DELETE.
            tombstone_key_names(&conn, id)?;
            purge_name_keyed_keys(&conn, id, self.wan_identity().as_deref())?;
            let rows = conn.execute("DELETE FROM db_agents WHERE id=?1", params![id])?;
            // Unconditional, same convergence rule as the registry sweep
            // below (codex P2 on PR #3262): if a previous attempt committed
            // the `db_agents` DELETE and then failed partway through the
            // dependent purge, the retry sees `rows == 0` — gating here
            // would strand that agent's credentials and signing keys
            // permanently, with no UI path left to try again. Every
            // statement in the purge is an idempotent DELETE, so running it
            // for an id with no rows costs a few no-op statements.
            purge_agent_dependents(&conn, id, self.token_index().as_deref())?;
            (rows, scope)
        };
        // Tombstone the global definition record so another channel's stale
        // SQLite can't resurrect this deleted user agent — AND so an agent
        // that exists in THIS channel only via the global overlay (no local
        // SQLite row, rows == 0) is actually deletable instead of reappearing
        // on the next agent_def_list. (P0.2b + codex P1 on #1385.)
        let global_retired = self.registry_def_retire(id);
        // Unconditional, deliberately.
        //
        // Gating this on `rows > 0` was the original bug's second half: the
        // instance registry is host-global while `db_agents` is
        // per-channel, so a cross-channel agent — one this channel only ever
        // saw through the global overlay — deletes with `rows == 0` and
        // still has a registry record another channel wrote. That record
        // stayed active with its definition now tombstoned, which
        // `listrecentsessions` still renders as a row (no provider, so no
        // icon, and "(missing definition)") — the delete looked like it
        // half-worked.
        //
        // `rows > 0 || global_retired` isn't enough either (codex P2 on
        // PR #3262): if an earlier attempt retired the definition but failed
        // to remove a registry file — or another channel wrote a stale
        // record after this process's startup reconcile — a RETRY sees
        // `rows == 0` AND `global_retired == false` (the tombstone already
        // exists), so it would skip the sweep and the ghost row would
        // survive every further Delete until restart. Sweeping
        // unconditionally makes Delete converge: the sweep is a no-op for an
        // id with no records, so there is nothing to gate on in the first
        // place.
        self.purge_agent_side_effects(id, "agent_def_delete", scope);
        Ok(rows > 0 || global_retired)
    }

    /// The [`RecordScope`] every registry operation on `id` must use.
    ///
    /// **Read this before adding another registry call.** The wide scope
    /// matches records by `definition_id`, and a TEMPLATE's id is the
    /// `definition_id` of every legacy, never-re-keyed launch record
    /// belonging to a REAL agent launched from it — so a wide match on a
    /// template deletes / retires / renames those agents' records along
    /// with the template, making them vanish from the picker. ReAgent found
    /// exactly that omission at four separate call sites on PR #3262
    /// (`agent_def_delete`, `instance_delete`, `instance_set_hidden`,
    /// `instance_rename`), which is why the rule lives here rather than as
    /// a fourth hand-written `if`.
    ///
    /// An id with no row at all is NOT a template: the global definition
    /// store only holds `is_seeded == 0` user agents, so a cross-channel id
    /// is always an agent — which is what lets the delete paths still sweep
    /// on a retry after the local row is already gone.
    ///
    /// Takes the connection rather than `&self` because every caller needs
    /// this read in the SAME critical section as its own UPDATE/DELETE —
    /// otherwise the row could change kind in between. A `&self` wrapper
    /// existed briefly and was dead on arrival for exactly that reason
    /// (ReAgent P2 on PR #3262).
    pub(in crate::backend::storage) fn scope_for_row(conn: &rusqlite::Connection, id: &str) -> RecordScope {
        let is_template = conn
            .query_row(
                "SELECT is_template FROM db_agents WHERE id = ?1",
                params![id],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .unwrap_or(false);
        if is_template {
            RecordScope::FileKeyOnly
        } else {
            RecordScope::Agent
        }
    }

    /// Purge this store's rows keyed to a deleted agent, without touching
    /// `db_agents` itself. For the OTHER physical database an agent has
    /// rows in: `agent_def_delete` covers the object store it runs against,
    /// and the `deleteagent` handler calls this on `state.identity_store`,
    /// which is where the live `db_agent_identity_links` /
    /// `db_agent_credentials` / native-memory rows actually are
    /// (`SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`). See
    /// [`purge_agent_dependents`] for the table list and why one list
    /// serves both stores. Returns rows removed.
    pub fn agent_dependents_purge(&self, id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        purge_agent_dependents(&conn, id, self.token_index().as_deref())
    }

    /// The non-SQLite half of deleting an agent row: observations keyed to
    /// it, and its records in the host-global instance registry. Shared by
    /// [`Self::agent_def_delete`] and [`Self::instance_delete`] — the two
    /// names one deletion is reached by since the definition flip (see
    /// `instance_delete`'s own doc comment). Best-effort throughout:
    /// SQLite has already committed, and neither side failing is a reason
    /// to report the delete as failed. `caller` only labels the logs.
    ///
    /// `scope` must come from `scope_for_row`, read
    /// BEFORE the row was deleted — afterwards there is nothing left to ask.
    pub(in crate::backend::storage) fn purge_agent_side_effects(&self, id: &str, caller: &'static str, scope: RecordScope) {
        // Project-instruction observations are keyed by agent id with no
        // foreign key (they record files this agent READS, which is not a
        // relationship SQLite can enforce), so nothing else would remove
        // them and a future agent reusing this id would inherit somebody
        // else's baseline — every file reporting `unchanged` against
        // observations that were never made about it. Cleaned up here,
        // beside the row it belongs to, rather than left to each caller
        // (ReAgent, PR #3162).
        if let Err(e) = self.project_instructions_forget(id) {
            tracing::warn!(
                agent_def_id = %id, caller, error = %e,
                "agent delete: project-instruction observations left behind"
            );
        }
        if let Some(reg) = self.registry() {
            // An AGENT's records are matched by either key — see
            // `Registry::hard_delete_for_agent` for why the file key and the
            // record's own `definition_id` can disagree, and what a missed
            // record looks like in the picker.
            //
            // A TEMPLATE's are matched by file key only. A template's id can
            // legitimately be the `definition_id` of a legacy,
            // never-re-keyed launch record belonging to a real agent
            // launched from it, and the wide match would delete that agent's
            // record along with the template — making agents disappear from
            // the picker instead of the one thing the user asked to delete
            // (ReAgent P1 round 3 on PR #3262;
            // `agent_def_delete_removes_only_its_own_registry_file` is the
            // protection, and
            // `deleting_a_template_spares_a_legacy_launch_record_pointing_at_it`
            // pins the legacy shape that test doesn't construct).
            match reg.hard_delete_for_agent(id, scope) {
                Ok(0) => {}
                Ok(removed) => tracing::debug!(
                    agent_def_id = %id, caller, removed,
                    "registry: purged instance records for deleted agent"
                ),
                Err(e) => tracing::warn!(
                    agent_def_id = %id, caller, error = %e,
                    "registry: failed to mirror agent delete"
                ),
            }
        }
    }

    /// One-shot grandfather pass for the layer-3 ambient-login opt-in
    /// (m0017, spec §2.4 of SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md).
    ///
    /// Agents WITHOUT any `db_agent_identity_links` row at migration time
    /// were de-facto ambient users → `use_ambient_login = 1`; agents WITH
    /// links opted into managed accounts → `0` (honest failure is the new
    /// behavior for them). `linked_agent_ids` comes from the SHARED store
    /// (the live links table); this method writes `db_agents` — the
    /// definition flip's only remaining caller of this is `m0017` on an
    /// install upgrading through the whole migration chain for the first
    /// time, where `db_agents` is already the sole live source. Returns
    /// (rows set to 1, rows set to 0).
    pub fn agents_grandfather_ambient_login(
        &self,
        linked_agent_ids: &std::collections::HashSet<String>,
    ) -> Result<(usize, usize), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Set everyone ambient first, then flip the linked set back to the
        // fail-by-default 0 — two passes instead of a dynamic IN() list.
        let ambient_defs = conn.execute("UPDATE db_agents SET use_ambient_login = 1", [])?;
        let mut linked_rows = 0usize;
        for id in linked_agent_ids {
            linked_rows += conn.execute(
                "UPDATE db_agents SET use_ambient_login = 0 WHERE id = ?1",
                params![id],
            )?;
        }
        Ok((ambient_defs.saturating_sub(linked_rows), linked_rows))
    }

    // AgentContent / AgentSkill / AgentHistory CRUD live in
    // `super::content` / `super::skills` / `super::history` —
    // each adds an `impl Store {}` block to this type.
}
