// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The passes `run_consolidate_migration` is made of: the marker gate, the
//! definitions pass, the instances pass and the closing marker write.

use std::path::Path;

use rusqlite::{params, Connection, Transaction};
use tracing::{info, warn};

use super::*;

/// True when the marker file says the backfill already ran and the data
/// bears it out.
pub(super) fn marker_says_done(
    conn: &Connection,
    data_dir: Option<&Path>,
) -> Result<bool, StoreError> {
    // Marker gate. Existence alone is no longer trusted (Phase 0b): if the
    // source tables have rows but db_agents doesn't, the marker is stale —
    // fall through and re-run instead of silently reporting "already done."
    if let Some(dir) = data_dir {
        let marker = dir.join(CONSOLIDATE_MARKER);
        if marker.exists() {
            if !consolidate_looks_incomplete(conn)? {
                return Ok(true);
            }
            warn!(
                marker = %marker.display(),
                "agents_consolidate: marker present but db_agents is empty while source tables have rows — \
                 treating as stale and re-running instead of trusting it",
            );
        }
    }
    Ok(false)
}

/// True when `db_agent_definitions` exists at all.
pub(super) fn definitions_table_exists(conn: &Connection) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_agent_definitions')",
        [],
        |row| row.get(0),
    )?)
}

/// Pass 1 + 2 — definitions.
pub(super) fn backfill_definitions(
    tx: &Transaction,
    stats: &mut ConsolidateStats,
) -> Result<(), StoreError> {
    let defs = load_definitions(tx)?;
    for def in &defs {
        insert_definition(tx, def, stats)?;
    }
    Ok(())
}

/// Every `db_agent_definitions` row, in table order.
fn load_definitions(tx: &Transaction) -> Result<Vec<DefRow>, StoreError> {
    let mut def_stmt = tx.prepare(
        "SELECT id, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_id, branch_label, updated_at, slug
             FROM db_agent_definitions",
    )?;
    let rows = def_stmt.query_map([], |row| {
        Ok(DefRow {
            id: row.get(0)?,
            name: row.get(1)?,
            icon: row.get(2)?,
            provider: row.get(3)?,
            description: row.get(4)?,
            working_directory: row.get(5)?,
            shell: row.get(6)?,
            provider_flags: row.get(7)?,
            auto_start: row.get(8)?,
            restart_on_crash: row.get(9)?,
            idle_timeout_minutes: row.get(10)?,
            created_at: row.get(11)?,
            agent_type: row.get(12)?,
            environment: row.get(13)?,
            agent_bus_id: row.get(14)?,
            is_seeded: row.get(15)?,
            accounts: row.get(16)?,
            parent_id: row.get(17)?,
            branch_label: row.get(18)?,
            updated_at: row.get(19)?,
            slug: row.get(20)?,
        })
    })?;
    let mut defs: Vec<DefRow> = Vec::new();
    for r in rows {
        defs.push(r?);
    }
    drop(def_stmt);
    Ok(defs)
}

/// Project one definition into `db_agents` and count it.
fn insert_definition(
    tx: &Transaction,
    def: &DefRow,
    stats: &mut ConsolidateStats,
) -> Result<(), StoreError> {
    let is_template = if def.is_seeded == 1 { 1_i64 } else { 0_i64 };
    let parent_template_id = if def.is_seeded == 1 {
        String::new()
    } else {
        def.parent_id.clone()
    };
    // Use INSERT OR REPLACE so a re-run after a partial state
    // (e.g. a developer deleted the marker manually) doesn't
    // explode on the PK; the user-clone-binding fold in
    // `fold_instance` relies on definition rows existing.
    tx.execute(
        "INSERT OR REPLACE INTO db_agents (
                    id, name, icon, description,
                    is_template, parent_template_id,
                    provider, provider_flags, shell, environment,
                    agent_type, agent_bus_id, accounts,
                    auto_start, restart_on_crash, idle_timeout_minutes,
                    slug, branch_label,
                    identity_id, memory_id, working_directory, github_context,
                    instance_name,
                    created_at, updated_at, is_seeded, user_hidden
                 ) VALUES (
                    ?1, ?2, ?3, ?4,
                    ?5, ?6,
                    ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13,
                    ?14, ?15, ?16,
                    ?17, ?18,
                    '', '', ?19, '',
                    '',
                    ?20, ?21, ?22, 0
                 )",
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
        ],
    )?;
    if def.is_seeded == 1 {
        stats.templates_inserted += 1;
    } else {
        stats.user_defs_inserted += 1;
    }
    Ok(())
}

/// Every `db_agent_instances` row with its definition's fields, newest first.
pub(super) fn load_instances(tx: &Transaction) -> Result<Vec<InstanceRow>, StoreError> {
    // Pass 3 + 4 — instances. Tolerates `db_agent_instances` not existing
    // even though `db_agent_definitions` does (checked separately from the
    // guard at the top of `run_consolidate_migration`, which only covers "neither table
    // exists at all") — a definition with no instance ever recorded against
    // it is a legitimate shape (pre-consolidation agents that were defined
    // but never launched), not a reason to skip Pass 1/2 too.
    //
    // Order by created_at DESC so the FIRST instance we see for a
    // given user-cloned def is the most recent (the spec wants
    // most-recent bindings to win on collision).
    let insts_exist: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_agent_instances')",
        [],
        |row| row.get(0),
    )?;
    if !insts_exist {
        return Ok(Vec::new());
    }
    let mut stmt = tx.prepare(
        "SELECT i.id, i.definition_id, i.parent_instance_id,
                    i.instance_name, i.identity_id, i.memory_id,
                    i.working_directory, i.github_context,
                    i.created_at, i.display_hidden,
                    d.is_seeded, d.name, d.icon, d.description,
                    d.provider, d.provider_flags, d.shell, d.environment,
                    d.agent_type, d.agent_bus_id, d.accounts,
                    d.auto_start, d.restart_on_crash, d.idle_timeout_minutes,
                    d.slug, d.branch_label
             FROM db_agent_instances i
             LEFT JOIN db_agent_definitions d ON d.id = i.definition_id
             ORDER BY i.created_at DESC",
    )?;
    let iter = stmt.query_map([], |row| {
        Ok(InstanceRow {
            id: row.get(0)?,
            definition_id: row.get(1)?,
            parent_instance_id: row.get(2)?,
            instance_name: row.get(3)?,
            identity_id: row.get(4)?,
            memory_id: row.get(5)?,
            working_directory: row.get(6)?,
            github_context: row.get(7)?,
            created_at: row.get(8)?,
            display_hidden: row.get::<_, i64>(9)? != 0,
            def_is_seeded: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
            def_name: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
            def_icon: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
            def_description: row.get::<_, Option<String>>(13)?.unwrap_or_default(),
            def_provider: row.get::<_, Option<String>>(14)?.unwrap_or_default(),
            def_provider_flags: row.get::<_, Option<String>>(15)?.unwrap_or_default(),
            def_shell: row.get::<_, Option<String>>(16)?.unwrap_or_default(),
            def_environment: row.get::<_, Option<String>>(17)?.unwrap_or_default(),
            def_agent_type: row
                .get::<_, Option<String>>(18)?
                .unwrap_or_else(|| "standalone".to_string()),
            def_agent_bus_id: row.get::<_, Option<String>>(19)?.unwrap_or_default(),
            def_accounts: row.get::<_, Option<String>>(20)?.unwrap_or_default(),
            def_auto_start: row.get::<_, Option<i64>>(21)?.unwrap_or(0),
            def_restart_on_crash: row.get::<_, Option<i64>>(22)?.unwrap_or(0),
            def_idle_timeout_minutes: row.get::<_, Option<i64>>(23)?.unwrap_or(0),
            def_slug: row.get::<_, Option<String>>(24)?.unwrap_or_default(),
            def_branch_label: row.get::<_, Option<String>>(25)?.unwrap_or_default(),
            def_present: row.get::<_, Option<i64>>(10)?.is_some(),
        })
    })?;
    let mut out = Vec::new();
    for r in iter {
        out.push(r?);
    }
    Ok(out)
}

/// Pass 3 + 4 — instances.
pub(super) fn project_instances(
    tx: &Transaction,
    inst_rows: &[InstanceRow],
    stats: &mut ConsolidateStats,
) -> Result<(), StoreError> {
    // Track which user-clone def-ids already had their bindings folded.
    // Multiple instances on the same user-clone-def → first wins (most
    // recent because we ordered DESC); the rest get a warning.
    let mut folded: std::collections::HashSet<String> = std::collections::HashSet::new();

    for inst in inst_rows {
        if !inst.parent_instance_id.is_empty() {
            stats.instances_skipped_continuation += 1;
            continue;
        }
        if !inst.def_present {
            // Orphaned instance — no definition row. The old schema's
            // FK cascade would have removed this; if it survived,
            // there's nothing to project against.
            stats.instances_skipped_no_definition += 1;
            warn!(
                instance_id = %inst.id,
                definition_id = %inst.definition_id,
                "agents_consolidate: instance has no definition; skipping",
            );
            continue;
        }
        if inst.def_is_seeded == 1 {
            // Instance of a TEMPLATE — INSERT a new user-clone row
            // keyed by the instance id.
            insert_template_clone(tx, inst)?;
            stats.instances_as_clone_inserted += 1;
        } else {
            // Instance of an already-user-cloned definition — UPDATE
            // the existing user-clone row (keyed by definition_id)
            // to fold in the instance bindings. Collision: only the
            // first (most recent) wins; warn on subsequent.
            fold_instance(tx, inst, &mut folded, stats)?;
        }
    }
    Ok(())
}

/// An instance of a template: insert a new user-clone row keyed by the
/// instance id.
fn insert_template_clone(tx: &Transaction, inst: &InstanceRow) -> Result<(), StoreError> {
    let name = if inst.instance_name.is_empty() {
        inst.def_name.clone()
    } else {
        inst.instance_name.clone()
    };
    tx.execute(
        "INSERT OR REPLACE INTO db_agents (
                    id, name, icon, description,
                    is_template, parent_template_id,
                    provider, provider_flags, shell, environment,
                    agent_type, agent_bus_id, accounts,
                    auto_start, restart_on_crash, idle_timeout_minutes,
                    slug, branch_label,
                    identity_id, memory_id, working_directory, github_context,
                    instance_name,
                    created_at, updated_at, is_seeded, user_hidden
                 ) VALUES (
                    ?1, ?2, ?3, ?4,
                    0, ?5,
                    ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12,
                    ?13, ?14, ?15,
                    ?16, ?17,
                    ?18, ?19, ?20, ?21,
                    ?22,
                    ?23, ?23, 0, ?24
                 )",
        params![
            inst.id,
            name,
            inst.def_icon,
            inst.def_description,
            inst.definition_id,
            inst.def_provider,
            inst.def_provider_flags,
            inst.def_shell,
            inst.def_environment,
            inst.def_agent_type,
            inst.def_agent_bus_id,
            inst.def_accounts,
            inst.def_auto_start,
            inst.def_restart_on_crash,
            inst.def_idle_timeout_minutes,
            inst.def_slug,
            inst.def_branch_label,
            inst.identity_id,
            inst.memory_id,
            inst.working_directory,
            inst.github_context,
            inst.instance_name,
            inst.created_at,
            if inst.display_hidden { 1_i64 } else { 0_i64 },
        ],
    )?;
    Ok(())
}

/// An instance of an already-user-cloned definition: fold its bindings into
/// that row, unless a newer instance already did.
fn fold_instance(
    tx: &Transaction,
    inst: &InstanceRow,
    folded: &mut std::collections::HashSet<String>,
    stats: &mut ConsolidateStats,
) -> Result<(), StoreError> {
    if folded.contains(&inst.definition_id) {
        stats.instances_collision_warned += 1;
        warn!(
            instance_id = %inst.id,
            definition_id = %inst.definition_id,
            "agents_consolidate: multiple instances on one user-cloned def; keeping most-recent bindings",
        );
        return Ok(());
    }
    let name = if inst.instance_name.is_empty() {
        inst.def_name.clone()
    } else {
        inst.instance_name.clone()
    };
    // Stamp `updated_at` with the folded instance's
    // `created_at` (its launch moment). Without this, the
    // backfill UPDATE leaves `updated_at` as whatever the
    // def's edit time was — which makes `db_agents.updated_at`
    // useless as a "most-recently-used" sort key for migrated
    // stores, breaking the ordering invariant that the live
    // launch write (`Store::instance_create`) maintains
    // for new launches. Surfaced in #1110 via the
    // first read-flip (the since-deleted `instance_get_by_name`)
    // ordering by `updated_at DESC`.
    //
    // `project_instances` iterates `ORDER BY i.created_at DESC`, so the
    // FIRST instance per def_id is the most recent, and that's
    // the one whose `created_at` we want imprinted as
    // `updated_at`. Collision warns skip subsequent rows for
    // the same def.
    tx.execute(
        "UPDATE db_agents SET
                    name = ?1,
                    identity_id = ?2,
                    memory_id = ?3,
                    working_directory = ?4,
                    github_context = ?5,
                    instance_name = ?6,
                    user_hidden = ?7,
                    updated_at = MAX(updated_at, ?8)
                 WHERE id = ?9 AND is_template = 0",
        params![
            name,
            inst.identity_id,
            inst.memory_id,
            inst.working_directory,
            inst.github_context,
            inst.instance_name,
            if inst.display_hidden { 1_i64 } else { 0_i64 },
            inst.created_at,
            inst.definition_id,
        ],
    )?;
    folded.insert(inst.definition_id.clone());
    stats.instances_folded_into_def += 1;
    Ok(())
}

/// Write the marker after a committed backfill.
pub(super) fn write_marker(dir: &Path) {
    let marker = dir.join(CONSOLIDATE_MARKER);
    if let Err(e) = std::fs::write(&marker, b"phase3a") {
        // The data is in place; failing to write the marker just
        // means the next startup will redo the work. Log + return
        // success.
        warn!(
            error = %e,
            marker = %marker.display(),
            "agents_consolidate: failed to write marker; next startup will redo backfill",
        );
    }
}

/// Log the closing counts.
pub(super) fn log_completed(stats: &ConsolidateStats) {
    info!(
        templates_inserted = stats.templates_inserted,
        user_defs_inserted = stats.user_defs_inserted,
        instances_as_clone_inserted = stats.instances_as_clone_inserted,
        instances_folded_into_def = stats.instances_folded_into_def,
        instances_skipped_continuation = stats.instances_skipped_continuation,
        instances_skipped_no_definition = stats.instances_skipped_no_definition,
        instances_collision_warned = stats.instances_collision_warned,
        "agents_consolidate: backfill completed",
    );
}

/// Snapshot of one `db_agent_definitions` row, narrow projection
/// matching what the backfill needs.
struct DefRow {
    id: String,
    name: String,
    icon: String,
    provider: String,
    description: String,
    working_directory: String,
    shell: String,
    provider_flags: String,
    auto_start: i64,
    restart_on_crash: i64,
    idle_timeout_minutes: i64,
    created_at: i64,
    agent_type: String,
    environment: String,
    agent_bus_id: String,
    is_seeded: i64,
    accounts: String,
    parent_id: String,
    branch_label: String,
    updated_at: i64,
    slug: String,
}

/// Snapshot of one `db_agent_instances` row plus the LEFT-JOINed
/// definition fields the backfill copies into `db_agents`.
pub(super) struct InstanceRow {
    id: String,
    definition_id: String,
    parent_instance_id: String,
    instance_name: String,
    identity_id: String,
    memory_id: String,
    working_directory: String,
    github_context: String,
    created_at: i64,
    display_hidden: bool,
    def_present: bool,
    def_is_seeded: i64,
    def_name: String,
    def_icon: String,
    def_description: String,
    def_provider: String,
    def_provider_flags: String,
    def_shell: String,
    def_environment: String,
    def_agent_type: String,
    def_agent_bus_id: String,
    def_accounts: String,
    def_auto_start: i64,
    def_restart_on_crash: i64,
    def_idle_timeout_minutes: i64,
    def_slug: String,
    def_branch_label: String,
}
