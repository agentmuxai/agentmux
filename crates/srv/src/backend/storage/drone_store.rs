// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Store extension methods for `db_drone_definitions` + `db_drone_runs`.
//! They sit with the storage layer because the job store, the job templates
//! and the migrations all read these rows; `crate::drone::storage` still
//! names this module.

use rusqlite::params;

use crate::backend::storage::error::StoreError;
use crate::backend::storage::store::Store;

use super::drone_types::{DroneDefinition, DroneRun};

/// The definition upsert, shared by [`DroneStore::drone_upsert`] and the
/// versioned save in `versions.rs`, so both write the same row.
pub fn upsert_definition(conn: &rusqlite::Connection, wf: &DroneDefinition) -> Result<(), StoreError> {
    let graph = serde_json::to_string(&wf.graph).unwrap_or_else(|_| "{}".to_string());
    let viewport = serde_json::to_string(&wf.viewport).unwrap_or_else(|_| "{}".to_string());
    conn.execute(
        "INSERT INTO db_drone_definitions
            (id, name, description, graph, viewport, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            graph = excluded.graph,
            viewport = excluded.viewport,
            updated_at = excluded.updated_at",
        params![
            wf.id,
            wf.name,
            wf.description,
            graph,
            viewport,
            wf.created_at,
            wf.updated_at,
        ],
    )?;
    Ok(())
}

/// One stored checkpoint of a run: step 0 is `full`, every later step a `diff`
/// from the one before (`drone::checkpoint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRow {
    pub step: u32,
    pub kind: String,
    pub body: Vec<u8>,
}

pub trait DroneStore {
    fn drone_list(&self) -> Result<Vec<DroneDefinition>, StoreError>;
    fn drone_get(&self, id: &str) -> Result<Option<DroneDefinition>, StoreError>;
    fn drone_upsert(&self, wf: &DroneDefinition) -> Result<(), StoreError>;
    fn drone_delete(&self, id: &str) -> Result<bool, StoreError>;
    fn drone_run_insert(&self, run: &DroneRun) -> Result<(), StoreError>;
    /// Update an existing run row in place. Returns rows affected
    /// (0 if no row matched `run.id`). Used to flip a placeholder
    /// `running` row into its terminal state once the drain task
    /// completes, so the row exists from the moment the RPC returns.
    fn drone_run_update(&self, run: &DroneRun) -> Result<usize, StoreError>;
    fn drone_runs_for(
        &self,
        drone_id: &str,
        limit: i64,
    ) -> Result<Vec<DroneRun>, StoreError>;
    /// Delete every run of `drone_id` from this store. Runs live in the
    /// version's `objects.db` and definitions in the shared store, so no FK
    /// cascades a definition's delete to its runs (m0038); the delete handler
    /// calls this instead. Returns rows deleted.
    fn drone_runs_delete_for(&self, drone_id: &str) -> Result<usize, StoreError>;
    /// Delete one run by its id. Returns rows deleted.
    fn drone_run_delete(&self, run_id: &str) -> Result<usize, StoreError>;
    /// After a restart, give every run still `running` the status `status`
    /// and end it at `now_ms`: its drain task died with the old process.
    /// The runs in `except` are the ones that are being taken up again from
    /// their checkpoints, and stay as they are. Returns rows changed.
    fn drone_runs_mark_interrupted(&self, status: &str, now_ms: i64, except: &[String]) -> Result<usize, StoreError>;
    /// The runs still `running`, oldest first: after a restart, the ones the
    /// old process left (their checkpoints say whether they can go on).
    fn drone_runs_running(&self) -> Result<Vec<DroneRun>, StoreError>;
    /// Keep checkpoint `step` of `run_id` (`kind` is `full` or `diff`). Writing a step that
    /// is already there is an error: a checkpoint is never rewritten.
    fn drone_checkpoint_put(&self, run_id: &str, step: u32, kind: &str, body: &[u8], now_ms: i64) -> Result<(), StoreError>;
    /// The checkpoints of `run_id`, oldest step first.
    fn drone_checkpoints_for(&self, run_id: &str) -> Result<Vec<CheckpointRow>, StoreError>;
}

impl DroneStore for Store {
    fn drone_list(&self) -> Result<Vec<DroneDefinition>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, graph, viewport, created_at, updated_at
             FROM db_drone_definitions
             ORDER BY updated_at DESC",
        )?;
        let iter = stmt.query_map([], |row| {
            let graph_s: String = row.get(3)?;
            let viewport_s: String = row.get(4)?;
            Ok(DroneDefinition {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                graph: serde_json::from_str(&graph_s).unwrap_or_default(),
                viewport: serde_json::from_str(&viewport_s).unwrap_or_default(),
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    fn drone_get(&self, id: &str) -> Result<Option<DroneDefinition>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, graph, viewport, created_at, updated_at
             FROM db_drone_definitions WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            let graph_s: String = row.get(3)?;
            let viewport_s: String = row.get(4)?;
            Ok(DroneDefinition {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                graph: serde_json::from_str(&graph_s).unwrap_or_default(),
                viewport: serde_json::from_str(&viewport_s).unwrap_or_default(),
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        });
        match result {
            Ok(w) => Ok(Some(w)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn drone_upsert(&self, wf: &DroneDefinition) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        upsert_definition(&conn, wf)
    }

    fn drone_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let rows =
            conn.execute("DELETE FROM db_drone_definitions WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    fn drone_run_insert(&self, run: &DroneRun) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        // Plain INSERT — the run-history table is append-only by
        // design (one row per RunDrone invocation). Switching from
        // INSERT OR REPLACE means a duplicate run_id fails loudly
        // rather than silently overwriting a historical record (kimi
        // P1 on PR #755). run_id is a fresh UUID per invocation so
        // collisions in normal flow are vanishingly unlikely; the
        // loud failure is the point — anything that does collide is
        // a real bug worth surfacing.
        //
        // Status transitions (running → done/failed) happen via
        // `drone_run_update`, NOT a second INSERT. See codex P1
        // on PR #755 v0.33.842 (RPC timeout truncating the drain).
        let block_states =
            serde_json::to_string(&run.block_states).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "INSERT INTO db_drone_runs
                (id, drone_id, status, started_at, ended_at, block_states, output, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                run.id,
                run.drone_id,
                run.status,
                run.started_at,
                run.ended_at,
                block_states,
                run.output,
                run.error,
            ],
        )?;
        Ok(())
    }

    fn drone_run_update(&self, run: &DroneRun) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        let block_states =
            serde_json::to_string(&run.block_states).unwrap_or_else(|_| "{}".to_string());
        let rows = conn.execute(
            "UPDATE db_drone_runs SET
                status      = ?2,
                ended_at    = ?3,
                block_states= ?4,
                output      = ?5,
                error       = ?6
             WHERE id = ?1",
            params![
                run.id,
                run.status,
                run.ended_at,
                block_states,
                run.output,
                run.error,
            ],
        )?;
        Ok(rows)
    }

    fn drone_runs_for(
        &self,
        drone_id: &str,
        limit: i64,
    ) -> Result<Vec<DroneRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, drone_id, status, started_at, ended_at, block_states, output, error
             FROM db_drone_runs
             WHERE drone_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;
        let iter = stmt.query_map(params![drone_id, limit], |row| {
            let block_states_s: String = row.get(5)?;
            Ok(DroneRun {
                id: row.get(0)?,
                drone_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: row.get(4)?,
                block_states: serde_json::from_str(&block_states_s).unwrap_or_default(),
                output: row.get(6)?,
                error: row.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    fn drone_runs_delete_for(&self, drone_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        // A run's checkpoints go with it (no FK ties them, m0050), and first:
        // they are found through the run rows that are about to go.
        conn.execute(
            "DELETE FROM db_drone_checkpoints WHERE run_id IN (SELECT id FROM db_drone_runs WHERE drone_id = ?1)",
            params![drone_id],
        )?;
        Ok(conn.execute("DELETE FROM db_drone_runs WHERE drone_id = ?1", params![drone_id])?)
    }

    fn drone_run_delete(&self, run_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("DELETE FROM db_drone_checkpoints WHERE run_id = ?1", params![run_id])?;
        Ok(conn.execute("DELETE FROM db_drone_runs WHERE id = ?1", params![run_id])?)
    }

    fn drone_runs_mark_interrupted(&self, status: &str, now_ms: i64, except: &[String]) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        let skip = (3..3 + except.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&status, &now_ms];
        args.extend(except.iter().map(|id| id as &dyn rusqlite::ToSql));
        Ok(conn.execute(
            &format!(
                "UPDATE db_drone_runs SET status = ?1, ended_at = ?2,
                        error = CASE WHEN error = '' THEN 'interrupted: AgentMux restarted during this run' ELSE error END
                  WHERE status = 'running' AND id NOT IN ({skip})"
            ),
            args.as_slice(),
        )?)
    }

    fn drone_runs_running(&self) -> Result<Vec<DroneRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, drone_id, status, started_at, ended_at, block_states, output, error
             FROM db_drone_runs WHERE status = 'running' ORDER BY started_at",
        )?;
        let iter = stmt.query_map([], |row| {
            let block_states_s: String = row.get(5)?;
            Ok(DroneRun {
                id: row.get(0)?,
                drone_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: row.get(4)?,
                block_states: serde_json::from_str(&block_states_s).unwrap_or_default(),
                output: row.get(6)?,
                error: row.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    fn drone_checkpoint_put(&self, run_id: &str, step: u32, kind: &str, body: &[u8], now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_drone_checkpoints (run_id, step, kind, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, step, kind, body, now_ms],
        )?;
        Ok(())
    }

    fn drone_checkpoints_for(&self, run_id: &str) -> Result<Vec<CheckpointRow>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare("SELECT step, kind, body FROM db_drone_checkpoints WHERE run_id = ?1 ORDER BY step")?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok(CheckpointRow { step: row.get(0)?, kind: row.get(1)?, body: row.get(2)? })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}
