// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `db_agents` column lists and row mappers the definition and instance
//! reads share.

use super::*;

/// The `db_agents` columns every instance read selects, in the order
/// [`map_instance_row`] expects.
pub(super) const INSTANCE_COLUMNS: &str = "id, last_block_id, session_id, status, github_context, started_at, ended_at, \
     created_at, identity_id, memory_id, instance_name, working_directory, user_hidden";

/// Project a non-template `db_agents` row (selected with
/// [`INSTANCE_COLUMNS`]) into the `AgentInstance` shape: the row's id is
/// both `id` and `definition_id`, `last_block_id` is `block_id`, chains are
/// pre-collapsed so `parent_instance_id` is empty, and a row that never
/// recorded a launch reports `created_at` as `started_at`.
pub(super) fn map_instance_row(row: &rusqlite::Row) -> rusqlite::Result<AgentInstance> {
    let id: String = row.get(0)?;
    let started_at: i64 = row.get(5)?;
    let created_at: i64 = row.get(7)?;
    Ok(AgentInstance {
        definition_id: id.clone(),
        id,
        parent_instance_id: String::new(),
        block_id: row.get(1)?,
        session_id: row.get(2)?,
        status: row.get(3)?,
        github_context: row.get(4)?,
        started_at: if started_at == 0 { created_at } else { started_at },
        ended_at: row.get(6)?,
        created_at,
        identity_id: row.get(8)?,
        memory_id: row.get(9)?,
        instance_name: row.get(10)?,
        working_directory: row.get(11)?,
        display_hidden: row.get::<_, i64>(12)? != 0,
    })
}

/// The `db_agents` SELECT that reads a row in the `AgentDefinition` shape —
/// the column order [`map_agent_definition_row`] expects. Shared by
/// `agent_def_list` (all rows) and `agent_row_get` (one row).
pub(super) const AGENT_DEFINITION_SELECT: &str = "SELECT id, slug, name, icon, provider, description,
            working_directory, shell, provider_flags, auto_start,
            restart_on_crash, idle_timeout_minutes, created_at,
            agent_type, environment, agent_bus_id, is_seeded,
            accounts, parent_template_id, branch_label, updated_at,
            user_hidden, container_image, container_volumes, container_name,
            use_ambient_login, model_vendor_base_url, auto_continue_enabled,
            default_memory_id, conversation_visibility
     FROM db_agents";

/// Row mapper for `db_agents` rows projected back into the
/// `AgentDefinition` shape. The column order MUST match the SELECT in
/// `agent_def_list`. `parent_template_id` maps to `parent_id` because
/// the consolidated table renamed the field to clarify its semantics
/// (template lineage), but the wire shape kept the old name.
pub(super) fn map_agent_definition_row(row: &rusqlite::Row) -> rusqlite::Result<AgentDefinition> {
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
}
