// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The jekt audit file (`reactive::audit_store`), read from its end: the file is never rotated, so a page
//! of the newest entries reads the tail of it, and a cursor says where the next page's lines end.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::backend::reactive::types::AuditLogEntry;

/// How much of the file is read at a time.
const CHUNK: u64 = 64 * 1024;

/// One line of the file: where it starts and ends (past its newline), and the raw bytes without the newline.
pub(super) struct RawLine {
    pub start: u64,
    pub bytes: Vec<u8>,
}

/// The lines of a file that end at or before `end`, last first.
pub(super) struct ReverseLines {
    file: File,
    /// The bytes `[pos, edge)` read so far and not yet given out.
    buf: Vec<u8>,
    pos: u64,
    chunk: u64,
}

impl ReverseLines {
    /// `end`: where the lines may end (a line boundary); None is the end of the file.
    pub fn open(path: &Path, end: Option<u64>) -> std::io::Result<Self> {
        Self::open_with(path, end, CHUNK)
    }

    fn open_with(path: &Path, end: Option<u64>, chunk: u64) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let edge = end.map_or(len, |e| e.min(len));
        Ok(Self { file, buf: Vec::new(), pos: edge, chunk: chunk.max(1) })
    }

    fn read_before(&mut self) -> std::io::Result<()> {
        let from = self.pos.saturating_sub(self.chunk);
        let mut more = vec![0u8; (self.pos - from) as usize];
        self.file.seek(SeekFrom::Start(from))?;
        self.file.read_exact(&mut more)?;
        more.extend_from_slice(&self.buf);
        self.buf = more;
        self.pos = from;
        Ok(())
    }
}

impl Iterator for ReverseLines {
    type Item = std::io::Result<RawLine>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Past a trailing newline there is nothing: the line before it ends there.
            let body_end = if self.buf.last() == Some(&b'\n') { self.buf.len() - 1 } else { self.buf.len() };
            if let Some(i) = self.buf[..body_end].iter().rposition(|b| *b == b'\n') {
                let line = RawLine { start: self.pos + i as u64 + 1, bytes: self.buf[i + 1..body_end].to_vec() };
                self.buf.truncate(i + 1);
                return Some(Ok(line));
            }
            if self.pos == 0 {
                if self.buf.is_empty() {
                    return None;
                }
                let line = RawLine { start: 0, bytes: self.buf[..body_end].to_vec() };
                self.buf.clear();
                return Some(Ok(line));
            }
            if let Err(e) = self.read_before() {
                // Nothing more is read after an error: asking again would fail the same way.
                self.pos = 0;
                self.buf.clear();
                return Some(Err(e));
            }
        }
    }
}

/// An entry of the file, with where its line starts and where it ends (the next line's start).
pub(super) fn entries(path: &Path, end: Option<u64>) -> impl Iterator<Item = (u64, u64, AuditLogEntry)> {
    ReverseLines::open(path, end).into_iter().flatten().map_while(Result::ok).filter_map(|line| {
        let to = line.start + line.bytes.len() as u64 + 1;
        serde_json::from_slice::<AuditLogEntry>(&line.bytes).ok().map(|e| (line.start, to, e))
    })
}

/// Calls `each` with every entry from byte `from` on, oldest first, and returns where the last complete line
/// ends: a line the writer hasn't finished isn't read, and is the next scan's first.
pub(super) fn scan(path: &Path, from: u64, mut each: impl FnMut(AuditLogEntry)) -> std::io::Result<u64> {
    use std::io::BufRead;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut reader = std::io::BufReader::new(file);
    let (mut at, mut line) = (from, Vec::new());
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            return Ok(at);
        }
        at += n as u64;
        if let Ok(entry) = serde_json::from_slice::<AuditLogEntry>(&line) {
            each(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_of(lines: &[&str], trailing_newline: bool) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jekt-audit.jsonl");
        let mut text = lines.join("\n");
        if trailing_newline {
            text.push('\n');
        }
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    fn all(path: &Path, end: Option<u64>, chunk: u64) -> Vec<(u64, String)> {
        ReverseLines::open_with(path, end, chunk)
            .unwrap()
            .map(|l| l.unwrap())
            .map(|l| (l.start, String::from_utf8(l.bytes).unwrap()))
            .collect()
    }

    #[test]
    fn lines_come_last_first_whatever_the_chunk_size() {
        let lines = ["alpha", "bb", "", "gamma-gamma-gamma", "d"];
        for trailing in [true, false] {
            let (_dir, path) = file_of(&lines, trailing);
            for chunk in [1, 2, 3, 7, 64] {
                let got = all(&path, None, chunk);
                let want: Vec<&str> = lines.iter().rev().copied().collect();
                assert_eq!(got.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), want, "chunk {chunk}, trailing {trailing}");
            }
        }
    }

    #[test]
    fn a_line_start_is_where_a_later_read_can_end() {
        let (_dir, path) = file_of(&["one", "two", "three"], true);
        let got = all(&path, None, 4);
        assert_eq!(got[0], (8, "three".to_string()));
        // Reading from the start of "three" gives what came before it.
        let before = all(&path, Some(8), 4);
        assert_eq!(before.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), ["two", "one"]);
        assert!(all(&path, Some(0), 4).is_empty());
    }

    #[test]
    fn a_read_error_ends_the_lines_instead_of_repeating() {
        let (_dir, path) = file_of(&["alpha", "bravo", "charlie", "delta"], true);
        let mut lines = ReverseLines::open_with(&path, None, 4).unwrap();
        // The file shrinks after it was measured: reading back past the new end fails.
        std::fs::write(&path, "al").unwrap();
        let got: Vec<_> = lines.by_ref().take(10).collect();
        assert!(got.iter().any(|l| l.is_err()), "the error is reported once");
        assert!(got.len() < 10 && lines.next().is_none(), "and then there is nothing more");
    }

    #[test]
    fn a_scan_stops_before_a_line_the_writer_has_not_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        let line = |ts: u64| format!("{}\n", serde_json::json!({"timestamp": ts, "target_agent": "x", "block_id": "", "message_hash": "", "message_length": 0, "success": true, "request_id": ""}));
        std::fs::write(&path, format!("{}{}{{\"timestamp\":3", line(1), line(2))).unwrap();
        let mut seen = Vec::new();
        let end = scan(&path, 0, |e| seen.push(e.timestamp)).unwrap();
        assert_eq!((seen.clone(), end), (vec![1, 2], (line(1).len() + line(2).len()) as u64));
        // Finished, it is read from where the scan stopped.
        std::fs::write(&path, format!("{}{}{}", line(1), line(2), line(3))).unwrap();
        seen.clear();
        scan(&path, end, |e| seen.push(e.timestamp)).unwrap();
        assert_eq!(seen, [3]);
    }

    #[test]
    fn an_empty_or_missing_file_has_no_lines() {
        let (_dir, path) = file_of(&[], false);
        assert!(all(&path, None, 4).is_empty());
        assert_eq!(entries(&path.with_file_name("nope"), None).count(), 0);
    }

    #[test]
    fn a_line_that_is_not_an_entry_is_skipped() {
        let (_dir, path) = file_of(&["{not json", "{\"x\":1}"], true);
        assert_eq!(entries(&path, None).count(), 0);
    }
}
