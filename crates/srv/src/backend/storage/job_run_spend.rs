// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What the run ledger knows about spending and sending, for the caps
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.3): a job's or an
//! account's spend since the start of the operator's day, the messages a job
//! delivered, and the cap skips a day already holds. A day's figures count
//! the runs that started in it.

use rusqlite::params;

use super::job_runs::{clip_summary, infra, reason, RunCost};
use super::store::Store;
use super::StoreError;

/// The run states that reached the agent: running, or done after running.
/// `scheduled` with no waiting reason is one being delivered right now.
const DELIVERED: &str = "(state IN ('running','finished') OR (state = 'scheduled' AND reason = '' AND ended_at_ms IS NULL))";

impl Store {
    /// The summed `cost_usd` of the runs on `account_id` that started at or
    /// after `since_ms`. A run still open has cost 0 until it ends, so its
    /// share is the caller's reservation.
    pub fn account_runs_cost_since(&self, account_id: &str, since_ms: i64) -> Result<f64, StoreError> {
        if account_id.is_empty() {
            return Ok(0.0);
        }
        let conn = self.conn().lock().unwrap();
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM db_job_runs
              WHERE account_id = ?1 AND collapsed_into = '' AND started_at_ms >= ?2",
            params![account_id, since_ms],
            |r| r.get(0),
        )?)
    }

    /// How many of the job's runs that started at or after `since_ms`
    /// reached their agent, or are being delivered now: the messages a
    /// `message` job sent.
    pub fn job_runs_delivered_since(&self, job_id: &str, since_ms: i64) -> Result<i64, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM db_job_runs
                  WHERE job_id = ?1 AND collapsed_into = '' AND started_at_ms >= ?2 AND {DELIVERED}"
            ),
            params![job_id, since_ms],
            |r| r.get(0),
        )?)
    }

    /// The cap reasons the job was skipped for since `since_ms`, newest
    /// first, each once.
    pub fn job_cap_skips_since(&self, job_id: &str, since_ms: i64) -> Result<Vec<String>, StoreError> {
        let list = reason::CAPS.iter().map(|r| format!("'{r}'")).collect::<Vec<_>>().join(",");
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT reason FROM db_job_runs
              WHERE job_id = ?1 AND state = 'skipped' AND started_at_ms >= ?2 AND reason IN ({list})
              GROUP BY reason ORDER BY MAX(started_at_ms) DESC"
        ))?;
        let rows = stmt.query_map(params![job_id, since_ms], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<String>, _>>()?)
    }

    /// Close a still-open run that reached its per-run cap: `finished`, infra
    /// `over_budget`, reason `run_cap`, with what it had spent. `false` when
    /// it was already closed.
    pub fn job_run_over_budget(&self, id: &str, cost: RunCost, summary: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_runs
                SET state = 'finished', infra = ?2, reason = ?3, ended_at_ms = ?4, summary = ?5,
                    cost_usd = ?6, cost_known = ?7, output_tokens = ?8, steps = ?9
              WHERE id = ?1 AND state IN ('scheduled','pending','running') AND ended_at_ms IS NULL",
            params![
                id,
                infra::OVER_BUDGET,
                reason::RUN_CAP,
                now_ms,
                clip_summary(summary),
                cost.cost_usd,
                cost.cost_known as i64,
                cost.output_tokens,
                cost.steps
            ],
        )?;
        Ok(n > 0)
    }

    /// Note the account an admitted run will use, before it is delivered, so
    /// the account's reservation counts it. `mark_running` writes the
    /// pane's account over it.
    pub fn job_run_set_account(&self, id: &str, account_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_runs SET account_id = ?2
              WHERE id = ?1 AND state IN ('scheduled','running') AND ended_at_ms IS NULL",
            params![id, account_id],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
#[path = "job_run_spend_tests.rs"]
mod tests;
