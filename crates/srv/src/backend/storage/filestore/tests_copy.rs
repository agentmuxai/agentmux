// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copying files between zones in the database (`copy.rs`).

use super::{FileMeta, FileOpts, FileStore};

const SRC: &str = "agent:parent:current";
const DST: &str = "agent:fork:current";

fn mem() -> FileStore {
    FileStore::open_in_memory().unwrap()
}

/// About 200 KB of JSON lines: more than three 64 KiB parts.
fn transcript() -> Vec<u8> {
    (0..2000).map(|i| format!("{{\"type\":\"assistant\",\"n\":{i},\"text\":\"{}\"}}\n", "x".repeat(80))).collect::<String>().into_bytes()
}

/// A new, empty file, as transcripts are made before their first append.
fn file(fs: &FileStore, zone: &str, name: &str) {
    fs.make_file(zone, name, FileMeta::new(), FileOpts::default()).unwrap();
}

fn all(fs: &FileStore, zone: &str, name: &str) -> Vec<u8> {
    let size = fs.line_state(zone, name).unwrap().unwrap().size;
    fs.read_bytes_db(zone, name, 0, size).unwrap()
}

#[test]
fn a_copy_is_byte_identical_counted_and_a_file_of_its_own() {
    let fs = mem();
    let data = transcript();
    file(&fs, SRC, "output");
    fs.append_lines_stamped(SRC, "output", &data, "output.tsidx").unwrap();

    let copied = fs.copy_files_to_zone(SRC, DST, &["output", "output.tsidx", "absent"]).unwrap();

    let tsidx = all(&fs, SRC, "output.tsidx");
    assert_eq!(copied, Some((data.len() + tsidx.len()) as u64));
    assert_eq!(all(&fs, DST, "output"), data);
    assert_eq!(all(&fs, DST, "output.tsidx"), tsidx);
    assert!(fs.line_state(DST, "absent").unwrap().is_none(), "a name the source lacks is skipped");
    let src = fs.line_state(SRC, "output").unwrap().unwrap().counted.unwrap();
    let dst = fs.line_state(DST, "output").unwrap().unwrap().counted.expect("the copy keeps a valid line count");
    assert_eq!(dst.lines, 2000);
    assert_eq!(dst.lines, src.lines);
    assert_ne!(dst.gen, src.gen, "a copy is a new file, with its own generation");

    // From here on they are two files.
    fs.append_lines(DST, "output", b"{\"type\":\"user\",\"fork\":true}\n").unwrap();
    fs.append_lines(SRC, "output", b"{\"type\":\"user\",\"parent\":true}\n").unwrap();
    let fork = all(&fs, DST, "output");
    assert!(fork.starts_with(&data) && fork.ends_with(b"{\"type\":\"user\",\"fork\":true}\n"));
    assert!(!fork.windows(13).any(|w| w == b"\"parent\":true"));
    assert_eq!(fs.line_state(DST, "output").unwrap().unwrap().counted.unwrap().lines, 2001);
}

#[test]
fn a_copy_never_overwrites_a_destination_that_has_any_of_the_files() {
    let fs = mem();
    for (zone, name) in [(SRC, "output"), (SRC, "output.tsidx"), (DST, "output.tsidx")] {
        file(&fs, zone, name);
    }
    fs.append_lines(SRC, "output", &transcript()).unwrap();
    fs.append_lines(SRC, "output.tsidx", b"{\"off\":0,\"ms\":1}\n").unwrap();
    fs.append_lines(DST, "output.tsidx", b"{\"off\":0,\"ms\":2}\n").unwrap();

    assert_eq!(fs.copy_files_to_zone(SRC, DST, &["output", "output.tsidx"]).unwrap(), None);
    assert!(fs.line_state(DST, "output").unwrap().is_none(), "nothing is copied, not even the free name");
    assert_eq!(all(&fs, DST, "output.tsidx"), b"{\"off\":0,\"ms\":2}\n");
}

#[test]
fn an_uncounted_source_is_copied_and_counted_from_zero() {
    let fs = mem();
    file(&fs, SRC, "output");
    fs.append_lines(SRC, "output", b"a\nb\n").unwrap();
    // A rewrite the count doesn't describe (an older build's write).
    fs.conn().lock().unwrap().execute_batch("UPDATE db_wave_file SET rev = 7 WHERE name = 'output'").unwrap();

    fs.copy_files_to_zone(SRC, DST, &["output"]).unwrap();

    assert_eq!(all(&fs, DST, "output"), b"a\nb\n");
    assert_eq!(fs.catch_up_line_counter(DST, "output").unwrap().unwrap().counted.unwrap().lines, 2);
}

/// A move renames the files: same bytes, same line count and generation, the
/// source is gone, and a second move of the same source finds nothing.
#[test]
fn a_move_takes_the_files_once_and_keeps_them_as_they_were() {
    let fs = mem();
    let data = transcript();
    file(&fs, SRC, "output");
    fs.append_lines_stamped(SRC, "output", &data, "output.tsidx").unwrap();
    let tsidx = all(&fs, SRC, "output.tsidx");
    let before = fs.line_state(SRC, "output").unwrap().unwrap().counted.unwrap();

    let moved = fs.move_files_to_zone(SRC, DST, &["output", "output.tsidx", "absent"]).unwrap();

    assert_eq!(moved, Some((data.len() + tsidx.len()) as u64));
    assert_eq!(all(&fs, DST, "output"), data);
    assert_eq!(all(&fs, DST, "output.tsidx"), tsidx);
    assert!(fs.line_state(SRC, "output").unwrap().is_none(), "the source is gone");
    let after = fs.line_state(DST, "output").unwrap().unwrap().counted.expect("still counted");
    assert_eq!((after.lines, after.gen), (before.lines, before.gen), "the same file, renamed");
    assert_eq!(fs.move_files_to_zone(SRC, "agent:other:current", &["output"]).unwrap(), Some(0), "nothing left to take");
}

#[test]
fn a_move_never_overwrites_a_destination_that_has_any_of_the_files() {
    let fs = mem();
    file(&fs, SRC, "output");
    fs.append_lines(SRC, "output", b"{\"src\":1}\n").unwrap();
    file(&fs, DST, "output.tsidx");

    assert_eq!(fs.move_files_to_zone(SRC, DST, &["output", "output.tsidx"]).unwrap(), None);
    assert_eq!(all(&fs, SRC, "output"), b"{\"src\":1}\n", "the source stays where it was");
}
