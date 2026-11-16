// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The table behind graph versions
//! (`docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md` §3.3, "Graph versions").
//!
//! Saving a drone records `db_drone_versions(drone_id, version, graph, hash, saved_at, saved_by)`,
//! keyed by `(drone_id, version)`; a job pins `latest` or a number. There is **no foreign key**
//! to `db_drone_definitions`: a drone's definition and its runs can live in different stores
//! (`m0038`), so a version of a drone that is only in another store must still save.
//!
//! The table belongs beside the definitions, in the shared store, and the per-channel store is
//! created too for the fallback `id_store` of an install whose shared backfill has not run
//! (`pick_id_store`). Both are `IF NOT EXISTS`, so a second run changes nothing.

use rusqlite::Connection;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0049DroneVersions;

/// The table and its primary key; the same DDL is in the store schemas for a fresh store.
const CREATE: &str = "\
CREATE TABLE IF NOT EXISTS db_drone_versions (
    drone_id TEXT NOT NULL,
    version  INTEGER NOT NULL,
    graph    TEXT NOT NULL,
    hash     TEXT NOT NULL,
    saved_at INTEGER NOT NULL,
    saved_by TEXT NOT NULL,
    PRIMARY KEY (drone_id, version)
);";

fn create(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(CREATE).map_err(|e| format!("db_drone_versions: {e}"))
}

impl Migration for M0049DroneVersions {
    fn id(&self) -> &'static str {
        "0049_drone_versions"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Create db_drone_versions (a numbered graph per drone, no definition FK)"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.shared_store_path.exists() {
            let conn = Connection::open(&ctx.shared_store_path)
                .map_err(|e| MigrationError(format!("drone_versions: open shared store: {e}")))?;
            create(&conn).map_err(MigrationError)?;
        }
        if ctx.channel_store_path.exists() {
            let conn = Connection::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("drone_versions: open channel store: {e}")))?;
            create(&conn).map_err(MigrationError)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "m0049_drone_versions_tests.rs"]
mod tests;
