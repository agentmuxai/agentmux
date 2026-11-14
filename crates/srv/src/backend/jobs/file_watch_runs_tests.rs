// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! File watch: the folder and glob a job names, the debounce, names only, and a batch of changes starting one run
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2).

use super::*;
use crate::backend::cron::ActionOutcome;
use crate::backend::fs_watch::{FsWatchEvent, FsWatchEventKind, FsWatchPool};
use crate::backend::jobs::definition::trigger_kind;
use crate::backend::jobs::file_watch::{self, glob_matches, Batches, FileWatch};
use crate::backend::storage::cron::CronJob;

fn filter(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

/// A folder deep enough to be a project, as this platform writes one.
fn project(dir: &std::path::Path) -> String {
    let p = dir.join("work").join("src").join("proj");
    std::fs::create_dir_all(&p).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn a_glob_stays_in_a_folder_level_with_a_star_and_crosses_levels_with_two() {
    for (pattern, name, yes) in [
        ("**", "a.rs", true),
        ("**", "x/y/a.rs", true),
        ("*.rs", "a.rs", true),
        ("*.rs", "x/a.rs", false),
        ("**/*.rs", "a.rs", true),
        ("**/*.rs", "x/y/a.rs", true),
        ("**/*.rs", "x/y/a.rsx", false),
        ("src/**", "src/a/b.txt", true),
        ("src/**", "lib/a.txt", false),
        ("a?c.txt", "abc.txt", true),
        ("a?c.txt", "ac.txt", false),
        ("*.md", ".md", true),
        ("docs/*.md", "docs/a/b.md", false),
    ] {
        assert_eq!(glob_matches(pattern, name), yes, "{pattern} vs {name}");
    }
}

#[test]
fn a_filter_names_a_project_folder_and_only_the_keys_a_file_watch_reads() {
    let dir = tempfile::tempdir().unwrap();
    let ok = project(dir.path());
    assert!(file_watch::validate_filter(&filter(json!({ "path": ok }))).is_ok());
    assert!(file_watch::validate_filter(&filter(json!({ "path": ok, "glob": "**/*.rs", "change": "created" }))).is_ok());
    let missing = file_watch::validate_filter(&filter(json!({ "glob": "*.rs" }))).unwrap_err();
    assert!(missing.contains("\"path\""), "{missing}");
    for bad in ["relative/dir/here", "", "/", "/tmp", "/Users/someone", "C:\\"] {
        assert!(file_watch::validate_filter(&filter(json!({ "path": bad }))).is_err(), "{bad:?} is not a project folder");
    }
    let up = format!("{ok}/../../..");
    assert!(file_watch::validate_filter(&filter(json!({ "path": up }))).unwrap_err().contains("\"..\""), "no way out of the folder");
    let key = file_watch::validate_filter(&filter(json!({ "path": ok, "event": "x" }))).unwrap_err();
    assert!(key.contains("no filter key \"event\"") && key.contains("path, glob, change"), "{key}");
    for glob in [json!(""), json!("/etc/*"), json!("../x"), json!(3)] {
        assert!(file_watch::validate_filter(&filter(json!({ "path": ok, "glob": glob }))).is_err());
    }
    assert!(file_watch::validate_filter(&filter(json!({ "path": ok, "change": "renamed" }))).unwrap_err().contains("created, modified, removed"));
}

#[test]
fn a_batch_waits_for_quiet_and_never_longer_than_thirty_seconds_and_carries_names_only() {
    let mut b = Batches::default();
    b.note("j", "a.rs", "modified", 1_000);
    b.note("j", "src/b.rs", "created", 2_500);
    b.note("j", "a.rs", "modified", 2_600);
    assert!(b.take_due(4_000).is_empty(), "still changing: quiet only since 2.6 s");
    let due = b.take_due(4_700);
    assert_eq!(due.len(), 1, "quiet for two seconds");
    let input = due[0].1.input(std::path::Path::new("/proj"));
    assert_eq!(input, json!({ "root": "/proj", "paths": ["a.rs", "src/b.rs"], "first": "a.rs", "count": 2, "change": "mixed" }), "a file changed twice is one name");
    assert!(b.take_due(99_000).is_empty(), "and the batch is gone");

    // A folder that never goes quiet still runs, thirty seconds after its first change.
    let mut busy = Batches::default();
    for t in (0..=30_000).step_by(1_000) {
        busy.note("j", "log.txt", "modified", t);
        if t < 30_000 {
            assert!(busy.take_due(t).is_empty(), "{t}");
        }
    }
    assert_eq!(busy.take_due(30_000).len(), 1);

    // Many files are counted, and only twenty names go in the event, cleaned of control characters.
    let mut many = Batches::default();
    for i in 0..50 {
        many.note("j", &format!("f{i:02}.txt"), "created", 100);
    }
    many.note("j", "bad\u{1b}[31mname\n.txt", "created", 100);
    let (_, batch) = many.take_due(100_000).remove(0);
    let input = batch.input(std::path::Path::new("/proj"));
    assert_eq!((input["count"].clone(), input["change"].clone()), (json!(51), json!("created")));
    assert_eq!(input["paths"].as_array().unwrap().len(), 20);
    assert!(input["paths"].as_array().unwrap().iter().all(|p| !p.as_str().unwrap().chars().any(char::is_control)));
}

fn watch_job(rig: &Rig, id: &str, f: serde_json::Value, edit: impl FnOnce(&mut CronJob)) -> CronJob {
    let mut job = test_job(id, "", "", "", "");
    job.created_by = BY_OPERATOR.to_string();
    job.trigger_kind = trigger_kind::EVENT.to_string();
    job.trigger_event = json!({ "source": "file_watch", "filter": f });
    job.action_kind = "drone".to_string();
    job.action = json!({ "kind": "drone", "drone_id": "d-1" });
    edit(&mut job);
    rig.store.cron_create(&job).unwrap();
    job
}

fn changed(path: std::path::PathBuf, kind: FsWatchEventKind) -> FsWatchEvent {
    FsWatchEvent { path, kind }
}

/// Changes under a watched folder start one run for the job that wants them, with names only; the same folder is
/// watched once for two jobs, and letting go of the last job lets go of the folder.
#[tokio::test(flavor = "multi_thread")]
async fn changes_in_a_watched_folder_start_one_run_with_names_only() {
    let rig = rig();
    let dir = tempfile::tempdir().unwrap();
    let root = project(dir.path());
    let canonical = std::path::Path::new(&root).canonicalize().unwrap();
    let pool = FsWatchPool::new();
    let watch = FileWatch::new(pool.clone());
    let rs = watch_job(&rig, "j-rs", json!({ "path": root, "glob": "**/*.rs" }), |_| {});
    let created = watch_job(&rig, "j-created", json!({ "path": root, "change": "created" }), |_| {});
    let off = watch_job(&rig, "j-off", json!({ "path": root }), |j| j.enabled = false);
    let elsewhere = watch_job(&rig, "j-gone", json!({ "path": format!("{root}/not-here") }), |_| {});
    watch.reconcile(vec![rs.clone(), created.clone(), off, elsewhere]);
    assert_eq!(pool.health().active_watches, 1, "one folder, watched once for both jobs; a folder that is not there, or a job that is off, adds none");

    let at = 1_000_000;
    watch.on_event(&changed(canonical.join("src").join("a.rs"), FsWatchEventKind::Modified), at);
    watch.on_event(&changed(canonical.join("README.md"), FsWatchEventKind::Created), at + 10);
    watch.on_event(&changed(canonical.join(".git").join("index"), FsWatchEventKind::Modified), at + 20);
    watch.on_event(&changed(std::env::temp_dir().join("elsewhere.rs"), FsWatchEventKind::Modified), at + 30);
    watch.on_event(&changed(canonical.join("b.rs"), FsWatchEventKind::Other), at + 40);
    assert!(watch.due(at + 1_000).is_empty(), "not quiet yet");
    let mut due = watch.due(at + 3_000);
    due.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    let seen: Vec<(String, serde_json::Value)> = due.iter().map(|(j, e)| (j.id.clone(), e.input.clone())).collect();
    assert_eq!(
        seen,
        [
            ("j-created".to_string(), json!({ "root": canonical.to_string_lossy(), "paths": ["README.md"], "first": "README.md", "count": 1, "change": "created" })),
            ("j-rs".to_string(), json!({ "root": canonical.to_string_lossy(), "paths": ["src/a.rs"], "first": "src/a.rs", "count": 1, "change": "modified" })),
        ],
        "each job its own files: the glob and the kind of change decide; .git, another folder and an unnamed kind of change do not count"
    );

    // The batch starts one run through the scheduler, with the names as its input.
    let started: Arc<Mutex<Vec<String>>> = Arc::default();
    let (sink, led) = (started.clone(), rig.ledger.clone());
    rig.sched.install_actions(Arc::new(move |job, run_id| {
        sink.lock().unwrap().push(job.id.clone());
        led.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        Box::pin(async { ActionOutcome::Drone })
    }));
    let (job, event) = due.into_iter().find(|(j, _)| j.id == "j-rs").unwrap();
    assert_eq!(event.source, "file_watch");
    let fired = rig.sched.fire_event(&job, event.clone()).await;
    let run_id = fired.run_id.expect("a run");
    assert_eq!(*started.lock().unwrap(), ["j-rs"]);
    assert_eq!(rig.ledger.event_input(&run_id), Some(event.input), "the run reads the names");
    assert!(watch.due(at + 60_000).is_empty(), "the batch was taken");

    // Jobs gone, folders let go.
    watch.reconcile(vec![]);
    assert_eq!(pool.health().active_watches, 0);
}
