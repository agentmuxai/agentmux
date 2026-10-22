// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Compressed frames (`frames.rs`): every read sees the same bytes framed or
//! not, every write path keeps them right, and nothing frames what it must not.

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};
use proptest::prelude::*;

const Z: &str = "agent:a:current";
const N: &str = "output";
const P: usize = PDS;

fn mem() -> FileStore {
    FileStore::open_in_memory().unwrap()
}

/// Transcript-like lines of varied lengths, deterministic for `seed`, at
/// least `min_bytes` long.
fn transcript(min_bytes: usize, seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(min_bytes + 4096);
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut n = 0u64;
    while out.len() < min_bytes {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let len = 20 + (x >> 33) as usize % 900;
        let body: String = (0..len).map(|i| (b'a' + ((x >> (i % 50)) as u8 % 26)) as char).collect();
        out.extend_from_slice(format!("{{\"type\":\"assistant\",\"n\":{n},\"text\":\"{body}\"}}\n").as_bytes());
        n += 1;
    }
    out
}

fn make(fs: &FileStore, zone: &str, data: &[u8]) {
    fs.make_file(zone, N, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(zone, N, data).unwrap();
}

fn all(fs: &FileStore, zone: &str) -> Vec<u8> {
    let size = fs.line_state(zone, N).unwrap().unwrap().size;
    fs.read_bytes_db(zone, N, 0, size).unwrap()
}

fn frames_of(fs: &FileStore, zone: &str) -> i64 {
    fs.lock_conn()
        .query_row("SELECT COUNT(*) FROM db_file_frame WHERE zoneid = ?1 AND name = ?2", params![zone, N], |r| r.get(0))
        .unwrap()
}

fn rows_of(fs: &FileStore, zone: &str) -> i64 {
    fs.lock_conn()
        .query_row("SELECT COUNT(*) FROM db_file_data WHERE zoneid = ?1 AND name = ?2", params![zone, N], |r| r.get(0))
        .unwrap()
}

/// A transcript of 40 full parts and a partial one, its first 32 parts in two frames.
fn framed_store() -> (FileStore, Vec<u8>) {
    let fs = mem();
    let mut data = transcript(40 * P + 3000, 7);
    data.truncate(data.iter().rposition(|&b| b == b'\n').unwrap() + 1);
    make(&fs, Z, &data);
    assert!(matches!(fs.frame_parts(Z, N, 0, 16, CODEC_ZSTD).unwrap(), FrameOutcome::Framed { .. }));
    assert!(matches!(fs.frame_parts(Z, N, 16, 16, CODEC_ZSTD).unwrap(), FrameOutcome::Framed { .. }));
    (fs, data)
}

#[test]
fn every_reader_sees_the_same_bytes_framed() {
    let (fs, data) = framed_store();
    assert_eq!(frames_of(&fs, Z), 2);
    assert_eq!(rows_of(&fs, Z) as usize, data.len().div_ceil(P) - 32, "framed parts have no row");

    assert_eq!(fs.read_file(Z, N).unwrap().unwrap(), data, "read_file");
    assert_eq!(all(&fs, Z), data, "read_bytes_db");
    // Ranges inside a frame, across frames, and across a frame and rows.
    for (off, len) in [(0, 10), (P - 3, 7), (15 * P + 100, P), (16 * P - 1, 2), (31 * P + 5, 2 * P), (33, 20 * P), (0, data.len())] {
        let (at, got) = fs.read_at(Z, N, off as i64, len as i64).unwrap();
        assert_eq!(at, off as i64);
        assert_eq!(got, &data[off..(off + len).min(data.len())], "read_at {off} {len}");
        let snap = fs.read_snapshot(|r| r.bytes(Z, N, off as i64, len.min(data.len() - off) as i64)).unwrap().unwrap();
        assert_eq!(snap, &data[off..(off + len).min(data.len())], "snapshot {off} {len}");
    }
    let (raw, stored, count) = fs.frame_totals().unwrap();
    assert_eq!((raw, count), (32 * P as i64, 2));
    assert!(stored < raw / 2, "compressed: {stored} of {raw}");
}

#[test]
fn framing_keeps_the_line_count() {
    let fs = mem();
    let data = transcript(20 * P, 3);
    make(&fs, Z, &data);
    let before = fs.line_state(Z, N).unwrap().unwrap().counted.expect("counted");
    assert!(matches!(fs.frame_parts(Z, N, 2, 16, CODEC_ZSTD).unwrap(), FrameOutcome::Framed { .. }));
    let after = fs.line_state(Z, N).unwrap().unwrap().counted.expect("still counted: the bytes didn't change");
    assert_eq!((after.lines, &after.gen), (before.lines, &before.gen));
    assert_eq!(fs.unframe_file(Z, N).unwrap(), 1);
    let back = fs.line_state(Z, N).unwrap().unwrap().counted.expect("still counted after unframing");
    assert_eq!((back.lines, &back.gen), (before.lines, &before.gen));
    assert_eq!(all(&fs, Z), data);
}

#[test]
fn appends_land_after_the_frames() {
    let (fs, mut data) = framed_store();
    let more = transcript(3 * P, 11);
    fs.append_lines(Z, N, &more).unwrap();
    data.extend_from_slice(&more);
    assert_eq!(all(&fs, Z), data);
    assert_eq!(frames_of(&fs, Z), 2);
    let counted = fs.line_state(Z, N).unwrap().unwrap().counted.unwrap();
    assert_eq!(counted.lines as usize, data.iter().filter(|&&b| b == b'\n').count());
}

#[test]
fn a_write_into_a_frame_turns_it_back_into_rows() {
    let (fs, mut data) = framed_store();
    let at = 20 * P + 17;
    fs.write_at(Z, N, at as i64, b"XYZ").unwrap();
    data[at..at + 3].copy_from_slice(b"XYZ");
    assert_eq!(all(&fs, Z), data);
    assert_eq!(frames_of(&fs, Z), 1, "only the frame written into is undone");
}

#[test]
fn a_truncate_inside_a_frame_keeps_what_comes_before() {
    let (fs, data) = framed_store();
    let new_size = (20 * P + 1234) as i64;
    fs.write_txn(|tx| {
        super::super::replace::truncate_parts_in(tx, Z, N, new_size)?;
        tx.execute("UPDATE db_wave_file SET size = ?1 WHERE zoneid = ?2 AND name = ?3", params![new_size, Z, N])?;
        Ok(())
    })
    .unwrap();
    fs.drop_cache_for_test();
    assert_eq!(frames_of(&fs, Z), 1, "the frame past the cut is gone, the one cut into is rows");
    assert_eq!(fs.read_bytes_db(Z, N, 0, new_size).unwrap(), &data[..new_size as usize]);
    fs.append_lines(Z, N, b"{\"after\":1}\n").unwrap();
    let got = all(&fs, Z);
    assert!(got.starts_with(&data[..new_size as usize]));
    assert!(got.ends_with(b"{\"after\":1}\n"));
}

#[test]
fn replacing_or_deleting_a_file_takes_its_frames() {
    let (fs, _) = framed_store();
    fs.replace_file(Z, N, b"{\"new\":true}\n", &[]).unwrap();
    assert_eq!(frames_of(&fs, Z), 0);
    assert_eq!(fs.read_file(Z, N).unwrap().unwrap(), b"{\"new\":true}\n");

    let (fs, _) = framed_store();
    fs.delete_file(Z, N).unwrap();
    assert_eq!(frames_of(&fs, Z), 0);
    let (fs, _) = framed_store();
    fs.delete_files(Z, &[N]).unwrap();
    assert_eq!(frames_of(&fs, Z), 0);
    let (fs, _) = framed_store();
    fs.delete_zone(Z).unwrap();
    assert_eq!(frames_of(&fs, Z), 0);
}

#[test]
fn copies_and_moves_take_the_frames_with_them() {
    let (fs, data) = framed_store();
    let (b, c, d) = ("agent:b:current", "agent:c:current", "agent:d:archive:1");

    fs.copy_files_to_zone(Z, b, &[N]).unwrap().unwrap();
    assert_eq!(all(&fs, b), data);
    assert_eq!(frames_of(&fs, b), 2);

    // A prefix that ends inside the second frame: the first frame is copied
    // as a frame, the second's parts up to the cut become rows.
    let cut = data[..20 * P + 500].iter().rposition(|&x| x == b'\n').unwrap() + 1;
    fs.copy_prefixes_to_zone(Z, c, &[(N, cut as i64)]).unwrap().unwrap();
    assert_eq!(all(&fs, c), &data[..cut]);
    assert_eq!(frames_of(&fs, c), 1);
    fs.append_lines(c, N, b"{\"fork\":1}\n").unwrap();
    assert_eq!(all(&fs, Z), data, "the source is untouched");

    fs.move_files_to_zone(Z, d, &[N]).unwrap().unwrap();
    assert_eq!(all(&fs, d), data);
    assert_eq!((frames_of(&fs, d), frames_of(&fs, Z)), (2, 0));
}

#[test]
fn a_damaged_frame_reads_as_blank_and_the_rest_as_written() {
    let (fs, data) = framed_store();
    fs.lock_conn()
        .execute("UPDATE db_file_frame SET data = zeroblob(100) WHERE zoneid = ?1 AND first_part = 16", params![Z])
        .unwrap();
    let mut expect = data.clone();
    expect[16 * P..32 * P].fill(b' ');
    assert_eq!(fs.read_file(Z, N).unwrap().unwrap(), expect, "read_file");
    assert_eq!(all(&fs, Z), expect, "read_bytes_db");
    // A frame whose bytes decode but don't match its hash is damaged too.
    let (fs, data) = framed_store();
    let other = encode(CODEC_ZSTD, &vec![b'x'; 16 * P]).unwrap();
    fs.lock_conn()
        .execute("UPDATE db_file_frame SET data = ?1 WHERE zoneid = ?2 AND first_part = 0", params![other, Z])
        .unwrap();
    let got = fs.read_file(Z, N).unwrap().unwrap();
    assert!(got[..16 * P].iter().all(|&b| b == b' '));
    assert_eq!(&got[16 * P..], &data[16 * P..]);
}

#[test]
fn framing_skips_what_it_must_not_touch() {
    let fs = mem();
    let data = transcript(20 * P + 10, 5);
    make(&fs, Z, &data);
    let last = ((data.len() - 1) / P) as i32;
    assert_eq!(fs.frame_parts(Z, N, last - 3, 4, CODEC_ZSTD).unwrap(), FrameOutcome::Skipped("reaches the file's last part"));
    assert_eq!(fs.frame_parts("nope", N, 0, 4, CODEC_ZSTD).unwrap(), FrameOutcome::Skipped("no such file"));
    assert!(matches!(fs.frame_parts(Z, N, 0, 4, CODEC_ZSTD).unwrap(), FrameOutcome::Framed { .. }));
    assert_eq!(fs.frame_parts(Z, N, 2, 4, CODEC_ZSTD).unwrap(), FrameOutcome::Skipped("already framed"));
    fs.lock_conn().execute("DELETE FROM db_file_data WHERE zoneid = ?1 AND partidx = 9", params![Z]).unwrap();
    assert_eq!(fs.frame_parts(Z, N, 8, 4, CODEC_ZSTD).unwrap(), FrameOutcome::Skipped("a part is missing or short"));
    fs.lock_conn().execute("UPDATE db_file_data SET data = x'41' WHERE zoneid = ?1 AND partidx = 13", params![Z]).unwrap();
    assert_eq!(fs.frame_parts(Z, N, 12, 4, CODEC_ZSTD).unwrap(), FrameOutcome::Skipped("a part is missing or short"));
    assert_eq!(fs.frame_parts(Z, N, 4, 0, CODEC_ZSTD).unwrap(), FrameOutcome::Skipped("bad range"));
}

#[test]
fn a_store_without_the_frame_table_reads_as_before() {
    let conn = Connection::open_in_memory().unwrap();
    let mut parts = HashMap::new();
    fill_parts(&conn, Z, N, 0, 3, &mut parts, false).unwrap();
    assert!(parts.is_empty());
    assert_eq!(unframe(&conn, Z, N, 0, 3).unwrap(), 0);
}

#[derive(Debug, Clone)]
enum Op {
    Append(usize, u64),
    Frame(u8, u8),
    Unframe,
    Write(usize, u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (1usize..200_000, any::<u64>()).prop_map(|(n, s)| Op::Append(n, s)),
        (0u8..40, 1u8..17).prop_map(|(f, c)| Op::Frame(f, c)),
        Just(Op::Unframe),
        (0usize..3_000_000, any::<u8>()).prop_map(|(at, b)| Op::Write(at, b)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 20, ..ProptestConfig::default() })]

    /// Appends, framings, unframings and writes in any order: the file reads
    /// back as the model says, whole and in pieces.
    #[test]
    fn any_mix_of_writes_and_framing_reads_back_exactly(seed in any::<u64>(), ops in prop::collection::vec(op(), 1..14)) {
        let fs = mem();
        let mut model = transcript(24 * P, seed);
        make(&fs, Z, &model);
        for op in ops {
            match op {
                Op::Append(n, s) => {
                    let more = transcript(n, s);
                    fs.append_lines(Z, N, &more).unwrap();
                    model.extend_from_slice(&more);
                }
                Op::Frame(first, count) => {
                    let _ = fs.frame_parts(Z, N, first as i32, count as i32, CODEC_ZSTD).unwrap();
                }
                Op::Unframe => {
                    fs.unframe_file(Z, N).unwrap();
                }
                Op::Write(at, b) => {
                    let at = at % (model.len() - 1); // never the last byte: a torn tail changes how the next append starts
                    fs.write_at(Z, N, at as i64, &[b]).unwrap();
                    model[at] = b;
                }
            }
            prop_assert_eq!(all(&fs, Z), model.clone());
        }
        prop_assert_eq!(fs.read_file(Z, N).unwrap().unwrap(), model.clone());
        for k in 0..8u64 {
            let off = ((seed ^ k.wrapping_mul(0x9e37_79b9)) as usize) % model.len();
            let len = (P / 3 + k as usize * 40_000).min(model.len() - off);
            let (_, got) = fs.read_at(Z, N, off as i64, len as i64).unwrap();
            prop_assert_eq!(&got[..], &model[off..off + len]);
        }
    }
}

/// Every query of `db_file_data` must know that a part may be in a frame. A
/// new one outside these files would read a framed part as missing: add it
/// here only after it reads through `parts.rs` or `frames.rs`.
#[test]
fn only_known_code_reads_the_part_table_directly() {
    const KNOWN: &[&str] = &[
        "backend/storage/filestore/copy.rs",
        "backend/storage/filestore/core.rs",
        "backend/storage/filestore/counter.rs",
        "backend/storage/filestore/frames.rs",
        "backend/storage/filestore/offset_ops.rs",
        "backend/storage/filestore/parts.rs",
        "backend/storage/filestore/replace.rs",
        "backend/storage/migrations.rs",
        "backend/transcript/damage.rs",
        // Reads per-channel stores, which are never framed.
        "backend/transcript_backfill.rs",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root.clone()];
    let mut unknown = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            let file = path.file_name().unwrap().to_string_lossy();
            let is_test = file.starts_with("tests") || file.ends_with("_tests.rs") || file.ends_with("_test.rs");
            if !rel.ends_with(".rs") || is_test || KNOWN.contains(&rel.as_str()) {
                continue;
            }
            if std::fs::read_to_string(&path).unwrap_or_default().contains("db_file_data") {
                unknown.push(rel);
            }
        }
    }
    assert!(unknown.is_empty(), "these read db_file_data without knowing about frames: {unknown:?}");
}

/// The interleaving review found: a reader sees a part with no row (it is in
/// a frame), another connection unframes it (rows back, frame gone), and the
/// reader's frame lookup then finds nothing. In one snapshot the reader still
/// sees the frame it started with.
#[test]
fn rows_and_frames_are_read_in_one_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let fs = FileStore::open(&path).unwrap();
    let data = transcript(20 * P, 21);
    make(&fs, Z, &data);
    assert!(matches!(fs.frame_parts(Z, N, 0, 16, CODEC_ZSTD).unwrap(), FrameOutcome::Framed { .. }));
    fs.checkpoint().unwrap();

    let reader = Connection::open(&path).unwrap();
    let got = in_snapshot(&reader, || {
        let rows: i64 = reader.query_row(
            "SELECT COUNT(*) FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx < 16",
            params![Z, N],
            |r| r.get(0),
        )?;
        assert_eq!(rows, 0, "the parts are in the frame");
        // Another connection turns the frame back into rows meanwhile.
        assert_eq!(fs.unframe_file(Z, N).unwrap(), 1);
        let mut parts = HashMap::new();
        fill_parts(&reader, Z, N, 0, 15, &mut parts, false)?;
        Ok(parts.len())
    })
    .unwrap();
    assert_eq!(got, 16, "the reader's snapshot still has the frame");
    // Outside the snapshot the rows are there, and the bytes are the same.
    assert_eq!(all(&fs, Z), data);
}
