// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A line-oriented file's tail row is stored longer than its content, filled with
//! spaces, and a line is appended in place (parts.rs `tail_capacity`,
//! `write_in_place`). The row's extra bytes are slack: never content, so every
//! reader stops at `size`, an older build's next append cuts the row back to the
//! content, and a kill in the middle of an append leaves a transcript that reads
//! back whole.

use rusqlite::{params, OptionalExtension};

use super::*;

const ZONE: &str = "agent:slack:current";
const NAME: &str = "output";
const PART: usize = 64 * 1024;

fn mem() -> FileStore {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs
}

fn line(i: usize, pad: usize) -> Vec<u8> {
    format!("{{\"n\":{i},\"p\":\"{}\"}}\n", "x".repeat(pad)).into_bytes()
}

/// The stored length of every part row of `name`, by index.
fn row_lens(fs: &FileStore, name: &str) -> Vec<(i64, usize)> {
    let conn = fs.conn().lock().unwrap();
    let mut stmt = conn.prepare("SELECT partidx, length(data) FROM db_file_data WHERE zoneid = ?1 AND name = ?2 ORDER BY partidx").unwrap();
    stmt.query_map(params![ZONE, name], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as usize))).unwrap().map(|r| r.unwrap()).collect()
}

fn row(fs: &FileStore, name: &str, idx: i64) -> Vec<u8> {
    fs.conn()
        .lock()
        .unwrap()
        .query_row("SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3", params![ZONE, name, idx], |r| r.get(0))
        .unwrap()
}

fn size_of(fs: &FileStore, name: &str) -> usize {
    fs.conn()
        .lock()
        .unwrap()
        .query_row("SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, name], |r| r.get::<_, i64>(0))
        .optional()
        .unwrap()
        .unwrap_or(0) as usize
}

/// What an older build reads: the rows end to end, cut to `size` (the rule every
/// reader has, readers.rs and `read_file`).
fn older_reader(fs: &FileStore, name: &str) -> Vec<u8> {
    let size = size_of(fs, name);
    let conn = fs.conn().lock().unwrap();
    let mut stmt = conn.prepare("SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 ORDER BY partidx").unwrap();
    let mut all: Vec<u8> = stmt.query_map(params![ZONE, name], |r| r.get::<_, Vec<u8>>(0)).unwrap().flat_map(|r| r.unwrap()).collect();
    all.truncate(size);
    all
}

/// What an older build does on append: the last row cut to the size, the new
/// bytes added, the whole row written back, then the size (no slack kept).
fn older_writer_append(fs: &FileStore, data: &[u8]) {
    let conn = fs.conn().lock().unwrap();
    let size: usize = conn
        .query_row("SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| r.get::<_, i64>(0))
        .unwrap() as usize;
    assert!(size % PART + data.len() <= PART, "the helper stays within one part");
    let idx = (size / PART) as i64;
    let mut part: Vec<u8> = conn
        .query_row("SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3", params![ZONE, NAME, idx], |r| r.get(0))
        .unwrap_or_default();
    part.truncate(size % PART);
    part.extend_from_slice(data);
    conn.execute("REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)", params![ZONE, NAME, idx, part]).unwrap();
    conn.execute(
        "UPDATE db_wave_file SET size = ?1, modts = modts + 1 WHERE zoneid = ?2 AND name = ?3",
        params![(size + data.len()) as i64, ZONE, NAME],
    )
    .unwrap();
}

#[test]
fn a_line_append_leaves_the_tail_row_longer_than_the_content_and_reads_stop_at_the_size() {
    let fs = mem();
    let first = line(0, 10);
    fs.append_lines(ZONE, NAME, &first).unwrap();
    assert_eq!(row_lens(&fs, NAME), vec![(0, 4096)], "a first line gets a 4 KB row");
    let stored = row(&fs, NAME, 0);
    assert_eq!(&stored[..first.len()], &first[..]);
    assert!(stored[first.len()..].iter().all(|&b| b == b' '), "the slack is spaces");
    assert_eq!(size_of(&fs, NAME), first.len());
    assert_eq!(fs.read_file(ZONE, NAME).unwrap().unwrap(), first);

    let second = line(1, 20);
    fs.append_lines(ZONE, NAME, &second).unwrap();
    assert_eq!(row_lens(&fs, NAME), vec![(0, 4096)], "the second line went into the slack");
    let both = [first.clone(), second.clone()].concat();
    assert_eq!(fs.read_file(ZONE, NAME).unwrap().unwrap(), both);
    assert_eq!(fs.read_at(ZONE, NAME, first.len() as i64, second.len() as i64).unwrap().1, second);
    assert_eq!(fs.read_bytes_db(ZONE, NAME, 0, both.len() as i64).unwrap(), both);
}

#[test]
fn the_tail_row_doubles_when_full_and_the_file_reads_back_through_every_part_boundary() {
    let fs = mem();
    let mut model: Vec<u8> = Vec::new();
    let mut i = 0usize;
    while model.len() < 2 * PART + 5000 {
        // Lines of assorted lengths, some long enough to cross a boundary by themselves.
        let l = line(i, (i * 37) % 3000);
        fs.append_lines(ZONE, NAME, &l).unwrap();
        model.extend_from_slice(&l);
        i += 1;
        if i % 7 == 0 || model.len() > 2 * PART {
            assert_eq!(size_of(&fs, NAME), model.len());
            assert_eq!(fs.read_file(ZONE, NAME).unwrap().unwrap(), model, "after {i} lines");
            assert_eq!(older_reader(&fs, NAME), model);
            let lens = row_lens(&fs, NAME);
            let tail = model.len() / PART;
            for (idx, len) in &lens {
                if (*idx as usize) < tail {
                    assert_eq!(*len, PART, "a part before the tail is a whole part");
                } else {
                    let content = model.len() - tail * PART;
                    assert!(*len >= content && len.is_power_of_two() && *len >= 4096 && *len <= PART, "the tail row ({len}) holds the content ({content}) in a power-of-two size");
                }
            }
            assert_eq!(lens.len(), tail + usize::from(model.len() % PART > 0), "no part past the content");
        }
    }
    for (off, len) in [(0usize, 100usize), (PART - 50, 100), (PART, 1), (2 * PART - 1, 2), (model.len() - 7, 7)] {
        assert_eq!(fs.read_at(ZONE, NAME, off as i64, len as i64).unwrap().1, model[off..off + len], "read_at({off}, {len})");
    }
    let state = fs.line_state(ZONE, NAME).unwrap().unwrap();
    assert_eq!(state.size as usize, model.len());
    assert_eq!(state.counted.expect("counted").lines as usize, i, "every line counted once");
}

#[test]
fn a_raw_append_stores_exactly_what_it_was_given() {
    let fs = mem();
    fs.append_data(ZONE, NAME, b"abc").unwrap();
    fs.append_data(ZONE, NAME, b"def").unwrap();
    assert_eq!(row_lens(&fs, NAME), vec![(0, 6)], "no slack on a raw append");
    assert_eq!(fs.read_file(ZONE, NAME).unwrap().unwrap(), b"abcdef");
}

#[test]
fn the_stamp_file_keeps_slack_too_and_stays_in_step_with_the_line() {
    let fs = mem();
    let now = agentmux_common::time::now_ms();
    for i in 0..30 {
        fs.append_lines_stamped_at(ZONE, NAME, &line(i, 50), "output.tsidx", now + (i as i64) * 5000).unwrap();
    }
    let stamps = fs.read_file(ZONE, "output.tsidx").unwrap().unwrap();
    assert!(stamps.ends_with(b"\n"));
    let text = String::from_utf8(stamps).unwrap();
    for l in text.lines() {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        assert!(v["off"].as_u64().unwrap() < size_of(&fs, NAME) as u64);
    }
    assert!(row_lens(&fs, "output.tsidx")[0].1 >= text.len());
    assert_eq!(older_reader(&fs, "output.tsidx"), text.as_bytes());
}

/// An older build on a padded store: it reads to `size`, its next append cuts the
/// row to the content (so a file it wrote has no slack), and the new build then
/// appends again, with slack back, without losing or doubling a byte.
#[test]
fn an_older_build_reads_a_padded_store_appends_to_it_and_the_new_build_carries_on() {
    let fs = mem();
    let mut model = Vec::new();
    for i in 0..5 {
        let l = line(i, 30);
        fs.append_lines(ZONE, NAME, &l).unwrap();
        model.extend_from_slice(&l);
    }
    assert!(row_lens(&fs, NAME)[0].1 > model.len(), "the new build left slack");
    // An older reader stops at size; one that ignores size sees only blanks past it.
    assert_eq!(older_reader(&fs, NAME), model);
    let unaware = row(&fs, NAME, 0);
    assert!(unaware[model.len()..].iter().all(|&b| b == b' '));
    assert!(String::from_utf8_lossy(&unaware).lines().filter(|l| !l.trim().is_empty()).count() == 5, "blank lines past the content are skipped");

    // An older writer's append: the row is cut to the content and written back.
    let older = line(5, 30);
    older_writer_append(&fs, &older);
    model.extend_from_slice(&older);
    assert_eq!(row_lens(&fs, NAME), vec![(0, model.len())], "its append removed the slack");
    assert_eq!(older_reader(&fs, NAME), model);
    assert_eq!(fs.read_bytes_db(ZONE, NAME, 0, model.len() as i64).unwrap(), model);

    // The new build again: a row with no room is rewritten with slack, then appended in place.
    for i in 6..12 {
        let l = line(i, 30);
        fs.append_lines(ZONE, NAME, &l).unwrap();
        model.extend_from_slice(&l);
    }
    assert!(row_lens(&fs, NAME)[0].1 > model.len() && row_lens(&fs, NAME)[0].1.is_power_of_two());
    assert_eq!(fs.read_bytes_db(ZONE, NAME, 0, model.len() as i64).unwrap(), model);
    assert_eq!(older_reader(&fs, NAME), model);
    let lines: Vec<String> = String::from_utf8(model.clone()).unwrap().lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 12);
}

/// The one cost of slack with an older build: its `REPLACE` writes a row shorter than the padded one, which
/// the revision trigger reads as a rewrite, so the line counter's epoch is dropped and the next count starts
/// a new generation (clients resync once). That happens only when a file's writer goes from the new build to
/// an older one; the new build's own appends never move `rev`, in place or when the row grows.
#[test]
fn an_older_builds_first_append_to_a_padded_row_starts_a_new_generation_and_nothing_else_does() {
    let fs = mem();
    let rev = |fs: &FileStore| -> i64 {
        fs.conn().lock().unwrap().query_row("SELECT COALESCE(rev, 0) FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| r.get(0)).unwrap()
    };
    for i in 0..400 {
        fs.append_lines(ZONE, NAME, &line(i, 40)).unwrap(); // crosses the 4 KB, 8 KB and 16 KB sizes of the row
    }
    assert_eq!(rev(&fs), 0, "in place, and when the row grows: an append, never a rewrite");
    let gen = fs.line_state(ZONE, NAME).unwrap().unwrap().counted.unwrap().gen;
    older_writer_append(&fs, &line(400, 40));
    assert_eq!(rev(&fs), 1, "the older build's REPLACE of a shorter row is read as a rewrite");
    older_writer_append(&fs, &line(401, 40));
    assert_eq!(rev(&fs), 1, "its next append finds an exact row: a pure extension again");
    fs.init_line_counter(ZONE, NAME).unwrap();
    let after = fs.line_state(ZONE, NAME).unwrap().unwrap().counted.unwrap();
    assert_ne!(after.gen, gen, "a new generation");
    assert_eq!(after.lines, 402, "every line counted once");
    fs.append_lines(ZONE, NAME, &line(402, 40)).unwrap();
    assert_eq!(rev(&fs), 1, "and the new build's append after it moves nothing");
}

/// The same, across a part boundary: the older writer fills the padded tail row to
/// the end of the part and starts the next one.
#[test]
fn an_older_writer_that_crosses_a_part_boundary_leaves_a_store_the_new_build_can_extend() {
    let fs = mem();
    let mut model = Vec::new();
    while model.len() < PART - 3000 {
        let l = line(model.len(), 900);
        fs.append_lines(ZONE, NAME, &l).unwrap();
        model.extend_from_slice(&l);
    }
    assert!(row_lens(&fs, NAME)[0].1 > model.len() % PART);
    let chunk = vec![b'z'; PART - model.len()]; // exactly to the end of part 0, as an older raw append
    older_writer_append(&fs, &chunk);
    model.extend_from_slice(&chunk);
    assert_eq!(row_lens(&fs, NAME), vec![(0, PART)]);
    // A last line missing its newline: the new build closes it first, then continues.
    let l = line(777, 5);
    fs.append_lines(ZONE, NAME, &l).unwrap();
    assert_eq!(fs.read_bytes_db(ZONE, NAME, 0, model.len() as i64).unwrap(), model);
    let total = size_of(&fs, NAME);
    let got = fs.read_bytes_db(ZONE, NAME, 0, total as i64).unwrap();
    assert!(got.starts_with(&model) && got.ends_with(&l), "the new line follows the closed one");
    assert_eq!(row_lens(&fs, NAME).len(), 2);
    assert_eq!(older_reader(&fs, NAME), got);
}

/// Truncating a padded file cuts its tail row to the new end, as for any file.
#[test]
fn a_truncate_of_a_padded_file_leaves_a_file_that_reads_and_extends() {
    let fs = mem();
    let mut model = Vec::new();
    for i in 0..40 {
        let l = line(i, 100);
        fs.append_lines(ZONE, NAME, &l).unwrap();
        model.extend_from_slice(&l);
    }
    let keep = model.iter().position(|&b| b == b'\n').unwrap() + 1;
    let cut = model[..keep].to_vec();
    fs.write_file(ZONE, NAME, &cut).unwrap();
    assert_eq!(fs.read_file(ZONE, NAME).unwrap().unwrap(), cut);
    let l = line(99, 5);
    fs.append_lines(ZONE, NAME, &l).unwrap();
    assert_eq!(fs.read_file(ZONE, NAME).unwrap().unwrap(), [cut, l].concat());
}

/// The child of the kill test: appends numbered lines (and their stamps) to the
/// store at `SLACK_KILL_DB`, forever; the parent kills it.
#[test]
#[ignore]
fn slack_kill_child() {
    let Some(db) = std::env::var_os("SLACK_KILL_DB") else { return };
    let fs = FileStore::open(std::path::Path::new(&db)).unwrap();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).ok();
    let mut i: usize = fs.line_state(ZONE, NAME).ok().flatten().and_then(|s| s.counted.map(|c| c.lines as usize)).unwrap_or(0);
    println!("CHILD-READY {i}");
    loop {
        // Lines of varying length; a stamp each time (now + i seconds: always due).
        let l = line(i, (i * 53) % 1500);
        let at = agentmux_common::time::now_ms() + (i as i64) * 5000;
        fs.append_lines_stamped_at(ZONE, NAME, &l, "output.tsidx", at).unwrap();
        i += 1;
    }
}

/// Kill the writer at random moments in the middle of its appends, many times over
/// the same store; after each kill the transcript must read back whole: complete
/// lines numbered 0..n with no gap and no repeat, `size` the length of what is read,
/// no NUL byte anywhere in the rows, the stamps parse, the line count agrees, and the
/// database passes its integrity check.
#[cfg(unix)]
#[test]
fn a_kill_in_the_middle_of_an_append_leaves_a_transcript_that_reads_back_whole() {
    use std::io::Read;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let mut expected_at_least = 0usize;
    for round in 0..14u64 {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backend::storage::filestore::tests_tail_slack::slack_kill_child", "--ignored", "--nocapture", "--test-threads=1"])
            .env("SLACK_KILL_DB", &path)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // Let it start, then run for a different time every round: 30 to 400 ms.
        let mut out = child.stdout.take().unwrap();
        let mut seen = String::new();
        let mut buf = [0u8; 256];
        while !seen.contains("CHILD-READY") {
            let n = out.read(&mut buf).unwrap();
            assert!(n > 0, "the child exited before it was ready: {seen}");
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        std::thread::sleep(Duration::from_millis(30 + (round * 97) % 370));
        child.kill().unwrap(); // SIGKILL
        child.wait().unwrap();

        let fs = FileStore::open(&path).unwrap();
        let bytes = fs.read_file(ZONE, NAME).unwrap().unwrap_or_default();
        assert_eq!(size_of(&fs, NAME), bytes.len(), "round {round}: size is what reads back");
        assert!(bytes.is_empty() || bytes.ends_with(b"\n"), "round {round}: a whole last line");
        let text = String::from_utf8(bytes).unwrap();
        let mut n = 0usize;
        for l in text.lines() {
            let v: serde_json::Value = serde_json::from_str(l).unwrap_or_else(|e| panic!("round {round}: line {n} is not whole: {e}: {l:?}"));
            assert_eq!(v["n"].as_u64().unwrap() as usize, n, "round {round}: lines in order, none missing or repeated");
            n += 1;
        }
        assert!(n >= expected_at_least, "round {round}: lines were lost ({n} < {expected_at_least})");
        expected_at_least = n;
        let rows_have_no_nul: i64 = fs
            .conn()
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM db_file_data WHERE zoneid = ?1 AND instr(data, x'00') > 0", params![ZONE], |r| r.get(0))
            .unwrap();
        assert_eq!(rows_have_no_nul, 0, "round {round}: no NUL byte in any row");
        let stamps = String::from_utf8(fs.read_file(ZONE, "output.tsidx").unwrap().unwrap_or_default()).unwrap();
        let size = size_of(&fs, NAME) as u64;
        for l in stamps.lines() {
            let v: serde_json::Value = serde_json::from_str(l).unwrap_or_else(|e| panic!("round {round}: stamp not whole: {e}: {l:?}"));
            assert!(v["off"].as_u64().unwrap() <= size, "round {round}: a stamp past the end");
        }
        assert_eq!(fs.line_state(ZONE, NAME).unwrap().and_then(|s| s.counted).map_or(0, |c| c.lines as usize), n, "round {round}: the line count agrees");
        let check: String = fs.conn().lock().unwrap().query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(check, "ok", "round {round}");
    }
    assert!(expected_at_least > 100, "the child wrote only {expected_at_least} lines in 14 rounds: the test proved little");
}
