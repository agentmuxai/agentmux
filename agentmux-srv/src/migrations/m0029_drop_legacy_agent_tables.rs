// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drop `db_agent_definitions` and `db_agent_instances` — the two legacy
//! tables the agent-concept consolidation retires.
//!
//! Agent-concept consolidation, Phase 3e (final)
//! (`SPEC_AGENT_ARCHITECTURE_2026_05_27.md`). Both tables have had no live
//! reader or writer since PR 2 (instance flip, #3080) and PR 4 (definition
//! flip, #3092); PR 3 (#3088) re-pointed the six agent-child tables' FK off
//! `db_agent_definitions` onto `db_agents`, so nothing on disk still
//! depends on either legacy table by the time this runs. There is nothing
//! left to migrate — this is a straight `DROP TABLE IF EXISTS`, no data to
//! carry forward.
//!
//! `run_object_schema` no longer declares either table (see
//! `OBJECT_SCHEMA_VERSION`'s v32 doc comment) — that has to ship in the
//! SAME release as this migration, since `run_object_schema` runs on every
//! `Store::open()` immediately after migrations, and `CREATE TABLE IF NOT
//! EXISTS` would otherwise silently recreate what this migration just
//! dropped, empty, on the very next boot.
//!
//! Idempotent (`DROP TABLE IF EXISTS`) and channel-scoped, matching every
//! other migration in this series.

use rusqlite::Connection;

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0029DropLegacyAgentTables;

fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(|e| format!("probe {table}: {e}"))
}

pub(super) fn drop_legacy_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS db_agent_definitions;
         DROP TABLE IF EXISTS db_agent_instances;",
    )
    .map_err(|e| format!("drop legacy agent tables: {e}"))
}

impl Migration for M0029DropLegacyAgentTables {
    fn id(&self) -> &'static str { "0029_drop_legacy_agent_tables" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Drop db_agent_definitions and db_agent_instances — retired by the agent-concept consolidation"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        // Store::open runs the migration chain's own schema setup first
        // (including `adopt_legacy_table_names` — an install old enough to
        // still carry `db_forge_agents` gets it renamed to
        // `db_agent_definitions` here, before the drop below removes it;
        // see OBJECT_SCHEMA_VERSION's v32 doc comment for why that ordering
        // is safe rather than a special case).
        drop(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("drop_legacy_agent_tables: open wstore: {e}")))?,
        );
        let conn = Connection::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("drop_legacy_agent_tables: open: {e}")))?;
        drop_legacy_tables(&conn).map_err(|e| MigrationError(format!("drop_legacy_agent_tables: {e}")))?;
        tracing::info!("m0029_drop_legacy_agent_tables: complete");
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let conn = match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(c)) => c,
            Ok(None) => return VerifyOutcome::Ok("no channel store on this data dir".into()),
            Err(e) => return VerifyOutcome::Error(e),
        };
        for table in ["db_agent_definitions", "db_agent_instances"] {
            match table_exists(&conn, table) {
                Ok(false) => {}
                Ok(true) => return VerifyOutcome::Mismatch(format!("{table} still exists")),
                Err(e) => return VerifyOutcome::Error(e),
            }
        }
        VerifyOutcome::Ok("both legacy agent tables are gone".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_both_tables_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        // A fresh store no longer creates either table at all (v32) — stand
        // up the legacy shape by hand so this test exercises a real DROP,
        // not a no-op.
        conn.execute_batch(
            "CREATE TABLE db_agent_definitions (id TEXT PRIMARY KEY);
             CREATE TABLE db_agent_instances (id TEXT PRIMARY KEY);
             INSERT INTO db_agent_definitions (id) VALUES ('leftover');
             INSERT INTO db_agent_instances (id) VALUES ('leftover');",
        )
        .unwrap();

        drop_legacy_tables(&conn).unwrap();
        assert!(!table_exists(&conn, "db_agent_definitions").unwrap());
        assert!(!table_exists(&conn, "db_agent_instances").unwrap());

        // Idempotent — a second run against an already-dropped store must
        // not error.
        drop_legacy_tables(&conn).unwrap();
    }

    #[test]
    fn a_fresh_store_with_neither_table_is_a_no_op() {
        let conn = Connection::open_in_memory().unwrap();
        drop_legacy_tables(&conn).unwrap();
    }

    #[test]
    fn verify_confirms_absence_and_flags_a_survivor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        let ctx = MigrationContext {
            home: dir.path().to_path_buf(),
            data_dir: dir.path().join("data"),
            shared_store_path: dir.path().join("shared").join("store.db"),
            channel_store_path: path.clone(),
        };
        // No store on disk at all — nothing to verify, not a mismatch.
        assert!(matches!(M0029DropLegacyAgentTables.verify(&ctx), VerifyOutcome::Ok(_)));

        drop(Store::open(&path).unwrap());
        // A fresh store already has neither table (v32) — verify passes.
        assert!(matches!(M0029DropLegacyAgentTables.verify(&ctx), VerifyOutcome::Ok(_)));

        // A survivor is flagged, not silently accepted.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE db_agent_definitions (id TEXT PRIMARY KEY);").unwrap();
        match M0029DropLegacyAgentTables.verify(&ctx) {
            VerifyOutcome::Mismatch(detail) => assert!(detail.contains("db_agent_definitions")),
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }
}
