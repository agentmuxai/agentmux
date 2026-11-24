// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The table behind who may fire a drone
//! (`docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` §6.5).
//!
//! `db_drone_access(drone_id, agent, kind, at, by)`: `owner` is the agent that created the drone, `granted`
//! an agent a person (or the owner) gave it to. An agent fires a drone it owns or was given at once; any other
//! drone goes through the sensitive-operation countdown. A drone that was there before this migration has no
//! row, so no agent owns it and every agent's fire of it takes the countdown. There is **no foreign key** to
//! `db_drone_definitions`, for the reason `m0049` has none (a definition and its runs can live in different stores).
//!
//! The table belongs beside the definitions, in the shared store, and the per-channel store is created too for the
//! fallback `id_store` of an install whose shared backfill has not run. Both are `IF NOT EXISTS`, so a second run
//! changes nothing.

use rusqlite::Connection;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0052DroneAccess;

/// The table and its primary key; the same DDL is in the store schemas for a fresh store.
const CREATE: &str = "CREATE TABLE IF NOT EXISTS db_drone_access (
    drone_id TEXT NOT NULL,
    agent    TEXT NOT NULL,
    kind     TEXT NOT NULL,
    set_at   INTEGER NOT NULL,
    set_by   TEXT NOT NULL,
    PRIMARY KEY (drone_id, agent, kind)
);";

fn create(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(CREATE).map_err(|e| format!("db_drone_access: {e}"))
}

impl Migration for M0052DroneAccess {
    fn id(&self) -> &'static str {
        "0052_drone_access"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Create db_drone_access (who owns a drone and which agents were given it, no definition FK)"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.shared_store_path.exists() {
            let conn = Connection::open(&ctx.shared_store_path)
                .map_err(|e| MigrationError(format!("drone_access: open shared store: {e}")))?;
            create(&conn).map_err(MigrationError)?;
        }
        if ctx.channel_store_path.exists() {
            let conn = Connection::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("drone_access: open channel store: {e}")))?;
            create(&conn).map_err(MigrationError)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "m0052_drone_access_tests.rs"]
mod tests;
