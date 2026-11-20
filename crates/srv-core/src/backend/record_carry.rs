// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What older builds kept of the record, brought into the folder it lives in now
//! (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §6, task #47820).
//!
//! The record first lived in the data folder. An installed AgentMux has a data folder per version, so every
//! upgrade began a new, empty record, and the old one went with the old version's folder: a record that must
//! outlast weeks never held more than the days between two upgrades. It lives in the channel's folder now
//! ([`agentmux_common::record_events::record_dir`]), which an upgrade keeps.
//!
//! [`bring_over`] moves what the data folders still hold into it, once: a file the record does not have is
//! moved, and where both have a file of the same name (the day of an upgrade, the month's rollups and events)
//! their lines are put together in time order. A file is taken from the old folder only after its lines are in
//! the new one, so a failure part of the way leaves the rest to the next start.

use std::path::{Path, PathBuf};

const EXT: &str = "jsonl";
const ROLLUP_PREFIX: &str = "rollup-";
/// How often a merge is read again because a line was appended to the record's file meanwhile (the window host
/// and srv's stall watch write events whenever they see one).
const TRIES: usize = 3;

/// What [`bring_over`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Brought {
    /// Files moved whole.
    pub moved: usize,
    /// Files whose lines were merged into one the record already had.
    pub merged: usize,
    /// Files that could not be brought; they stay where they were.
    pub failed: usize,
}

/// The folders an older build kept its record in, under `instance_dir` (the channel's folder, or a dev build's
/// own): `data/record`, and `versions/<version>/data/record` of every version, oldest name first.
pub fn old_dirs(instance_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![instance_dir.join("data").join("record")];
    if let Ok(entries) = std::fs::read_dir(instance_dir.join("versions")) {
        let mut versions: Vec<PathBuf> = entries.flatten().map(|e| e.path().join("data").join("record")).collect();
        versions.sort();
        dirs.extend(versions);
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// The time a line is ordered by: the hour of a rollup, the time of a sample or an event.
fn time_of(line: &str, field: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(line).ok()?.get(field)?.as_u64()
}

/// The lines of `ours` and `theirs` in time order, each ending in a newline. A line of `theirs` that `ours`
/// already has is left out (a merge that was cut short before the old file was removed is run again). A line
/// that cannot be read stays behind the line it followed.
fn merged(ours: &str, theirs: &str, field: &str) -> String {
    let have: std::collections::HashSet<&str> = ours.lines().collect();
    let mut lines: Vec<(u64, &str)> = Vec::new();
    for (text, is_theirs) in [(ours, false), (theirs, true)] {
        let mut last = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            if is_theirs && have.contains(line) {
                continue;
            }
            if let Some(t) = time_of(line, field) {
                last = t;
            }
            lines.push((last, line));
        }
    }
    // Stable: lines of the same time keep the order they were written in.
    lines.sort_by_key(|(t, _)| *t);
    let mut out = String::with_capacity(ours.len() + theirs.len());
    for (_, line) in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn len_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Bring the file `from` into the record as `to`. `Ok(true)` when it was moved whole, `Ok(false)` when its lines
/// were merged into the file already there.
fn bring_file(from: &Path, to: &Path, name: &str) -> std::io::Result<bool> {
    if !to.exists() && std::fs::rename(from, to).is_ok() {
        return Ok(true);
    }
    let field = if name.starts_with(ROLLUP_PREFIX) { "h" } else { "t" };
    let theirs = std::fs::read_to_string(from)?;
    let tmp = to.with_file_name(format!("{name}.tmp"));
    for _ in 0..TRIES {
        let before = len_of(to);
        let ours = match std::fs::read_to_string(to) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        std::fs::write(&tmp, merged(&ours, &theirs, field))?;
        if len_of(to) != before {
            // A line was appended while the two were put together: read it again.
            continue;
        }
        std::fs::rename(&tmp, to)?;
        std::fs::remove_file(from)?;
        return Ok(false);
    }
    let _ = std::fs::remove_file(&tmp);
    Err(std::io::Error::other("the record's file kept changing"))
}

/// Move what the folders of [`old_dirs`] hold into `dest`, the record's folder. BLOCKING (file reads and
/// writes); call it once at start, before the first line is written. A file that cannot be brought is logged
/// and left where it is: the record is never a reason for anything else to fail.
pub fn bring_over(instance_dir: &Path, dest: &Path) -> Brought {
    let mut brought = Brought::default();
    for dir in old_dirs(instance_dir) {
        if dir == dest {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n.ends_with(&format!(".{EXT}")))
            .collect();
        names.sort();
        if names.is_empty() {
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(dest) {
            tracing::warn!(error = %e, "[record] the record's folder could not be made");
            brought.failed += names.len();
            continue;
        }
        for name in names {
            match bring_file(&dir.join(&name), &dest.join(&name), &name) {
                Ok(true) => brought.moved += 1,
                Ok(false) => brought.merged += 1,
                Err(e) => {
                    brought.failed += 1;
                    tracing::warn!(error = %e, file = %name, "[record] an older build's file could not be brought into the record");
                }
            }
        }
        // Gone only when nothing is left in it.
        let _ = std::fs::remove_dir(&dir);
    }
    if brought != Brought::default() {
        tracing::info!(moved = brought.moved, merged = brought.merged, failed = brought.failed, "[record] brought over what older builds kept");
    }
    brought
}

/// [`bring_over`] for this process: from the folders under its channel into its record. Nothing when the
/// launcher gave it no folders.
pub fn bring_over_here() -> Brought {
    let Some(paths) = agentmux_common::data_paths::DataPaths::from_env() else { return Brought::default() };
    let Some(dest) = agentmux_common::record_events::record_dir() else { return Brought::default() };
    bring_over(&paths.instance_dir, &dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::record;
    use serde_json::json;

    /// 2026-11-16 12:00 UTC.
    const NOON: u64 = 1_794_830_400_000;
    const HOUR: u64 = 3_600_000;

    fn write(path: &Path, lines: &[serde_json::Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(path, text).unwrap();
    }

    fn times(path: &Path, field: &str) -> Vec<u64> {
        std::fs::read_to_string(path).unwrap().lines().map(|l| time_of(l, field).unwrap()).collect()
    }

    fn old(home: &Path, version: &str) -> PathBuf {
        home.join("versions").join(version).join("data").join("record")
    }

    #[test]
    fn the_old_folders_are_each_versions_and_a_dev_builds_own() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(old_dirs(home).is_empty());
        std::fs::create_dir_all(old(home, "0.60.34")).unwrap();
        std::fs::create_dir_all(old(home, "0.60.35")).unwrap();
        // A version that never kept a record, and a file where a folder is expected.
        std::fs::create_dir_all(home.join("versions").join("0.60.30").join("data")).unwrap();
        std::fs::write(home.join("versions").join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(home.join("data").join("record")).unwrap();
        assert_eq!(old_dirs(home), vec![home.join("data").join("record"), old(home, "0.60.34"), old(home, "0.60.35")]);
    }

    #[test]
    fn lines_are_put_in_time_order_and_one_already_there_is_not_added_twice() {
        let a = format!("{}\n{}\n", json!({ "t": 10, "kind": "x" }), json!({ "t": 30, "kind": "x" }));
        let b = format!("{}\n{}\n\n{}\n", json!({ "t": 20, "kind": "y" }), json!({ "t": 30, "kind": "x" }), json!({ "t": 5, "kind": "y" }));
        let out = merged(&a, &b, "t");
        let got: Vec<(u64, String)> =
            out.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).map(|v| (v["t"].as_u64().unwrap(), v["kind"].as_str().unwrap().to_string())).collect();
        assert_eq!(got, [(5, "y".into()), (10, "x".into()), (20, "y".into()), (30, "x".into())]);
        assert!(out.ends_with('\n'));
        // A line cut short by a crash stays behind the line it followed.
        let torn = format!("{}\n{{\"t\": 4\n{}\n", json!({ "t": 40 }), json!({ "t": 50 }));
        let out = merged(&a, &torn, "t");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[3], "{\"t\": 4");
        // Rollups are ordered by their hour.
        let out = merged(&format!("{}\n", json!({ "h": 7, "last_t": 1 })), &format!("{}\n", json!({ "h": 3, "last_t": 9 })), "h");
        assert!(out.starts_with("{\"h\":3"));
    }

    #[test]
    fn each_versions_record_becomes_one_record_in_time_order() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = home.join("record");
        // The older version: a day of its own, the morning of the day of the upgrade, and the month's files.
        let v34 = old(home, "0.60.34");
        write(&v34.join("2026-11-15.jsonl"), &[json!({ "t": NOON - 24 * HOUR })]);
        write(&v34.join("2026-11-16.jsonl"), &[json!({ "t": NOON - 3 * HOUR }), json!({ "t": NOON - 2 * HOUR })]);
        write(&v34.join("rollup-2026-11.jsonl"), &[json!({ "h": NOON - 24 * HOUR }), json!({ "h": NOON - 3 * HOUR })]);
        write(&v34.join("events-2026-11.jsonl"), &[json!({ "t": NOON - 2 * HOUR, "kind": "ui_stall", "ms": 6_000 })]);
        // The newer one: the afternoon of the same day.
        let v35 = old(home, "0.60.35");
        write(&v35.join("2026-11-16.jsonl"), &[json!({ "t": NOON + HOUR })]);
        write(&v35.join("rollup-2026-11.jsonl"), &[json!({ "h": NOON })]);
        // What is not the record's stays.
        std::fs::write(v35.join("notes.txt"), "x").unwrap();
        // The record already holds a line written since this start.
        write(&dest.join("events-2026-11.jsonl"), &[json!({ "t": NOON + 2 * HOUR, "kind": "srv_stall", "ms": 5_500 })]);

        assert_eq!(bring_over(home, &dest), Brought { moved: 3, merged: 3, failed: 0 });

        assert_eq!(times(&dest.join("2026-11-15.jsonl"), "t"), [NOON - 24 * HOUR]);
        assert_eq!(times(&dest.join("2026-11-16.jsonl"), "t"), [NOON - 3 * HOUR, NOON - 2 * HOUR, NOON + HOUR]);
        assert_eq!(times(&dest.join("rollup-2026-11.jsonl"), "h"), [NOON - 24 * HOUR, NOON - 3 * HOUR, NOON]);
        assert_eq!(times(&dest.join("events-2026-11.jsonl"), "t"), [NOON - 2 * HOUR, NOON + 2 * HOUR]);
        // Nothing is left behind but what was never the record's, and no half-written file.
        assert!(!v34.exists());
        assert_eq!(std::fs::read_dir(&v35).unwrap().count(), 1);
        assert!(std::fs::read_dir(&dest).unwrap().flatten().all(|e| e.file_name().to_str().unwrap().ends_with(".jsonl")));
        // The record reads it back as one: both stalls, oldest first.
        let (total, events) = record::read_events(&dest, 0, u64::MAX, 10);
        assert_eq!(total, 2);
        assert_eq!(events[0]["kind"], "ui_stall");
        // A second start finds nothing to bring.
        assert_eq!(bring_over(home, &dest), Brought::default());
    }

    #[test]
    fn a_dev_builds_record_moves_out_of_its_data_folder_and_no_old_record_is_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = home.join("record");
        assert_eq!(bring_over(home, &dest), Brought::default());
        assert!(!dest.exists());
        write(&home.join("data").join("record").join("2026-11-16.jsonl"), &[json!({ "t": NOON })]);
        assert_eq!(bring_over(home, &dest), Brought { moved: 1, merged: 0, failed: 0 });
        assert_eq!(times(&dest.join("2026-11-16.jsonl"), "t"), [NOON]);
        assert!(!home.join("data").join("record").exists());
    }

    #[test]
    fn a_merge_run_again_after_being_cut_short_adds_nothing_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = home.join("record");
        let lines = [json!({ "t": NOON }), json!({ "t": NOON + HOUR })];
        // The lines are in the record already, and the old file was not removed.
        write(&dest.join("2026-11-16.jsonl"), &lines);
        write(&old(home, "0.60.34").join("2026-11-16.jsonl"), &lines);
        assert_eq!(bring_over(home, &dest), Brought { moved: 0, merged: 1, failed: 0 });
        assert_eq!(times(&dest.join("2026-11-16.jsonl"), "t"), [NOON, NOON + HOUR]);
    }
}
