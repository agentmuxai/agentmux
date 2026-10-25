// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copy every cron job into `db_jobs` (`SPEC_JOBS_AND_TEMPLATES_2026_10_24.md`
//! §5.1): a job with a `cron` trigger in UTC and a `message` action, **keeping
//! its id**, so Swarm, Schedule and the run ledger (whose rows name the job by
//! id) carry on as before.
//!
//! Both tables are in the shared store (`ctx.shared_store_path`, per channel
//! under isolated auth): `db_jobs` sits beside the `db_cron_jobs` it replaces.
//! `db_cron_jobs` is only read: older builds keep scheduling from it. A job
//! both an older build and this one schedule fires once per slot, because the
//! run ledger's slot lease (`db_job_runs`, unique on job id and slot) is
//! shared and the id is the same.
//!
//! Each job keeps its catch-up policy: one created before P1 has `latest`
//! (the column's default), so it keeps its one startup catch-up fire; a later
//! one keeps what it was created with. Its owner is its creator's UID.
//!
//! A frozen copy of the row mapping on purpose (`migrations/mod.rs`): later
//! changes to `storage::cron` must not change what this writes. `INSERT OR
//! IGNORE` by id, so a re-run adds nothing twice and keeps later changes.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0040CronJobsToDbJobs;

impl Migration for M0040CronJobsToDbJobs {
    fn id(&self) -> &'static str {
        "0040_cron_jobs_to_db_jobs"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Global
    }

    fn description(&self) -> &'static str {
        "Copy cron jobs into db_jobs, keeping their ids"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.shared_store_path.exists() {
            return Ok(());
        }
        let copied = copy_cron_jobs(&ctx.shared_store_path).map_err(MigrationError)?;
        if copied > 0 {
            tracing::info!(copied, "cron_jobs_to_db_jobs: copied cron jobs into db_jobs");
        }
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        if !ctx.shared_store_path.exists() {
            return VerifyOutcome::Ok("no shared store".into());
        }
        // `db_cron_jobs` is no longer written, so a job deleted since leaves its
        // old row behind: a gap alone is not a failure. The migration wrote
        // nothing only when `db_jobs` is missing or empty while cron jobs exist.
        match missing(&ctx.shared_store_path) {
            Ok(0) => VerifyOutcome::Ok("every cron job is in db_jobs".into()),
            Ok(n) => match jobs_rows(&ctx.shared_store_path) {
                Ok(0) => VerifyOutcome::Mismatch(format!("{n} cron job(s), none in db_jobs")),
                Ok(rows) => VerifyOutcome::Ok(format!("db_jobs has {rows} job(s); {n} cron row(s) were deleted since")),
                Err(e) => VerifyOutcome::Error(e),
            },
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

/// Copy the shared store's cron jobs into its `db_jobs`. Opening it brings
/// both tables to this build's shape (a store an old build last opened gains
/// the later `db_cron_jobs` columns, with their defaults). Returns how many
/// rows were new.
pub(super) fn copy_cron_jobs(shared: &Path) -> Result<usize, String> {
    let store = Store::open_shared(shared).map_err(|e| format!("cron_jobs_to_db_jobs: open the shared store: {e}"))?;
    let conn = store.conn().lock().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO db_jobs
            (id, job_version, name, owner, created_by, created_by_uid, trigger_kind, expression, tz,
             action_kind, prompt, target, target_uid, action, policy, enabled, last_fired, fire_count,
             max_fires, created_at, max_age_secs, catch_up, missed_count, last_missed_at, last_miss_reason)
         SELECT id, 1, name, created_by_uid, created_by, created_by_uid, 'cron', expression, 'UTC',
                'message', prompt, target, target_uid, 'null', '{}', enabled, last_fired, fire_count,
                max_fires, created_at, max_age_secs, catch_up, missed_count, last_missed_at, last_miss_reason
         FROM db_cron_jobs WHERE id <> ''",
        [],
    )
    .map_err(|e| format!("cron_jobs_to_db_jobs: copy: {e}"))
}

/// How many cron jobs `db_jobs` lacks. Read-only.
fn missing(shared: &Path) -> Result<usize, String> {
    let conn = Connection::open_with_flags(shared, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| format!("open: {e}"))?;
    let has = |table: &str| -> Result<bool, String> {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)", [table], |r| r.get(0))
            .map_err(|e| format!("probe {table}: {e}"))
    };
    if !has("db_cron_jobs")? {
        return Ok(0);
    }
    let sql = if has("db_jobs")? {
        "SELECT COUNT(*) FROM db_cron_jobs c WHERE c.id <> '' AND NOT EXISTS (SELECT 1 FROM db_jobs j WHERE j.id = c.id)"
    } else {
        "SELECT COUNT(*) FROM db_cron_jobs WHERE id <> ''"
    };
    conn.query_row(sql, [], |r| r.get::<_, i64>(0)).map(|n| n as usize).map_err(|e| format!("count: {e}"))
}

/// How many jobs `db_jobs` holds (0 when the table is missing). Read-only.
fn jobs_rows(shared: &Path) -> Result<usize, String> {
    let conn = Connection::open_with_flags(shared, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| format!("open: {e}"))?;
    let has: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_jobs')", [], |r| r.get(0))
        .map_err(|e| format!("probe db_jobs: {e}"))?;
    if !has {
        return Ok(0);
    }
    conn.query_row("SELECT COUNT(*) FROM db_jobs", [], |r| r.get::<_, i64>(0)).map(|n| n as usize).map_err(|e| format!("count: {e}"))
}

#[cfg(test)]
#[path = "m0040_cron_jobs_to_db_jobs_tests.rs"]
mod tests;
