// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The receive-time sidecar is written at most once a second per file, not once
//! per line: a record costs a rewrite of the sidecar's last part, and lines
//! arrive hundreds a second.
//! docs/reports/REPORT_FILESTORE_WRITE_VOLUME_2026_10_19.md

use super::{FileMeta, FileOpts, FileStore};

const ZONE: &str = "block-stamps";
const SIDECAR: &str = "output.tsidx";

fn store() -> FileStore {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file(ZONE, "output", FileMeta::new(), FileOpts::default()).unwrap();
    fs
}

fn append(fs: &FileStore, i: usize, now: i64) -> i64 {
    fs.append_lines_stamped_at(ZONE, "output", format!("{{\"i\":{i}}}\n").as_bytes(), SIDECAR, now).unwrap().offset
}

fn records(fs: &FileStore) -> Vec<(i64, i64)> {
    let text = fs.read_file(ZONE, SIDECAR).unwrap().map(|b| String::from_utf8(b).unwrap()).unwrap_or_default();
    text.lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (v["off"].as_i64().unwrap(), v["ms"].as_i64().unwrap())
        })
        .collect()
}

#[test]
fn lines_inside_a_second_share_one_record() {
    let fs = store();
    let t0 = 10_000;
    let offsets: Vec<i64> = [0, 10, 500, 999, 1000, 1500, 1999, 2001, 2002]
        .iter()
        .enumerate()
        .map(|(i, dt)| append(&fs, i, t0 + dt))
        .collect();
    // Records at t0 (line 0), t0+1000 (line 4) and t0+2001 (line 7).
    assert_eq!(records(&fs), vec![(offsets[0], t0), (offsets[4], t0 + 1000), (offsets[7], t0 + 2001)]);
}

#[test]
fn the_first_line_of_a_file_and_the_first_after_a_pause_are_stamped() {
    let fs = store();
    append(&fs, 0, 5);
    append(&fs, 1, 300_000); // five minutes later
    let r = records(&fs);
    assert_eq!(r.len(), 2, "{r:?}");
    assert_eq!(r[1].1, 300_000);
}

#[test]
fn a_clock_that_goes_backwards_is_stamped_again() {
    let fs = store();
    append(&fs, 0, 50_000);
    append(&fs, 1, 10_000);
    assert_eq!(records(&fs).len(), 2);
}

#[test]
fn a_failed_append_does_not_count_as_a_record() {
    let fs = store();
    let missing = fs.append_lines_stamped_at(ZONE, "no-such-file", b"{\"x\":1}\n", SIDECAR, 7_000);
    assert!(missing.is_err());
    append(&fs, 0, 7_000);
    assert_eq!(records(&fs).len(), 1, "the failed call did not use up the second");
}

#[test]
fn each_sidecar_has_its_own_clock() {
    let fs = store();
    fs.make_file(ZONE, "other", FileMeta::new(), FileOpts::default()).unwrap();
    append(&fs, 0, 9_000);
    fs.append_lines_stamped_at(ZONE, "other", b"{\"y\":1}\n", "other.tsidx", 9_001).unwrap();
    assert_eq!(records(&fs).len(), 1);
    assert!(fs.read_file(ZONE, "other.tsidx").unwrap().is_some(), "another file is stamped on its own schedule");
}

#[test]
fn the_sidecar_stays_small_while_lines_pour_in() {
    let fs = store();
    // 3000 lines over ten seconds: 300 a second.
    for i in 0..3000 {
        append(&fs, i, 100_000 + (i as i64) * 10 / 3);
    }
    let r = records(&fs);
    assert!(r.len() <= 11, "{} records for ten seconds of lines", r.len());
    assert!(r.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1), "records ascend in offset and time");
}

#[test]
fn a_transcript_recreated_within_a_second_is_stamped_from_its_first_line() {
    let fs = store();
    append(&fs, 0, 20_000);
    append(&fs, 1, 20_100); // no record of its own
    // The transcript and its sidecar are deleted (a session archive) and the transcript starts again.
    fs.delete_files(ZONE, &["output", SIDECAR]).unwrap();
    fs.make_file(ZONE, "output", FileMeta::new(), FileOpts::default()).unwrap();
    let off = append(&fs, 2, 20_200);
    assert_eq!(off, 0);
    assert_eq!(records(&fs), vec![(0, 20_200)], "the first line of the new transcript has a record");
}

#[test]
fn a_sidecar_that_was_emptied_while_the_transcript_stayed_is_stamped_at_the_next_line() {
    let fs = store();
    append(&fs, 0, 30_000);
    let second = append(&fs, 1, 30_050);
    fs.delete_files(ZONE, &[SIDECAR]).unwrap();
    let third = append(&fs, 2, 30_100); // inside the second, but there is no sidecar
    assert!(third > second);
    assert_eq!(records(&fs), vec![(third, 30_100)]);
}
