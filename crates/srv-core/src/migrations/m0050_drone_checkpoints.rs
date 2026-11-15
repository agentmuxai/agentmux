// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The table behind a run's checkpoints
//! (`docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md` §3.3).
//!
//! After every block the engine writes a checkpoint of the run: step 0 whole,
//! each later one as the diff from the one before (`drone::checkpoint`). They
//! are kept beside the run row in this version's store (`db_drone_runs` is in
//! the channel store, so a run and its checkpoints are never in two files), keyed
//! by `(run_id, step)`. There is **no foreign key** to `db_drone_runs`: the run's
//! row is written after its first checkpoint, and a delete of the run clears its
//! checkpoints in the same handler, as it does for the drone's runs (m0038).
//!
//! `IF NOT EXISTS`, so a second run changes nothing, and a channel store that does not
//! exist yet is left to create the table with the rest of its schema.

use rusqlite::Connection;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0050DroneCheckpoints;

/// The table and its primary key; the same DDL is in the store schema for a fresh store.
const CREATE: &str = "\
CREATE TABLE IF NOT EXISTS db_drone_checkpoints (
    run_id     TEXT NOT NULL,
    step       INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    body       BLOB NOT NULL,
    written_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, step)
);";

impl Migration for M0050DroneCheckpoints {
    fn id(&self) -> &'static str {
        "0050_drone_checkpoints"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Create db_drone_checkpoints (a Drone run's checkpoints, no run FK)"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.channel_store_path.exists() {
            let conn = Connection::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("drone_checkpoints: open channel store: {e}")))?;
            conn.execute_batch(CREATE).map_err(|e| MigrationError(format!("db_drone_checkpoints: {e}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "m0050_drone_checkpoints_tests.rs"]
mod tests;
