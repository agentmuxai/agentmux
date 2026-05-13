// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WaveStore extension methods for `db_workflow_definitions` +
//! `db_workflow_runs`. Lives in the workflows module because the table
//! schema is local to this feature; if Workflows ever ships separately
//! we can pull these methods out into a wstore module without changing
//! call sites.

use rusqlite::params;

use crate::backend::storage::error::StoreError;
use crate::backend::storage::wstore::WaveStore;

use super::types::{WorkflowDefinition, WorkflowRun};

pub trait WorkflowStore {
    fn workflow_list(&self) -> Result<Vec<WorkflowDefinition>, StoreError>;
    fn workflow_get(&self, id: &str) -> Result<Option<WorkflowDefinition>, StoreError>;
    fn workflow_upsert(&self, wf: &WorkflowDefinition) -> Result<(), StoreError>;
    fn workflow_delete(&self, id: &str) -> Result<bool, StoreError>;
    fn workflow_run_insert(&self, run: &WorkflowRun) -> Result<(), StoreError>;
    fn workflow_runs_for(
        &self,
        workflow_id: &str,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, StoreError>;
}

impl WorkflowStore for WaveStore {
    fn workflow_list(&self) -> Result<Vec<WorkflowDefinition>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, graph, viewport, created_at, updated_at
             FROM db_workflow_definitions
             ORDER BY updated_at DESC",
        )?;
        let iter = stmt.query_map([], |row| {
            let graph_s: String = row.get(3)?;
            let viewport_s: String = row.get(4)?;
            Ok(WorkflowDefinition {
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

    fn workflow_get(&self, id: &str) -> Result<Option<WorkflowDefinition>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, graph, viewport, created_at, updated_at
             FROM db_workflow_definitions WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            let graph_s: String = row.get(3)?;
            let viewport_s: String = row.get(4)?;
            Ok(WorkflowDefinition {
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

    fn workflow_upsert(&self, wf: &WorkflowDefinition) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        let graph = serde_json::to_string(&wf.graph).unwrap_or_else(|_| "{}".to_string());
        let viewport = serde_json::to_string(&wf.viewport).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "INSERT INTO db_workflow_definitions
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

    fn workflow_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let rows =
            conn.execute("DELETE FROM db_workflow_definitions WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    fn workflow_run_insert(&self, run: &WorkflowRun) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        // Plain INSERT — the run-history table is append-only by
        // design (one row per RunWorkflow invocation). Switching from
        // INSERT OR REPLACE means a duplicate run_id fails loudly
        // rather than silently overwriting a historical record (kimi
        // P1 on PR #755). run_id is a fresh UUID per invocation so
        // collisions in normal flow are vanishingly unlikely; the
        // loud failure is the point — anything that does collide is
        // a real bug worth surfacing.
        let block_states =
            serde_json::to_string(&run.block_states).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "INSERT INTO db_workflow_runs
                (id, workflow_id, status, started_at, ended_at, block_states, output, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                run.id,
                run.workflow_id,
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

    fn workflow_runs_for(
        &self,
        workflow_id: &str,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, status, started_at, ended_at, block_states, output, error
             FROM db_workflow_runs
             WHERE workflow_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;
        let iter = stmt.query_map(params![workflow_id, limit], |row| {
            let block_states_s: String = row.get(5)?;
            Ok(WorkflowRun {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
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
}
