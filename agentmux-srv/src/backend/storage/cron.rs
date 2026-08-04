// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Persistent cron job storage — CRUD methods on `db_cron_jobs` in the
//! global shared store. See `docs/specs/SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md`.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    /// Standard 5-field cron expression evaluated in UTC.
    pub expression: String,
    pub prompt: String,
    /// Target agent id for injection.
    pub target: String,
    pub created_by: String,
    pub enabled: bool,
    pub last_fired: Option<i64>,
    pub fire_count: i64,
    /// `None` = unlimited.
    pub max_fires: Option<i64>,
    pub created_at: i64,
    /// Hard expiry bound in seconds since `created_at`. `None` = no expiry
    /// (the default — existing jobs created before this field are
    /// unaffected). See
    /// docs/specs/SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md Phase 0.
    pub max_age_secs: Option<i64>,
}

impl Store {
    pub fn cron_create(&self, job: &CronJob) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_cron_jobs
                (id, name, expression, prompt, target, created_by, enabled,
                 last_fired, fire_count, max_fires, created_at, max_age_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                job.id,
                job.name,
                job.expression,
                job.prompt,
                job.target,
                job.created_by,
                job.enabled as i64,
                job.last_fired,
                job.fire_count,
                job.max_fires,
                job.created_at,
                job.max_age_secs,
            ],
        )?;
        Ok(())
    }

    pub fn cron_list(&self) -> Result<Vec<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, expression, prompt, target, created_by, enabled,
                    last_fired, fire_count, max_fires, created_at, max_age_secs
             FROM db_cron_jobs ORDER BY created_at ASC",
        )?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn cron_list_enabled(&self) -> Result<Vec<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, expression, prompt, target, created_by, enabled,
                    last_fired, fire_count, max_fires, created_at, max_age_secs
             FROM db_cron_jobs WHERE enabled = 1 ORDER BY created_at ASC",
        )?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn cron_get(&self, id: &str) -> Result<Option<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, expression, prompt, target, created_by, enabled,
                    last_fired, fire_count, max_fires, created_at, max_age_secs
             FROM db_cron_jobs WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], map_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn cron_set_enabled(&self, id: &str, enabled: bool) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_cron_jobs SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )?;
        Ok(rows > 0)
    }

    pub fn cron_record_fire(&self, id: &str, now: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_cron_jobs
             SET last_fired = ?1,
                 fire_count = fire_count + 1,
                 enabled    = CASE
                     WHEN max_fires IS NOT NULL AND fire_count + 1 >= max_fires THEN 0
                     ELSE enabled
                 END
             WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn cron_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_cron_jobs WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronJob> {
    Ok(CronJob {
        id:            row.get(0)?,
        name:          row.get(1)?,
        expression:    row.get(2)?,
        prompt:        row.get(3)?,
        target:        row.get(4)?,
        created_by:    row.get(5)?,
        enabled:       row.get::<_, i64>(6)? != 0,
        last_fired:    row.get(7)?,
        fire_count:    row.get(8)?,
        max_fires:     row.get(9)?,
        created_at:    row.get(10)?,
        max_age_secs:  row.get(11)?,
    })
}
