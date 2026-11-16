// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `filesearchbytes`: the next match of a byte pattern in a file, for the hex
//! viewer's find. The file is read here, in blocks, so one bigger than memory
//! can be searched; each call does a bounded amount of work and says where to
//! go on from, so a search of a very large file never holds a worker.
//!
//! docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{FileSearchBytesReq, FileSearchBytesResult};

/// The longest pattern.
const MAX_PATTERN: usize = 256;
/// One read.
const BLOCK: u64 = 1 << 20;
/// The most a call reads before it hands back where it got to.
const BUDGET_BYTES: u64 = 256 << 20;
/// The longest a call goes on before it hands back where it got to.
const BUDGET_TIME: Duration = Duration::from_millis(1500);

/// A pattern of bytes, some of which match any byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    bytes: Vec<u8>,
    /// Per byte: it must match (false for `??`).
    literal: Vec<bool>,
}

impl Pattern {
    /// "DE AD ?? EF", with or without the spaces.
    pub fn parse(text: &str) -> Result<Pattern, String> {
        let digits: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        if digits.is_empty() {
            return Err("the pattern is empty".into());
        }
        if digits.len() % 2 != 0 {
            return Err("a pattern is whole bytes: two hex digits each, or ??".into());
        }
        let mut bytes = Vec::new();
        let mut literal = Vec::new();
        for pair in digits.chunks(2) {
            match (pair[0], pair[1]) {
                ('?', '?') => {
                    bytes.push(0);
                    literal.push(false);
                }
                (a, b) => {
                    let hex = |c: char| c.to_digit(16).ok_or_else(|| format!("'{c}' is not a hex digit"));
                    bytes.push((hex(a)? * 16 + hex(b)?) as u8);
                    literal.push(true);
                }
            }
        }
        if bytes.len() > MAX_PATTERN {
            return Err(format!("a pattern is at most {MAX_PATTERN} bytes"));
        }
        if !literal.iter().any(|l| *l) {
            return Err("a pattern of only ?? matches everywhere".into());
        }
        Ok(Pattern { bytes, literal })
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The index of the byte the scan looks for with memchr: the first one that must match.
    fn anchor(&self) -> usize {
        self.literal.iter().position(|l| *l).unwrap_or(0)
    }

    /// Does the pattern match `window` (exactly its length)?
    fn matches(&self, window: &[u8]) -> bool {
        window.iter().zip(&self.bytes).zip(&self.literal).all(|((w, b), lit)| !lit || w == b)
    }
}

/// Every start in `buf[..starts]` where the pattern matches, first or last
/// of them. `buf` holds at least `starts - 1 + pattern.len()` bytes.
fn scan(buf: &[u8], starts: usize, pat: &Pattern, last: bool) -> Option<usize> {
    let k = pat.anchor();
    let lit = pat.bytes[k];
    let hay = &buf[k..k + starts];
    let hit = |i: &usize| pat.matches(&buf[*i..*i + pat.len()]);
    if last {
        memchr::memrchr_iter(lit, hay).find(hit)
    } else {
        memchr::memchr_iter(lit, hay).find(hit)
    }
}

/// The search over `file` (`size` bytes). Forward: the first match starting
/// at or after `from`; backward: the last one starting at or before it.
pub fn search<F: Read + Seek>(
    file: &mut F,
    size: u64,
    pat: &Pattern,
    from: u64,
    backward: bool,
    budget_bytes: u64,
    budget_time: Duration,
) -> std::io::Result<FileSearchBytesResult> {
    let plen = pat.len() as u64;
    let length = plen;
    let not_found = |done: bool, next_from: u64| FileSearchBytesResult { found: false, offset: 0, length, done, next_from };
    if size < plen {
        return Ok(not_found(true, 0));
    }
    // The last place a match can start.
    let last_start = size - plen;
    let started = Instant::now();
    let mut read = 0u64;
    let mut buf = Vec::new();
    if !backward {
        let mut at = from;
        while at <= last_start {
            let starts = BLOCK.min(last_start - at + 1);
            let want = (starts - 1 + plen) as usize;
            buf.resize(want, 0);
            file.seek(SeekFrom::Start(at))?;
            file.read_exact(&mut buf)?;
            if let Some(i) = scan(&buf, starts as usize, pat, false) {
                return Ok(FileSearchBytesResult { found: true, offset: at + i as u64, length, done: false, next_from: 0 });
            }
            at += starts;
            read += want as u64;
            if at <= last_start && (read >= budget_bytes || started.elapsed() >= budget_time) {
                return Ok(not_found(false, at));
            }
        }
        return Ok(not_found(true, size));
    }
    // Backward: windows of starts [lo, hi], going down.
    let mut hi = from.min(last_start);
    loop {
        let lo = hi.saturating_sub(BLOCK - 1);
        let starts = hi - lo + 1;
        let want = (starts - 1 + plen) as usize;
        buf.resize(want, 0);
        file.seek(SeekFrom::Start(lo))?;
        file.read_exact(&mut buf)?;
        if let Some(i) = scan(&buf, starts as usize, pat, true) {
            return Ok(FileSearchBytesResult { found: true, offset: lo + i as u64, length, done: false, next_from: 0 });
        }
        if lo == 0 {
            return Ok(not_found(true, 0));
        }
        hi = lo - 1;
        read += want as u64;
        if read >= budget_bytes || started.elapsed() >= budget_time {
            return Ok(not_found(false, hi));
        }
    }
}

/// `filesearchbytes`. This computer's files only, like `fileprobe`.
pub fn register(engine: &std::sync::Arc<WshRpcEngine>) {
    engine.register_typed("filesearchbytes", move |cmd: FileSearchBytesReq, _ctx| async move {
        let pattern = Pattern::parse(&cmd.pattern).map_err(|e| format!("filesearchbytes: {e}"))?;
        let path = expand_home_dir_safe(&cmd.path);
        tokio::task::spawn_blocking(move || search_path(&path, &pattern, cmd.from, cmd.backward).map_err(|e| format!("filesearchbytes: {e}")))
            .await
            .map_err(|e| format!("filesearchbytes: {e}"))?
    });
}

fn search_path(path: &Path, pat: &Pattern, from: u64, backward: bool) -> std::io::Result<FileSearchBytesResult> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Err(std::io::Error::other("is a directory"));
    }
    let mut file = std::fs::File::open(path)?;
    search(&mut file, meta.len(), pat, from, backward, BUDGET_BYTES, BUDGET_TIME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn find(data: &[u8], pattern: &str, from: u64, backward: bool) -> FileSearchBytesResult {
        let pat = Pattern::parse(pattern).unwrap();
        search(&mut Cursor::new(data.to_vec()), data.len() as u64, &pat, from, backward, u64::MAX, Duration::from_secs(60)).unwrap()
    }

    #[test]
    fn parses_hex_with_wildcards_and_refuses_the_rest() {
        let p = Pattern::parse("DE AD ?? ef").unwrap();
        assert_eq!(p.bytes, vec![0xde, 0xad, 0, 0xef]);
        assert_eq!(p.literal, vec![true, true, false, true]);
        assert_eq!(Pattern::parse("DEAD??EF").unwrap(), p);
        assert!(Pattern::parse("").is_err());
        assert!(Pattern::parse("DEA").is_err());
        assert!(Pattern::parse("ZZ").is_err());
        assert!(Pattern::parse("?? ??").is_err());
        assert!(Pattern::parse(&"AA".repeat(257)).is_err());
        assert!(Pattern::parse(&"AA".repeat(256)).is_ok());
    }

    #[test]
    fn finds_the_next_match_forward_and_the_one_before_backward() {
        let mut data = vec![0u8; 100];
        data[10..14].copy_from_slice(&[0xde, 0xad, 0x00, 0xef]);
        data[50..54].copy_from_slice(&[0xde, 0xad, 0x77, 0xef]);
        let a = find(&data, "DE AD ?? EF", 0, false);
        assert!(a.found);
        assert_eq!((a.offset, a.length), (10, 4));
        // From just after the first: the second.
        assert_eq!(find(&data, "DE AD ?? EF", 11, false).offset, 50);
        // The match may start exactly at `from`.
        assert_eq!(find(&data, "DE AD ?? EF", 50, false).offset, 50);
        assert!(find(&data, "DE AD ?? EF", 51, false).done);
        assert_eq!(find(&data, "DE AD ?? EF", 99, true).offset, 50);
        assert_eq!(find(&data, "DE AD ?? EF", 49, true).offset, 10);
        assert!(!find(&data, "DE AD ?? EF", 9, true).found);
        assert!(find(&data, "DE AD ?? EF", 9, true).done);
    }

    #[test]
    fn a_match_across_a_block_boundary_and_at_the_very_end_is_found() {
        let edge = BLOCK as usize;
        let mut data = vec![0u8; edge * 2 + 10];
        data[edge - 2..edge + 2].copy_from_slice(&[1, 2, 3, 4]);
        let end = data.len();
        data[end - 3..].copy_from_slice(&[9, 8, 7]);
        assert_eq!(find(&data, "01 02 03 04", 0, false).offset, (edge - 2) as u64);
        assert_eq!(find(&data, "01 02 03 04", (edge * 2) as u64, true).offset, (edge - 2) as u64);
        assert_eq!(find(&data, "09 08 07", 0, false).offset, (end - 3) as u64);
        assert_eq!(find(&data, "09 08 07", end as u64, true).offset, (end - 3) as u64);
        assert!(!find(&data, "09 08 06", 0, false).found);
    }

    #[test]
    fn a_pattern_longer_than_the_file_is_not_found_and_done() {
        let r = find(&[1, 2], "01 02 03", 0, false);
        assert!(!r.found && r.done);
        let r = find(&[], "01", 0, true);
        assert!(!r.found && r.done);
    }

    #[test]
    fn hands_back_where_it_got_to_when_its_budget_runs_out() {
        let data = vec![0u8; (BLOCK as usize) * 4];
        let pat = Pattern::parse("01").unwrap();
        // A budget of one block: one block read, then the way on.
        let r = search(&mut Cursor::new(data.clone()), data.len() as u64, &pat, 0, false, 1, Duration::from_secs(60)).unwrap();
        assert!(!r.found && !r.done);
        assert_eq!(r.next_from, BLOCK);
        let r = search(&mut Cursor::new(data.clone()), data.len() as u64, &pat, data.len() as u64, true, 1, Duration::from_secs(60)).unwrap();
        assert!(!r.found && !r.done);
        assert_eq!(r.next_from, BLOCK * 3 - 1);
    }

    #[test]
    fn searches_a_real_file_and_refuses_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.bin");
        std::fs::write(&file, [0u8, 1, 2, 3, 4, 5]).unwrap();
        let pat = Pattern::parse("03 ?? 05").unwrap();
        assert_eq!(search_path(&file, &pat, 0, false).unwrap().offset, 3);
        assert!(search_path(dir.path(), &pat, 0, false).is_err());
    }

    #[tokio::test]
    async fn filesearchbytes_records_its_types() {
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine);
        let schema = engine.schema_json();
        let row = schema.as_array().unwrap().iter().find(|r| r["command"] == "filesearchbytes").expect("filesearchbytes missing from the schema");
        assert_eq!(row["requestName"], "FileSearchBytesReq");
        assert_eq!(row["responseName"], "FileSearchBytesResult");
    }
}
