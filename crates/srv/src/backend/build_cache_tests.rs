// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

fn mk(p: &Path) {
    std::fs::create_dir_all(p).unwrap();
}

/// A fake clone: Cargo.toml, a target with a debug profile, and `crates` incremental crate
/// directories named `<crate>-<hash>`, created oldest first.
fn clone_with_incremental(ws: &Path, name: &str, krate: &str, hashes: &[&str]) -> PathBuf {
    let clone = ws.join(name);
    mk(&clone.join("target").join("debug").join("deps"));
    std::fs::write(clone.join("Cargo.toml"), "[workspace]\n").unwrap();
    let inc = clone.join("target").join("debug").join("incremental");
    for h in hashes {
        let sess = inc.join(format!("{krate}-{h}")).join("s-aaa-0");
        mk(&sess);
        std::fs::write(sess.join("dep-graph.bin"), vec![0u8; 4096]).unwrap();
        std::thread::sleep(Duration::from_millis(30));
    }
    clone
}

fn in_days(d: u64) -> SystemTime {
    SystemTime::now() + Duration::from_secs(d * 86_400)
}

#[test]
fn the_policy_defaults_and_what_the_settings_change() {
    let d = Policy::default();
    assert!(d.enabled && d.orphans);
    assert_eq!((d.orphan_keep, d.orphan_idle, d.stale), (3, Duration::from_secs(86_400), Duration::from_secs(3 * 86_400)));
    assert_eq!((d.warn_free_bytes, d.warn_free_pct), (20 << 30, 10.0));
    assert_eq!(Policy::from_extra(&HashMap::new()), d);

    let mut extra = HashMap::new();
    extra.insert(KEY_ENABLED.into(), json!(false));
    extra.insert(KEY_ORPHAN_KEEP.into(), json!(0)); // never fewer than the live one
    extra.insert(KEY_ORPHAN_IDLE_HOURS.into(), json!(6));
    extra.insert(KEY_STALE_DAYS.into(), json!(0)); // off
    extra.insert(KEY_WARN_FREE_GB.into(), json!(50));
    extra.insert(KEY_WARN_FREE_PCT.into(), json!("lots")); // mistyped: default
    let p = Policy::from_extra(&extra);
    assert!(!p.enabled);
    assert_eq!((p.orphan_keep, p.orphan_idle, p.stale), (1, Duration::from_secs(6 * 3600), Duration::ZERO));
    assert_eq!((p.warn_free_bytes, p.warn_free_pct), (50 << 30, 10.0));

    // A value too large to be a duration keeps the default instead of panicking.
    let mut huge = HashMap::new();
    huge.insert(KEY_ORPHAN_IDLE_HOURS.into(), json!(1e300));
    huge.insert(KEY_STALE_DAYS.into(), json!(1e300));
    let p = Policy::from_extra(&huge);
    assert_eq!((p.orphan_idle, p.stale), (d.orphan_idle, d.stale));
}

#[test]
fn a_disk_is_low_by_bytes_or_by_percentage() {
    let p = Policy::default();
    let gb = 1u64 << 30;
    assert!(p.is_low(15 * gb, 1000 * gb), "under 20 GB");
    assert!(p.is_low(50 * gb, 1000 * gb), "under 10%");
    assert!(!p.is_low(150 * gb, 1000 * gb));
    assert!(!p.is_low(0, 0), "a disk that reports nothing is not a warning");
}

#[test]
fn clones_are_the_workspace_or_a_folder_in_it_with_cargo_and_a_target() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path();
    let a = clone_with_incremental(ws, "agentmux", "agentmux_srv", &["a"]);
    let b = clone_with_incremental(ws, "agentmux-d", "agentmux_srv", &["a"]);
    mk(&ws.join("scratch")); // no Cargo.toml
    mk(&ws.join("notes").join("target")); // a target but no Cargo.toml
    mk(&ws.join(".hidden").join("target"));
    std::fs::write(ws.join(".hidden").join("Cargo.toml"), "x").unwrap();
    assert_eq!(clones_in(ws), vec![a.clone(), b]);
    let inner = clone_with_incremental(&a, "sub", "x", &["a"]);
    // The workspace itself counts when it has the files.
    std::fs::write(a.join("Cargo.toml"), "[workspace]").unwrap();
    assert!(clones_in(&a).contains(&a) && clones_in(&a).contains(&inner));
}

#[test]
fn incremental_roots_find_profiles_and_cross_builds() {
    let t = tempfile::tempdir().unwrap();
    let target = t.path().join("target");
    mk(&target.join("debug").join("incremental"));
    mk(&target.join("release").join("deps"));
    mk(&target.join("x86_64-unknown-linux-musl").join("debug").join("incremental"));
    let roots = incremental_roots(&target);
    assert_eq!(roots.len(), 2);
    assert!(roots.iter().all(|r| r.ends_with("incremental")));
}

#[test]
fn orphans_are_all_but_the_newest_few_per_crate_and_only_when_idle() {
    let t = tempfile::tempdir().unwrap();
    let clone = clone_with_incremental(t.path(), "agentmux", "agentmux_srv", &["h1", "h2", "h3", "h4", "h5"]);
    let root = clone.join("target").join("debug").join("incremental");
    // Another crate with one directory: never an orphan.
    mk(&root.join("agentmux_common-zz").join("s-1-0"));
    let name = |p: &PathBuf| p.file_name().unwrap().to_string_lossy().into_owned();

    // Everything is fresh: nothing has been idle for a day.
    assert!(orphans_in(&root, 3, Duration::from_secs(86_400), SystemTime::now()).is_empty());
    // Two days on, the two oldest of five go; the three newest and the other crate stay.
    let gone = orphans_in(&root, 3, Duration::from_secs(86_400), in_days(2));
    assert_eq!(gone.iter().map(name).collect::<Vec<_>>(), ["agentmux_srv-h1", "agentmux_srv-h2"]);
    // Keeping one removes four.
    assert_eq!(orphans_in(&root, 1, Duration::from_secs(86_400), in_days(2)).len(), 4);
    // A directory written to again counts as the newest, whatever its name.
    std::fs::write(root.join("agentmux_srv-h1").join("s-aaa-0").join("dep-graph.bin"), vec![1u8; 10]).unwrap();
    let gone = orphans_in(&root, 3, Duration::from_secs(86_400), in_days(2));
    assert_eq!(gone.iter().map(name).collect::<Vec<_>>(), ["agentmux_srv-h2", "agentmux_srv-h3"]);
}

#[test]
fn a_task_done_clears_incremental_and_the_hourly_pass_prunes_orphans_and_logs_both() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("agent");
    let clone = clone_with_incremental(&ws, "agentmux", "agentmux_srv", &["h1", "h2", "h3", "h4"]);
    std::fs::write(clone.join("target").join("debug").join("deps").join("keep.rlib"), b"x").unwrap();
    let root = clone.join("target").join("debug").join("incremental");
    let log = t.path().join("logs").join("cleanup.log");
    let procs = |_: &Path| false;
    let policy = Policy::default();

    let r = run_pass(&ws, Why::Hourly, &policy, &procs, in_days(2), Some(&log));
    assert_eq!(r.removed.len(), 1, "four dirs, keep three: {r:?}");
    assert!(r.removed[0].bytes >= 4096 && r.removed[0].what == "orphan incremental");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);

    let off = Policy { orphans: false, ..policy.clone() };
    assert!(run_pass(&ws, Why::Hourly, &off, &procs, in_days(2), Some(&log)).removed.is_empty());
    assert!(run_pass(&ws, Why::Hourly, &Policy { enabled: false, ..policy.clone() }, &procs, in_days(2), Some(&log)).removed.is_empty());

    let r = run_pass(&ws, Why::TaskDone, &policy, &procs, SystemTime::now(), Some(&log));
    assert_eq!(r.removed.len(), 3, "all three that are left: {r:?}");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "incremental is empty");
    assert!(clone.join("target").join("debug").join("deps").join("keep.rlib").exists(), "the built dependencies stay");

    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().all(|l| l.len() == 6 && l[2] == ws.display().to_string()), "the log names the workspace: {text}");
    assert_eq!((lines[0][1], lines[0][3]), ("hourly", "orphan incremental"));
    assert_eq!((lines[1][1], lines[1][3]), ("task-done", "incremental"));
}

#[test]
fn a_clone_nobody_built_in_for_days_loses_its_target_when_a_task_ends_or_the_disk_is_low_not_hourly() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path().join("agent");
    let clone = clone_with_incremental(&ws, "agentmux-old", "agentmux_srv", &["h1"]);
    let procs = |_: &Path| false;
    let policy = Policy::default();
    let target = clone.join("target");

    assert!(run_pass(&ws, Why::TaskDone, &policy, &procs, in_days(1), None).removed.iter().all(|r| r.what != "stale target"), "built a day ago: kept");
    let hourly = run_pass(&ws, Why::Hourly, &policy, &procs, in_days(10), None);
    assert!(hourly.removed.iter().all(|r| r.what != "stale target") && target.exists(), "the hourly pass leaves a quiet clone alone");
    let r = run_pass(&ws, Why::LowDisk, &policy, &procs, in_days(10), None);
    assert_eq!(r.removed.len(), 1);
    assert_eq!(r.removed[0].what, "stale target");
    assert!(!target.exists());
    // The setting 0 turns it off.
    let clone = clone_with_incremental(&ws, "agentmux-old2", "agentmux_srv", &["h1"]);
    let off = Policy { stale: Duration::ZERO, ..policy };
    run_pass(&ws, Why::TaskDone, &off, &procs, in_days(10), None);
    assert!(clone.join("target").exists());
}

#[test]
fn a_clone_with_a_build_running_in_it_is_left_alone() {
    // This test binary runs from <target>/debug/deps, so a folder that holds it has a process in it.
    let exe = std::env::current_exe().unwrap();
    let procs = snapshot_processes();
    assert!(build_running_in(exe.parent().unwrap(), &procs), "our own test binary runs from {}", exe.display());
    let t = tempfile::tempdir().unwrap();
    assert!(!build_running_in(t.path(), &procs), "nothing runs in an empty temp folder");

    // A pass skips a clone the check calls busy and reports it, and clears the others.
    let ws = t.path().join("agent");
    let busy = clone_with_incremental(&ws, "agentmux", "agentmux_srv", &["h1", "h2"]);
    let other = clone_with_incremental(&ws, "agentmux-b", "agentmux_srv", &["h1", "h2"]);
    let is_busy = |c: &Path| c == busy;
    let report = run_pass(&ws, Why::TaskDone, &Policy::default(), &is_busy, SystemTime::now(), None);
    assert_eq!(report.busy, vec![busy.clone()]);
    assert_eq!(report.removed.len(), 2, "only the other clone's two: {report:?}");
    assert_eq!(std::fs::read_dir(busy.join("target/debug/incremental")).unwrap().count(), 2, "untouched");
    assert_eq!(std::fs::read_dir(other.join("target/debug/incremental")).unwrap().count(), 0);
}

#[test]
fn sizes_are_summed_and_shown_in_the_unit_people_read() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path();
    let big = clone_with_incremental(ws, "big", "agentmux_srv", &["a", "b"]);
    let small = clone_with_incremental(ws, "small", "agentmux_srv", &["a"]);
    std::fs::write(big.join("target").join("debug").join("deps").join("x.rlib"), vec![0u8; 100_000]).unwrap();
    let sizes = target_sizes(ws);
    assert_eq!(sizes.iter().map(|s| s.clone.clone()).collect::<Vec<_>>(), vec![big, small], "largest first");
    assert!(sizes[0].incremental_bytes >= 8192 && sizes[0].target_bytes > sizes[0].incremental_bytes);
    assert_eq!(human(820 << 20), "820 MB");
    assert_eq!(human(5 * (1 << 30) + (1 << 29)), "5.5 GB");
    assert_eq!(human(193 << 30), "193 GB");
}

#[test]
fn the_disk_holding_a_path_reports_free_and_total_space() {
    let t = tempfile::tempdir().unwrap();
    let (free, total) = disk_of(t.path()).expect("the temp folder is on some disk");
    assert!(total > 0 && free <= total);
}

/// Not run by default: what the orphan rule would remove from a real `incremental` folder
/// (`AGENTMUX_BC_REAL=<path to target/debug/incremental> cargo test ... real_world -- --ignored --nocapture`).
#[test]
#[ignore]
fn real_world_orphans_dry_run() {
    let Some(root) = std::env::var_os("AGENTMUX_BC_REAL") else { return };
    let root = PathBuf::from(root);
    let p = Policy::default();
    let gone = orphans_in(&root, p.orphan_keep, p.orphan_idle, SystemTime::now());
    let total: u64 = gone.iter().map(|d| dir_size(d)).sum();
    println!("{} orphan dirs, {}", gone.len(), human(total));
    for d in gone.iter().take(5) {
        println!("  {}", d.display());
    }
}
