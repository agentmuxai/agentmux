// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drop `db_drone_runs.drone_id`'s foreign key to this channel's local
//! `db_drone_definitions`.
//!
//! Drone definitions moved to the shared store (`id_store`,
//! `server/drone_handlers.rs`), but runs are still written to this version's
//! `objects.db`, whose run table declared `FOREIGN KEY (drone_id) REFERENCES
//! db_drone_definitions(id)`. With `PRAGMA foreign_keys=ON` on every
//! production connection, running a drone that exists only in the shared store
//! (every drone created since the move) failed with `FOREIGN KEY constraint
//! failed` before the run started. The same shape as m0032's catalog FKs, fixed
//! the same way: the table is rebuilt without the FK, with m0032's helpers, and
//! its two indexes are recreated (the rebuild drops them with the old table).
//! The `ON DELETE CASCADE` the FK gave moves to the delete handler, which
//! removes a deleted drone's runs from this version's store.
//!
//! Fresh stores already create the table without the FK, so this has nothing to
//! do there. Tolerates a channel store that does not exist yet. Idempotent.

use rusqlite::Connection;

use crate::backend::storage::store::Store;

use super::m0032_drop_catalog_fk_from_ref_tables::{already_dropped, rebuild_table, TableSpec};
use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0038DropDroneRunsDefinitionFk;

const DRONE_RUNS: TableSpec = TableSpec {
    name: "db_drone_runs",
    ref_col: "drone_id",
    catalog_table: "db_drone_definitions",
    create_sql: "CREATE TABLE db_drone_runs (
        id           TEXT PRIMARY KEY,
        drone_id     TEXT NOT NULL,
        status       TEXT NOT NULL DEFAULT 'running',
        started_at   INTEGER NOT NULL DEFAULT 0,
        ended_at     INTEGER NOT NULL DEFAULT 0,
        block_states TEXT NOT NULL DEFAULT '{}',
        output       TEXT NOT NULL DEFAULT '',
        error        TEXT NOT NULL DEFAULT ''
    )",
};

const DRONE_RUNS_INDEXES: &str = "
    CREATE INDEX IF NOT EXISTS idx_drone_runs_drone_started ON db_drone_runs(drone_id, started_at DESC);
    CREATE INDEX IF NOT EXISTS idx_drone_runs_status ON db_drone_runs(status);";

/// Rebuilds `db_drone_runs` without the definition FK if it still has it,
/// and makes sure both indexes exist either way: a run interrupted after the
/// rebuild committed but before the indexes were created finds the FK gone
/// on its next pass, and must still create them. Returns whether a rebuild
/// happened.
pub(super) fn drop_drone_runs_fk(conn: &Connection) -> Result<bool, String> {
    let rebuilt = if already_dropped(conn, &DRONE_RUNS)? {
        false
    } else {
        rebuild_table(conn, &DRONE_RUNS)?;
        true
    };
    let table_exists: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_drone_runs')", [], |r| r.get(0))
        .map_err(|e| format!("db_drone_runs: probe: {e}"))?;
    if table_exists {
        conn.execute_batch(DRONE_RUNS_INDEXES).map_err(|e| format!("db_drone_runs: recreate indexes: {e}"))?;
    }
    Ok(rebuilt)
}

impl Migration for M0038DropDroneRunsDefinitionFk {
    fn id(&self) -> &'static str {
        "0038_drop_drone_runs_definition_fk"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Drop db_drone_runs.drone_id's FK to the local db_drone_definitions (definitions live in the shared store)"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        // Store::open lays down the base schema first, as in m0032.
        drop(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("drop_drone_runs_definition_fk: open mstore: {e}")))?,
        );
        let conn = Connection::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("drop_drone_runs_definition_fk: open: {e}")))?;
        let rebuilt =
            drop_drone_runs_fk(&conn).map_err(|e| MigrationError(format!("drop_drone_runs_definition_fk: {e}")))?;
        tracing::info!(rebuilt, "m0038_drop_drone_runs_definition_fk: complete");
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let conn = match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(c)) => c,
            Ok(None) => return VerifyOutcome::Ok("no channel store on this data dir".into()),
            Err(e) => return VerifyOutcome::Error(e),
        };
        match already_dropped(&conn, &DRONE_RUNS) {
            Ok(true) => VerifyOutcome::Ok("db_drone_runs has no FK to db_drone_definitions".into()),
            Ok(false) => VerifyOutcome::Mismatch("db_drone_runs still has a drone_id FK to db_drone_definitions".into()),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

#[cfg(test)]
#[path = "m0038_drop_drone_runs_definition_fk_tests.rs"]
mod tests;
