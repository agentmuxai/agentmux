// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The append path against a naive reference: a plain `Vec<u8>` that every
//! append is also applied to. Many small appends interleaved with reads at
//! part boundaries must read back exactly what the reference holds, for the
//! raw append and the line append, including appends that land exactly on a
//! part boundary, end one byte short of it, cross it, or span several parts.
//! docs/specs/SPEC_FILESTORE_APPEND_WITHOUT_COPIES_2026_10_18.md

use super::core::PART_DATA_SIZE;
use super::{FileMeta, FileOpts, FileStore};

const ZONE: &str = "block-ref";
const PART: usize = PART_DATA_SIZE;

/// A tiny deterministic generator (xorshift), so a failure reproduces.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn store_with(name: &str) -> FileStore {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file(ZONE, name, FileMeta::new(), FileOpts::default()).unwrap();
    fs
}

/// The bytes of one append: its length chosen to hit the part boundaries as
/// often as it hits anything else. `seq` makes every byte position distinguishable.
fn chunk(rng: &mut Rng, stored: usize, seq: u64) -> Vec<u8> {
    let to_boundary = PART - (stored % PART);
    let len = match rng.below(10) {
        0 => to_boundary,                 // ends exactly on a boundary
        1 => to_boundary.saturating_sub(1).max(1), // one short of it
        2 => to_boundary + 1,             // one over
        3 => PART * 2 + rng.below(50) as usize + 1, // spans several parts
        4 => 0,                           // nothing to append
        _ => 1 + rng.below(200) as usize, // the common case: a small record
    };
    (0..len).map(|i| ((seq as usize * 31 + i * 7) % 251) as u8).collect()
}

/// Reads around every boundary and the tail, compared with the reference.
fn check_reads(fs: &FileStore, name: &str, reference: &[u8]) {
    let size = reference.len() as i64;
    let read = |offset: i64, len: i64| fs.read_at(ZONE, name, offset, len).unwrap().1;
    for k in 1..=(reference.len() / PART) as i64 {
        let b = k * PART as i64;
        let (from, to) = ((b - 3).max(0), (b + 3).min(size));
        if from < to {
            assert_eq!(read(from, to - from), reference[from as usize..to as usize], "boundary {b}");
        }
    }
    let tail_from = (size - 100).max(0);
    if size > 0 {
        assert_eq!(read(tail_from, size - tail_from), reference[tail_from as usize..], "tail");
    }
}

#[test]
fn raw_appends_match_a_vec_through_every_part_boundary() {
    let name = "raw";
    let fs = store_with(name);
    let mut reference: Vec<u8> = Vec::new();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for seq in 0..1500u64 {
        let data = chunk(&mut rng, reference.len(), seq);
        let offset = fs.append_data_at(ZONE, name, &data).unwrap();
        assert_eq!(offset, reference.len() as i64, "append {seq}: the offset it landed at");
        reference.extend_from_slice(&data);
        assert_eq!(fs.stat(ZONE, name).unwrap().unwrap().size, reference.len() as i64, "append {seq}: size");
        if seq % 7 == 0 || data.len() >= PART {
            check_reads(&fs, name, &reference);
        }
    }
    assert!(reference.len() > PART * 3, "the run crossed several part boundaries ({} bytes)", reference.len());
    assert_eq!(fs.read_file(ZONE, name).unwrap().unwrap(), reference, "the whole file");
}

/// The reference for the line append, written independently of the code under
/// test: complete non-blank lines, each ending in `\n`; a file that ends
/// mid-line gets a `\n` first.
fn reference_append_lines(reference: &mut Vec<u8>, data: &[u8]) {
    let blank = |l: &[u8]| l.iter().all(|&b| matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r'));
    let mut records = Vec::new();
    for line in data.split(|&b| b == b'\n') {
        if !blank(line) {
            records.extend_from_slice(line);
            records.push(b'\n');
        }
    }
    if records.is_empty() {
        return;
    }
    if reference.last().is_some_and(|&b| b != b'\n') {
        reference.push(b'\n');
    }
    reference.extend_from_slice(&records);
}

#[test]
fn line_appends_match_a_vec_including_a_torn_tail_and_the_counter() {
    let name = "lines";
    let fs = store_with(name);
    let mut reference: Vec<u8> = Vec::new();
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for seq in 0..1200u64 {
        // A few lines per call, some blank, now and then a long one that
        // pushes the file across a part boundary.
        let mut data = Vec::new();
        for l in 0..rng.below(4) {
            match rng.below(8) {
                0 => data.extend_from_slice(b"   "),
                1 => {
                    let n = to_boundary_len(reference.len(), &mut rng);
                    data.extend((0..n).map(|i| b'a' + ((seq as usize + i) % 26) as u8));
                }
                _ => data.extend(format!("{{\"seq\":{seq},\"l\":{l},\"pad\":\"{}\"}}", "x".repeat(rng.below(120) as usize)).bytes()),
            }
            data.push(b'\n');
        }
        // Sometimes leave the last line unterminated (a torn input).
        if rng.below(6) == 0 && data.last() == Some(&b'\n') {
            data.pop();
        }
        let before = reference.len();
        reference_append_lines(&mut reference, &data);
        let pos = fs.append_lines(ZONE, name, &data).unwrap();
        assert!(pos.offset == before as i64, "append {seq}: landed at {} not {before}", pos.offset);
        assert_eq!(fs.stat(ZONE, name).unwrap().unwrap().size, reference.len() as i64, "append {seq}: size");
        if seq % 5 == 0 {
            check_reads(&fs, name, &reference);
        }
    }
    assert!(reference.len() > PART * 3, "the run crossed several part boundaries ({} bytes)", reference.len());
    assert_eq!(fs.read_file(ZONE, name).unwrap().unwrap(), reference, "the whole file");
    // The line counter never drifts from the bytes: one per newline-terminated record.
    let lines = reference.iter().filter(|&&b| b == b'\n').count() as u64;
    if let Some(c) = fs.line_state(ZONE, name).unwrap().and_then(|s| s.counted) {
        assert_eq!(c.lines, lines, "counted lines");
    }
}

/// A line length that makes the next append end exactly at, one short of, or one past a part boundary.
fn to_boundary_len(stored: usize, rng: &mut Rng) -> usize {
    let to_boundary = PART - (stored % PART);
    match rng.below(3) {
        0 => to_boundary.saturating_sub(1).max(1), // + the "\n" ends exactly on it
        1 => to_boundary.saturating_sub(2).max(1),
        _ => to_boundary,
    }
}

#[test]
fn an_unterminated_raw_tail_is_closed_by_the_next_line_append() {
    let name = "torn";
    let fs = store_with(name);
    let mut reference = Vec::new();
    // A raw append that stops mid-line, right at the end of part 0.
    let raw: Vec<u8> = std::iter::repeat(b'r').take(PART).collect();
    fs.append_data_at(ZONE, name, &raw).unwrap();
    reference.extend_from_slice(&raw);
    // The line append has to close it (a "\n" in part 1) and keep going.
    for i in 0..5 {
        let data = format!("line {i}\n").into_bytes();
        reference_append_lines(&mut reference, &data);
        fs.append_lines(ZONE, name, &data).unwrap();
        check_reads(&fs, name, &reference);
    }
    assert_eq!(fs.read_file(ZONE, name).unwrap().unwrap(), reference);
}
