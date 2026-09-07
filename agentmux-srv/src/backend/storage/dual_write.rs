// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `db_agents` dual-write helpers — **definition side only**.
//!
//! Every remaining `db_agent_definitions` mutation mirrors into
//! `db_agents`, which is what every reader now reads. The instance-side
//! helpers that used to live here (create / update / set_hidden / delete /
//! repoint, plus the chain-walking projection-key resolver) went away with
//! consolidation Phase 3b PR 2: `Store`'s instance API writes `db_agents`
//! directly, so nothing mirrors a launch any more — there is no second
//! table holding one.
//!
//! The rest of this file goes the same way in Phase 3c, when
//! `db_agent_definitions` is dropped and `agent_def_*` writes `db_agents`
//! directly too. See `docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md`.
//!
//! Extracted from `store.rs` in Phase R.5 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`) — keeping it
//! isolated makes that future deletion clean.

use rusqlite::params;

use super::error::StoreError;
use super::store::{AgentDefinition, Store};

impl Store {
    /// Mirror a `db_agent_definitions` row into `db_agents` as the
    /// canonical row for that definition.
    ///
    /// - `is_seeded = 1` rows become `is_template = 1` (canonical
    ///   template; bindings stay empty).
    /// - `is_seeded = 0` rows become `is_template = 0` with
    ///   `parent_template_id = parent_id` (user-cloned from a template;
    ///   its bindings and launch state come from `instance_create`, which
    ///   writes this same row).
    ///
    /// Idempotent: uses `INSERT … ON CONFLICT(id) DO UPDATE`. Bindings and
    /// launch state already on the row are preserved — only
    /// definition-side fields are overwritten.
    ///
    /// Returns `Err` on failure (it once logged and continued): readers
    /// see `db_agents`, so a silent failure would leak stale data into the
    /// picker.
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
    pub(crate) fn agents_dual_write_seeded_delete(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Templates only — every `is_template = 0` row is an agent that
        // survives a reseed (consolidation Phase 3b, PR 2).
        conn.execute("DELETE FROM db_agents WHERE is_template = 1", [])?;
        Ok(())
    }

}
