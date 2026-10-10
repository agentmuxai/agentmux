// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `instance_*`: per-launch instance rows, named-agent continuation, and
//! identity-bound active-for-block resolution, on `db_agents`.

use super::*;

impl Store {

    // ---- Agent instance CRUD ----
    //
    // Agent-concept consolidation Phase 3b, PR 2 of 4
    // (SPEC_AGENT_ARCHITECTURE_2026_05_27.md): every method in this block
    // reads and writes `db_agents` only. `db_agent_instances` is no longer
    // touched by the live instance API; it stays on disk until PR 4 drops it.
    //
    // What an "instance" is now: the launch-side view of a non-template
    // `db_agents` row. One row per agent, so:
    //   - `id` / `definition_id` are both the row's `id` — the agent's
    //     identity (the rule `instance_list` adopted in PR #1111).
    //   - `block_id`/`session_id`/`status`/`started_at`/`ended_at` are the
    //     row's LATEST launch state (schema v29); a continuation moves them
    //     to the new launch instead of adding a row, so `parent_instance_id`
    //     is always empty and chains are pre-collapsed.
    //   - Launching a TEMPLATE creates a new user-agent row keyed by the
    //     launch's instance id (`parent_template_id` = the template);
    //     launching a USER agent folds into its own row.
    //   - `display_hidden` is `user_hidden` (one flag, one row).

    /// List agents as instances. Both filters are optional — pass `None`
    /// to scan all. Ordered by `updated_at` descending, `created_at` as a
    /// tiebreaker (most recent activity first — every launch, continuation
    /// and lifecycle write bumps `updated_at`).
    ///
    /// The `definition_id` filter matches the agent's own `id` only:
    /// templates are not agents and user agents derived from one template
    /// are separate agents (see `instance_list_named` for the one caller
    /// that wants "everything derived from this template"). The `status`
    /// filter reads the row's latest launch status (v29).
    pub fn instance_list(
        &self,
        definition_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut sql = format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents WHERE is_template = 0 AND user_hidden = 0"
        );
        let mut param_vals: Vec<String> = Vec::new();
        if let Some(d) = definition_id {
            sql.push_str(&format!(" AND id = ?{}", param_vals.len() + 1));
            param_vals.push(d.to_string());
        }
        if let Some(s) = status {
            sql.push_str(&format!(" AND status = ?{}", param_vals.len() + 1));
            param_vals.push(s.to_string());
        }
        sql.push_str(" ORDER BY updated_at DESC, created_at DESC");
        let mut stmt = conn.prepare(&sql)?;
        let iter = stmt.query_map(rusqlite::params_from_iter(param_vals.iter()), map_instance_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// One agent by id, as an instance. `None` for a template or an
    /// unknown id.
    pub fn instance_get(&self, id: &str) -> Result<Option<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents WHERE id = ?1 AND is_template = 0"
        ))?;
        match stmt.query_row(params![id], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The Runtime menu choices the user last made for agent `id` (JSON, see
    /// [`normalize_last_runtime`]), or '' when there are none or the agent is
    /// unknown or a template.
    pub fn agent_last_runtime_get(&self, id: &str) -> Result<String, StoreError> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT last_runtime FROM db_agents WHERE id = ?1 AND is_template = 0",
            params![id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Replace agent `id`'s remembered runtime. Returns false for an unknown
    /// agent or a template (templates are not agents anyone continues, so
    /// they remember nothing). Deliberately leaves `updated_at` alone: this is
    /// a note about how the agent was last run, not an edit of its definition.
    pub fn agent_last_runtime_set(&self, id: &str, runtime: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_agents SET last_runtime = ?2 WHERE id = ?1 AND is_template = 0",
            params![id, runtime],
        )?;
        Ok(rows > 0)
    }

    /// A single `db_agents` row in the `AgentDefinition` shape, no registry
    /// overlay — the definition a launch resolves against. Templates and
    /// user agents alike.
    pub(in crate::backend::storage) fn agent_row_get(&self, id: &str) -> Result<Option<AgentDefinition>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("{AGENT_DEFINITION_SELECT} WHERE id = ?1"))?;
        match stmt.query_row(params![id], map_agent_definition_row) {
            Ok(d) => Ok(Some(d)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Record a launch. Returns the agent row the launch now lives on, as an
    /// instance — **its `id` may differ from `inst.id`**: a launch of a user
    /// agent folds into that agent's row, and a continuation (non-empty
    /// `parent_instance_id` naming an existing row) folds into the row it
    /// continues. Only a fresh launch of a TEMPLATE creates a new row, keyed
    /// by `inst.id`. Callers must persist the returned id, not the one they
    /// passed (the frontend stores it as the block's `agentInstanceId`).
    ///
    /// `inst.definition_id` must name a `db_agents` row; when it only exists
    /// in the global cross-channel definition registry (an agent created in
    /// another channel), the local definition is backfilled first, exactly
    /// as before (`REPORT_AGENT_DEFINITION_DB_GAP_2026_07_27.md`). A
    /// definition that exists nowhere is `StoreError::NotFound` — the error
    /// the legacy FK used to raise, minus the FK.
    pub fn instance_create(&self, inst: &AgentInstance) -> Result<AgentInstance, StoreError> {
        let def = match self.agent_row_get(&inst.definition_id)? {
            Some(d) => d,
            None => {
                if let Some(reg) = self.shared_def_registry() {
                    match reg.get(&inst.definition_id) {
                        Ok(Some(record)) => {
                            if let Err(e) = self.agent_def_backfill_local_from_registry(&record) {
                                tracing::warn!(
                                    definition_id = %inst.definition_id,
                                    error = %e,
                                    "instance_create: registry backfill failed"
                                );
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!(
                                definition_id = %inst.definition_id,
                                error = %e,
                                "instance_create: registry lookup failed"
                            );
                        }
                    }
                }
                match self.agent_row_get(&inst.definition_id)? {
                    Some(d) => d,
                    None => {
                        tracing::warn!(
                            definition_id = %inst.definition_id,
                            instance_id = %inst.id,
                            "instance_create: definition exists nowhere; refusing the launch"
                        );
                        return Err(StoreError::NotFound);
                    }
                }
            }
        };

        let display_name = if inst.instance_name.is_empty() { def.name.clone() } else { inst.instance_name.clone() };
        let hidden = if inst.display_hidden { 1_i64 } else { 0_i64 };
        // The agent's working directory is the launch's RESOLVED path when
        // the launch supplies one (`agent.open` resolves it, including slug
        // collision suffixing), falling back to the definition's configured
        // cwd. Phase 3a's dual-write always wrote the definition's, on the
        // reasoning that "db_agents holds durable agent config; the
        // per-launch resolved cwd lives on the block" — that split ends with
        // the instance table: one row per agent means the row holds the
        // agent's real workspace, which is also what the cross-version JSON
        // registry mirror records (and it silently skips any row whose
        // working_directory is not under the agents root).
        let working_directory = if inst.working_directory.is_empty() {
            def.working_directory.clone()
        } else {
            inst.working_directory.clone()
        };
        // The row this launch lands on. A user agent IS its row; a template
        // launch is its own row unless it continues one that already exists.
        let key = if def.is_seeded == 0 {
            def.id.clone()
        } else if !inst.parent_instance_id.is_empty() && self.instance_get(&inst.parent_instance_id)?.is_some() {
            inst.parent_instance_id.clone()
        } else {
            inst.id.clone()
        };
        {
            let conn = self.conn.lock().unwrap();
            let now_ms = Self::monotonic_updated_at(&conn, inst.created_at);
            if key == inst.id {
                // Fresh template launch — its own user-agent row, config
                // copied from the template, bindings + launch state from the
                // launch. ON CONFLICT covers an id the caller reuses on a
                // retry (the App-API stub path).
                //
                // The slug is collision-resolved rather than copied verbatim
                // from the template (#3497 §4). `agent_def_insert` has always
                // done this; this path did not, making it the one writer that
                // could mint a duplicate slug into a column with no UNIQUE
                // constraint. In practice the launch flow names its agent and
                // goes through `agent_def_insert`, so this branch is reached
                // only by handing `instance_create` a seeded template id
                // directly — this closes the function's contract rather than an
                // observed fault (§2.5.2).
                let launch_slug = resolve_slug_collision(&conn, &def.slug)?;
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
                        use_ambient_login, model_vendor_base_url, auto_continue_enabled,
                        conversation_visibility,
                        session_id, status, started_at, ended_at
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
                        ?30, ?31, ?32,
                        ?33,
                        ?34, ?35, ?36, ?37
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
                        last_block_id = CASE WHEN excluded.last_block_id = '' THEN db_agents.last_block_id ELSE excluded.last_block_id END,
                        session_id = CASE WHEN excluded.session_id = '' THEN db_agents.session_id ELSE excluded.session_id END,
                        status = excluded.status,
                        started_at = excluded.started_at,
                        ended_at = excluded.ended_at",
                    params![
                        inst.id,
                        display_name,
                        def.icon,
                        def.description,
                        def.id,
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
                        launch_slug,
                        def.branch_label,
                        inst.identity_id,
                        inst.memory_id,
                        working_directory,
                        inst.github_context,
                        inst.instance_name,
                        inst.created_at,
                        now_ms,
                        hidden,
                        inst.block_id,
                        def.container_image,
                        def.container_volumes,
                        def.container_name,
                        def.use_ambient_login,
                        def.model_vendor_base_url,
                        def.auto_continue_enabled,
                        def.conversation_visibility,
                        inst.session_id,
                        inst.status,
                        inst.started_at,
                        inst.ended_at,
                    ],
                )?;
            } else {
                // Fold: the launch (or continuation) moves the row's bindings
                // and launch state to this launch. Config stays the row's own.
                //
                // An EMPTY `session_id` never clears the row's: a fresh
                // continuation is created session-less and the CLI only
                // reports the real session id later, so overwriting here
                // would drop the pointer `--resume` needs (the same reason
                // `last_block_id` is guarded). A deliberate clear goes
                // through `instance_update_partial(Some(""))`, which does
                // write it — see `poison_resume`.
                conn.execute(
                    "UPDATE db_agents SET
                        name = ?2,
                        identity_id = ?3,
                        memory_id = ?4,
                        working_directory = ?14,
                        github_context = ?5,
                        instance_name = ?6,
                        updated_at = ?7,
                        user_hidden = ?8,
                        last_block_id = CASE WHEN ?9 = '' THEN last_block_id ELSE ?9 END,
                        session_id = CASE WHEN ?10 = '' THEN session_id ELSE ?10 END,
                        status = ?11,
                        started_at = ?12,
                        ended_at = ?13
                     WHERE id = ?1 AND is_template = 0",
                    params![
                        key,
                        display_name,
                        inst.identity_id,
                        inst.memory_id,
                        inst.github_context,
                        inst.instance_name,
                        now_ms,
                        hidden,
                        inst.block_id,
                        inst.session_id,
                        inst.status,
                        inst.started_at,
                        inst.ended_at,
                        working_directory,
                    ],
                )?;
            }
        }
        let canonical = self.instance_get(&key)?.ok_or(StoreError::NotFound)?;
        self.registry_upsert_if_named(&canonical, !inst.session_id.is_empty());
        // A freshly created launch's session_id is never a genuine capture
        // in production (continuations start with ""), so only a non-empty
        // one is worth propagating to the registry.
        if !inst.session_id.is_empty() {
            self.registry_propagate_continuation_session_id(&canonical);
        }
        Ok(canonical)
    }

    /// An `updated_at` strictly greater than any already on `db_agents`, so
    /// recency ordering survives fast successive writes within one
    /// millisecond (reagent P2 round 3 on #1013 — the rule the dual-write
    /// used; inherited here).
    fn monotonic_updated_at(conn: &rusqlite::Connection, floor: i64) -> i64 {
        let wall_now: i64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(floor);
        let global_prior: i64 = conn
            .query_row("SELECT COALESCE(MAX(updated_at), 0) FROM db_agents", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0);
        std::cmp::max(wall_now, global_prior.saturating_add(1))
    }

    /// Set the hidden flag on an agent row. Used by the "Forget agent"
    /// affordance — soft-delete only; the row + working directory remain on
    /// disk for audit + recovery.
    ///
    /// Cross-version case: an agent migrated into the registry from another
    /// version's SQLite won't have a row in the current version's SQLite.
    /// The UPDATE returns 0 rows, but the registry still needs to flip —
    /// otherwise "Forget agent" silently no-ops for those. Returns `true`
    /// when either side acted.
    pub fn instance_set_hidden(&self, id: &str, hidden: bool) -> Result<bool, StoreError> {
        // Read in the same critical section as the UPDATE, so the row can't
        // change kind underneath the registry call below. See
        // `scope_for_row`: retiring a TEMPLATE with the wide scope
        // would hide every real agent launched from it (ReAgent P1 round 4
        // on PR #3262).
        let (rows, scope) = {
            let conn = self.conn.lock().unwrap();
            let scope = Self::scope_for_row(&conn, id);
            let rows = conn.execute(
                "UPDATE db_agents SET user_hidden = ?1 WHERE id = ?2 AND is_template = 0",
                params![if hidden { 1_i64 } else { 0_i64 }, id],
            )?;
            (rows, scope)
        };
        let mut registry_acted = false;
        if let Some(reg) = self.registry() {
            // Matches by file key OR the record's own `definition_id`.
            // Retiring by file key alone left any launch-keyed record for
            // this agent active — literally the "'Forget agent' retires the
            // NEW key while the stale file stays active, so the forgotten
            // agent reappears" failure `m0026_registry_agent_id_rekey`'s own
            // doc comment describes, still reachable on an install where
            // that migration no-ops (see
            // `Registry::hard_delete_for_agent`). A count of 0 is a genuine
            // no-op for an unrelated id, which is what the old
            // `exists_anywhere` pre-check was guarding against.
            let res = if hidden {
                reg.retire_for_agent(id, scope)
            } else {
                reg.unretire_for_agent(id, scope)
            };
            match res {
                Ok(moved) => registry_acted = moved > 0,
                Err(e) => tracing::warn!(
                    instance_id = %id,
                    hidden,
                    error = %e,
                    "registry: failed to mirror instance_set_hidden"
                ),
            }
        }
        Ok(rows > 0 || registry_acted)
    }

    /// Rename the agent's PICKER-VISIBLE display name (`instance_name`),
    /// in SQLite and across every registry record for it.
    ///
    /// `renameagentdefinitiontitle` on its own writes `name` (or
    /// `branch_label` for a fork) — the fields the pane tab strip's
    /// `titleOf()` reads. "My Agents" rows render
    /// `instance_name || definition_name` instead, and `instance_name` is
    /// non-empty for every launched or registry-backed row, so a rename
    /// closed the editor, refetched, and redisplayed the OLD name; for a
    /// fork it changed `branch_label`, which no picker row shows at all
    /// (codex P1 on PR #3262). Renaming an agent has to move the name the
    /// user is actually looking at, so the handler calls this too.
    ///
    /// Scoped to rows that ALREADY have an `instance_name`: writing one
    /// onto an agent that has never had a named launch would make it appear
    /// in `instance_list_named` (which filters `instance_name <> ''`) —
    /// conjuring a picker row rather than renaming one. Those rows fall
    /// back to `definition_name`, which the title update already moved.
    ///
    /// Returns whether anything changed, counting the registry: a
    /// cross-channel agent has no local row to update but does have records.
    pub fn instance_rename(&self, id: &str, instance_name: &str) -> Result<bool, StoreError> {
        // Same critical-section read as `instance_set_hidden`, for the same
        // reason: renaming a TEMPLATE with the wide scope would rewrite
        // `instance_name` on every real agent launched from it (ReAgent P1
        // round 4 on PR #3262).
        let (rows, scope) = {
            let conn = self.conn.lock().unwrap();
            let scope = Self::scope_for_row(&conn, id);
            let now_ms = Self::monotonic_updated_at(&conn, 0);
            let rows = conn.execute(
                "UPDATE db_agents SET instance_name = ?1, updated_at = ?2
                 WHERE id = ?3 AND is_template = 0 AND instance_name <> ''",
                params![instance_name, now_ms, id],
            )?;
            (rows, scope)
        };
        let mut registry_renamed = 0usize;
        if let Some(reg) = self.registry() {
            match reg.set_instance_name_for_agent(id, scope, instance_name) {
                Ok(n) => registry_renamed = n,
                Err(e) => tracing::warn!(
                    agent_id = %id, error = %e,
                    "registry: failed to mirror instance_rename — picker may keep showing the old name"
                ),
            }
        }
        Ok(rows > 0 || registry_renamed > 0)
    }

    /// Named agents for the launch modal's "Continue agent" dropdown and the
    /// picker's "My Agents" surface: non-hidden rows with an `instance_name`,
    /// newest launch first, capped by `limit`.
    ///
    /// `definition_id` restricts the result to that agent itself OR every
    /// agent derived from it (`parent_template_id`) — the launch modal opens
    /// per template, and what a user wants to continue there is any agent
    /// they launched from it. `identity_id` filters on the row's identity
    /// binding.
    ///
    /// `include_continuations` is accepted for API compatibility but no
    /// longer changes the query: chains are pre-collapsed (one row per
    /// agent carrying its latest launch), so the legacy "heads only" and
    /// "latest per chain" modes both yield exactly this list.
    pub fn instance_list_named(
        &self,
        limit: usize,
        definition_id: Option<&str>,
        identity_id: Option<&str>,
        _include_continuations: bool,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut sql = format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE is_template = 0 AND user_hidden = 0 AND instance_name <> ''"
        );
        let mut param_vals: Vec<String> = Vec::new();
        if let Some(id) = identity_id {
            sql.push_str(&format!(" AND identity_id = ?{}", param_vals.len() + 1));
            param_vals.push(id.to_string());
        }
        if let Some(def) = definition_id {
            let n = param_vals.len() + 1;
            sql.push_str(&format!(" AND (id = ?{n} OR parent_template_id = ?{n})"));
            param_vals.push(def.to_string());
        }
        sql.push_str(&format!(
            " ORDER BY started_at DESC, updated_at DESC, id DESC LIMIT ?{}",
            param_vals.len() + 1
        ));
        param_vals.push(limit.to_string());
        let mut stmt = conn.prepare(&sql)?;
        let iter = stmt.query_map(rusqlite::params_from_iter(param_vals.iter()), map_instance_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// Force a row's `slug` to an exact value, bypassing collision resolution.
    ///
    /// **Tests only, and deliberately awkward to reach for.** Both write paths
    /// now suffix-resolve (#3497 §4), so a test cannot build two rows sharing a
    /// slug through the normal API any more — and the read-path guards
    /// (`instance_get_by_slug`, `agent_resolve::resolve_agent_id`, `check_s1`)
    /// exist precisely for that state. Without this, those tests still *pass*,
    /// but vacuously: the lookup returns nothing because nothing matches the
    /// slug, not because the guard refused an ambiguous one. That is the
    /// failure mode the guards were written to prevent, reproduced in their own
    /// tests.
    ///
    /// The state is still reachable in production — no `UNIQUE` constraint
    /// backs the column, so a future writer, a migration, or hand-edited data
    /// can produce it. Defence in depth: prevent it on write, refuse it on
    /// read, and test both.
    /// Make every `db_agents` read fail, to test how a caller treats a store
    /// fault (as opposed to an empty answer).
    #[cfg(test)]
    pub(crate) fn test_break_agents_table(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DROP TABLE db_agents")?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_force_slug(&self, id: &str, slug: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE db_agents SET slug = ?2 WHERE id = ?1", params![id, slug])?;
        Ok(())
    }

    /// Resolve an agent by its persisted `slug` — the App-API self-lookup
    /// (`AGENTMUX_AGENT_ID` is the slug) — **only if exactly one agent has
    /// it**. Matches ONLY `slug`; deliberately single-purpose after reagentx
    /// P1 on PR #2428 (round 2): a lookup that also matched another column
    /// let a coincidental cross-namespace collision return an unrelated
    /// agent's memory, accounts or bundle. Hidden rows are excluded.
    ///
    /// `None` means "no agent" *or* "more than one agent" — callers already
    /// treat both as "fall back to another source", so they are not
    /// distinguished. The fallbacks are themselves fail-closed
    /// (`agent_registry_lookup::find_active_record_by_slug`), so refusing
    /// here routes to another refusal rather than to a guess.
    ///
    /// **Why ambiguity returns `None` rather than the most recently updated
    /// row:** `db_agents.slug` has no `UNIQUE` constraint — `migrations.rs`
    /// declines the index deliberately. Both write paths now suffix-resolve
    /// (#3497 §4), so neither mints a duplicate, but nothing at the schema
    /// level prevents one: a future writer, a migration, or hand-edited data
    /// still can. (An earlier revision of this comment claimed
    /// `instance_create` copied its template's slug onto every launch, so two
    /// launches collided. That was never true of the real launch flow — see
    /// the spec's §2.5.2 — and is no longer true of the function either.) This
    /// function used to `ORDER BY updated_at DESC LIMIT 1` and hand the
    /// winner's `definition_id` and `working_directory` to the caller.
    /// `HistoryService::sessions_for_agent` consults it to resolve an
    /// agent's *conversation history*, so guessing wrong does not degrade
    /// gracefully — it discloses one agent's transcripts to another, and
    /// deterministically: the same wrong agent won every time. A slug simply
    /// does not identify an agent when it collides, so the honest answer is
    /// "unknown". Same rule and same reasoning as
    /// [`crate::backend::agent_registry_lookup::find_active_record_by_slug`]
    /// (PR #3480), which this mirrors for `db_agents`.
    pub fn instance_get_by_slug(&self, slug: &str) -> Result<Option<AgentInstance>, StoreError> {
        if slug.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE slug = ?1 AND is_template = 0 AND user_hidden = 0"
        ))?;
        let mut matches = stmt
            .query_map(params![slug], map_instance_row)?
            .collect::<Result<Vec<_>, _>>()?;
        if matches.len() > 1 {
            tracing::warn!(
                slug = %slug,
                matches = matches.len(),
                "db_agents slug collision: refusing to resolve an agent by slug alone"
            );
            return Ok(None);
        }
        Ok(matches.pop())
    }

    /// Every non-template, non-hidden agent a typed name could mean (identity
    /// M3, spec §5): exact slug, or `name`/`instance_name` case-insensitively.
    /// Lists rather than picks — the caller decides what to do with several.
    pub fn agents_matching_name(&self, typed: &str) -> Result<Vec<AgentNameMatch>, StoreError> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, slug, instance_name FROM db_agents
             WHERE is_template = 0 AND user_hidden = 0
               AND (slug = ?1 OR name = ?1 COLLATE NOCASE OR instance_name = ?1 COLLATE NOCASE)
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![typed], |row| {
            Ok(AgentNameMatch {
                id: row.get(0)?,
                name: row.get(1)?,
                slug: row.get(2)?,
                instance_name: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Every row a name selects, **templates and hidden rows included** — the
    /// identity M4a-2 ambiguity check (spec §6.5.3). Unlike
    /// [`Self::agents_matching_name`], which lists only the agents a typed
    /// name could mean, this asks whether the name is *anyone else's*: slug
    /// collision suffixing (`resolve_slug_collision`) counts template rows,
    /// so a "Claude" made from the "Claude" template is `claude-2`, and a name
    /// its MCP sends as `claude` belongs to the template (#3573).
    pub fn rows_answering_to(&self, name: &str) -> Result<Vec<AgentNameMatch>, StoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, slug, instance_name FROM db_agents
             WHERE slug = ?1 OR name = ?1 COLLATE NOCASE OR instance_name = ?1 COLLATE NOCASE
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![name], |row| {
            Ok(AgentNameMatch {
                id: row.get(0)?,
                name: row.get(1)?,
                slug: row.get(2)?,
                instance_name: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// The names row `id` answers to — the by-id counterpart of
    /// [`Self::agents_matching_name`], for checking a body's actor name
    /// against the calling agent's own row (identity M4a-2, spec §6.5.3).
    /// Any row, template or hidden: the caller's token already names it.
    pub fn agent_names_by_id(&self, id: &str) -> Result<Option<AgentNameMatch>, StoreError> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT id, name, slug, instance_name FROM db_agents WHERE id = ?1",
            params![id],
            |row| {
                Ok(AgentNameMatch {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    slug: row.get(2)?,
                    instance_name: row.get(3)?,
                })
            },
        ) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Whether `id` names an agent that was **deleted**: no local row (template
    /// or not), and the shared definition registry holds a record for it that
    /// is retired rather than active — deleting a user agent retires its
    /// global record (`registry_def_retire`). A template, a removed template
    /// and a live cross-channel agent all read false: templates are never
    /// mirrored, and a cross-channel agent's record is active. With no shared
    /// registry this cannot be told and reads false. For the identity spawn
    /// gate (#3577).
    pub fn agent_was_deleted(&self, id: &str) -> bool {
        let id = id.trim();
        if id.is_empty() {
            return false;
        }
        match self.agent_row_get(id) {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => return false,
        }
        match self.shared_def_registry() {
            Some(reg) => reg.exists_anywhere(id) && !reg.exists(id),
            None => false,
        }
    }

    /// Ensure a user agent `id` has a local `db_agents` row, backfilling it
    /// from the shared definition registry when it exists only there (a
    /// cross-channel agent) — the same backfill `instance_create` runs, without
    /// its fold. Identity M4b-4 (spec §6.5.8): so a block naming a
    /// cross-channel agent resolves to a local row at its spawn. Returns
    /// whether the row exists afterwards.
    pub fn agent_row_ensure_local(&self, id: &str) -> Result<bool, StoreError> {
        if self.agent_row_get(id)?.is_some() {
            return Ok(true);
        }
        if let Some(reg) = self.shared_def_registry() {
            match reg.get(id) {
                Ok(Some(record)) => {
                    if let Err(e) = self.agent_def_backfill_local_from_registry(&record) {
                        tracing::warn!(definition_id = %id, error = %e, "agent_row_ensure_local: registry backfill failed");
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(definition_id = %id, error = %e, "agent_row_ensure_local: registry lookup failed");
                }
            }
        }
        Ok(self.agent_row_get(id)?.is_some())
    }

    /// Record a launch of user agent `id` on `block_id`: a **lifecycle-only**
    /// update — `last_block_id`, `status = running`, `started_at`,
    /// `ended_at = 0`, a monotonic `updated_at` and the registry mirror — the
    /// columns `instance_create`'s fold moves, and none of the ones it would
    /// overwrite from a caller's record (`name`, `identity_id`, `memory_id`,
    /// `instance_name`, `github_context`, `user_hidden`). Identity M4b-4 (spec
    /// §6.5.8), for `agent.open`, which has no such record. Returns false for
    /// a template or a missing row.
    pub fn instance_record_launch(
        &self,
        id: &str,
        block_id: &str,
        started_at: i64,
    ) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            let now_ms = Self::monotonic_updated_at(&conn, 0);
            conn.execute(
                "UPDATE db_agents
                 SET last_block_id = ?2, status = 'running', started_at = ?3, ended_at = 0, updated_at = ?4
                 WHERE id = ?1 AND is_template = 0",
                params![id, block_id, started_at, now_ms],
            )?
        };
        if rows == 0 {
            return Ok(false);
        }
        if let Some(fresh) = self.instance_get(id)? {
            self.registry_upsert_if_named(&fresh, false);
        }
        Ok(true)
    }

    /// Partial update of an agent's launch state. Only `Some` fields are
    /// written; `None` leaves that column untouched. `Some("")` explicitly
    /// clears (the `updateagentinstance` command contract, and
    /// `poison_resume`'s deliberate stale-session clear).
    ///
    /// Returns the post-update row. `None` is reserved for **not-found**;
    /// an all-`None` update on an existing id is a no-op that returns the
    /// unchanged row, so callers can tell "nothing to change" from "no such
    /// agent". See SPEC_UPDATEAGENTINSTANCE_PARTIAL_UPDATE_2026_05_29.md.
    pub fn instance_update_partial(
        &self,
        id: &str,
        upd: &InstanceUpdate,
    ) -> Result<Option<AgentInstance>, StoreError> {
        use rusqlite::types::ToSql;

        let mut sets: Vec<&str> = Vec::new();
        let mut vals: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(v) = &upd.block_id {
            sets.push("last_block_id = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = &upd.session_id {
            sets.push("session_id = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = &upd.status {
            sets.push("status = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = &upd.github_context {
            sets.push("github_context = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = upd.ended_at {
            sets.push("ended_at = ?");
            vals.push(Box::new(v));
        }
        if sets.is_empty() {
            return self.instance_get(id);
        }

        let rows = {
            let conn = self.conn.lock().unwrap();
            let now_ms = Self::monotonic_updated_at(&conn, 0);
            sets.push("updated_at = ?");
            vals.push(Box::new(now_ms));
            let sql = format!("UPDATE db_agents SET {} WHERE id = ? AND is_template = 0", sets.join(", "));
            vals.push(Box::new(id.to_string()));
            let params: Vec<&dyn ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, params.as_slice())?
        };
        if rows == 0 {
            return Ok(None);
        }
        let fresh = self.instance_get(id)?;
        if let Some(f) = &fresh {
            self.registry_upsert_if_named(f, upd.session_id.is_some());
            // Only propagate when THIS call actually targeted session_id —
            // `Some("")` (a deliberate clear) and `Some(real_sid)` both mean
            // this call wrote it; `None` means the row's existing value is
            // not this call's to broadcast. See
            // registry_propagate_continuation_session_id's doc comment.
            if upd.session_id.is_some() {
                self.registry_propagate_continuation_session_id(f);
            }
        }
        Ok(fresh)
    }

    /// Full-struct update of the mutable launch-state fields (`block_id`,
    /// `session_id`, `status`, `github_context`, `ended_at`). Retained as a
    /// convenience for tests + internal callers; the `updateagentinstance`
    /// handler uses [`Self::instance_update_partial`].
    pub fn instance_update(&self, inst: &AgentInstance) -> Result<bool, StoreError> {
        let upd = InstanceUpdate {
            block_id: Some(inst.block_id.clone()),
            session_id: Some(inst.session_id.clone()),
            status: Some(inst.status.clone()),
            github_context: Some(inst.github_context.clone()),
            ended_at: Some(inst.ended_at),
        };
        Ok(self.instance_update_partial(&inst.id, &upd)?.is_some())
    }

    /// Repoint every agent derived from `old_def_id` at `new_def_id`. Used by
    /// the Phase 1 two-tier-picker migration
    /// (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md): when a seeded template has
    /// been used directly, the migration clones it into a user agent and
    /// repoints its launches so the existing reattach flow keeps working.
    /// Returns the number of rows updated.
    pub fn instance_repoint_definition(&self, old_def_id: &str, new_def_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            // `id != ?1` keeps the promote target out of its own result set:
            // the clone is itself derived from the template, so an unguarded
            // update would re-parent it to itself and count as a repoint.
            "UPDATE db_agents SET parent_template_id = ?1
             WHERE parent_template_id = ?2 AND is_template = 0 AND id != ?1",
            params![new_def_id, old_def_id],
        )?)
    }

    /// Delete an agent row (never a template). The row IS the agent in the
    /// consolidated model, so this removes the agent — previously a launch
    /// row of a user-clone definition could be deleted while the definition
    /// stayed; there is no such split any more. As of the definition flip,
    /// this and `agent_def_delete` are the same operation reached from two
    /// names: neither writes `db_agent_definitions`, and nothing reads it
    /// per-boot any more to resurrect what either one deletes (the startup
    /// gap-repair that once could is gone — codex P2 on #3080 closed the
    /// resurrection path this way rather than by re-adding a legacy-table
    /// delete here).
    pub fn instance_delete(&self, id: &str) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            // Refuse a template explicitly, rather than leaning on the
            // DELETE's own `is_template = 0` guard. The registry sweep below
            // runs even when the DELETE matches nothing, and a template's id
            // can appear as the `definition_id` of a legacy, never-re-keyed
            // launch record — sweeping on it would take records belonging to
            // agents launched from that template, which
            // `agent_def_delete_removes_only_its_own_registry_file` exists
            // to forbid. A missing row reads as "not a template" so the
            // retry path below still works.
            if Self::scope_for_row(&conn, id) == RecordScope::FileKeyOnly {
                return Ok(false);
            }
            // Identity M4a: read the names before the row goes.
            tombstone_key_names(&conn, id)?;
            purge_name_keyed_keys(&conn, id, self.wan_identity().as_deref())?;
            let rows =
                conn.execute("DELETE FROM db_agents WHERE id = ?1 AND is_template = 0", params![id])?;
            // Same dependent purge `agent_def_delete` runs, and
            // unconditional for the same reason — this is that deletion
            // arriving from the launch side, and an agent deleted through
            // this name used to keep its credentials and signing keys on
            // disk purely because the cleanup lived in the other function.
            purge_agent_dependents(&conn, id, self.token_index().as_deref())?;
            rows
        };
        // Unconditional, for the reason `agent_def_delete`'s own sweep is
        // (ReAgent P1 round 2 on PR #3262): a retry after a previous attempt
        // removed the `db_agents` row but failed to remove the registry file
        // sees `rows == 0`, so gating here left the iconless ghost row
        // behind indefinitely — the exact bug this PR exists to fix, just
        // unfixed on this entry point. "The same deletion under a second
        // name" has to mean the same convergence too, not just the same
        // table list.
        // Always `Agent`: the template case returned early above, because
        // this method's contract is "never a template" — it reports no
        // deletion for one rather than sweeping narrowly the way
        // `agent_def_delete` does.
        self.purge_agent_side_effects(id, "instance_delete", RecordScope::Agent);
        // Tombstone the GLOBAL definition record too. Clearing the local
        // tables alone leaves an active record in the cross-channel
        // definition registry, and `agent_def_list` overlays every active
        // record back onto the local list on every read — so the deleted
        // agent reappears on the very next My Agents fetch, no restart
        // needed (reagent P1 round 2 on #3080). Same tombstone
        // `agent_def_delete` writes; this is that deletion arriving from the
        // launch side.
        //
        // Unconditional, like everything else in this function (ReAgent P2
        // on PR #3262). This was the last step still gated on `rows > 0`,
        // on the reasoning that an agent existing only in the global
        // registry is `agent_def_delete`'s business — but once the purge
        // above went unconditional, that gate left a genuinely incoherent
        // outcome reachable here: a cross-channel agent's dependent rows
        // and registry records swept while its definition stays ACTIVE, so
        // the overlay keeps serving a definition nothing backs. Retiring an
        // id with no global record is a no-op, and a template can't reach
        // this line (early return above), so there is nothing to gate on.
        let global_retired = self.registry_def_retire(id);
        Ok(rows > 0 || global_retired)
    }

    /// Resolve the agent bindings tied to a block.
    ///
    /// Resolves through `block.meta.agentId` (or legacy `agent:id`) first —
    /// the block says which agent it shows — returning that agent's row
    /// with `block_id` set to the asked-for block. Falls back to the agent
    /// whose latest launch is on this block and still active, which is what
    /// a block from before the block meta carried `agentId` needs.
    ///
    /// `block.meta.agentInstanceId` never overrides `agentId`: codex P1 on
    /// PR #1114 round 3 surfaced that pane reuse can leave a stale one
    /// behind. It is consulted only for a block whose `agentId` names no
    /// agent row, and only within what that block names, while it is still
    /// the block's latest launch (see the fallback below).
    pub fn instance_get_active_for_block(&self, block_id: &str) -> Result<Option<AgentInstance>, StoreError> {
        let block: crate::backend::obj::Block = match self.get(block_id)? {
            Some(b) => b,
            None => return Ok(None),
        };
        let agent_id = crate::backend::obj::block_meta_agent_id(&block.meta)
            .unwrap_or("")
            .to_string();
        if !agent_id.is_empty() {
            // Any non-template row, hidden or not: an existing pane of a
            // "forgotten" agent must still resolve its bindings.
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(&format!(
                "SELECT {INSTANCE_COLUMNS} FROM db_agents WHERE id = ?1 AND is_template = 0"
            ))?;
            match stmt.query_row(params![agent_id], map_instance_row) {
                Ok(mut a) => {
                    a.block_id = block_id.to_string();
                    return Ok(Some(a));
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => return Err(e.into()),
            }
            // The block names something that is not an agent row: a
            // template (a template launch or continuation finds the row it
            // created or folded into this way), a template since removed
            // from the manifest (its launched agents are kept, and their
            // blocks still name it), a deleted agent whose pane survived, or
            // a provider key. Fall back only to a row *launched from* what
            // the block names (`parent_template_id`), and — when the frontend
            // has stamped the launched row (`agentInstanceId`) — only to that
            // row. Row status is never corrected when a pane is switched or
            // a process exits, so the latest launch on the block can be
            // another agent's stale `running` row, including a sibling
            // launched from the same template; resolving to it handed the
            // pane that agent's UID, token, slug and provider credentials.
            // A stamped row that no longer exists resolves to nothing.
            // "Launched from" reaches one hop: the template-promotion
            // migration repointed launch rows' `parent_template_id` from the
            // seeded template to its promoted clone (itself launched from
            // the template), while pre-migration blocks still name the
            // template (Codex P1 on #3576). Neither the row nor the hop may
            // be a fork: `forkagentdefinition` also stores its source in
            // `parent_template_id`, and `branch_label` (always set on a
            // fork, never on a template launch) is what tells them apart
            // (`template.rs`; Codex P1 on #3576).
            let stamped = block
                .meta
                .get("agentInstanceId")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            if !stamped.is_empty() {
                // The stamp is the frontend's own record of the row it
                // launched on this block — stronger evidence than lineage,
                // which migrations and deletions rewrite (a removed
                // template, a promoted clone deleted after the repoint).
                // Accepted only while it is still the block's latest launch
                // (by `started_at`, which only a launch or fold moves; a tie
                // with another active launch is undecided and rejected —
                // Codex P2 on #3576): a
                // stale stamp left by pane reuse, with a newer launch since
                // folded onto the block, resolves to nothing rather than to
                // the older row. Never a fork: a fork is launched under its
                // own row id, so a fork reached here is a stale stamp. And
                // the row must still relate to what the block names —
                // launched from it, or one hop from it — so a stale stamp
                // naming an unrelated agent cannot select it (ReAgent P0/P1
                // on #3576). Recorded, fail-safe: a launch the promotion
                // migration repointed to a clone that was later deleted has
                // no lineage left and resolves to nothing until relaunched;
                // and `started_at` is wall-clock, so a backward clock step
                // between two launches on one block makes the stamped row
                // look older — it then resolves to nothing, never to another
                // row. (Only an unstamped legacy block, reused, across such a
                // step could pick the older launch.)
                // Recorded: a legacy continuation stamp naming an id that
                // never had a row (m0025 keyed chains to their root) resolves
                // to nothing, not to the root — nothing distinguishes it from
                // a deleted launch, and no identity is safer than a wrong one.
                let mut stmt = conn.prepare(&format!(
                    "SELECT {INSTANCE_COLUMNS} FROM db_agents a
                     WHERE a.id = ?2 AND a.last_block_id = ?1 AND a.is_template = 0
                       AND a.status IN ('running', 'paused') AND a.branch_label = ''
                       AND (a.parent_template_id = ?3
                            OR a.parent_template_id IN (
                                SELECT id FROM db_agents
                                WHERE parent_template_id = ?3 AND is_template = 0 AND branch_label = '')
                            )
                       AND NOT EXISTS (
                           SELECT 1 FROM db_agents b
                           WHERE b.last_block_id = ?1 AND b.is_template = 0 AND b.id != a.id
                             AND b.status IN ('running', 'paused') AND b.started_at >= a.started_at)"
                ))?;
                return match stmt.query_row(params![block_id, stamped, agent_id], map_instance_row)
                {
                    Ok(a) => Ok(Some(a)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(e.into()),
                };
            }
            // Unstamped (a block from before `agentInstanceId`): lineage.
            let mut stmt = conn.prepare(&format!(
                "SELECT {INSTANCE_COLUMNS} FROM db_agents
                 WHERE last_block_id = ?1 AND is_template = 0 AND status IN ('running', 'paused')
                   AND branch_label = ''
                   AND (parent_template_id = ?2
                        OR parent_template_id IN (
                            SELECT id FROM db_agents
                            WHERE parent_template_id = ?2 AND is_template = 0 AND branch_label = ''))
                 ORDER BY started_at DESC, updated_at DESC, id DESC
                 LIMIT 1"
            ))?;
            return match stmt.query_row(params![block_id, agent_id], map_instance_row) {
                Ok(a) => Ok(Some(a)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            };
        }
        // A block whose meta names no agent (from before `agentId`): the
        // agent whose latest launch is on it — by launch time, which a
        // rename or lifecycle write does not move (Codex P1 on #3576).
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE last_block_id = ?1 AND is_template = 0 AND status IN ('running', 'paused')
             ORDER BY started_at DESC, updated_at DESC, id DESC
             LIMIT 1"
        ))?;
        match stmt.query_row(params![block_id], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The agent whose latest launch is on `block_id`, regardless of status
    /// — with its real `session_id`, so callers that write back to that
    /// exact row (keeping `session_id` live as the CLI emits it — see
    /// `persist_session_id` /
    /// SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.1) have
    /// the right id to pass to `instance_update_partial`.
    ///
    /// When a pane is reused, several rows can name the block. The one the
    /// pane runs is the block's own `agentId` (or legacy `agent:id`), while
    /// its latest launch is still this block; otherwise the most recent
    /// *launch* (`started_at`). Never the most recently *updated* row: a
    /// rename or definition edit bumps `updated_at`, and a session id then
    /// landed on the renamed agent instead of the one running (#3580).
    pub fn instance_get_by_block_id(&self, block_id: &str) -> Result<Option<AgentInstance>, StoreError> {
        let shown = self
            .get::<crate::backend::obj::Block>(block_id)?
            .and_then(|b| crate::backend::obj::block_meta_agent_id(&b.meta).map(str::to_string))
            .filter(|id| !id.is_empty());
        let conn = self.conn.lock().unwrap();
        if let Some(agent_id) = shown {
            let mut stmt = conn.prepare(&format!(
                "SELECT {INSTANCE_COLUMNS} FROM db_agents
                 WHERE id = ?1 AND last_block_id = ?2 AND is_template = 0"
            ))?;
            match stmt.query_row(params![agent_id, block_id], map_instance_row) {
                Ok(a) => return Ok(Some(a)),
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE last_block_id = ?1 AND is_template = 0
             ORDER BY started_at DESC, updated_at DESC, id DESC
             LIMIT 1"
        ))?;
        match stmt.query_row(params![block_id], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
