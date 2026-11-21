// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `filesearchtext`: the next match of a text, or a regular expression, in a UTF-8
//! file, for the Large text viewer's find (case-insensitive, regex). Like
//! `filesearchbytes` the file is read here in blocks, each call does a bounded
//! amount of work and says where to go on from.
//!
//! A match is found when it lies inside one block and the 64 KiB after it: a match
//! longer than that (a regex that spans many lines) is not found. A `^` or `$` is
//! judged with 64 KiB of the file before the block, so on a line longer than that
//! it can match in the middle of the line.
//!
//! docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use regex::bytes::{Regex, RegexBuilder};

use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{FileSearchBytesResult, FileSearchTextReq};

/// The longest query, in characters.
const MAX_QUERY: usize = 1000;
/// One block of match starts.
const BLOCK: u64 = 1 << 20;
/// The file read beyond a block, for a match that starts in it and runs past its end.
const AFTER: u64 = 64 << 10;
/// The file read before a block, for the text a `^`, `$` or `\b` looks at.
const BEFORE: u64 = 64 << 10;
/// The most a call reads before it hands back where it got to.
const BUDGET_BYTES: u64 = 256 << 20;
/// The longest a call goes on before it hands back where it got to.
const BUDGET_TIME: Duration = Duration::from_millis(1500);
/// The most memory a compiled expression may take.
const REGEX_SIZE_LIMIT: usize = 8 << 20;

/// The expression for a query: a literal text is escaped; `^` and `$` match at each line (a line ends at `\n` or `\r\n`).
pub fn compile(query: &str, regex: bool, ignore_case: bool) -> Result<Regex, String> {
    if query.is_empty() {
        return Err("nothing to find".into());
    }
    if query.chars().count() > MAX_QUERY {
        return Err(format!("at most {MAX_QUERY} characters"));
    }
    let source = if regex { query.to_string() } else { regex::escape(query) };
    RegexBuilder::new(&source)
        .case_insensitive(ignore_case)
        .multi_line(true)
        .crlf(true)
        .unicode(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("not a valid expression: {}", first_line(&e.to_string())))
}

/// The last line of a regex error, which is the reason (the first lines draw the expression).
fn first_line(message: &str) -> String {
    message.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(message).trim().trim_start_matches("error: ").to_string()
}

/// The first non-empty match that starts in `hay[from..to]` (an empty one, of `a*`, is no place to stop).
fn first_in(re: &Regex, hay: &[u8], from: usize, to: usize) -> Option<(usize, usize)> {
    let mut pos = from;
    while pos <= to && pos <= hay.len() {
        let m = re.find_at(hay, pos)?;
        if m.start() > to {
            return None;
        }
        if m.end() > m.start() {
            return Some((m.start(), m.end()));
        }
        pos = m.end() + 1;
    }
    None
}

/// The last non-empty match that starts in `hay[from..=to]`, among the matches found one after another (each
/// begins where the one before ended), as a viewer marks them. The walk begins at the start of the line that
/// holds `from`, so which matches there are does not depend on where a block happens to begin.
fn last_in(re: &Regex, hay: &[u8], from: usize, to: usize) -> Option<(usize, usize)> {
    let mut best = None;
    let mut pos = memchr::memrchr(b'\n', &hay[..from]).map_or(0, |i| i + 1);
    while pos <= to && pos <= hay.len() {
        let Some(m) = re.find_at(hay, pos) else { break };
        if m.start() > to {
            break;
        }
        if m.end() > m.start() {
            if m.start() >= from {
                best = Some((m.start(), m.end()));
            }
            pos = m.end();
        } else {
            pos = m.end() + 1;
        }
    }
    best
}

fn read_window<F: Read + Seek>(file: &mut F, buf: &mut Vec<u8>, lo: u64, hi: u64) -> std::io::Result<()> {
    buf.resize((hi - lo) as usize, 0);
    file.seek(SeekFrom::Start(lo))?;
    file.read_exact(buf)
}

/// The search over `file` (`size` bytes). Forward: the first match starting at or after `from`; backward: the last
/// one starting at or before it.
pub fn search<F: Read + Seek>(
    file: &mut F,
    size: u64,
    re: &Regex,
    from: u64,
    backward: bool,
    budget_bytes: u64,
    budget_time: Duration,
) -> std::io::Result<FileSearchBytesResult> {
    let not_found = |done: bool, next_from: u64| FileSearchBytesResult { found: false, offset: 0, length: 0, done, next_from };
    let found = |start: u64, end: u64| FileSearchBytesResult { found: true, offset: start, length: end - start, done: false, next_from: 0 };
    if size == 0 {
        return Ok(not_found(true, 0));
    }
    let started = Instant::now();
    let mut read = 0u64;
    let mut buf = Vec::new();
    if !backward {
        let mut at = from;
        while at < size {
            let end = (at + BLOCK).min(size);
            let ctx = at.saturating_sub(BEFORE);
            let stop = (end + AFTER).min(size);
            read_window(file, &mut buf, ctx, stop)?;
            if let Some((s, e)) = first_in(re, &buf, (at - ctx) as usize, (end - 1 - ctx) as usize) {
                return Ok(found(ctx + s as u64, ctx + e as u64));
            }
            read += stop - ctx;
            at = end;
            if at < size && (read >= budget_bytes || started.elapsed() >= budget_time) {
                return Ok(not_found(false, at));
            }
        }
        return Ok(not_found(true, size));
    }
    // Backward: blocks of starts [lo, hi], going down.
    let mut hi = from.min(size - 1);
    loop {
        let lo = hi.saturating_sub(BLOCK - 1);
        let ctx = lo.saturating_sub(BEFORE);
        let stop = (hi + 1 + AFTER).min(size);
        read_window(file, &mut buf, ctx, stop)?;
        if let Some((s, e)) = last_in(re, &buf, (lo - ctx) as usize, (hi - ctx) as usize) {
            return Ok(found(ctx + s as u64, ctx + e as u64));
        }
        if lo == 0 {
            return Ok(not_found(true, 0));
        }
        hi = lo - 1;
        read += stop - ctx;
        if read >= budget_bytes || started.elapsed() >= budget_time {
            return Ok(not_found(false, hi));
        }
    }
}

/// `filesearchtext`. This computer's files only, like `filesearchbytes`.
pub fn register(engine: &std::sync::Arc<WshRpcEngine>) {
    engine.register_typed("filesearchtext", move |cmd: FileSearchTextReq, _ctx| async move {
        let re = compile(&cmd.query, cmd.regex, cmd.ignore_case).map_err(|e| format!("filesearchtext: {e}"))?;
        let path = expand_home_dir_safe(&cmd.path);
        tokio::task::spawn_blocking(move || search_path(&path, &re, cmd.from, cmd.backward).map_err(|e| format!("filesearchtext: {e}")))
            .await
            .map_err(|e| format!("filesearchtext: {e}"))?
    });
}

fn search_path(path: &Path, re: &Regex, from: u64, backward: bool) -> std::io::Result<FileSearchBytesResult> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Err(std::io::Error::other("is a directory"));
    }
    let mut file = std::fs::File::open(path)?;
    search(&mut file, meta.len(), re, from, backward, BUDGET_BYTES, BUDGET_TIME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run(data: &[u8], query: &str, regex: bool, ignore_case: bool, from: u64, backward: bool) -> FileSearchBytesResult {
        let re = compile(query, regex, ignore_case).unwrap();
        search(&mut Cursor::new(data.to_vec()), data.len() as u64, &re, from, backward, u64::MAX, Duration::from_secs(60)).unwrap()
    }

    #[test]
    fn a_literal_is_exact_and_its_metacharacters_mean_themselves() {
        let data = b"a.c abc ABC";
        assert_eq!(run(data, "a.c", false, false, 0, false).offset, 0);
        assert_eq!(run(data, "a.c", false, false, 1, false).found, false);
        // Exact case: "ABC" is the only one.
        assert_eq!(run(data, "ABC", false, false, 0, false).offset, 8);
    }

    #[test]
    fn ignoring_case_finds_either_case_and_unicode_letters() {
        let data = "xx Straße STRASSE ÉCOLE école".as_bytes();
        let r = run(data, "école", false, true, 0, false);
        assert_eq!(&data[r.offset as usize..(r.offset + r.length) as usize], "ÉCOLE".as_bytes());
        let r = run(data, "école", false, true, r.offset + 1, false);
        assert_eq!(&data[r.offset as usize..(r.offset + r.length) as usize], "école".as_bytes());
        assert_eq!(run(data, "ÉCOLE", false, false, 0, false).offset, 19);
    }

    #[test]
    fn a_regex_gives_the_match_length_and_anchors_match_each_line() {
        let data = b"one\nfoo123 bar\nfoo45\n";
        let r = run(data, r"foo\d+", true, false, 0, false);
        assert_eq!((r.offset, r.length), (4, 6));
        let r = run(data, r"^foo\d+$", true, false, 5, false);
        assert_eq!((r.offset, r.length), (15, 5));
        // `^bar` is not at the start of a line.
        assert!(!run(data, "^bar", true, false, 0, false).found);
    }

    #[test]
    fn backward_finds_the_last_match_at_or_before_the_offset() {
        let data = b"ab ab ab";
        assert_eq!(run(data, "ab", false, false, 7, true).offset, 6);
        assert_eq!(run(data, "ab", false, false, 5, true).offset, 3);
        assert_eq!(run(data, "ab", false, false, 2, true).offset, 0);
        assert!(run(data, "ab", false, false, 2, true).found);
        let r = run(data, "zz", false, false, 7, true);
        assert!(!r.found && r.done);
    }

    #[test]
    fn backward_goes_by_whole_matches_never_to_the_tail_of_one() {
        // "foo123 456": the matches of \d+ are 123 (at 3) and 456 (at 7); "23" and "3" are not matches.
        let data = b"foo123 456\n78\n";
        assert_eq!(run(data, r"\d+", true, false, 6, true).offset, 3);
        assert_eq!(run(data, r"\d+", true, false, 6, true).length, 3);
        assert_eq!(run(data, r"\d+", true, false, 9, true).offset, 7);
        // From the start of 123, no match before it.
        assert!(!run(data, r"\d+", true, false, 2, true).found);
        // Forward from the end of a match goes to the next whole one.
        assert_eq!(run(data, r"\d+", true, false, 6, false).offset, 7);
    }

    #[test]
    fn a_backward_walk_starts_at_a_line_start_whichever_block_it_is_in() {
        // "aaaaa" starting a line: the matches of "aa" are at 0 and 2 of it, never at 1 or 3. The first block of a
        // backward search begins at the 2nd character (`size - BLOCK`), mid-match: the walk still starts at the line.
        let edge = BLOCK as usize;
        let mut data = vec![b'\n'; edge * 2 + 7];
        data[edge + 6..edge + 11].copy_from_slice(b"aaaaa");
        let line = (edge + 6) as u64;
        assert_eq!(run(&data, "aa", false, false, data.len() as u64, true).offset, line + 2);
    }

    #[test]
    fn a_line_ending_is_crlf_as_well_as_lf() {
        let data = b"foo\r\nbar\r\nfoo\n";
        let r = run(data, "^foo$", true, false, 0, false);
        assert_eq!((r.offset, r.length), (0, 3));
        let r = run(data, "^foo$", true, false, 1, false);
        assert_eq!((r.offset, r.length), (10, 3));
        assert_eq!(run(data, "^bar$", true, false, 0, false).offset, 5);
    }

    #[test]
    fn a_match_that_could_be_empty_is_skipped_not_returned() {
        let data = b"xxaaay";
        let r = run(data, "a*", true, false, 0, false);
        assert_eq!((r.offset, r.length), (2, 3));
        let r = run(data, "a*", true, false, 5, true);
        assert_eq!((r.offset, r.length), (2, 3));
    }

    #[test]
    fn a_match_across_a_block_edge_and_at_the_end_is_found_both_ways() {
        let edge = BLOCK as usize;
        let mut data = vec![b'.'; edge * 2 + 10];
        data[edge - 2..edge + 2].copy_from_slice(b"WXYZ");
        let end = data.len();
        data[end - 3..].copy_from_slice(b"end");
        assert_eq!(run(&data, "wxyz", false, true, 0, false).offset, (edge - 2) as u64);
        assert_eq!(run(&data, "wxyz", false, true, (edge * 2) as u64, true).offset, (edge - 2) as u64);
        assert_eq!(run(&data, "END", false, true, 0, false).offset, (end - 3) as u64);
        assert_eq!(run(&data, "end", false, true, end as u64, true).offset, (end - 3) as u64);
        // A start in the next block, from just past the edge.
        assert!(!run(&data, "wxyz", false, true, (edge - 1) as u64, false).found);
    }

    #[test]
    fn a_line_start_is_judged_with_the_text_before_the_block() {
        // "x" fills the block edge: the "^y" after it is mid-line, not a line start.
        let edge = BLOCK as usize;
        let mut data = vec![b'x'; edge + 4];
        data[edge..edge + 2].copy_from_slice(b"y!");
        assert!(!run(&data, "^y", true, false, 0, false).found);
        data[edge - 1] = b'\n';
        assert_eq!(run(&data, "^y", true, false, 0, false).offset, edge as u64);
    }

    #[test]
    fn hands_back_where_it_got_to_when_its_budget_runs_out() {
        let data = vec![b'.'; (BLOCK as usize) * 4];
        let re = compile("zz", false, false).unwrap();
        let r = search(&mut Cursor::new(data.clone()), data.len() as u64, &re, 0, false, 1, Duration::from_secs(60)).unwrap();
        assert!(!r.found && !r.done);
        assert_eq!(r.next_from, BLOCK);
        let r = search(&mut Cursor::new(data.clone()), data.len() as u64, &re, data.len() as u64, true, 1, Duration::from_secs(60)).unwrap();
        assert!(!r.found && !r.done);
        assert_eq!(r.next_from, BLOCK * 3 - 1);
    }

    #[test]
    fn an_empty_file_and_a_bad_query_are_refused_or_not_found() {
        let re = compile("a", false, false).unwrap();
        let r = search(&mut Cursor::new(Vec::new()), 0, &re, 0, false, u64::MAX, Duration::from_secs(60)).unwrap();
        assert!(!r.found && r.done);
        assert!(compile("", false, false).is_err());
        assert!(compile("(", true, false).is_err());
        assert!(compile("(", false, false).is_ok());
        assert!(compile(&"a".repeat(1001), false, false).is_err());
    }

    #[test]
    fn searches_a_real_file_and_refuses_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hello World").unwrap();
        let re = compile("world", false, true).unwrap();
        assert_eq!(search_path(&file, &re, 0, false).unwrap().offset, 6);
        assert!(search_path(dir.path(), &re, 0, false).is_err());
    }

    #[tokio::test]
    async fn filesearchtext_records_its_types() {
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine);
        let schema = engine.schema_json();
        let row = schema.as_array().unwrap().iter().find(|r| r["command"] == "filesearchtext").expect("filesearchtext missing from the schema");
        assert_eq!(row["requestName"], "FileSearchTextReq");
        assert_eq!(row["responseName"], "FileSearchBytesResult");
    }
}
