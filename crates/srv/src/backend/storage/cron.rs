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
    /// Identity M1b/M3 (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md
    /// §9.1): the UID (`db_agents.id`) of `target`, resolved once at the
    /// authoring boundary and carried in — spec §5.4, a job firing at 03:00
    /// must never resolve a name. Since M3 a job with a `target_uid` fires
    /// by it; empty = none was carried, and the job fires by `target`.
    #[serde(default)]
    pub target_uid: String,
    pub created_by: String,
    /// Identity M4c-1 (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md
    /// §6.5.9): the creator's UID, taken from the create request's `Caller`
    /// — its `X-Agent-Token` — never from a body field. Empty = the create
    /// was Unattributed; never guessed. Dual-written beside `created_by`;
    /// not read yet (M4c-3 carries it as the fire's attribution).
    #[serde(default)]
    pub created_by_uid: String,
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
    /// What an undelivered fire leaves (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
    /// §5.2): `skip` (a missed run) or `latest` (one coalesced pending fire,
    /// delivered when the agent is back). Jobs from before the column read
    /// `latest`, today's startup catch-up; new jobs default to `skip`.
    #[serde(default = "default_catch_up")]
    pub catch_up: String,
    /// Scheduled fires that reached no one at their time (the target wasn't
    /// running, or names no agent). They don't count in `fire_count` or
    /// toward `max_fires`. A `latest` job's pending fire delivered later
    /// counts then, as a fire.
    #[serde(default)]
    pub missed_count: i64,
    #[serde(default)]
    pub last_missed_at: Option<i64>,
    /// Why the last miss happened: `target_stopped`, `target_missing` or
    /// `failed: <error>` (`FireOutcome::miss_reason`). Empty = never missed.
    #[serde(default)]
    pub last_miss_reason: String,
}

fn default_catch_up() -> String {
    "skip".to_string()
}

/// Columns added to `db_cron_jobs` after its table, run by both stores'
/// ALTER loops (`migrations.rs`). A new column with a default needs no
/// schema version bump: an older build never reads it.
///
/// `catch_up` defaults to `latest` so every existing job keeps today's
/// startup catch-up; a create writes `skip` unless asked.
pub(super) const CRON_JOBS_COLUMNS: [&str; 4] = [
    "ALTER TABLE db_cron_jobs ADD COLUMN catch_up TEXT NOT NULL DEFAULT 'latest'",
    // The miss record (`CronJob::missed_count`).
    "ALTER TABLE db_cron_jobs ADD COLUMN missed_count INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE db_cron_jobs ADD COLUMN last_missed_at INTEGER",
    "ALTER TABLE db_cron_jobs ADD COLUMN last_miss_reason TEXT NOT NULL DEFAULT ''",
];

impl Store {
    pub fn cron_create(&self, job: &CronJob) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_cron_jobs
                (id, name, expression, prompt, target, created_by, enabled,
                 last_fired, fire_count, max_fires, created_at, max_age_secs,
                 target_uid, created_by_uid, catch_up)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
                job.target_uid,
                job.created_by_uid,
                job.catch_up,
            ],
        )?;
        Ok(())
    }

    pub fn cron_list(&self) -> Result<Vec<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, expression, prompt, target, created_by, enabled,
                    last_fired, fire_count, max_fires, created_at, max_age_secs,
                    target_uid, created_by_uid, catch_up, missed_count, last_missed_at, last_miss_reason
             FROM db_cron_jobs ORDER BY created_at ASC",
        )?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn cron_list_enabled(&self) -> Result<Vec<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, expression, prompt, target, created_by, enabled,
                    last_fired, fire_count, max_fires, created_at, max_age_secs,
                    target_uid, created_by_uid, catch_up, missed_count, last_missed_at, last_miss_reason
             FROM db_cron_jobs WHERE enabled = 1 ORDER BY created_at ASC",
        )?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn cron_get(&self, id: &str) -> Result<Option<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, expression, prompt, target, created_by, enabled,
                    last_fired, fire_count, max_fires, created_at, max_age_secs,
                    target_uid, created_by_uid, catch_up, missed_count, last_missed_at, last_miss_reason
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

    /// A scheduled fire that reached no one: count the miss and say why,
    /// leaving `last_fired`, `fire_count` and `max_fires` alone.
    pub fn cron_record_miss(&self, id: &str, now: i64, reason: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_cron_jobs
             SET missed_count = missed_count + 1, last_missed_at = ?1, last_miss_reason = ?2
             WHERE id = ?3",
            params![now, reason, id],
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
        target_uid:    row.get(12)?,
        created_by_uid: row.get(13)?,
        catch_up:      row.get(14)?,
        missed_count:  row.get(15)?,
        last_missed_at: row.get(16)?,
        last_miss_reason: row.get(17)?,
    })
}
