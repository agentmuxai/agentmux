// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Re-point six agent-child tables' `agent_id` foreign key from
//! `db_agent_definitions(id)` to `db_agents(id)`.
//!
//! Agent-concept consolidation, Phase 3c
//! (`SPEC_AGENT_ARCHITECTURE_2026_05_27.md`). `db_agent_content`,
//! `db_agent_skills`, `db_agent_history`, `db_agent_identity_links`,
//! `db_agent_skills_ref`, and `db_agent_mcp_ref` all carry an `agent_id`
//! that has, until now, only ever been a `db_agent_definitions` id — so a
//! template-launched agent (a `db_agents` row with no definition row of its
//! own) could not hold any of these directly. That FK is the reason
//! `listrecentsessions` (PR 2, #3080) had to fall back to the launch's
//! TEMPLATE for identity-link lookups instead of the agent's own row.
//!
//! Every existing `db_agent_definitions` row already has a `db_agents`
//! mirror at the SAME id (the dual-write PR 1 established and PR 2 kept
//! current, backstopped by the gap-repair this migration also runs), so
//! re-pointing the FK does not orphan a single existing row: every
//! `agent_id` value already in these tables satisfies the new target.
//! What it unlocks is new rows keyed by a launch id that has no
//! `db_agent_definitions` counterpart at all.
//!
//! SQLite has no `ALTER TABLE ... DROP CONSTRAINT` / redefine-FK — changing
//! a foreign key's target means rebuilding the table: rename, recreate with
//! the new FK, copy, drop the renamed original, recreate any secondary
//! indexes. Wrapped in one transaction per table with `foreign_keys=OFF`
//! for the swap (SQLite defers FK enforcement of the NEW schema to
//! `PRAGMA foreign_key_check`, which this migration runs before committing
//! rather than trusting each swap silently).
//!
//! Tolerates a table whose FK already targets `db_agents` (a fresh install:
//! `run_object_schema` creates the six tables with the new FK directly, so
//! this migration has nothing to do there) and a store where a table is
//! missing entirely (Phase 3c-or-later: nothing to repoint). Idempotent.

use rusqlite::Connection;

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0028AgentChildTablesRepointFk;

/// One table this migration may need to rebuild.
struct TableSpec {
    name: &'static str,
    create_sql: &'static str,
    index_sql: &'static [&'static str],
}

const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "db_agent_content",
        create_sql: "CREATE TABLE db_agent_content (
            agent_id     TEXT NOT NULL,
            content_type TEXT NOT NULL,
            content      TEXT NOT NULL DEFAULT '',
            updated_at   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, content_type),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        )",
        index_sql: &[],
    },
    TableSpec {
        name: "db_agent_skills",
        create_sql: "CREATE TABLE db_agent_skills (
            id          TEXT PRIMARY KEY,
            agent_id    TEXT NOT NULL,
            name        TEXT NOT NULL,
            trigger     TEXT NOT NULL DEFAULT '',
            skill_type  TEXT NOT NULL DEFAULT 'prompt',
            description TEXT NOT NULL DEFAULT '',
            content     TEXT NOT NULL DEFAULT '',
            created_at  INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        )",
        index_sql: &[],
    },
    TableSpec {
        name: "db_agent_history",
        create_sql: "CREATE TABLE db_agent_history (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id     TEXT NOT NULL,
            session_date TEXT NOT NULL,
            entry        TEXT NOT NULL,
            timestamp    INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        )",
        index_sql: &["CREATE INDEX idx_agent_history_agent_date ON db_agent_history(agent_id, session_date)"],
    },
    TableSpec {
        name: "db_agent_identity_links",
        create_sql: "CREATE TABLE db_agent_identity_links (
            agent_id   TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider   TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider),
            FOREIGN KEY (agent_id)   REFERENCES db_agents(id) ON DELETE CASCADE,
            FOREIGN KEY (account_id) REFERENCES db_accounts(id) ON DELETE CASCADE
        )",
        index_sql: &["CREATE INDEX idx_agent_identity_links_account ON db_agent_identity_links(account_id)"],
    },
    TableSpec {
        name: "db_agent_skills_ref",
        create_sql: "CREATE TABLE db_agent_skills_ref (
            agent_id TEXT NOT NULL,
            skill_id TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_id),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE,
            FOREIGN KEY (skill_id) REFERENCES db_skills(id) ON DELETE CASCADE
        )",
        index_sql: &[],
    },
    TableSpec {
        name: "db_agent_mcp_ref",
        create_sql: "CREATE TABLE db_agent_mcp_ref (
            agent_id TEXT NOT NULL,
            mcp_id   TEXT NOT NULL,
            PRIMARY KEY (agent_id, mcp_id),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE,
            FOREIGN KEY (mcp_id)   REFERENCES db_mcp_servers(id) ON DELETE CASCADE
        )",
        index_sql: &[],
    },
];

/// True if `table`'s `agent_id` FK already targets `db_agents` — either it
/// was rebuilt by an earlier run of this migration, or it's a fresh install
/// whose base schema already declares the new target. `Ok(true)` for a
/// missing table too: nothing to repoint is the same as already done.
fn already_targets_db_agents(conn: &Connection, table: &str) -> Result<bool, String> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(|e| format!("probe {table}: {e}"))?;
    if !exists {
        return Ok(true);
    }
    let mut stmt = conn
        .prepare(&format!("PRAGMA foreign_key_list({table})"))
        .map_err(|e| format!("foreign_key_list {table}: {e}"))?;
    let targets: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(2)) // column 2 = "table" (the referenced table)
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("foreign_key_list {table}: {e}"))?;
    // The agent_id FK is the one that used to say db_agent_definitions;
    // account_id/skill_id/mcp_id FKs on the same table are untouched by
    // this migration and must not affect the verdict.
    Ok(!targets.iter().any(|t| t == "db_agent_definitions"))
}

/// Rebuild one table so its `agent_id` FK targets `db_agents` instead of
/// `db_agent_definitions`. Rows whose `agent_id` has no `db_agents` row at
/// all are dropped (logged) rather than left to violate the new FK — the
/// gap-repair pass immediately before this loop is what makes that count
/// zero in the overwhelmingly common case.
fn rebuild_table(conn: &Connection, spec: &TableSpec) -> Result<usize, String> {
    let old_name = format!("{}_pre_repoint_fk", spec.name);
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| format!("{}: foreign_keys off: {e}", spec.name))?;
    let result: Result<usize, String> = (|| {
        conn.execute_batch("BEGIN;").map_err(|e| format!("{}: begin: {e}", spec.name))?;
        conn.execute(&format!("ALTER TABLE {} RENAME TO {old_name}", spec.name), [])
            .map_err(|e| format!("{}: rename: {e}", spec.name))?;
        conn.execute(spec.create_sql, [])
            .map_err(|e| format!("{}: recreate: {e}", spec.name))?;
        let columns: String = {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({old_name})"))
                .map_err(|e| format!("{}: table_info: {e}", spec.name))?;
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                .map_err(|e| format!("{}: table_info: {e}", spec.name))?;
            cols.join(", ")
        };
        let dropped = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {old_name} WHERE agent_id NOT IN (SELECT id FROM db_agents)"),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| format!("{}: count orphans: {e}", spec.name))?;
        conn.execute(
            &format!(
                "INSERT INTO {} ({columns}) SELECT {columns} FROM {old_name} WHERE agent_id IN (SELECT id FROM db_agents)",
                spec.name
            ),
            [],
        )
        .map_err(|e| format!("{}: copy: {e}", spec.name))?;
        conn.execute(&format!("DROP TABLE {old_name}"), [])
            .map_err(|e| format!("{}: drop old: {e}", spec.name))?;
        for idx in spec.index_sql {
            conn.execute(idx, []).map_err(|e| format!("{}: recreate index: {e}", spec.name))?;
        }
        // Confirm the rebuild is internally consistent before committing —
        // cheap (this table only) and turns a mistake in this migration
        // into a hard failure instead of a silently corrupt swap.
        let mut check_stmt = conn
            .prepare(&format!("PRAGMA foreign_key_check({})", spec.name))
            .map_err(|e| format!("{}: foreign_key_check: {e}", spec.name))?;
        let violations = check_stmt
            .query_map([], |_| Ok(()))
            .map_err(|e| format!("{}: foreign_key_check: {e}", spec.name))?
            .count();
        if violations > 0 {
            return Err(format!("{}: {violations} foreign key violation(s) after rebuild", spec.name));
        }
        conn.execute_batch("COMMIT;").map_err(|e| format!("{}: commit: {e}", spec.name))?;
        Ok(dropped as usize)
    })();
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("{}: foreign_keys on: {e}", spec.name))?;
    result
}

pub(super) fn repoint(conn: &mut Connection) -> Result<usize, String> {
    crate::backend::storage::agents_consolidate::repair_def_gaps(conn)
        .map_err(|e| format!("repair definition gaps: {e}"))?;
    let mut orphans_dropped = 0usize;
    for spec in TABLES {
        if already_targets_db_agents(conn, spec.name)? {
            continue;
        }
        let dropped = rebuild_table(conn, spec)?;
        if dropped > 0 {
            tracing::warn!(
                table = spec.name,
                dropped,
                "m0028_agent_child_tables_repoint_fk: dropped row(s) whose agent_id matched no db_agents row"
            );
        }
        orphans_dropped += dropped;
    }
    Ok(orphans_dropped)
}

impl Migration for M0028AgentChildTablesRepointFk {
    fn id(&self) -> &'static str { "0028_agent_child_tables_repoint_fk" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Re-point db_agent_content/skills/history/identity_links/skills_ref/mcp_ref's agent_id FK from db_agent_definitions to db_agents"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        // Store::open lays down the base schema (fresh tables already carry
        // the new FK) before the raw connection below touches anything.
        drop(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("agent_child_tables_repoint_fk: open wstore: {e}")))?,
        );
        let mut conn = Connection::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("agent_child_tables_repoint_fk: open: {e}")))?;
        let dropped = repoint(&mut conn).map_err(|e| MigrationError(format!("agent_child_tables_repoint_fk: {e}")))?;
        tracing::info!(orphans_dropped = dropped, "m0028_agent_child_tables_repoint_fk: complete");
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let conn = match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(c)) => c,
            Ok(None) => return VerifyOutcome::Ok("no channel store on this data dir".into()),
            Err(e) => return VerifyOutcome::Error(e),
        };
        for spec in TABLES {
            match already_targets_db_agents(&conn, spec.name) {
                Ok(true) => {}
                Ok(false) => {
                    return VerifyOutcome::Mismatch(format!(
                        "{}'s agent_id FK still targets db_agent_definitions",
                        spec.name
                    ))
                }
                Err(e) => return VerifyOutcome::Error(e),
            }
        }
        VerifyOutcome::Ok("every agent-child table's agent_id FK targets db_agents".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six tables in their PRE-repoint shape (FK on
    /// `db_agent_definitions`), on top of a store whose base schema is the
    /// REAL current one (`Store::open`) — everything else (`db_agents`,
    /// `db_agent_definitions`, `db_accounts`, `db_skills`, `db_mcp_servers`)
    /// is exactly what production has, so `repair_def_gaps`'s full-column
    /// read/write works unmodified. Only the six tables under test are
    /// dropped and recreated with the OLD FK to simulate a pre-migration
    /// store.
    fn legacy_store() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=OFF;
             DROP TABLE db_agent_content;
             DROP TABLE db_agent_skills;
             DROP TABLE db_agent_history;
             DROP TABLE db_agent_identity_links;
             DROP TABLE db_agent_skills_ref;
             DROP TABLE db_agent_mcp_ref;

             CREATE TABLE db_agent_content (
                agent_id TEXT NOT NULL, content_type TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (agent_id, content_type),
                FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
             );
             CREATE TABLE db_agent_skills (
                id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, name TEXT NOT NULL,
                trigger TEXT NOT NULL DEFAULT '', skill_type TEXT NOT NULL DEFAULT 'prompt',
                description TEXT NOT NULL DEFAULT '', content TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
             );
             CREATE TABLE db_agent_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT, agent_id TEXT NOT NULL,
                session_date TEXT NOT NULL, entry TEXT NOT NULL, timestamp INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
             );
             CREATE INDEX idx_agent_history_agent_date ON db_agent_history(agent_id, session_date);
             CREATE TABLE db_agent_identity_links (
                agent_id TEXT NOT NULL, account_id TEXT NOT NULL, provider TEXT NOT NULL,
                PRIMARY KEY (agent_id, provider),
                FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
                FOREIGN KEY (account_id) REFERENCES db_accounts(id) ON DELETE CASCADE
             );
             CREATE INDEX idx_agent_identity_links_account ON db_agent_identity_links(account_id);
             CREATE TABLE db_agent_skills_ref (
                agent_id TEXT NOT NULL, skill_id TEXT NOT NULL,
                PRIMARY KEY (agent_id, skill_id),
                FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
                FOREIGN KEY (skill_id) REFERENCES db_skills(id) ON DELETE CASCADE
             );
             CREATE TABLE db_agent_mcp_ref (
                agent_id TEXT NOT NULL, mcp_id TEXT NOT NULL,
                PRIMARY KEY (agent_id, mcp_id),
                FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
                FOREIGN KEY (mcp_id) REFERENCES db_mcp_servers(id) ON DELETE CASCADE
             );
             PRAGMA foreign_keys=ON;",
        )
        .unwrap();
        (dir, conn)
    }

    #[test]
    fn every_table_ends_up_targeting_db_agents_and_data_survives() {
        let (_dir, conn) = legacy_store();
        conn.execute_batch(
            "INSERT INTO db_agent_definitions (
                id, slug, name, icon, provider, description, working_directory, shell,
                provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
                created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
                parent_id, branch_label, updated_at, user_hidden
             ) VALUES ('def-a', 'def-a', 'Def A', '', 'claude', '', '', '', '', 0, 0, 0,
                       1, 'standalone', '', '', 0, '', '', '', 1, 0);
             INSERT INTO db_agents (id, is_template, name, provider, created_at, updated_at, is_seeded)
                VALUES ('def-a', 0, 'Def A', 'claude', 1, 1, 0);
             INSERT INTO db_accounts (id, name, provider, kind, secret_ref) VALUES ('acct-1', 'Acct', 'claude', 'oauth', 'ref');
             INSERT INTO db_skills (id, name) VALUES ('sk-1', 'Greet');
             INSERT INTO db_mcp_servers (id, name) VALUES ('mcp-1', 'Server');
             INSERT INTO db_agent_content (agent_id, content_type, content) VALUES ('def-a', 'agentmd', 'be helpful');
             INSERT INTO db_agent_skills (id, agent_id, name) VALUES ('s1', 'def-a', 'greet');
             INSERT INTO db_agent_history (agent_id, session_date, entry) VALUES ('def-a', '2026-09-07', 'did a thing');
             INSERT INTO db_agent_identity_links (agent_id, account_id, provider) VALUES ('def-a', 'acct-1', 'claude');
             INSERT INTO db_agent_skills_ref (agent_id, skill_id) VALUES ('def-a', 'sk-1');
             INSERT INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES ('def-a', 'mcp-1');",
        )
        .unwrap();

        let mut conn = conn;
        assert_eq!(repoint(&mut conn).unwrap(), 0, "no orphans in this fixture");

        for spec in TABLES {
            assert!(
                already_targets_db_agents(&conn, spec.name).unwrap(),
                "{} should target db_agents after repoint",
                spec.name
            );
        }
        let content: String = conn
            .query_row("SELECT content FROM db_agent_content WHERE agent_id = 'def-a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content, "be helpful", "data must survive the rebuild");
        let link_account: String = conn
            .query_row("SELECT account_id FROM db_agent_identity_links WHERE agent_id = 'def-a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(link_account, "acct-1");
        let skill_ref: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM db_agent_skills_ref WHERE agent_id = 'def-a' AND skill_id = 'sk-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(skill_ref, 1);
        // Deleting the db_agents row must still cascade to all six now that
        // the FK targets it directly.
        conn.execute("DELETE FROM db_agents WHERE id = 'def-a'", []).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM db_agent_content WHERE agent_id = 'def-a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0, "deleting the agent row must cascade through the new FK");
    }

    /// The actual unblock this migration exists for: a template-LAUNCH row
    /// (a `db_agents` id with no `db_agent_definitions` counterpart) can now
    /// hold an identity link of its own — impossible before the repoint.
    #[test]
    fn a_launch_only_agent_row_can_hold_its_own_identity_link_after_repoint() {
        let (_dir, conn) = legacy_store();
        conn.execute_batch(
            "INSERT INTO db_agents (id, is_template, name, provider, created_at, updated_at, is_seeded, parent_template_id)
                VALUES ('launch-1', 0, 'Launch', 'claude', 1, 1, 0, 'tpl-1');
             INSERT INTO db_accounts (id, name, provider, kind, secret_ref) VALUES ('acct-1', 'Acct', 'claude', 'oauth', 'ref');",
        )
        .unwrap();
        let mut conn = conn;
        repoint(&mut conn).unwrap();

        // Before the repoint this INSERT would fail: no db_agent_definitions
        // row named 'launch-1' could ever exist for a template launch.
        conn.execute(
            "INSERT INTO db_agent_identity_links (agent_id, account_id, provider) VALUES ('launch-1', 'acct-1', 'claude')",
            [],
        )
        .expect("a bare db_agents row must be able to hold its own identity link now");
    }

    /// A row whose `agent_id` matches neither a definition nor a `db_agents`
    /// row (a pre-existing inconsistency, not something this migration
    /// should be able to create) is dropped rather than left to violate the
    /// new FK, and the count is reported.
    #[test]
    fn a_row_with_no_matching_db_agents_id_is_dropped_and_counted() {
        let (_dir, conn) = legacy_store();
        // A content row whose definition (and any db_agents mirror) is
        // simply gone — an inconsistency that predates this migration and
        // could only exist today via a manual edit or a gap in FK
        // enforcement, so the insert itself needs FK checks off to set up.
        conn.execute_batch(
            "PRAGMA foreign_keys=OFF;
             INSERT INTO db_agent_content (agent_id, content_type, content) VALUES ('ghost', 'agentmd', 'orphaned');
             PRAGMA foreign_keys=ON;",
        )
        .unwrap();
        let mut conn = conn;
        assert_eq!(repoint(&mut conn).unwrap(), 1);
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM db_agent_content", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    /// A definition written after the Phase 3a marker but before its
    /// `db_agents` mirror existed (the same gap `m0025` repairs) must not
    /// lose its content to the orphan filter — `repoint()` runs gap-repair
    /// first specifically to prevent this.
    #[test]
    fn gap_repair_runs_first_so_an_unmirrored_definitions_content_survives() {
        let (_dir, conn) = legacy_store();
        conn.execute_batch(
            "INSERT INTO db_agent_definitions (
                id, slug, name, icon, provider, description, working_directory, shell,
                provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
                created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
                parent_id, branch_label, updated_at, user_hidden
             ) VALUES ('def-b', 'def-b', 'Unmirrored', '', 'claude', '', '', '', '', 0, 0, 0,
                       1, 'standalone', '', '', 0, '', '', '', 1, 0);
             INSERT INTO db_agent_content (agent_id, content_type, content) VALUES ('def-b', 'agentmd', 'still here');",
        )
        .unwrap();
        // 'def-b' has NO db_agents row — the exact gap m0025 exists for
        // (a definition written after the Phase 3a marker but before its
        // mirror caught up).

        let mut conn = conn;
        assert_eq!(repoint(&mut conn).unwrap(), 0, "gap-repair must have projected def-b before the orphan filter ran");
        let content: String = conn
            .query_row("SELECT content FROM db_agent_content WHERE agent_id = 'def-b'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content, "still here");
    }

    /// `db_agent_definitions` always exists at this point in the migration
    /// sequence (it's dropped only in Phase 3d), so this exercises the
    /// scenario that IS realistic: a fresh channel where `run_object_schema`
    /// has already created all six tables with the new FK, so there is
    /// nothing left for THIS migration to rebuild — the `TABLES` loop finds
    /// every one of them already pointing at `db_agents`.
    #[test]
    fn a_fresh_schema_with_no_agent_child_rows_at_all_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let mut conn = Connection::open(&path).unwrap();
        assert_eq!(repoint(&mut conn).unwrap(), 0);
        for spec in TABLES {
            assert!(already_targets_db_agents(&conn, spec.name).unwrap());
        }
    }
}
