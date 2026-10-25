// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Persistent job storage: CRUD on `db_jobs`
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1), in the same store
//! as the `db_cron_jobs` it replaced: the shared store (`store.db`), which
//! is per channel under isolated auth. `db_cron_jobs` stays as it was for
//! older builds; `m0040_cron_jobs_to_db_jobs` copied its rows here, keeping
//! their ids. See also `SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md`.
//!
//! A row is a job definition (`backend::jobs::definition`) plus its runtime
//! state (fires, misses, limits). The cron trigger and the `message` action
//! keep the columns `db_cron_jobs` had (`expression`, `prompt`, `target`,
//! `target_uid`), so the slug migration and older readers see the same
//! shape; the other kinds' fields are in `action`.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::error::StoreError;
use super::store::Store;
use crate::backend::jobs::definition::{Policy, SecretRef, TemplateRef};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    /// Standard 5-field cron expression, read in `tz`. Empty for a job
    /// whose trigger isn't cron.
    pub expression: String,
    /// What a `message` job sends (its brief).
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
    /// was Unattributed; never guessed.
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
    /// delivered when the agent is back). The job's `policy.catch_up`; kept
    /// in its own column, which is the one read.
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
    /// The agent responsible for the job, whose leader its failures go to
    /// (§3.4). Empty: its creator ([`Self::owner_uid`]).
    #[serde(default)]
    pub owner: String,
    /// `cron` (also when empty), `every`, `once` or `manual`.
    #[serde(default)]
    pub trigger_kind: String,
    /// The cron trigger's IANA zone; empty is UTC.
    #[serde(default)]
    pub tz: String,
    #[serde(default)]
    pub every_secs: Option<i64>,
    #[serde(default)]
    pub once_at_ms: Option<i64>,
    /// `message` (also when empty), `task` or `spawn`.
    #[serde(default)]
    pub action_kind: String,
    /// A `task` or `spawn` action's fields; null for `message`, whose fields
    /// are `prompt`, `target` and `target_uid`.
    #[serde(default)]
    pub action: Value,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub template: Option<TemplateRef>,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<SecretRef>,
}

impl CronJob {
    /// The owner's UID: the definition's `owner`, else the creator's.
    pub fn owner_uid(&self) -> &str {
        match self.owner.trim() {
            "" => self.created_by_uid.trim(),
            owner => owner,
        }
    }
}

fn default_catch_up() -> String {
    "skip".to_string()
}

/// What both stores' ALTER loops run after their CREATE batch
/// (`run_shared_store_schema`, `run_identity_store_schema` in
/// `migrations.rs`), beside `db_cron_jobs`: first the columns that table
/// gained after it was created, then `db_jobs`, then any column `db_jobs`
/// gains later. None needs a schema version bump: an older build never reads
/// them, and a bump would lock older builds out (`check_schema_compat`).
/// "duplicate column" is the column already being there.
///
/// `db_cron_jobs.catch_up` defaults to `latest` so every job from before it
/// keeps the startup catch-up. `db_cron_jobs` is no longer written; older
/// builds still use it.
pub(super) const JOB_TABLES: [&str; 5] = [
    "ALTER TABLE db_cron_jobs ADD COLUMN catch_up TEXT NOT NULL DEFAULT 'latest'",
    // The miss record (`CronJob::missed_count`).
    "ALTER TABLE db_cron_jobs ADD COLUMN missed_count INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE db_cron_jobs ADD COLUMN last_missed_at INTEGER",
    "ALTER TABLE db_cron_jobs ADD COLUMN last_miss_reason TEXT NOT NULL DEFAULT ''",
    JOBS_SCHEMA,
];

/// `db_jobs` and its index. Columns added later go after it in
/// [`JOB_TABLES`], as `ALTER TABLE db_jobs ADD COLUMN …`.
const JOBS_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_jobs (
        id               TEXT PRIMARY KEY,
        job_version      INTEGER NOT NULL DEFAULT 1,
        name             TEXT NOT NULL,
        owner            TEXT NOT NULL DEFAULT '',
        created_by       TEXT NOT NULL DEFAULT '',
        created_by_uid   TEXT NOT NULL DEFAULT '',
        template         TEXT NOT NULL DEFAULT '',
        params           TEXT NOT NULL DEFAULT '{}',
        trigger_kind     TEXT NOT NULL DEFAULT 'cron',
        expression       TEXT NOT NULL DEFAULT '',
        tz               TEXT NOT NULL DEFAULT 'UTC',
        every_secs       INTEGER,
        once_at_ms       INTEGER,
        action_kind      TEXT NOT NULL DEFAULT 'message',
        prompt           TEXT NOT NULL DEFAULT '',
        target           TEXT NOT NULL DEFAULT '',
        target_uid       TEXT NOT NULL DEFAULT '',
        action           TEXT NOT NULL DEFAULT 'null',
        policy           TEXT NOT NULL DEFAULT '{}',
        outputs          TEXT NOT NULL DEFAULT '[]',
        secrets          TEXT NOT NULL DEFAULT '[]',
        enabled          INTEGER NOT NULL DEFAULT 1,
        last_fired       INTEGER,
        fire_count       INTEGER NOT NULL DEFAULT 0,
        max_fires        INTEGER,
        created_at       INTEGER NOT NULL DEFAULT 0,
        max_age_secs     INTEGER,
        catch_up         TEXT NOT NULL DEFAULT 'skip',
        missed_count     INTEGER NOT NULL DEFAULT 0,
        last_missed_at   INTEGER,
        last_miss_reason TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_jobs_enabled ON db_jobs(enabled);
";

const COLS: &str = "id, name, expression, prompt, target, created_by, enabled, last_fired, fire_count, max_fires, \
     created_at, max_age_secs, target_uid, created_by_uid, catch_up, missed_count, last_missed_at, last_miss_reason, \
     owner, trigger_kind, tz, every_secs, once_at_ms, action_kind, action, policy, template, params, outputs, secrets";

fn json_text<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
}

impl Store {
    pub fn cron_create(&self, job: &CronJob) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO db_jobs ({COLS}) VALUES \
                 (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30)"
            ),
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
                job.missed_count,
                job.last_missed_at,
                job.last_miss_reason,
                job.owner,
                job.trigger_kind,
                job.tz,
                job.every_secs,
                job.once_at_ms,
                job.action_kind,
                json_text(&job.action),
                json_text(&job.policy),
                job.template.as_ref().map(json_text).unwrap_or_default(),
                json_text(&job.params),
                json_text(&job.outputs),
                json_text(&job.secrets),
            ],
        )?;
        Ok(())
    }

    /// Replace a job's definition (an edit), leaving its fires, misses and
    /// creation alone. `Ok(false)` when there is no such job.
    pub fn cron_update_definition(&self, job: &CronJob) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE db_jobs SET name = ?2, expression = ?3, prompt = ?4, target = ?5, target_uid = ?6, enabled = ?7,
                catch_up = ?8, owner = ?9, trigger_kind = ?10, tz = ?11, every_secs = ?12, once_at_ms = ?13,
                action_kind = ?14, action = ?15, policy = ?16, template = ?17, params = ?18, outputs = ?19, secrets = ?20
             WHERE id = ?1",
            params![
                job.id,
                job.name,
                job.expression,
                job.prompt,
                job.target,
                job.target_uid,
                job.enabled as i64,
                job.catch_up,
                job.owner,
                job.trigger_kind,
                job.tz,
                job.every_secs,
                job.once_at_ms,
                job.action_kind,
                json_text(&job.action),
                json_text(&job.policy),
                job.template.as_ref().map(json_text).unwrap_or_default(),
                json_text(&job.params),
                json_text(&job.outputs),
                json_text(&job.secrets),
            ],
        )?;
        Ok(n > 0)
    }

    pub fn cron_list(&self) -> Result<Vec<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM db_jobs ORDER BY created_at ASC"))?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn cron_list_enabled(&self) -> Result<Vec<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM db_jobs WHERE enabled = 1 ORDER BY created_at ASC"))?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn cron_get(&self, id: &str) -> Result<Option<CronJob>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM db_jobs WHERE id = ?1"))?;
        let mut rows = stmt.query_map(params![id], map_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn cron_set_enabled(&self, id: &str, enabled: bool) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_jobs SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )?;
        Ok(rows > 0)
    }

    pub fn cron_record_fire(&self, id: &str, now: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_jobs
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
            "UPDATE db_jobs
             SET missed_count = missed_count + 1, last_missed_at = ?1, last_miss_reason = ?2
             WHERE id = ?3",
            params![now, reason, id],
        )?;
        Ok(())
    }

    pub fn cron_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_jobs WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }
}

/// A JSON column, or the type's default when it doesn't parse (a row an
/// older or newer build wrote).
fn json_col<T: serde::de::DeserializeOwned + Default>(row: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<T> {
    let text: String = row.get(i)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronJob> {
    let template: String = row.get(26)?;
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
        owner:         row.get(18)?,
        trigger_kind:  row.get(19)?,
        tz:            row.get(20)?,
        every_secs:    row.get(21)?,
        once_at_ms:    row.get(22)?,
        action_kind:   row.get(23)?,
        action:        json_col(row, 24)?,
        policy:        json_col(row, 25)?,
        template:      serde_json::from_str(&template).ok(),
        params:        json_col(row, 27)?,
        outputs:       json_col(row, 28)?,
        secrets:       json_col(row, 29)?,
    })
}
