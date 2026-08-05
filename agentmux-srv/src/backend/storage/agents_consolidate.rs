// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Phase 3a — one-shot backfill from `db_agent_definitions` +
//! `db_agent_instances` into the new consolidated `db_agents` table.
//!
//! See `docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md`.
//!
//! Phase 3a is **write-only**: this migration populates `db_agents` so
//! a later Phase 3b PR can flip readers over with full confidence the
//! data is there. Phase 3c drops the old tables.
//!
//! Marker-file gated (`<data_dir>/migration_agents_consolidate_v1.flag`)
//! so the backfill only runs once per data dir. Idempotent on second
//! run (marker check short-circuits).
//!
//! Algorithm (per spec §"What `db_agent_instances` rows become"):
//!
//! 1. For each `db_agent_definitions WHERE is_seeded = 1`: INSERT a
//!    template projection (`is_template = 1`, bindings empty).
//!
//! 2. For each `db_agent_definitions WHERE is_seeded = 0`: INSERT a
//!    user-clone projection (`is_template = 0`, `parent_template_id =
//!    parent_id`).
//!
//! 3. For each `db_agent_instances` row whose `definition_id` points at
//!    a TEMPLATE: INSERT a new user-clone projection keyed by
//!    `instance.id`, `parent_template_id = definition_id`, name +
//!    bindings from the instance.
//!
//! 4. For each `db_agent_instances` row whose `definition_id` points at
//!    an already-user-cloned definition: UPDATE the existing
//!    user-clone projection (keyed by the definition id from pass 2) to
//!    fold in the instance's bindings. If multiple instances point at
//!    the same user-clone, the most-recent (`created_at` DESC) wins
//!    and a warning is logged.
//!
//! Continuation rows (`parent_instance_id` non-empty) are skipped — the
//! consolidated model has no place for them; they were the
//! pre-Option-E continuation chain.

use std::path::Path;

use rusqlite::{params, Connection};
use tracing::{info, warn};

use super::error::StoreError;

/// Marker filename. Lives in the data dir (one level above the `db/`
/// subdir that holds `objects.db`).
pub const CONSOLIDATE_MARKER: &str = "migration_agents_consolidate_v1.flag";

/// Backfill statistics — useful for logs + tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConsolidateStats {
    pub templates_inserted: usize,
    pub user_defs_inserted: usize,
    pub instances_as_clone_inserted: usize,
    pub instances_folded_into_def: usize,
    pub instances_skipped_continuation: usize,
    pub instances_skipped_no_definition: usize,
    pub instances_collision_warned: usize,
    pub already_done: bool,
}

/// True when the backfill's source tables have data but its target
/// (`db_agents`) doesn't — i.e. a marker (or a `db_migrations` stamp) claims
/// this migration ran, but the data it's supposed to have produced isn't
/// there. Used to stop trusting marker/stamp *existence* as proof of
/// *effect* (see `docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md`
/// Phase 0a/0b) — a stale marker (rebuilt/restored `objects.db` sitting next
/// to an old flag file, or `m0000_bootstrap` stamping from a copied flag
/// file with no matching data) is now detectable instead of silently
/// trusted.
///
/// Cheap: three `COUNT(*)` queries, safe to call on every startup.
pub fn consolidate_looks_incomplete(conn: &Connection) -> Result<bool, StoreError> {
    let defs: i64 = conn.query_row("SELECT COUNT(*) FROM db_agent_definitions", [], |r| r.get(0))?;
    let insts: i64 = conn.query_row("SELECT COUNT(*) FROM db_agent_instances", [], |r| r.get(0))?;
    let agents: i64 = conn.query_row("SELECT COUNT(*) FROM db_agents", [], |r| r.get(0))?;
    Ok((defs > 0 || insts > 0) && agents == 0)
}

/// Run the one-shot consolidation backfill, gated by the marker file.
///
/// `data_dir` is the directory that holds the marker file (typically
/// the parent of the `db/` directory). Pass `None` to skip marker
/// gating — only intended for tests + in-memory stores.
///
/// Returns `Ok(stats)` on success (incl. the marker-already-present
/// short-circuit). Failures roll back the active transaction and
/// return the underlying SQLite error; the marker is NOT written.
pub fn run_consolidate_migration(
    conn: &mut Connection,
    data_dir: Option<&Path>,
) -> Result<ConsolidateStats, StoreError> {
    // Marker gate. Existence alone is no longer trusted (Phase 0b): if the
    // source tables have rows but db_agents doesn't, the marker is stale —
    // fall through and re-run instead of silently reporting "already done."
    if let Some(dir) = data_dir {
        let marker = dir.join(CONSOLIDATE_MARKER);
        if marker.exists() {
            if !consolidate_looks_incomplete(conn)? {
                return Ok(ConsolidateStats {
                    already_done: true,
                    ..Default::default()
                });
            }
            warn!(
                marker = %marker.display(),
                "agents_consolidate: marker present but db_agents is empty while source tables have rows — \
                 treating as stale and re-running instead of trusting it",
            );
        }
    }

    // The backfill runs inside a transaction so a mid-flight failure
    // leaves db_agents empty rather than half-populated. The dual-
    // write call sites tolerate empty + idempotently upsert on the
    // next mutation, so partial backfill state is recoverable.
    let tx = conn.transaction()?;

    let mut stats = ConsolidateStats::default();

    // Pass 1 + 2 — definitions.
    {
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
        for def in &defs {
            let is_template = if def.is_seeded == 1 { 1_i64 } else { 0_i64 };
            let parent_template_id = if def.is_seeded == 1 {
                String::new()
            } else {
                def.parent_id.clone()
            };
            // Use INSERT OR REPLACE so a re-run after a partial state
            // (e.g. a developer deleted the marker manually) doesn't
            // explode on the PK; the user-clone-binding-fold path
            // immediately below relies on definition rows existing.
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
        }
    }

    // Pass 3 + 4 — instances.
    // Order by created_at DESC so the FIRST instance we see for a
    // given user-cloned def is the most recent (the spec wants
    // most-recent bindings to win on collision).
    let inst_rows: Vec<InstanceRow> = {
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
        out
    };

    // Track which user-clone def-ids already had their bindings folded.
    // Multiple instances on the same user-clone-def → first wins (most
    // recent because we ordered DESC); the rest get a warning.
    let mut folded: std::collections::HashSet<String> = std::collections::HashSet::new();

    for inst in &inst_rows {
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
            stats.instances_as_clone_inserted += 1;
        } else {
            // Instance of an already-user-cloned definition — UPDATE
            // the existing user-clone row (keyed by definition_id)
            // to fold in the instance bindings. Collision: only the
            // first (most recent) wins; warn on subsequent.
            if folded.contains(&inst.definition_id) {
                stats.instances_collision_warned += 1;
                warn!(
                    instance_id = %inst.id,
                    definition_id = %inst.definition_id,
                    "agents_consolidate: multiple instances on one user-cloned def; keeping most-recent bindings",
                );
                continue;
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
            // dual-write (`agents_dual_write_instance_insert`) maintains
            // for new launches. Codex P2 on PR #1110 — surfaced via the
            // first read-flip (`instance_get_by_name`) ordering by
            // `updated_at DESC`.
            //
            // The loop iterates `ORDER BY i.created_at DESC`, so the
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
        }
    }

    tx.commit()?;

    // Marker written AFTER successful commit so a crash mid-backfill
    // leaves the marker absent → next start retries from scratch.
    if let Some(dir) = data_dir {
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
    Ok(stats)
}

/// Delta repair: backfill any `db_agent_definitions` rows that are
/// missing from `db_agents`.
///
/// This closes a gap the one-shot consolidation migration cannot cover:
/// agents defined after the marker file was written (and before Phase 3b
/// dual-write landed) live only in `db_agent_definitions`.  Phase 3b
/// readers look exclusively at `db_agents`, so those agents are invisible
/// — no icon, no reattach, "clicking does nothing".
///
/// Unlike `run_consolidate_migration`, this is **not** marker-gated.  It
/// runs on every startup (cheap — one indexed LEFT JOIN + a handful of
/// inserts at most) and is idempotent via `INSERT OR IGNORE`.
///
/// Returns the number of definitions inserted.
pub fn repair_def_gaps(conn: &mut Connection) -> Result<usize, StoreError> {
    // Find every db_agent_definitions row that has no matching id in
    // db_agents.  These were written between Phase 3a marker creation and
    // Phase 3b dual-write landing.
    let mut stmt = conn.prepare(
        "SELECT d.id, d.name, d.icon, d.provider, d.description,
                d.working_directory, d.shell, d.provider_flags, d.auto_start,
                d.restart_on_crash, d.idle_timeout_minutes, d.created_at,
                d.agent_type, d.environment, d.agent_bus_id, d.is_seeded,
                d.accounts, d.parent_id, d.branch_label, d.updated_at,
                d.slug, d.user_hidden
         FROM db_agent_definitions d
         LEFT JOIN db_agents a ON a.id = d.id
         WHERE a.id IS NULL",
    )?;

    #[allow(clippy::type_complexity)]
    let missing: Vec<(
        String, String, String, String, String, String, String, String,
        i64, i64, i64, i64, String, String, String, i64, String, String,
        String, i64, String, i64,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,   // id
                row.get::<_, String>(1)?,   // name
                row.get::<_, String>(2)?,   // icon
                row.get::<_, String>(3)?,   // provider
                row.get::<_, String>(4)?,   // description
                row.get::<_, String>(5)?,   // working_directory
                row.get::<_, String>(6)?,   // shell
                row.get::<_, String>(7)?,   // provider_flags
                row.get::<_, i64>(8)?,      // auto_start
                row.get::<_, i64>(9)?,      // restart_on_crash
                row.get::<_, i64>(10)?,     // idle_timeout_minutes
                row.get::<_, i64>(11)?,     // created_at
                row.get::<_, String>(12)?,  // agent_type
                row.get::<_, String>(13)?,  // environment
                row.get::<_, String>(14)?,  // agent_bus_id
                row.get::<_, i64>(15)?,     // is_seeded
                row.get::<_, String>(16)?,  // accounts
                row.get::<_, String>(17)?,  // parent_id
                row.get::<_, String>(18)?,  // branch_label
                row.get::<_, i64>(19)?,     // updated_at
                row.get::<_, String>(20)?,  // slug
                row.get::<_, i64>(21)?,     // user_hidden
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    if missing.is_empty() {
        return Ok(0);
    }

    let tx = conn.transaction()?;
    let mut inserted = 0usize;
    for (
        id, name, icon, provider, description, working_directory, shell,
        provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
        created_at, agent_type, environment, agent_bus_id, is_seeded,
        accounts, parent_id, branch_label, updated_at, slug, user_hidden,
    ) in &missing
    {
        let is_template = if *is_seeded == 1 { 1_i64 } else { 0_i64 };
        let parent_template_id = if *is_seeded == 1 {
            String::new()
        } else {
            parent_id.clone()
        };
        let affected = tx.execute(
            "INSERT OR IGNORE INTO db_agents (
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
                ?20, ?21, ?22, ?23
             )",
            params![
                id, name, icon, description,
                is_template, parent_template_id,
                provider, provider_flags, shell, environment,
                agent_type, agent_bus_id, accounts,
                auto_start, restart_on_crash, idle_timeout_minutes,
                slug, branch_label,
                working_directory,
                created_at, updated_at, is_seeded, user_hidden,
            ],
        )?;
        if affected > 0 {
            warn!(
                def_id = %id,
                name = %name,
                "agents_consolidate: gap-repair inserted missing definition into db_agents"
            );
            inserted += 1;
        }
    }
    tx.commit()?;

    if inserted > 0 {
        info!(
            count = inserted,
            "agents_consolidate: gap-repair complete — definitions backfilled"
        );
    }

    Ok(inserted)
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
struct InstanceRow {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::migrations::run_object_schema;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        conn
    }

    fn insert_def(
        conn: &Connection,
        id: &str,
        name: &str,
        is_seeded: i64,
        parent_id: &str,
    ) {
        conn.execute(
            "INSERT INTO db_agent_definitions
                (id, slug, name, icon, provider, description, working_directory, shell,
                 provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
                 created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
                 parent_id, branch_label, updated_at)
             VALUES (?1, ?2, ?3, '✦', 'claude', 'desc', '', 'bash',
                     '', 0, 0, 0,
                     ?4, 'standalone', '', '', ?5, '',
                     ?6, '', ?4)",
            params![id, id, name, 1000_i64, is_seeded, parent_id],
        )
        .unwrap();
    }

    fn insert_instance(
        conn: &Connection,
        id: &str,
        definition_id: &str,
        instance_name: &str,
        identity_id: &str,
        memory_id: &str,
        working_directory: &str,
        created_at: i64,
        display_hidden: bool,
    ) {
        conn.execute(
            "INSERT INTO db_agent_instances
                (id, definition_id, parent_instance_id, block_id, session_id, status,
                 github_context, started_at, ended_at, created_at, identity_id, memory_id,
                 instance_name, working_directory, display_hidden)
             VALUES (?1, ?2, '', '', '', 'running', '', ?3, 0, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                definition_id,
                created_at,
                identity_id,
                memory_id,
                instance_name,
                working_directory,
                if display_hidden { 1_i64 } else { 0_i64 },
            ],
        )
        .unwrap();
    }

    fn count_agents(conn: &Connection, where_clause: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM db_agents WHERE {where_clause}");
        conn.query_row(&sql, [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn round_trip_template_user_clone_and_instance_lifecycle() {
        let mut conn = fresh_conn();

        // 2 templates, 1 user-cloned definition (from template tpl-1),
        // 3 instances:
        //   - inst-A on template tpl-1 (named "Maks")
        //   - inst-B on template tpl-2 (unnamed)
        //   - inst-C on user-clone-def def-u1 (named "Custom")
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        insert_def(&conn, "tpl-2", "Reviewer", 1, "");
        insert_def(&conn, "def-u1", "Maks-the-Coder", 0, "tpl-1");
        insert_instance(&conn, "inst-A", "tpl-1", "Maks", "id-1", "mem-1", "/wd/a", 100, false);
        insert_instance(&conn, "inst-B", "tpl-2", "", "", "", "/wd/b", 200, false);
        insert_instance(&conn, "inst-C", "def-u1", "Custom", "id-2", "mem-2", "/wd/c", 300, false);

        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.templates_inserted, 2);
        assert_eq!(stats.user_defs_inserted, 1);
        assert_eq!(stats.instances_as_clone_inserted, 2);
        assert_eq!(stats.instances_folded_into_def, 1);
        assert_eq!(stats.instances_skipped_continuation, 0);

        // Templates present, is_template=1, parent_template_id empty.
        assert_eq!(count_agents(&conn, "is_template = 1"), 2);
        for tpl in &["tpl-1", "tpl-2"] {
            let parent: String = conn
                .query_row(
                    "SELECT parent_template_id FROM db_agents WHERE id = ?1",
                    params![tpl],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(parent, "");
        }

        // 3 user-clone rows: def-u1 (folded from inst-C), inst-A, inst-B.
        assert_eq!(count_agents(&conn, "is_template = 0"), 3);

        // inst-A → parent_template_id = tpl-1, name = "Maks".
        let (parent, name, identity): (String, String, String) = conn
            .query_row(
                "SELECT parent_template_id, name, identity_id FROM db_agents WHERE id = 'inst-A'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(parent, "tpl-1");
        assert_eq!(name, "Maks");
        assert_eq!(identity, "id-1");

        // inst-B (unnamed) → name falls back to template name "Reviewer".
        let name: String = conn
            .query_row(
                "SELECT name FROM db_agents WHERE id = 'inst-B'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "Reviewer");

        // def-u1 (user-clone) folded inst-C's bindings.
        let (name, identity, memory): (String, String, String) = conn
            .query_row(
                "SELECT name, identity_id, memory_id FROM db_agents WHERE id = 'def-u1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "Custom"); // instance_name overrode def name
        assert_eq!(identity, "id-2");
        assert_eq!(memory, "mem-2");
    }

    #[test]
    fn multiple_instances_on_one_user_clone_keeps_most_recent_bindings() {
        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        insert_def(&conn, "def-u1", "User Coder", 0, "tpl-1");
        // Two instances on def-u1; most recent has id-RECENT.
        insert_instance(&conn, "inst-old", "def-u1", "Old", "id-OLD", "mem-OLD", "/wd/old", 100, false);
        insert_instance(&conn, "inst-new", "def-u1", "New", "id-RECENT", "mem-RECENT", "/wd/new", 999, false);

        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_collision_warned, 1);

        let identity: String = conn
            .query_row(
                "SELECT identity_id FROM db_agents WHERE id = 'def-u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(identity, "id-RECENT", "most-recent instance's bindings must win");
    }

    #[test]
    fn fold_into_user_clone_stamps_updated_at_from_instance_created_at() {
        // Codex P2 on PR #1110: ordering `db_agents` by `updated_at`
        // would misbehave on migrated stores if the fold UPDATE
        // didn't write `updated_at`. After the fix, the folded
        // user-clone row's `updated_at` equals the most-recent
        // instance's `created_at` (the launch moment), not the def's
        // edit time — matching what the live dual-write
        // (`agents_dual_write_instance_insert`) stamps for new
        // launches.
        let mut conn = fresh_conn();
        // Def created at t=1000 (insert_def uses ?4 for both
        // created_at and updated_at).
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        insert_def(&conn, "def-u1", "User Coder", 0, "tpl-1");
        // Instance launched at t=5000 — later than the def.
        insert_instance(
            &conn, "inst-1", "def-u1", "Maks", "id-1", "mem-1", "/wd/m", 5000, false,
        );

        run_consolidate_migration(&mut conn, None).unwrap();

        let updated_at: i64 = conn
            .query_row(
                "SELECT updated_at FROM db_agents WHERE id = 'def-u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            updated_at, 5000,
            "folded user-clone row's updated_at must reflect the instance's launch time, \
             not the def's edit time, so ORDER BY updated_at picks the right row"
        );
    }

    #[test]
    fn fold_into_user_clone_keeps_higher_updated_at() {
        // MAX(updated_at, ?) semantics: if the def was edited AFTER
        // the instance launched, the def's edit time should stay as
        // updated_at (the def edit IS the most-recent touch).
        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        // Def edited at t=8000.
        conn.execute(
            "UPDATE db_agent_definitions SET updated_at = 8000 WHERE id = 'tpl-1'",
            [],
        )
        .unwrap();
        insert_def(&conn, "def-u1", "User Coder", 0, "tpl-1");
        conn.execute(
            "UPDATE db_agent_definitions SET updated_at = 8000 WHERE id = 'def-u1'",
            [],
        )
        .unwrap();
        // Now insert an OLDER instance.
        insert_instance(
            &conn, "inst-1", "def-u1", "Maks", "id-1", "mem-1", "/wd/m", 3000, false,
        );

        run_consolidate_migration(&mut conn, None).unwrap();

        let updated_at: i64 = conn
            .query_row(
                "SELECT updated_at FROM db_agents WHERE id = 'def-u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            updated_at, 8000,
            "later def edit beats older instance launch — MAX(updated_at, inst.created_at)"
        );
    }

    #[test]
    fn marker_short_circuits_second_run() {
        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        let tmp = tempfile::tempdir().unwrap();

        let stats1 = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert_eq!(stats1.templates_inserted, 1);
        assert!(!stats1.already_done);
        assert!(tmp.path().join(CONSOLIDATE_MARKER).exists());

        // Insert a NEW row after the marker; second call must NOT see
        // it.
        insert_def(&conn, "tpl-2", "Reviewer", 1, "");
        let stats2 = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert!(stats2.already_done);
        assert_eq!(stats2.templates_inserted, 0);
        // db_agents only has tpl-1 — the post-marker insert is the
        // dual-write hook's problem, not the backfill's.
        assert_eq!(count_agents(&conn, "is_template = 1"), 1);
    }

    #[test]
    fn skips_continuation_rows() {
        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        insert_instance(&conn, "inst-A", "tpl-1", "Original", "", "", "/wd/a", 100, false);
        // Continuation row (parent_instance_id = inst-A).
        conn.execute(
            "INSERT INTO db_agent_instances
                (id, definition_id, parent_instance_id, block_id, session_id, status,
                 github_context, started_at, ended_at, created_at, identity_id, memory_id,
                 instance_name, working_directory, display_hidden)
             VALUES ('inst-cont', 'tpl-1', 'inst-A', '', '', 'running', '', 200, 0, 200,
                     '', '', 'Original', '/wd/a', 0)",
            [],
        )
        .unwrap();

        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_skipped_continuation, 1);
        // Only inst-A and the template made it into db_agents.
        assert_eq!(count_agents(&conn, "1 = 1"), 2);
        assert_eq!(count_agents(&conn, "id = 'inst-cont'"), 0);
    }

    #[test]
    fn empty_database_is_clean_noop() {
        let mut conn = fresh_conn();
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats, ConsolidateStats::default());
        assert_eq!(count_agents(&conn, "1 = 1"), 0);
    }

    #[test]
    fn preserves_hidden_flag_into_user_hidden() {
        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        insert_instance(&conn, "inst-H", "tpl-1", "Hidden", "", "", "/wd/h", 100, true);
        run_consolidate_migration(&mut conn, None).unwrap();
        let hidden: i64 = conn
            .query_row(
                "SELECT user_hidden FROM db_agents WHERE id = 'inst-H'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hidden, 1);
    }

    #[test]
    fn definition_working_directory_survives_the_pass_1_2_backfill() {
        // Regression: the Pass 1/2 INSERT (templates + user-cloned
        // definitions with no instance to fold) used to hardcode
        // working_directory as '' instead of binding def.working_directory,
        // silently discarding it during consolidation — audit finding,
        // docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_20.md §1.2. A
        // definition that was never instantiated (no db_agent_instances
        // row) only ever goes through this path — Pass 3/4 (instances)
        // never touches it — so this case wasn't covered by any existing
        // test here.
        let mut conn = fresh_conn();
        conn.execute(
            "INSERT INTO db_agent_definitions
                (id, slug, name, icon, provider, description, working_directory, shell,
                 provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
                 created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
                 parent_id, branch_label, updated_at)
             VALUES ('tpl-wd', 'tpl-wd', 'Coder', '✦', 'claude', 'desc',
                     '/home/user/my-project', 'bash',
                     '', 0, 0, 0,
                     1000, 'standalone', '', '', 1, '',
                     '', '', 1000)",
            [],
        )
        .unwrap();

        run_consolidate_migration(&mut conn, None).unwrap();

        let working_directory: String = conn
            .query_row(
                "SELECT working_directory FROM db_agents WHERE id = 'tpl-wd'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            working_directory, "/home/user/my-project",
            "a definition's working_directory must survive Pass 1/2 consolidation \
             even when it was never instantiated (no instance row to fold bindings from)"
        );
    }

    // ── Phase 0 hardening (SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03) ──────

    #[test]
    fn consolidate_looks_incomplete_is_false_on_a_fresh_db() {
        let conn = fresh_conn();
        // No source rows, no db_agents rows — nothing to backfill, not "incomplete".
        assert!(!consolidate_looks_incomplete(&conn).unwrap());
    }

    #[test]
    fn consolidate_looks_incomplete_is_false_once_backfilled() {
        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        run_consolidate_migration(&mut conn, None).unwrap();
        assert!(!consolidate_looks_incomplete(&conn).unwrap());
    }

    #[test]
    fn consolidate_looks_incomplete_is_true_when_source_has_rows_but_db_agents_is_empty() {
        let conn = fresh_conn();
        // Source rows present, but db_agents was never populated (the exact
        // shape a stale/orphaned marker file produces).
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        assert!(consolidate_looks_incomplete(&conn).unwrap());
    }

    #[test]
    fn stale_marker_does_not_block_a_real_backfill() {
        // Acceptance test for Phase 0a/0b: a data dir with a marker file
        // present but an empty db_agents table (simulating a rebuilt/restored
        // objects.db sitting next to a stale flag file) must NOT be trusted
        // as "already done" — the migration must detect this and actually
        // run the backfill instead of silently no-op'ing.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join(CONSOLIDATE_MARKER), b"phase3a").unwrap();

        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        assert_eq!(count_agents(&conn, "1=1"), 0, "db_agents must start empty for this test to be meaningful");

        let stats = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert!(!stats.already_done, "a stale marker must not short-circuit as already_done");
        assert_eq!(stats.templates_inserted, 1);
        assert_eq!(count_agents(&conn, "1=1"), 1, "db_agents must actually get populated, not silently skipped");
    }

    #[test]
    fn genuinely_completed_marker_still_short_circuits() {
        // The flip side: once db_agents is correctly populated, a present
        // marker SHOULD short-circuit (already_done=true) — Phase 0's fix
        // must not make every run pay the full backfill cost forever.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join(CONSOLIDATE_MARKER), b"phase3a").unwrap();

        let mut conn = fresh_conn();
        insert_def(&conn, "tpl-1", "Coder", 1, "");
        run_consolidate_migration(&mut conn, None).unwrap(); // populate db_agents directly
        assert_eq!(count_agents(&conn, "1=1"), 1);

        let stats = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert!(stats.already_done, "a marker backed by real data must still short-circuit");
        assert_eq!(count_agents(&conn, "1=1"), 1, "must not double-insert");
    }
}
