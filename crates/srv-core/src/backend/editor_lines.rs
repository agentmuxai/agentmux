// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `fileindexlines`: where the lines of a text file start, for the Large text viewer. The file is read here, in
//! blocks, once per `(path, modified time)`: what is kept is the byte offset of every `STEP`-th line, so the
//! viewer asks for "lines 5,000,000 to 5,000,100" and gets the byte range to fetch, and a file bigger than memory
//! never leaves srv. A line ends at `\n` (so `\r\n` and `\n` files both work); in UTF-16 it ends at the two-byte
//! `\n`. A final line with no newline is a line; a newline at the very end starts none.
//!
//! docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{FileIndexLinesReq, FileIndexLinesResult};

/// One read.
const BLOCK: usize = 1 << 20;
/// The index keeps the offset of every this-many-th line.
const STEP: u64 = 1024;
/// The most lines one call may span, so an answer is a screenful, not a file.
const MAX_COUNT: u64 = 100_000;
/// The indexes kept (the most recently used).
const CACHE_ENTRIES: usize = 6;

/// How a file's text marks the end of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// One byte, `\n`: UTF-8, and the legacy single-byte encodings.
    Byte,
    /// Two bytes, `0A 00`.
    Utf16Le,
    /// Two bytes, `00 0A`.
    Utf16Be,
}

impl Unit {
    /// From a probe's `encoding`; anything else is an error naming it.
    pub fn parse(encoding: &str) -> Result<Unit, String> {
        match encoding {
            "" | "utf-8" | "legacy" => Ok(Unit::Byte),
            "utf-16le" => Ok(Unit::Utf16Le),
            "utf-16be" => Ok(Unit::Utf16Be),
            other => Err(format!("an encoding of utf-8, utf-16le or utf-16be, not {other:?}")),
        }
    }
}

/// Where the lines of one file start, as of one `(size, mtime)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    pub size: u64,
    pub mtime: i64,
    pub total_lines: u64,
    pub step: u64,
    /// The start of line 0, `step`, `2 * step`, ...
    pub checkpoints: Vec<u64>,
}

/// The offsets just after each line break in `block` (which starts at file offset `base`, an even number for UTF-16).
fn breaks_in(block: &[u8], base: u64, unit: Unit, mut each: impl FnMut(u64)) {
    match unit {
        Unit::Byte => memchr::memchr_iter(b'\n', block).for_each(|i| each(base + i as u64 + 1)),
        Unit::Utf16Le | Unit::Utf16Be => {
            let (hi, lo) = if unit == Unit::Utf16Le { (1, 0) } else { (0, 1) };
            for (i, pair) in block.chunks_exact(2).enumerate() {
                if pair[lo] == b'\n' && pair[hi] == 0 {
                    each(base + (i as u64) * 2 + 2);
                }
            }
        }
    }
}

/// A read size that is a whole number of code units.
fn block_size(unit: Unit, block: usize) -> usize {
    if unit == Unit::Byte {
        block.max(1)
    } else {
        (block & !1).max(2)
    }
}

/// Reads `file` (`size` bytes) front to back and notes where every `step`-th line starts.
pub fn build_index<R: Read>(file: &mut R, size: u64, mtime: i64, unit: Unit, step: u64, block: usize) -> std::io::Result<LineIndex> {
    let block = block_size(unit, block);
    let mut buf = vec![0u8; block];
    let mut checkpoints = Vec::new();
    let mut lines = 0u64;
    let mut note_line = |start: u64, lines: &mut u64| {
        if *lines % step == 0 {
            checkpoints.push(start);
        }
        *lines += 1;
    };
    if size > 0 {
        note_line(0, &mut lines);
    }
    let mut at = 0u64;
    while at < size {
        let want = (size - at).min(block as u64) as usize;
        file.read_exact(&mut buf[..want])?;
        // A line starts after a break unless the file ends there.
        breaks_in(&buf[..want], at, unit, |end| {
            if end < size {
                note_line(end, &mut lines);
            }
        });
        at += want as u64;
    }
    Ok(LineIndex { size, mtime, total_lines: lines, step, checkpoints })
}

/// The offset `n` lines after the line that starts at `from` (the size, when the file has fewer lines).
fn skip_lines<F: Read + Seek>(file: &mut F, from: u64, n: u64, size: u64, unit: Unit, block: usize) -> std::io::Result<u64> {
    if n == 0 {
        return Ok(from.min(size));
    }
    let block = block_size(unit, block);
    let mut buf = vec![0u8; block];
    let (mut at, mut left) = (from, n);
    file.seek(SeekFrom::Start(from))?;
    while at < size {
        let want = (size - at).min(block as u64) as usize;
        file.read_exact(&mut buf[..want])?;
        let mut found = None;
        breaks_in(&buf[..want], at, unit, |end| {
            if found.is_none() {
                left -= 1;
                if left == 0 {
                    found = Some(end);
                }
            }
        });
        if let Some(end) = found {
            return Ok(end.min(size));
        }
        at += want as u64;
    }
    Ok(size)
}

/// Where line `line` (0-based) starts; the size for a line past the last.
pub fn line_start<F: Read + Seek>(file: &mut F, idx: &LineIndex, line: u64, unit: Unit, block: usize) -> std::io::Result<u64> {
    if line >= idx.total_lines {
        return Ok(idx.size);
    }
    let Some(&checkpoint) = idx.checkpoints.get((line / idx.step) as usize) else { return Ok(idx.size) };
    skip_lines(file, checkpoint, line % idx.step, idx.size, unit, block)
}

type CacheKey = (PathBuf, u8);

fn cache() -> &'static Mutex<Vec<(CacheKey, Arc<LineIndex>)>> {
    static CACHE: OnceLock<Mutex<Vec<(CacheKey, Arc<LineIndex>)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The index of `path` as it is now: the one kept when the file has not changed, else a new one.
fn index_of(path: &Path, unit: Unit) -> std::io::Result<Arc<LineIndex>> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Err(std::io::Error::other("is a directory"));
    }
    let (size, mtime) = (meta.len(), mtime_ms(&meta));
    let key: CacheKey = (path.to_path_buf(), unit as u8);
    {
        let mut kept = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = kept.iter().position(|(k, v)| *k == key && v.size == size && v.mtime == mtime) {
            let hit = kept.remove(i);
            let idx = hit.1.clone();
            kept.insert(0, hit);
            return Ok(idx);
        }
    }
    let mut file = std::fs::File::open(path)?;
    let built = Arc::new(build_index(&mut file, size, mtime, unit, STEP, BLOCK)?);
    let mut kept = cache().lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|(k, _)| *k != key);
    kept.insert(0, (key, built.clone()));
    kept.truncate(CACHE_ENTRIES);
    Ok(built)
}

/// The answer for `count` lines from line `from_line` of `path`.
fn lines_of(path: &Path, unit: Unit, from_line: u64, count: u64) -> std::io::Result<FileIndexLinesResult> {
    let idx = index_of(path, unit)?;
    let mut file = std::fs::File::open(path)?;
    let start = line_start(&mut file, &idx, from_line, unit, BLOCK)?;
    let wanted = count.min(idx.total_lines.saturating_sub(from_line));
    let end = if from_line.saturating_add(count) >= idx.total_lines { idx.size } else { skip_lines(&mut file, start, wanted, idx.size, unit, BLOCK)? };
    Ok(FileIndexLinesResult { total_lines: idx.total_lines, size: idx.size, mtime: idx.mtime, start, end })
}

/// `fileindexlines`. This computer's files only, like `fileprobe`.
pub fn register(engine: &std::sync::Arc<WshRpcEngine>) {
    engine.register_typed("fileindexlines", move |cmd: FileIndexLinesReq, _ctx| async move {
        let unit = Unit::parse(cmd.encoding.as_deref().unwrap_or("")).map_err(|e| format!("fileindexlines: {e}"))?;
        if cmd.count > MAX_COUNT {
            return Err(format!("fileindexlines: at most {MAX_COUNT} lines a call"));
        }
        let path = expand_home_dir_safe(&cmd.path);
        tokio::task::spawn_blocking(move || lines_of(&path, unit, cmd.from_line, cmd.count).map_err(|e| format!("fileindexlines: {e}")))
            .await
            .map_err(|e| format!("fileindexlines: {e}"))?
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// The index of `data` with a step of 3 and blocks of 5 bytes, so checkpoints and block edges both get crossed.
    fn small(data: &[u8], unit: Unit) -> LineIndex {
        build_index(&mut Cursor::new(data.to_vec()), data.len() as u64, 7, unit, 3, 5).unwrap()
    }

    fn start_of(data: &[u8], idx: &LineIndex, line: u64, unit: Unit) -> u64 {
        line_start(&mut Cursor::new(data.to_vec()), idx, line, unit, 5).unwrap()
    }

    /// Every line's start, by walking the text the slow way.
    fn starts(data: &[u8]) -> Vec<u64> {
        let mut out = Vec::new();
        if !data.is_empty() {
            out.push(0);
        }
        for (i, b) in data.iter().enumerate() {
            if *b == b'\n' && i + 1 < data.len() {
                out.push(i as u64 + 1);
            }
        }
        out
    }

    #[test]
    fn counts_lines_of_lf_crlf_and_mixed_text_with_or_without_a_final_break() {
        for text in ["a\nbb\nccc\ndddd\n", "a\nbb\nccc\ndddd", "a\r\nbb\r\nccc\r\ndddd\r\n", "a\nbb\r\nccc\ndddd\r\n", "\n\n\n", "one"] {
            let data = text.as_bytes();
            let idx = small(data, Unit::Byte);
            assert_eq!(idx.total_lines, starts(data).len() as u64, "{text:?}");
            for (n, want) in starts(data).into_iter().enumerate() {
                assert_eq!(start_of(data, &idx, n as u64, Unit::Byte), want, "{text:?} line {n}");
            }
            assert_eq!(start_of(data, &idx, idx.total_lines, Unit::Byte), data.len() as u64, "{text:?} past the end");
            assert_eq!(start_of(data, &idx, idx.total_lines + 50, Unit::Byte), data.len() as u64);
        }
    }

    #[test]
    fn an_empty_file_has_no_lines_and_a_lone_break_has_one() {
        let idx = small(b"", Unit::Byte);
        assert_eq!((idx.total_lines, idx.checkpoints.len()), (0, 0));
        assert_eq!(start_of(b"", &idx, 0, Unit::Byte), 0);
        assert_eq!(small(b"\n", Unit::Byte).total_lines, 1);
    }

    #[test]
    fn keeps_the_start_of_every_stepth_line() {
        let data = b"0\n1\n2\n3\n4\n5\n6\n7\n";
        let idx = small(data, Unit::Byte);
        assert_eq!(idx.total_lines, 8);
        assert_eq!(idx.checkpoints, vec![0, 6, 12]);
    }

    #[test]
    fn indexes_utf16_by_its_two_byte_breaks() {
        let text = "ab\ncd\r\nef\n\u{0a0a}x";
        for (unit, bytes) in [
            (Unit::Utf16Le, text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>()),
            (Unit::Utf16Be, text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>()),
        ] {
            let idx = small(&bytes, unit);
            // The text has three breaks, so four lines; U+0A0A holds a 0A byte that is no break.
            assert_eq!(idx.total_lines, 4, "{unit:?}");
            let units: Vec<u64> = text.encode_utf16().scan(0u64, |at, u| { let here = *at; *at += 2; Some((here, u)) }).filter(|(_, u)| *u == 0x0a).map(|(at, _)| at + 2).collect();
            assert_eq!(start_of(&bytes, &idx, 1, unit), units[0], "{unit:?}");
            assert_eq!(start_of(&bytes, &idx, 3, unit), units[2], "{unit:?}");
        }
    }

    #[test]
    fn a_five_million_line_file_jumps_to_its_last_line() {
        let data: Vec<u8> = b"x\n".repeat(5_000_000);
        let idx = build_index(&mut Cursor::new(&data[..]), data.len() as u64, 0, Unit::Byte, STEP, BLOCK).unwrap();
        assert_eq!(idx.total_lines, 5_000_000);
        assert_eq!(idx.checkpoints.len(), 5_000_000usize.div_ceil(STEP as usize));
        let at = |line| line_start(&mut Cursor::new(&data[..]), &idx, line, Unit::Byte, BLOCK).unwrap();
        assert_eq!((at(0), at(1), at(4_999_999), at(2_500_000)), (0, 2, 9_999_998, 5_000_000));
        assert_eq!(at(5_000_000), 10_000_000);
    }

    #[test]
    fn the_answer_is_the_byte_range_of_the_lines_asked_for() {
        let dir = std::env::temp_dir().join(format!("fileindexlines-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.txt");
        std::fs::write(&path, "a\nbb\nccc\ndddd\n").unwrap();
        let r = lines_of(&path, Unit::Byte, 1, 2).unwrap();
        assert_eq!((r.total_lines, r.size, r.start, r.end), (4, 14, 2, 9));
        // Past the last line: the rest of the file, then nothing.
        let r = lines_of(&path, Unit::Byte, 3, 10).unwrap();
        assert_eq!((r.start, r.end), (9, 14));
        let r = lines_of(&path, Unit::Byte, 9, 10).unwrap();
        assert_eq!((r.start, r.end), (14, 14));
        let r = lines_of(&path, Unit::Byte, 0, 0).unwrap();
        assert_eq!((r.start, r.end), (0, 0));
        // The file grew: the kept index is not reused.
        std::fs::write(&path, "a\nbb\nccc\ndddd\neeeee\nf").unwrap();
        let r = lines_of(&path, Unit::Byte, 0, 100).unwrap();
        assert_eq!((r.total_lines, r.size), (6, 21));
        assert!(lines_of(&dir, Unit::Byte, 0, 1).is_err(), "a folder is no file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_an_encoding_it_cannot_index() {
        assert_eq!(Unit::parse("utf-8"), Ok(Unit::Byte));
        assert_eq!(Unit::parse("utf-16be"), Ok(Unit::Utf16Be));
        assert!(Unit::parse("shift-jis").unwrap_err().contains("shift-jis"));
    }
}
