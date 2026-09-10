// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drop the `skill_id`/`mcp_id` foreign key from all four skill/MCP ref
//! tables (`db_agent_skills_ref`, `db_agent_mcp_ref`, `db_bundle_skills_ref`,
//! `db_bundle_mcp_ref`), leaving every other constraint verbatim (the
//! agent-level pair's `agent_id → db_agents` FK, every `PRIMARY KEY`, all
//! column types).
//!
//! Part A of Phase 2's application-layer redirect,
//! `SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.4 (Codex P1, PR #3182). Once
//! catalog reads/writes redirect to `identity_store`, this channel's LOCAL
//! `db_skills`/`db_mcp_servers` stop gaining new rows — but
//! `PRAGMA foreign_keys=ON` is set on every production connection
//! (`Store::configure_and_migrate`), and the four ref tables' declared FK
//! targeted exactly those local tables (`OBJECT_SCHEMA_VERSION` v10/v23,
//! pre-v34). Binding any catalog row created after the redirect ships would
//! hit a hard `FOREIGN KEY constraint failed`, not just read stale data —
//! this is a **write**-path bug the read-side redirect does nothing for.
//!
//! `db_bundle_skills_ref`/`db_bundle_mcp_ref` already solved the identical
//! problem for `db_bundles` (their own doc comment in `storage/migrations.rs`
//! explains why they carry no FK to it at all) — this migration applies the
//! same shape one layer over: existence moves to the application layer
//! (`Store::managed_bind_agent`/`managed_bind_bundle`'s new `catalog: &Store`
//! parameter, see `storage/managed.rs`).
//!
//! SQLite has no `ALTER TABLE ... DROP CONSTRAINT`, so each table is rebuilt:
//! rename, recreate with the trimmed FK set, copy every row verbatim (no
//! filtering needed — every existing row already satisfies the LOOSER new
//! schema, unlike `m0028`'s FK-retarget which could orphan rows), drop the
//! renamed original. `PRAGMA foreign_keys=OFF` for the swap, `foreign_keys=ON`
//! restored afterward, with a `PRAGMA foreign_key_check` before commit —
//! mirrors `m0028_agent_child_tables_repoint_fk`'s rebuild-and-swap exactly.
//!
//! Tolerates a table that already lacks the catalog FK (a fresh install:
//! `run_object_schema` creates all four tables with the trimmed FK set
//! directly as of `OBJECT_SCHEMA_VERSION` v34, so this migration has nothing
//! to do there) and a channel store that does not exist yet. Idempotent.

use rusqlite::Connection;

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0032DropCatalogFkFromRefTables;

/// One ref table this migration may need to rebuild.
struct TableSpec {
    name: &'static str,
    /// The ref column that used to FK to a catalog table (`skill_id` or
    /// `mcp_id`) — checked via `PRAGMA foreign_key_list` to decide whether a
    /// rebuild is still needed.
    ref_col: &'static str,
    /// The catalog table the ref column used to point at, for the same check.
    catalog_table: &'static str,
    create_sql: &'static str,
}

const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "db_agent_skills_ref",
        ref_col: "skill_id",
        catalog_table: "db_skills",
        create_sql: "CREATE TABLE db_agent_skills_ref (
            agent_id TEXT NOT NULL,
            skill_id TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_id),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        )",
    },
    TableSpec {
        name: "db_agent_mcp_ref",
        ref_col: "mcp_id",
        catalog_table: "db_mcp_servers",
        create_sql: "CREATE TABLE db_agent_mcp_ref (
            agent_id TEXT NOT NULL,
            mcp_id   TEXT NOT NULL,
            PRIMARY KEY (agent_id, mcp_id),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        )",
    },
    TableSpec {
        name: "db_bundle_skills_ref",
        ref_col: "skill_id",
        catalog_table: "db_skills",
        create_sql: "CREATE TABLE db_bundle_skills_ref (
            bundle_id TEXT NOT NULL,
            skill_id  TEXT NOT NULL,
            PRIMARY KEY (bundle_id, skill_id)
        )",
    },
    TableSpec {
        name: "db_bundle_mcp_ref",
        ref_col: "mcp_id",
        catalog_table: "db_mcp_servers",
        create_sql: "CREATE TABLE db_bundle_mcp_ref (
            bundle_id TEXT NOT NULL,
            mcp_id    TEXT NOT NULL,
            PRIMARY KEY (bundle_id, mcp_id)
        )",
    },
];

/// True if `table` no longer has an FK from `ref_col` to `catalog_table` —
/// either this migration already rebuilt it, or it's a fresh install whose
/// base schema already declares the trimmed FK set. `Ok(true)` for a missing
/// table too: nothing to rebuild is the same as already done.
fn already_dropped(conn: &Connection, spec: &TableSpec) -> Result<bool, String> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [spec.name],
            |row| row.get(0),
        )
        .map_err(|e| format!("probe {}: {e}", spec.name))?;
    if !exists {
        return Ok(true);
    }
    let mut stmt = conn
        .prepare(&format!("PRAGMA foreign_key_list({})", spec.name))
        .map_err(|e| format!("foreign_key_list {}: {e}", spec.name))?;
    // Columns: id, seq, table, from, to, on_update, on_delete, match.
    let fks: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(3)?, row.get::<_, String>(2)?)))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("foreign_key_list {}: {e}", spec.name))?;
    Ok(!fks.iter().any(|(from, table)| from == spec.ref_col && table == spec.catalog_table))
}

/// Rebuild one table so its catalog-pointing FK is gone, preserving every
/// other constraint and every row verbatim.
fn rebuild_table(conn: &Connection, spec: &TableSpec) -> Result<(), String> {
    let old_name = format!("{}_pre_drop_catalog_fk", spec.name);
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| format!("{}: foreign_keys off: {e}", spec.name))?;
    let result: Result<(), String> = (|| {
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
        // No filtering: every row satisfies the new (looser) schema — this
        // rebuild only REMOVES a constraint, unlike m0028's FK-retarget
        // which could orphan a row against the NEW target.
        conn.execute(&format!("INSERT INTO {} ({columns}) SELECT {columns} FROM {old_name}", spec.name), [])
            .map_err(|e| format!("{}: copy: {e}", spec.name))?;
        conn.execute(&format!("DROP TABLE {old_name}"), [])
            .map_err(|e| format!("{}: drop old: {e}", spec.name))?;
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
        Ok(())
    })();
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("{}: foreign_keys on: {e}", spec.name))?;
    result
}

pub(super) fn drop_catalog_fks(conn: &Connection) -> Result<usize, String> {
    let mut rebuilt = 0usize;
    for spec in TABLES {
        if already_dropped(conn, spec)? {
            continue;
        }
        rebuild_table(conn, spec)?;
        rebuilt += 1;
    }
    Ok(rebuilt)
}

impl Migration for M0032DropCatalogFkFromRefTables {
    fn id(&self) -> &'static str {
        "0032_drop_catalog_fk_from_ref_tables"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Drop db_agent_skills_ref/db_agent_mcp_ref/db_bundle_skills_ref/db_bundle_mcp_ref's skill_id/mcp_id FK to this channel's local catalog tables"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        // Store::open lays down the base schema (fresh tables already carry
        // the trimmed FK set as of OBJECT_SCHEMA_VERSION v34) before the raw
        // connection below touches anything.
        drop(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("drop_catalog_fk_from_ref_tables: open wstore: {e}")))?,
        );
        let conn = Connection::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("drop_catalog_fk_from_ref_tables: open: {e}")))?;
        let rebuilt =
            drop_catalog_fks(&conn).map_err(|e| MigrationError(format!("drop_catalog_fk_from_ref_tables: {e}")))?;
        tracing::info!(rebuilt, "m0032_drop_catalog_fk_from_ref_tables: complete");
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let conn = match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(c)) => c,
            Ok(None) => return VerifyOutcome::Ok("no channel store on this data dir".into()),
            Err(e) => return VerifyOutcome::Error(e),
        };
        for spec in TABLES {
            match already_dropped(&conn, spec) {
                Ok(true) => {}
                Ok(false) => {
                    return VerifyOutcome::Mismatch(format!(
                        "{} still has a {} FK to {}",
                        spec.name, spec.ref_col, spec.catalog_table
                    ))
                }
                Err(e) => return VerifyOutcome::Error(e),
            }
        }
        VerifyOutcome::Ok("every ref table's catalog-pointing FK is gone".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(channel_path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-shared-store.db"),
            channel_store_path: channel_path.to_path_buf(),
        }
    }

    #[test]
    fn a_fresh_store_already_has_the_trimmed_fk_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        for spec in TABLES {
            assert!(already_dropped(&conn, spec).unwrap(), "{} should already lack the catalog FK", spec.name);
        }
        assert_eq!(drop_catalog_fks(&conn).unwrap(), 0, "nothing to rebuild on a fresh store");
    }

    /// Simulates a pre-v34 store: the four tables still declare the catalog
    /// FK. Rebuild must preserve every row and end up with no catalog FK.
    fn legacy_store() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=OFF;
             DROP TABLE db_agent_skills_ref;
             DROP TABLE db_agent_mcp_ref;
             DROP TABLE db_bundle_skills_ref;
             DROP TABLE db_bundle_mcp_ref;
             CREATE TABLE db_agent_skills_ref (
                agent_id TEXT NOT NULL, skill_id TEXT NOT NULL,
                PRIMARY KEY (agent_id, skill_id),
                FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE,
                FOREIGN KEY (skill_id) REFERENCES db_skills(id) ON DELETE CASCADE
             );
             CREATE TABLE db_agent_mcp_ref (
                agent_id TEXT NOT NULL, mcp_id TEXT NOT NULL,
                PRIMARY KEY (agent_id, mcp_id),
                FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE,
                FOREIGN KEY (mcp_id) REFERENCES db_mcp_servers(id) ON DELETE CASCADE
             );
             CREATE TABLE db_bundle_skills_ref (
                bundle_id TEXT NOT NULL, skill_id TEXT NOT NULL,
                PRIMARY KEY (bundle_id, skill_id),
                FOREIGN KEY (skill_id) REFERENCES db_skills(id) ON DELETE CASCADE
             );
             CREATE TABLE db_bundle_mcp_ref (
                bundle_id TEXT NOT NULL, mcp_id TEXT NOT NULL,
                PRIMARY KEY (bundle_id, mcp_id),
                FOREIGN KEY (mcp_id) REFERENCES db_mcp_servers(id) ON DELETE CASCADE
             );
             PRAGMA foreign_keys=ON;",
        )
        .unwrap();
        (dir, conn)
    }

    #[test]
    fn an_existing_store_with_real_ref_rows_survives_the_rebuild_with_every_row_intact() {
        let (_dir, conn) = legacy_store();
        conn.execute_batch(
            "INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude');
             INSERT INTO db_skills (id, name) VALUES ('skill-1', 'Greet');
             INSERT INTO db_mcp_servers (id, name) VALUES ('mcp-1', 'Server');
             INSERT INTO db_agent_skills_ref (agent_id, skill_id) VALUES ('agent-1', 'skill-1');
             INSERT INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES ('agent-1', 'mcp-1');
             INSERT INTO db_bundle_skills_ref (bundle_id, skill_id) VALUES ('bundle-1', 'skill-1');
             INSERT INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES ('bundle-1', 'mcp-1');",
        )
        .unwrap();

        assert_eq!(drop_catalog_fks(&conn).unwrap(), 4, "all four tables need a rebuild");

        for spec in TABLES {
            assert!(already_dropped(&conn, spec).unwrap(), "{} must have no catalog FK after rebuild", spec.name);
        }
        let counts: Vec<i64> = [
            "db_agent_skills_ref",
            "db_agent_mcp_ref",
            "db_bundle_skills_ref",
            "db_bundle_mcp_ref",
        ]
        .iter()
        .map(|t| conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap())
        .collect();
        assert_eq!(counts, vec![1, 1, 1, 1], "every row must survive the rebuild");

        // agent_id's FK to db_agents must still be enforced on the two
        // agent-level tables — only the catalog FK was dropped.
        let agent_fk_err = conn.execute(
            "INSERT INTO db_agent_skills_ref (agent_id, skill_id) VALUES ('no-such-agent', 'skill-1')",
            [],
        );
        assert!(agent_fk_err.is_err(), "agent_id FK to db_agents must still be enforced");
    }

    /// The whole point of Part A: binding a skill_id with NO local db_skills
    /// row (the post-redirect state, once catalog writes move to
    /// identity_store) must succeed once the FK is gone.
    #[test]
    fn binding_a_skill_id_with_no_local_catalog_row_succeeds_after_the_rebuild() {
        let (_dir, conn) = legacy_store();
        conn.execute("INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')", [])
            .unwrap();
        drop_catalog_fks(&conn).unwrap();

        // No db_skills/db_mcp_servers row exists locally for these ids at
        // all — simulating a catalog row that only ever lived in
        // identity_store post-redirect.
        conn.execute(
            "INSERT INTO db_agent_skills_ref (agent_id, skill_id) VALUES ('agent-1', 'identity-store-only-skill')",
            [],
        )
        .expect("binding a skill with no local catalog row must succeed once the FK is dropped");
        conn.execute(
            "INSERT INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES ('agent-1', 'identity-store-only-mcp')",
            [],
        )
        .expect("binding an mcp server with no local catalog row must succeed once the FK is dropped");
        conn.execute(
            "INSERT INTO db_bundle_skills_ref (bundle_id, skill_id) VALUES ('bundle-1', 'identity-store-only-skill')",
            [],
        )
        .expect("bundle-level skill bind with no local catalog row must succeed once the FK is dropped");
        conn.execute(
            "INSERT INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES ('bundle-1', 'identity-store-only-mcp')",
            [],
        )
        .expect("bundle-level mcp bind with no local catalog row must succeed once the FK is dropped");
    }

    #[test]
    fn a_non_existent_channel_store_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_for(&dir.path().join("does-not-exist.db"));
        assert!(M0032DropCatalogFkFromRefTables.up(&ctx).is_ok());
    }

    #[test]
    fn re_running_after_a_rebuild_is_idempotent() {
        let (_dir, conn) = legacy_store();
        conn.execute("INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')", [])
            .unwrap();
        assert_eq!(drop_catalog_fks(&conn).unwrap(), 4);
        assert_eq!(drop_catalog_fks(&conn).unwrap(), 0, "a second pass must find nothing left to rebuild");
    }
}
