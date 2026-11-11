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

use rusqlite::Connection;

use super::error::StoreError;

#[path = "agents_consolidate_steps.rs"]
mod steps;

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
/// to an old flag file, or a bootstrap stamping from a copied flag file with
/// no matching data) is now detectable instead of silently trusted.
///
/// Cheap: up to three `COUNT(*)` queries, safe to call on every startup.
///
/// Tolerates either source table being physically absent (Phase 3e,
/// `SPEC_AGENT_ARCHITECTURE_2026_05_27.md` — `db_agent_definitions`/
/// `db_agent_instances` are dropped by `m0029_drop_legacy_agent_tables`):
/// an absent table categorically has no rows, so it can never be the
/// "source has rows but the target doesn't" shape this function detects.
/// Load-bearing, not defensive-only — a fresh (v32+) channel is stamped
/// `0007_agents_consolidate` applied via its own early-return no-op
/// (`m0007_agents_consolidate.rs::up`'s `channel_store_path.exists()`
/// guard, for a channel created before migrations ever ran), and a later
/// `muxspect migrations --verify` doctor pass calls `m0007`'s `verify()` —
/// which calls this — against that channel once it has real, ordinarily-
/// populated `db_agents` data. Without this guard that doctor pass would
/// error on "no such table" for every fresh install, not just an upgraded
/// one that genuinely went through the old table pair.
pub fn consolidate_looks_incomplete(conn: &Connection) -> Result<bool, StoreError> {
    let defs = table_row_count_if_exists(conn, "db_agent_definitions")?;
    let insts = table_row_count_if_exists(conn, "db_agent_instances")?;
    let agents: i64 = conn.query_row("SELECT COUNT(*) FROM db_agents", [], |r| r.get(0))?;
    Ok((defs > 0 || insts > 0) && agents == 0)
}

/// `SELECT COUNT(*) FROM <table>`, or `0` if the table doesn't exist at
/// all — distinct from a genuinely empty table, which also counts as `0`
/// here on purpose (both mean "no source rows to backfill").
fn table_row_count_if_exists(conn: &Connection, table: &str) -> Result<i64, StoreError> {
    if !crate::migrations::table_exists(conn, table)? {
        return Ok(0);
    }
    Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
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
    if steps::marker_says_done(conn, data_dir)? {
        return Ok(ConsolidateStats {
            already_done: true,
            ..Default::default()
        });
    }

    // Nothing to consolidate FROM if neither source table exists — a fresh
    // (v32+) channel, or one already past `m0029_drop_legacy_agent_tables`.
    // `up()`'s own `channel_store_path.exists()` guard already keeps this
    // from firing for a channel created after this shipped (see
    // `consolidate_looks_incomplete`'s doc comment for the doctor-pass path
    // that reaches this function on an EXISTING, already-migrated channel
    // instead).
    if !steps::definitions_table_exists(conn)? {
        return Ok(ConsolidateStats::default());
    }

    // The backfill runs inside a transaction so a mid-flight failure
    // leaves db_agents empty rather than half-populated. The dual-
    // write call sites tolerate empty + idempotently upsert on the
    // next mutation, so partial backfill state is recoverable.
    let tx = conn.transaction()?;

    let mut stats = ConsolidateStats::default();

    steps::backfill_definitions(&tx, &mut stats)?;
    let inst_rows = steps::load_instances(&tx)?;
    steps::project_instances(&tx, &inst_rows, &mut stats)?;

    tx.commit()?;

    // Marker written AFTER successful commit so a crash mid-backfill
    // leaves the marker absent → next start retries from scratch.
    if let Some(dir) = data_dir {
        steps::write_marker(dir);
    }

    steps::log_completed(&stats);
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::migrations::run_object_schema;
    use rusqlite::params;

    /// `db_agent_definitions`/`db_agent_instances` are gone from the flat
    /// schema as of v32 (`OBJECT_SCHEMA_VERSION`'s v32 doc comment,
    /// `SPEC_AGENT_ARCHITECTURE_2026_05_27.md` Phase 3e) — this fixture
    /// stands up the pre-drop legacy shape by hand, since `run_object_schema`
    /// no longer will. Every column `insert_def`/`insert_instance` write is
    /// present; the FK matches what the tables actually had before the
    /// drop, so cascade behavior this module's own logic depends on
    /// (`run_consolidate_migration` reads both, unrelated to cascades) still
    /// matches production history.
    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE db_agent_definitions (
                id                   TEXT PRIMARY KEY,
                slug                 TEXT NOT NULL DEFAULT '',
                name                 TEXT NOT NULL,
                icon                 TEXT NOT NULL DEFAULT '✦',
                provider             TEXT NOT NULL,
                description          TEXT NOT NULL DEFAULT '',
                working_directory    TEXT NOT NULL DEFAULT '',
                shell                TEXT NOT NULL DEFAULT '',
                provider_flags       TEXT NOT NULL DEFAULT '',
                auto_start           INTEGER NOT NULL DEFAULT 0,
                restart_on_crash     INTEGER NOT NULL DEFAULT 0,
                idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                agent_type           TEXT NOT NULL DEFAULT 'standalone',
                environment          TEXT NOT NULL DEFAULT '',
                agent_bus_id         TEXT NOT NULL DEFAULT '',
                is_seeded            INTEGER NOT NULL DEFAULT 0,
                accounts             TEXT NOT NULL DEFAULT '',
                parent_id            TEXT NOT NULL DEFAULT '',
                branch_label         TEXT NOT NULL DEFAULT '',
                created_at           INTEGER NOT NULL DEFAULT 0,
                updated_at           INTEGER NOT NULL DEFAULT 0,
                user_hidden          INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE db_agent_instances (
                id                 TEXT PRIMARY KEY,
                definition_id      TEXT NOT NULL,
                parent_instance_id TEXT NOT NULL DEFAULT '',
                block_id           TEXT NOT NULL DEFAULT '',
                session_id         TEXT NOT NULL DEFAULT '',
                status             TEXT NOT NULL DEFAULT 'running',
                github_context     TEXT NOT NULL DEFAULT '',
                identity_id        TEXT NOT NULL DEFAULT '',
                memory_id          TEXT NOT NULL DEFAULT '',
                instance_name      TEXT NOT NULL DEFAULT '',
                working_directory  TEXT NOT NULL DEFAULT '',
                display_hidden     INTEGER NOT NULL DEFAULT 0,
                started_at         INTEGER NOT NULL DEFAULT 0,
                ended_at           INTEGER NOT NULL DEFAULT 0,
                created_at         INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (definition_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
             );",
        )
        .unwrap();
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
        // edit time — matching what the live launch write
        // (`Store::instance_create`) stamps for new
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

    /// The scenario `consolidate_looks_incomplete`'s own doc comment names:
    /// a genuinely v32+ channel, born without either legacy table at all
    /// (not just empty — physically absent, unlike `fresh_conn`'s fixture
    /// above, which stands them up empty for the OTHER tests in this file
    /// that still need to insert legacy rows). Must not error.
    #[test]
    fn consolidate_looks_incomplete_tolerates_both_tables_being_physically_absent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        assert!(!consolidate_looks_incomplete(&conn).unwrap());
    }

    /// Same scenario, through `run_consolidate_migration` directly (not
    /// just its `consolidate_looks_incomplete` sub-check) — the doctor-pass
    /// path this guards against calls `verify()`, which only reads via
    /// `consolidate_looks_incomplete`, but `run_consolidate_migration`
    /// itself must also tolerate this shape rather than erroring on the
    /// first `SELECT ... FROM db_agent_definitions` if anything ever calls
    /// it directly against a table-less store.
    #[test]
    fn run_consolidate_migration_tolerates_both_tables_being_physically_absent() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats, ConsolidateStats::default());
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

    // ── Row-level characterisation of every pass ────────────────────────────

    const AGENT_COLUMNS: &str = "id, name, icon, description, is_template, parent_template_id, \
        provider, provider_flags, shell, environment, agent_type, agent_bus_id, accounts, \
        auto_start, restart_on_crash, idle_timeout_minutes, slug, branch_label, \
        identity_id, memory_id, working_directory, github_context, instance_name, \
        created_at, updated_at, is_seeded, user_hidden";

    /// One `db_agents` row as `|`-joined text, columns in `AGENT_COLUMNS` order.
    fn row_of(conn: &Connection, id: &str) -> String {
        let cols: Vec<String> = AGENT_COLUMNS
            .split(',')
            .map(|c| format!("CAST({} AS TEXT)", c.trim()))
            .collect();
        let sql = format!("SELECT {} FROM db_agents WHERE id = ?1", cols.join(", "));
        conn.query_row(&sql, params![id], |r| {
            let mut parts = Vec::new();
            for i in 0..27 {
                parts.push(r.get::<_, String>(i)?);
            }
            Ok(parts.join("|"))
        })
        .unwrap()
    }

    /// A definition with a distinct value in every column.
    fn rich_def(conn: &Connection, id: &str, name: &str, is_seeded: i64, parent_id: &str) {
        conn.execute(
            "INSERT INTO db_agent_definitions
                (id, slug, name, icon, provider, description, working_directory, shell,
                 provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
                 created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
                 parent_id, branch_label, updated_at)
             VALUES (?1, ?1, ?2, 'ic', 'prov', 'descr', '/def/wd', 'zsh',
                     '--fl', 1, 1, 7,
                     111, 'sub', 'K=V', 'bus-9', ?3, 'acct-1',
                     ?4, 'br-1', 222)",
            params![id, name, is_seeded, parent_id],
        )
        .unwrap();
    }

    /// An instance with a distinct value in every column the backfill copies.
    fn rich_instance(
        conn: &Connection,
        id: &str,
        definition_id: &str,
        parent_instance_id: &str,
        instance_name: &str,
        created_at: i64,
        display_hidden: bool,
    ) {
        conn.execute(
            "INSERT INTO db_agent_instances
                (id, definition_id, parent_instance_id, block_id, session_id, status,
                 github_context, started_at, ended_at, created_at, identity_id, memory_id,
                 instance_name, working_directory, display_hidden)
             VALUES (?1, ?2, ?3, 'blk', 'sess', 'running', 'ghc', 1, 0, ?4, 'idn', 'mem',
                     ?5, '/inst/wd', ?6)",
            params![id, definition_id, parent_instance_id, created_at, instance_name, display_hidden as i64],
        )
        .unwrap();
    }

    #[test]
    fn a_seeded_definition_becomes_a_template_carrying_every_column() {
        let mut conn = fresh_conn();
        // The parent id is set on purpose: a template ignores it.
        rich_def(&conn, "tpl", "Tpl", 1, "ignored-parent");
        run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(
            row_of(&conn, "tpl"),
            "tpl|Tpl|ic|descr|1||prov|--fl|zsh|K=V|sub|bus-9|acct-1|1|1|7|tpl|br-1|||/def/wd|||111|222|1|0"
        );
    }

    #[test]
    fn an_unseeded_definition_becomes_a_user_clone_of_its_parent() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_def(&conn, "usr", "Usr", 0, "tpl");
        run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(
            row_of(&conn, "usr"),
            "usr|Usr|ic|descr|0|tpl|prov|--fl|zsh|K=V|sub|bus-9|acct-1|1|1|7|usr|br-1|||/def/wd|||111|222|0|0"
        );
    }

    #[test]
    fn an_instance_of_a_template_becomes_a_user_clone_keyed_by_the_instance() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_instance(&conn, "i1", "tpl", "", "Inst", 555, true);
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_as_clone_inserted, 1);
        // Definition fields from the template, bindings and timestamps from the
        // instance; the clone is never seeded even though its template is.
        assert_eq!(
            row_of(&conn, "i1"),
            "i1|Inst|ic|descr|0|tpl|prov|--fl|zsh|K=V|sub|bus-9|acct-1|1|1|7|tpl|br-1|idn|mem|/inst/wd|ghc|Inst|555|555|0|1"
        );
    }

    #[test]
    fn two_instances_of_one_template_each_get_their_own_clone() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_instance(&conn, "i1", "tpl", "", "One", 100, false);
        rich_instance(&conn, "i2", "tpl", "", "Two", 200, false);
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_as_clone_inserted, 2);
        assert_eq!(stats.instances_collision_warned, 0);
        assert_eq!(count_agents(&conn, "is_template = 0"), 2);
    }

    #[test]
    fn folding_an_instance_into_a_user_clone_rewrites_only_the_binding_columns() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_def(&conn, "usr", "Usr", 0, "tpl");
        // Launched after the definition was last edited (222).
        rich_instance(&conn, "i2", "usr", "", "Folded", 333, false);
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_folded_into_def, 1);
        assert_eq!(
            row_of(&conn, "usr"),
            "usr|Folded|ic|descr|0|tpl|prov|--fl|zsh|K=V|sub|bus-9|acct-1|1|1|7|usr|br-1|idn|mem|/inst/wd|ghc|Folded|111|333|0|0"
        );
        // No row is keyed by the folded instance.
        assert_eq!(count_agents(&conn, "id = 'i2'"), 0);
    }

    #[test]
    fn an_unnamed_instance_folds_in_under_its_definitions_name() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_def(&conn, "usr", "Usr", 0, "tpl");
        rich_instance(&conn, "i2", "usr", "", "", 333, true);
        run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(
            row_of(&conn, "usr"),
            "usr|Usr|ic|descr|0|tpl|prov|--fl|zsh|K=V|sub|bus-9|acct-1|1|1|7|usr|br-1|idn|mem|/inst/wd|ghc||111|333|0|1"
        );
    }

    #[test]
    fn three_instances_on_one_user_clone_fold_the_newest_and_warn_twice() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_def(&conn, "usr", "Usr", 0, "tpl");
        rich_instance(&conn, "old", "usr", "", "Old", 100, false);
        rich_instance(&conn, "mid", "usr", "", "Mid", 200, false);
        rich_instance(&conn, "new", "usr", "", "New", 300, false);
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_folded_into_def, 1);
        assert_eq!(stats.instances_collision_warned, 2);
        assert_eq!(row_of(&conn, "usr").split('|').nth(22), Some("New"));
    }

    #[test]
    fn a_newer_continuation_row_does_not_use_up_the_fold_slot() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_def(&conn, "usr", "Usr", 0, "tpl");
        rich_instance(&conn, "real", "usr", "", "Real", 100, false);
        rich_instance(&conn, "cont", "usr", "real", "Cont", 900, false);
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_skipped_continuation, 1);
        assert_eq!(stats.instances_collision_warned, 0);
        assert_eq!(stats.instances_folded_into_def, 1);
        assert_eq!(row_of(&conn, "usr").split('|').nth(22), Some("Real"));
    }

    #[test]
    fn an_instance_with_no_definition_is_skipped_and_counted() {
        let mut conn = fresh_conn();
        // The cascade would have removed this row; switch the key check off to
        // get one that survived.
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_instance(&conn, "orphan", "ghost", "", "Orphan", 100, false);
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_skipped_no_definition, 1);
        assert_eq!(stats.instances_skipped_continuation, 0);
        assert_eq!(stats.instances_as_clone_inserted, 0);
        assert_eq!(count_agents(&conn, "1 = 1"), 1);
    }

    #[test]
    fn an_orphaned_continuation_row_counts_as_a_continuation_not_an_orphan() {
        let mut conn = fresh_conn();
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        rich_instance(&conn, "cont", "ghost", "parent", "Cont", 100, false);
        // No definitions at all: the table exists but is empty, so the pass runs.
        let stats = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(stats.instances_skipped_continuation, 1);
        assert_eq!(stats.instances_skipped_no_definition, 0);
    }

    #[test]
    fn a_missing_instances_table_still_backfills_the_definitions() {
        let mut conn = fresh_conn();
        conn.execute_batch("DROP TABLE db_agent_instances;").unwrap();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_def(&conn, "usr", "Usr", 0, "tpl");
        let tmp = tempfile::tempdir().unwrap();
        let stats = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert_eq!(stats.templates_inserted, 1);
        assert_eq!(stats.user_defs_inserted, 1);
        assert_eq!(stats.instances_as_clone_inserted, 0);
        assert_eq!(count_agents(&conn, "1 = 1"), 2);
        assert!(tmp.path().join(CONSOLIDATE_MARKER).exists());
    }

    #[test]
    fn a_missing_definitions_table_returns_empty_stats_and_writes_no_marker() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap();
        conn.execute_batch("CREATE TABLE db_agent_instances (id TEXT PRIMARY KEY);").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let stats = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert_eq!(stats, ConsolidateStats::default());
        assert!(!tmp.path().join(CONSOLIDATE_MARKER).exists());
    }

    #[test]
    fn a_completed_run_writes_the_marker_even_with_nothing_to_move() {
        let mut conn = fresh_conn();
        let tmp = tempfile::tempdir().unwrap();
        let stats = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert_eq!(stats, ConsolidateStats::default());
        assert_eq!(std::fs::read(tmp.path().join(CONSOLIDATE_MARKER)).unwrap(), b"phase3a");
    }

    #[test]
    fn a_marker_that_cannot_be_written_does_not_fail_the_run() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no-such-dir");
        let stats = run_consolidate_migration(&mut conn, Some(&missing)).unwrap();
        assert_eq!(stats.templates_inserted, 1);
        assert_eq!(count_agents(&conn, "1 = 1"), 1);
        assert!(!missing.exists());
    }

    #[test]
    fn a_failure_part_way_rolls_everything_back_and_leaves_no_marker() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_instance(&conn, "i1", "tpl", "", "Inst", 100, false);
        // The template inserts fine; the clone for the instance is refused.
        conn.execute_batch(
            "CREATE TRIGGER refuse_clone BEFORE INSERT ON db_agents
             WHEN NEW.id = 'i1'
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(run_consolidate_migration(&mut conn, Some(tmp.path())).is_err());
        assert_eq!(count_agents(&conn, "1 = 1"), 0, "the template insert must roll back too");
        assert!(!tmp.path().join(CONSOLIDATE_MARKER).exists());

        // The next start retries from scratch.
        conn.execute_batch("DROP TRIGGER refuse_clone;").unwrap();
        let stats = run_consolidate_migration(&mut conn, Some(tmp.path())).unwrap();
        assert_eq!(stats.templates_inserted, 1);
        assert_eq!(stats.instances_as_clone_inserted, 1);
        assert_eq!(count_agents(&conn, "1 = 1"), 2);
        assert!(tmp.path().join(CONSOLIDATE_MARKER).exists());
    }

    #[test]
    fn running_twice_without_a_marker_replaces_rows_instead_of_failing() {
        let mut conn = fresh_conn();
        rich_def(&conn, "tpl", "Tpl", 1, "");
        rich_def(&conn, "usr", "Usr", 0, "tpl");
        rich_instance(&conn, "i1", "tpl", "", "Inst", 100, false);
        rich_instance(&conn, "i2", "usr", "", "Folded", 333, false);
        let first = run_consolidate_migration(&mut conn, None).unwrap();
        let rows_after_first: Vec<String> =
            ["tpl", "usr", "i1"].iter().map(|id| row_of(&conn, id)).collect();
        let second = run_consolidate_migration(&mut conn, None).unwrap();
        assert_eq!(first, second);
        let rows_after_second: Vec<String> =
            ["tpl", "usr", "i1"].iter().map(|id| row_of(&conn, id)).collect();
        assert_eq!(rows_after_first, rows_after_second);
        assert_eq!(count_agents(&conn, "1 = 1"), 3);
    }
}
