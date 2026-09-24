// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The transcript line rule, shared by the `output.idx` indexer and the
//! per-file line counter (Phase 5a-2,
//! SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7).
//!
//! A line is the bytes between two `\n` (the last one may be unterminated).
//! It is counted unless it is blank: `from_utf8_lossy(line).trim()` is
//! empty, so `\r` (CRLF) and Unicode whitespace are blank, and invalid UTF-8
//! is not. This is the reader's addressing (`String::lines()` filtered on
//! `!trim().is_empty()`); line `k` of a file is the `k`-th counted line.
//! Both users call [`is_blank_line`], so they can't drift apart.

/// Whether `line` (without its `\n`) is blank under the reader's rule.
pub(crate) fn is_blank_line(line: &[u8]) -> bool {
    // Fast path for ASCII, nearly every transcript line: the ASCII characters
    // `char::is_whitespace` (and so `str::trim`) accepts. Unlike
    // `u8::is_ascii_whitespace`, that includes U+000B (vertical tab).
    if line.is_ascii() {
        return line.iter().all(|&b| matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r'));
    }
    String::from_utf8_lossy(line).trim().is_empty()
}

/// Where a file's line counter stands after appending bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineCount {
    /// Counted lines, including an unterminated last line if it is not blank.
    pub lines: u64,
    /// Byte offset where the last (possibly unterminated) line starts: just
    /// past the last `\n`, or 0. Equal to the file size when the file ends
    /// with `\n` or is empty.
    pub tail_start: u64,
}

/// The counter after appending `data` at byte offset `at` to a file whose
/// counter was `before`. `open_tail` is the file's bytes from
/// `before.tail_start` to `at` — the unterminated last line, empty when the
/// file ends with `\n` — because appended bytes continue that line and can
/// change whether it is blank.
pub(crate) fn count_append(before: LineCount, open_tail: &[u8], at: u64, data: &[u8]) -> LineCount {
    debug_assert_eq!(before.tail_start + open_tail.len() as u64, at);
    let old_tail_counted = !open_tail.is_empty() && !is_blank_line(open_tail);
    let mut lines = before.lines.saturating_sub(u64::from(old_tail_counted));

    let Some(first_nl) = data.iter().position(|&b| b == b'\n') else {
        // No newline: the open line just grows.
        let mut line = Vec::with_capacity(open_tail.len() + data.len());
        line.extend_from_slice(open_tail);
        line.extend_from_slice(data);
        lines += u64::from(!line.is_empty() && !is_blank_line(&line));
        return LineCount { lines, tail_start: before.tail_start };
    };

    // The open line, completed by the bytes up to the first newline.
    let mut first = Vec::with_capacity(open_tail.len() + first_nl);
    first.extend_from_slice(open_tail);
    first.extend_from_slice(&data[..first_nl]);
    lines += u64::from(!is_blank_line(&first));

    // Every later segment: complete lines, then a possibly empty remainder.
    let rest = &data[first_nl + 1..];
    let mut segments = rest.split(|&b| b == b'\n');
    let remainder = segments.next_back().unwrap_or(&[]);
    lines += segments.filter(|l| !is_blank_line(l)).count() as u64;
    lines += u64::from(!remainder.is_empty() && !is_blank_line(remainder));

    let last_nl = data.iter().rposition(|&b| b == b'\n').expect("data has a newline");
    LineCount { lines, tail_start: at + last_nl as u64 + 1 }
}

/// The counter for a whole file's content.
pub(crate) fn count_all(data: &[u8]) -> LineCount {
    count_append(LineCount { lines: 0, tail_start: 0 }, &[], 0, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The reader's addressing, written the way the read path does it.
    fn reader_count(data: &[u8]) -> u64 {
        String::from_utf8_lossy(data).lines().filter(|l| !l.trim().is_empty()).count() as u64
    }

    #[test]
    fn blank_rule_matches_trim() {
        for (line, blank) in [
            (&b""[..], true),
            (b" \t", true),
            (b"\r", true),
            (b"\x0b", true),
            (b"\x0c", true),
            ("\u{3000}".as_bytes(), true),
            ("\u{a0}".as_bytes(), true),
            (b"x", false),
            (b" {} ", false),
            (b"\xff", false),
            (b"\xe3\x80", false), // a truncated U+3000 decodes to U+FFFD
        ] {
            assert_eq!(is_blank_line(line), blank, "{line:?}");
            assert_eq!(is_blank_line(line), String::from_utf8_lossy(line).trim().is_empty(), "{line:?}");
        }
    }

    #[test]
    fn appends_that_complete_or_extend_an_open_line() {
        let c = count_all(b"a\n  ");
        assert_eq!(c, LineCount { lines: 1, tail_start: 2 });
        // The blank open line becomes a counted line.
        let c = count_append(c, b"  ", 4, b"b");
        assert_eq!(c, LineCount { lines: 2, tail_start: 2 });
        // Appending to a counted open line doesn't count it twice.
        let c = count_append(c, b"  b", 5, b"c\n\n \nd\n");
        assert_eq!(c, LineCount { lines: 3, tail_start: 12 });
        assert_eq!(c.lines, reader_count(b"a\n  bc\n\n \nd\n"));
    }

    fn arb_bytes() -> impl Strategy<Value = Vec<u8>> {
        // Weighted towards the bytes the rule cares about.
        let byte = prop_oneof![
            4 => Just(b'\n'),
            2 => Just(b' '),
            1 => Just(b'\r'),
            1 => Just(b'\t'),
            1 => Just(0x0bu8),
            1 => Just(0xe3u8), // lead byte of U+3000
            1 => Just(0x80u8),
            1 => Just(0xffu8),
            6 => b'a'..=b'z',
        ];
        prop::collection::vec(byte, 0..40)
    }

    proptest! {
        /// Appending in any chunks gives the reader's count of the whole.
        #[test]
        fn chunked_count_equals_the_readers_count(chunks in prop::collection::vec(arb_bytes(), 0..12)) {
            let mut file: Vec<u8> = Vec::new();
            let mut c = LineCount { lines: 0, tail_start: 0 };
            for chunk in &chunks {
                let open = file[c.tail_start as usize..].to_vec();
                c = count_append(c, &open, file.len() as u64, chunk);
                file.extend_from_slice(chunk);
                prop_assert_eq!(c.lines, reader_count(&file), "after {:?}", file);
                let expected_tail = file.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1) as u64;
                prop_assert_eq!(c.tail_start, expected_tail);
            }
            prop_assert_eq!(count_all(&file), c);
        }
    }
}
