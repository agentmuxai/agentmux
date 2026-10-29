// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn layout(root: &Path, running: &str) -> Layout {
    Layout { versions_root: root.join("versions"), running: running.into(), channel: "chan".into(), portable_parents: vec![root.join("desk")] }
}

/// A version folder with some data and a log in it.
fn version(l: &Layout, v: &str) -> PathBuf {
    let dir = l.versions_root.join(v);
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::write(dir.join("data").join("objects.db"), vec![7u8; 1000]).unwrap();
    std::fs::write(dir.join("logs").join("srv.log"), b"log").unwrap();
    dir
}

/// A policy that finds a folder idle at once, so the guards under test are the only ones.
fn eager() -> Policy {
    Policy { idle: Duration::ZERO, ..Policy::default() }
}

fn nothing_running() -> InUse {
    InUse { procs: Some(Vec::new()) }
}

fn proc(pid: u32, name: &str, strings: &[&str]) -> ProcView {
    ProcView { pid, name: name.into(), strings: strings.iter().map(|s| s.to_string()).collect() }
}

fn names(dirs: &[PathBuf]) -> Vec<String> {
    let mut v: Vec<String> = dirs.iter().map(|d| d.file_name().unwrap().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn removable(p: &Plan) -> Vec<String> {
    names(&p.removable.iter().map(|c| c.dir.clone()).collect::<Vec<_>>())
}

fn kept_as(p: &Plan, v: &str) -> Option<Keep> {
    p.kept.iter().find(|(d, _)| d.file_name().is_some_and(|n| n == v)).map(|(_, k)| *k)
}

#[test]
fn the_running_version_a_newer_one_and_the_way_back_are_kept_and_older_ones_go() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.1", "0.60.2", "0.60.4", "0.60.5", "0.60.6", "0.60.7"] {
        version(&l, v);
    }
    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    assert_eq!(kept_as(&p, "0.60.6"), Some(Keep::Running), "the running version");
    assert_eq!(kept_as(&p, "0.60.7"), Some(Keep::NotOlder), "newer than ours: not ours to judge");
    assert_eq!(kept_as(&p, "0.60.5"), Some(Keep::Rollback), "the newest previous version");
    assert_eq!(removable(&p), ["0.60.1", "0.60.2", "0.60.4"]);
    assert_eq!(p.reclaimable_bytes(), 3 * 1003);
}

#[test]
fn versions_are_ordered_by_number_not_by_text() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.12");
    for v in ["0.60.2", "0.60.9", "0.60.10", "0.60.11", "0.60.12"] {
        version(&l, v);
    }
    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    assert_eq!(kept_as(&p, "0.60.11"), Some(Keep::Rollback));
    assert_eq!(removable(&p), ["0.60.10", "0.60.2", "0.60.9"]);
}

#[test]
fn keep_previous_sets_how_many_older_versions_stay() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.2", "0.60.3", "0.60.4", "0.60.5", "0.60.6"] {
        version(&l, v);
    }
    let two = Policy { keep_previous: 2, ..eager() };
    let p = plan(&l, &two, SystemTime::now(), &nothing_running());
    assert_eq!(removable(&p), ["0.60.2", "0.60.3"]);
    assert_eq!(kept_as(&p, "0.60.4"), Some(Keep::Rollback));
}

#[test]
fn a_running_version_is_kept_whatever_its_age_when_a_live_srv_holds_its_lock() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.2", "0.60.3", "0.60.5", "0.60.6"] {
        version(&l, v);
    }
    // An older AgentMux still running beside this one: its srv.lock names its pid.
    std::fs::write(l.versions_root.join("0.60.2/data/srv.lock"), "4242").unwrap();
    // A stale lock: its pid is now some other program.
    std::fs::write(l.versions_root.join("0.60.3/data/srv.lock"), "4343").unwrap();
    let live = InUse { procs: Some(vec![proc(4242, "agentmux-srv-0.60.2-windows.x64.exe", &[]), proc(4343, "notepad.exe", &[])]) };
    let p = plan(&l, &eager(), SystemTime::now(), &live);
    assert_eq!(kept_as(&p, "0.60.2"), Some(Keep::InUse));
    assert_eq!(removable(&p), ["0.60.3"]);
}

#[test]
fn a_process_naming_the_folder_keeps_it_by_command_working_folder_program_or_environment() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.9");
    for v in ["0.60.1", "0.60.2", "0.60.3", "0.60.4", "0.60.8", "0.60.9"] {
        version(&l, v);
    }
    let at = |v: &str, rest: &str| format!("{}/{rest}", l.versions_root.join(v).display());
    let backslashed = |v: &str, rest: &str| at(v, rest).replace('/', "\\").to_ascii_uppercase();
    let live = InUse {
        procs: Some(vec![
            proc(1, "agentmux-mcp.exe", &[&format!("AGENTMUX_DATA_DIR={}", at("0.60.1", "data"))]),
            proc(2, "node.exe", &[&backslashed("0.60.2", "logs\\x.js")]),
            proc(3, "tail.exe", &[&at("0.60.3", "logs/srv.log")]),
            // The folder of a longer version number is not a mention of 0.60.4.
            proc(4, "agentmux-srv.exe", &[&at("0.60.40", "data")]),
        ]),
    };
    let p = plan(&l, &eager(), SystemTime::now(), &live);
    assert_eq!(kept_as(&p, "0.60.1"), Some(Keep::InUse), "environment");
    assert_eq!(kept_as(&p, "0.60.2"), Some(Keep::InUse), "other slashes and case");
    assert_eq!(kept_as(&p, "0.60.3"), Some(Keep::InUse), "argument");
    assert_eq!(removable(&p), ["0.60.4"], "0.60.40 does not keep 0.60.4");
}

#[test]
fn when_the_processes_cannot_be_read_nothing_is_removable() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.1", "0.60.2", "0.60.6"] {
        version(&l, v);
    }
    let blind = InUse { procs: None };
    let p = plan(&l, &eager(), SystemTime::now(), &blind);
    assert!(p.removable.is_empty());
    assert_eq!(kept_as(&p, "0.60.1"), Some(Keep::ScanFailed));
    let report = remove(&p, &eager(), SystemTime::now(), &|| InUse { procs: None });
    assert!(report.removed.is_empty() && root.path().join("versions/0.60.1").is_dir());
}

#[test]
fn a_folder_written_to_lately_stays() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.1", "0.60.5", "0.60.6"] {
        version(&l, v);
    }
    let lazy = Policy { idle: Duration::from_secs(3600), ..Policy::default() };
    let p = plan(&l, &lazy, SystemTime::now(), &nothing_running());
    assert_eq!(kept_as(&p, "0.60.1"), Some(Keep::RecentlyActive));
    // The same folder, judged a day and a minute later, is idle.
    let later = SystemTime::now() + Duration::from_secs(25 * 3600);
    let p = plan(&l, &Policy::default(), later, &nothing_running());
    assert_eq!(removable(&p), ["0.60.1"]);
}

#[test]
fn names_that_are_not_plain_versions_and_symlinks_are_left_alone() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.1", "0.60.5", "0.60.6", "latest", "0.60.2+gabc123", "0.60", "0.60.3.1"] {
        version(&l, v);
    }
    #[cfg(unix)]
    {
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, l.versions_root.join("0.60.0")).unwrap();
    }
    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    assert_eq!(removable(&p), ["0.60.1"]);
    for v in ["latest", "0.60.2+gabc123", "0.60", "0.60.3.1"] {
        assert_eq!(kept_as(&p, v), Some(Keep::NotOurs), "{v}");
    }
    #[cfg(unix)]
    assert_eq!(kept_as(&p, "0.60.0"), Some(Keep::NotOurs), "a symlink is never followed");
}

#[test]
fn a_running_version_that_is_not_a_release_gives_nothing_to_judge_by() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6+glocal");
    version(&l, "0.60.1");
    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    assert!(p.removable.is_empty() && p.kept.is_empty());
}

#[test]
fn removal_deletes_the_folders_and_only_those_while_a_dry_run_deletes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.1", "0.60.2", "0.60.5", "0.60.6"] {
        version(&l, v);
    }
    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    assert_eq!(removable(&p), ["0.60.1", "0.60.2"]);
    assert!(l.versions_root.join("0.60.1").is_dir(), "planning removed nothing");

    let report = remove(&p, &eager(), SystemTime::now(), &nothing_running);
    assert_eq!(names(&report.removed.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>()), ["0.60.1", "0.60.2"]);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let left = names(&std::fs::read_dir(&l.versions_root).unwrap().flatten().map(|e| e.path()).collect::<Vec<_>>());
    assert_eq!(left, ["0.60.5", "0.60.6"], "the running and the way back are still there, and no renamed leftovers");
}

#[test]
fn removal_asks_again_and_skips_what_has_started_being_used_since_the_plan() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.1", "0.60.2", "0.60.5", "0.60.6"] {
        version(&l, v);
    }
    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    let started = format!("{}/data", l.versions_root.join("0.60.2").display());
    let report = remove(&p, &eager(), SystemTime::now(), &|| InUse { procs: Some(vec![proc(9, "agentmux-srv.exe", &[&started])]) });
    assert_eq!(report.removed.len(), 1);
    assert!(l.versions_root.join("0.60.2").is_dir(), "in use since the plan: kept");
    assert!(!l.versions_root.join("0.60.1").exists());
}

#[test]
fn a_folder_left_half_removed_is_finished_on_the_next_pass() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.5", "0.60.6"] {
        version(&l, v);
    }
    let leftover = version(&l, "0.60.1.removing-777");
    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    assert_eq!(p.removable.len(), 1);
    assert_eq!(p.removable[0].kind, Kind::Leftover);
    remove(&p, &eager(), SystemTime::now(), &nothing_running);
    assert!(!leftover.exists());
}

#[test]
fn run_with_dry_run_reports_and_removes_nothing_and_only_limits_a_removal_to_what_was_shown() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    for v in ["0.60.1", "0.60.2", "0.60.5", "0.60.6"] {
        version(&l, v);
    }
    let dry = run(&l, &eager(), true, None, SystemTime::now());
    assert!(dry.report.is_none());
    assert!(dry.scan_ok, "the real process scan works on this machine");
    assert_eq!(removable(&dry.plan), ["0.60.1", "0.60.2"]);
    assert!(l.versions_root.join("0.60.1").is_dir() && l.versions_root.join("0.60.2").is_dir());

    let shown = vec![l.versions_root.join("0.60.1").to_string_lossy().into_owned()];
    let real = run(&l, &eager(), false, Some(&shown), SystemTime::now());
    assert_eq!(real.report.unwrap().removed.len(), 1);
    assert!(!l.versions_root.join("0.60.1").exists());
    assert!(l.versions_root.join("0.60.2").is_dir(), "not in the list that was shown");
}

/// An unpacked portable of `v`, as the updater leaves it, recording `channel` when there is one.
fn portable(l: &Layout, v: &str, channel: Option<&str>) -> PathBuf {
    let dir = l.portable_parents[0].join(if cfg!(windows) { format!("agentmux-{v}-x64-portable") } else { format!("AgentMux-{v}") });
    crate::backend::update_download::fake_portable(&dir, v);
    if let Some(c) = channel {
        let at = if cfg!(windows) { dir.join("runtime") } else { dir.clone() };
        std::fs::write(at.join(agentmux_common::runtime_mode::RECORDED_CHANNEL_FILE), format!("{c}\n")).unwrap();
    }
    dir
}

#[test]
fn only_portables_the_updater_unpacked_for_this_channel_are_candidates_and_only_when_turned_on() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    version(&l, "0.60.6");
    let old = portable(&l, "0.60.2", Some("chan"));
    let rollback = portable(&l, "0.60.5", Some("chan"));
    let running = portable(&l, "0.60.6", Some("chan"));
    let other_channel = portable(&l, "0.60.1", Some("someone-elses"));
    let by_hand = portable(&l, "0.60.0", None);
    std::fs::create_dir_all(l.portable_parents[0].join("Documents")).unwrap();

    let p = plan(&l, &eager(), SystemTime::now(), &nothing_running());
    assert_eq!(p.removable.iter().map(|c| c.dir.clone()).collect::<Vec<_>>(), [old.clone()]);
    assert_eq!(p.removable[0].kind, Kind::Portable);
    assert_eq!(p.kept.iter().find(|(d, _)| *d == rollback).map(|k| k.1), Some(Keep::Rollback));
    assert_eq!(p.kept.iter().find(|(d, _)| *d == running).map(|k| k.1), Some(Keep::Running));
    assert_eq!(p.kept.iter().find(|(d, _)| *d == other_channel).map(|k| k.1), Some(Keep::NotOurs));
    assert_eq!(p.kept.iter().find(|(d, _)| *d == by_hand).map(|k| k.1), Some(Keep::NotOurs));
    assert!(p.kept.iter().all(|(d, _)| d.file_name().unwrap() != "Documents"), "unrelated folders are not even listed");

    let off = remove(&p, &eager(), SystemTime::now(), &nothing_running);
    assert!(off.removed.is_empty() && old.is_dir(), "portable cleanup is off by default");
    let on = Policy { portables: true, ..eager() };
    let report = remove(&p, &on, SystemTime::now(), &nothing_running);
    assert_eq!(report.removed.len(), 1);
    assert!(!old.exists() && rollback.is_dir() && running.is_dir() && other_channel.is_dir() && by_hand.is_dir());
}

#[test]
fn a_portable_whose_program_is_running_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let l = layout(root.path(), "0.60.6");
    version(&l, "0.60.6");
    portable(&l, "0.60.5", Some("chan"));
    let in_use = portable(&l, "0.60.2", Some("chan"));
    let exe = in_use.join("runtime").join("agentmux-srv.exe").to_string_lossy().into_owned();
    let live = InUse { procs: Some(vec![proc(5, "agentmux-srv.exe", &[&exe])]) };
    let p = plan(&l, &eager(), SystemTime::now(), &live);
    assert!(p.removable.is_empty());
    assert_eq!(p.kept.iter().find(|(d, _)| *d == in_use).map(|k| k.1), Some(Keep::InUse));
}

#[test]
fn the_policy_reads_the_settings_and_never_keeps_fewer_than_one_previous_version() {
    let extra: HashMap<String, serde_json::Value> = serde_json::from_str(
        r#"{"oldversions:clean": true, "oldversions:keep_previous": 0, "oldversions:idle_hours": 2, "oldversions:portables": true}"#,
    )
    .unwrap();
    let p = Policy::from_extra(&extra);
    assert!(p.clean && p.portables);
    assert_eq!(p.keep_previous, 1, "zero would delete the way back");
    assert_eq!(p.idle, Duration::from_secs(7200));
    assert_eq!(Policy::from_extra(&HashMap::new()), Policy::default());
    assert!(!Policy::default().clean, "reporting only until a host turns cleaning on");
    let bad: HashMap<String, serde_json::Value> = serde_json::from_str(r#"{"oldversions:keep_previous": "many", "oldversions:idle_hours": -3}"#).unwrap();
    assert_eq!(Policy::from_extra(&bad), Policy::default());
}
