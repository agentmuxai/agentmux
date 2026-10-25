// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The cron migration keeps ids and behaviour (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §8, P3).

use super::*;
use crate::backend::cron::schedule::JobSchedule;
use crate::backend::jobs::definition::{Action, JobDefinition, Trigger};

/// A shared store as a build before P3 left it: `db_cron_jobs` with two
/// jobs, one from before P1 (catch-up `latest` by the column's default) and
/// one created with `skip`.
fn shared_store(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("store.db");
    drop(Store::open_shared(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "INSERT INTO db_cron_jobs (id, name, expression, prompt, target, target_uid, created_by, created_by_uid,
                                   enabled, last_fired, fire_count, max_fires, created_at, max_age_secs)
         VALUES ('cron-old', 'nightly', '0 3 * * *', 'run the build', 'builder', 'uid-builder', 'lead', 'uid-lead',
                 1, 1700000000, 12, 30, 1690000000, NULL);
         INSERT INTO db_cron_jobs (id, name, expression, prompt, target, created_by, created_by_uid, enabled,
                                   created_at, max_age_secs, catch_up, missed_count, last_miss_reason)
         VALUES ('cron-new', 'standup', '0 9 * * 1-5', 'standup?', 'naki', 'lead', '', 0, 1700000500, 86400,
                 'skip', 2, 'target_stopped');",
    )
    .unwrap();
    path
}

#[test]
fn every_cron_job_is_copied_with_its_id_fields_and_counters() {
    let dir = tempfile::tempdir().unwrap();
    let path = shared_store(dir.path());
    assert_eq!(missing(&path).unwrap(), 2);
    assert_eq!(copy_cron_jobs(&path).unwrap(), 2);
    assert_eq!(missing(&path).unwrap(), 0);

    let store = Store::open_shared(&path).unwrap();
    let old = store.cron_get("cron-old").unwrap().expect("kept its id");
    assert_eq!((old.name.as_str(), old.expression.as_str(), old.prompt.as_str()), ("nightly", "0 3 * * *", "run the build"));
    assert_eq!((old.target.as_str(), old.target_uid.as_str()), ("builder", "uid-builder"));
    assert_eq!((old.created_by.as_str(), old.created_by_uid.as_str(), old.owner_uid()), ("lead", "uid-lead", "uid-lead"));
    assert_eq!((old.enabled, old.last_fired, old.fire_count, old.max_fires), (true, Some(1_700_000_000), 12, Some(30)));
    assert_eq!(old.created_at, 1_690_000_000);
    assert_eq!(old.catch_up, "latest", "a job from before P1 keeps its startup catch-up");
    assert_eq!((old.trigger_kind.as_str(), old.tz.as_str(), old.action_kind.as_str()), ("cron", "UTC", "message"));

    let new = store.cron_get("cron-new").unwrap().unwrap();
    assert_eq!(new.catch_up, "skip", "a later job keeps the policy it was created with");
    assert_eq!((new.enabled, new.max_age_secs, new.missed_count), (false, Some(86_400), 2));
    assert_eq!((new.target_uid.as_str(), new.last_miss_reason.as_str()), ("", "target_stopped"));
}

/// The copied job is the definition §5.1 describes, and it fires at the
/// same times it did: the expression read in UTC.
#[test]
fn a_copied_job_is_a_utc_cron_message_job_firing_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let path = shared_store(dir.path());
    copy_cron_jobs(&path).unwrap();
    let job = Store::open_shared(&path).unwrap().cron_get("cron-old").unwrap().unwrap();

    let def = JobDefinition::from_job(&job);
    assert_eq!(def.trigger, Trigger::Cron { cron: "0 3 * * *".into(), tz: "UTC".into() });
    assert_eq!(def.action, Action::Message { to: "uid-builder".into(), brief: "run the build".into() });
    assert_eq!((def.id.as_str(), def.owner.as_str()), ("cron-old", "uid-lead"));

    let before = crate::backend::cron::expr::parse("0 3 * * *").unwrap();
    let now = chrono::Utc::now();
    let want: Vec<i64> = before.after(&now).take(3).map(|t| t.timestamp_millis()).collect();
    let got: Vec<i64> = JobSchedule::of(&job).unwrap().after(now.timestamp_millis()).take(3).collect();
    assert_eq!(got, want);
}

/// A second run adds nothing, and doesn't undo a change made since.
#[test]
fn a_rerun_adds_nothing_and_keeps_later_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = shared_store(dir.path());
    copy_cron_jobs(&path).unwrap();
    Store::open_shared(&path).unwrap().cron_set_enabled("cron-old", false).unwrap();

    assert_eq!(copy_cron_jobs(&path).unwrap(), 0);
    let store = Store::open_shared(&path).unwrap();
    assert_eq!(store.cron_list().unwrap().len(), 2);
    assert!(!store.cron_get("cron-old").unwrap().unwrap().enabled, "the pause made after the copy stays");
}

/// A store an old build last opened has no catch-up or miss columns: its
/// jobs come across with `latest` and no misses, and kept as they were.
#[test]
fn a_store_an_old_build_left_is_read_with_the_later_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE db_cron_jobs (id TEXT PRIMARY KEY, name TEXT NOT NULL, expression TEXT NOT NULL,
                prompt TEXT NOT NULL, target TEXT NOT NULL, created_by TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1, last_fired INTEGER, fire_count INTEGER NOT NULL DEFAULT 0,
                max_fires INTEGER, created_at INTEGER NOT NULL DEFAULT 0);
             INSERT INTO db_cron_jobs (id, name, expression, prompt, target) VALUES ('ancient', 'a', '*/5 * * * *', 'p', 't');",
        )
        .unwrap();
    assert_eq!(missing(&path).unwrap(), 1, "no db_jobs yet");
    assert_eq!(copy_cron_jobs(&path).unwrap(), 1);
    let job = Store::open_shared(&path).unwrap().cron_get("ancient").unwrap().unwrap();
    assert_eq!((job.catch_up.as_str(), job.missed_count, job.target_uid.as_str()), ("latest", 0, ""));
    assert_eq!(job.expression, "*/5 * * * *", "kept as it was: the 15-minute minimum is for new jobs only");
}

#[test]
fn a_store_without_cron_jobs_copies_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    drop(Store::open_shared(&path).unwrap());
    assert_eq!(missing(&path).unwrap(), 0);
    assert_eq!(copy_cron_jobs(&path).unwrap(), 0);
}

/// `db_cron_jobs` is no longer written: deleting a migrated job leaves its old
/// row behind, which is a healthy store. Only a migration that wrote nothing
/// fails the check.
#[test]
fn verify_tolerates_a_deleted_job_and_flags_a_copy_that_wrote_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = shared_store(dir.path());
    let ctx = MigrationContext {
        home: dir.path().to_path_buf(),
        data_dir: dir.path().to_path_buf(),
        shared_store_path: path.clone(),
        channel_store_path: dir.path().join("unused-channel.db"),
    };
    assert!(matches!(M0040CronJobsToDbJobs.verify(&ctx), VerifyOutcome::Mismatch(_)), "recorded, nothing copied");

    copy_cron_jobs(&path).unwrap();
    assert!(matches!(M0040CronJobsToDbJobs.verify(&ctx), VerifyOutcome::Ok(_)));

    Store::open_shared(&path).unwrap().cron_delete("cron-old").unwrap();
    assert!(matches!(M0040CronJobsToDbJobs.verify(&ctx), VerifyOutcome::Ok(_)), "a deleted job is not a failure");
}
