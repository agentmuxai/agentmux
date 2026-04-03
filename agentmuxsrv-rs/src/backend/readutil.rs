// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! File reading utilities for forward and tail (reverse) reading.
//! Port of Go's `pkg/util/readutil/readutil.go`.
//!
//! Provides line-based file reading with support for line counts,
//! byte limits, line skipping, and progressive tail reading.


use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};

/// Stop reason: beginning of file reached.
pub const STOP_REASON_BOF: &str = "bof";
/// Stop reason: end of file reached.
pub const STOP_REASON_EOF: &str = "eof";
/// Stop reason: byte read limit reached.
pub const STOP_REASON_READ_LIMIT: &str = "read_limit";

/// Read lines forward from a reader.
///
/// - `line_count`: max lines to return (0 = unlimited)
/// - `skip_lines`: number of initial lines to skip
/// - `read_limit`: max bytes to read (0 = unlimited)
///
/// Returns `(lines, stop_reason)`.
pub fn read_lines<R: Read>(
    reader: R,
    line_count: usize,
    skip_lines: usize,
    read_limit: usize,
) -> io::Result<(Vec<String>, String)> {
    let mut buf_reader = BufReader::new(reader);
    let mut lines = Vec::new();
    let mut bytes_read = 0usize;
    let mut skipped = 0usize;

    loop {
        let mut line = String::new();
        match buf_reader.read_line(&mut line) {
            Ok(0) => return Ok((lines, STOP_REASON_EOF.to_string())),
            Ok(n) => {
                bytes_read += n;

                if skipped < skip_lines {
                    skipped += 1;
                } else {
                    lines.push(line.clone());
                    if line_count > 0 && lines.len() >= line_count {
                        return Ok((lines, String::new()));
                    }
                }

                if read_limit > 0 && bytes_read >= read_limit {
                    return Ok((lines, STOP_REASON_READ_LIMIT.to_string()));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Find the byte offsets of the last N lines in a seekable reader.
///
/// Returns `(offsets, total_lines)`.
/// If `keep_first` is true, always includes offset 0 (start of file).
pub fn read_last_n_line_offsets<R: Read + Seek>(
    rs: &mut R,
    max_lines: usize,
    keep_first: bool,
) -> io::Result<(Vec<i64>, usize)> {
    rs.seek(SeekFrom::Start(0))?;

    let mut offsets: Vec<i64> = Vec::new();
    let mut reader = BufReader::new(rs);
    let mut current_pos: i64 = 0;
    let mut total_lines = 0;

    if keep_first {
        offsets.push(0);
        total_lines = 1;
    }

    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(n) => {
                current_pos += n as i64;
                offsets.push(current_pos);
                total_lines += 1;
                if offsets.len() > max_lines + 1 {
                    offsets.remove(0);
                }
            }
            Err(e) => return Err(e),
        }
    }

    if !offsets.is_empty() {
        offsets.pop();
        total_lines -= 1;
    }

    Ok((offsets, total_lines))
}

/// Read the last `line_count` lines from a seekable reader,
/// optionally skipping the last `line_offset` lines.
///
/// Returns `(lines, has_more)`.
fn read_tail_lines_internal<R: Read + Seek>(
    rs: &mut R,
    line_count: usize,
    line_offset: usize,
    keep_first: bool,
) -> io::Result<(Vec<String>, bool)> {
    let max_offsets = line_count + line_offset;
    let (offsets, total_lines) = read_last_n_line_offsets(rs, max_offsets, keep_first)?;

    if total_lines <= line_offset {
        return Ok((Vec::new(), false));
    }

    let lines_to_read = line_count.min(total_lines - line_offset);
    let start_idx = offsets.len().saturating_sub(line_offset + lines_to_read);
    let has_more = total_lines > line_count + line_offset;

    rs.seek(SeekFrom::Start(offsets[start_idx] as u64))?;

    let (lines, _) = read_lines(rs, lines_to_read, 0, 0)?;
    Ok((lines, has_more))
}

/// Read the last `line_count` lines from a seekable reader with a byte limit.
///
/// Progressively reads larger sections from the end (starting at 1MB, doubling)
/// until enough lines are found or the limit is reached.
///
/// Returns `(lines, stop_reason)`.
pub fn read_tail_lines<R: Read + Seek>(
    rs: &mut R,
    total_size: u64,
    line_count: usize,
    line_offset: usize,
    read_limit: u64,
) -> io::Result<(Vec<String>, String)> {
    if read_limit == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("read_limit must be positive, got {}", read_limit),
        ));
    }

    let mut read_bytes: u64 = (1024 * 1024).min(read_limit);

    loop {
        let start_pos = if total_size > read_bytes {
            total_size - read_bytes
        } else {
            read_bytes = total_size;
            0
        };

        rs.seek(SeekFrom::Start(start_pos))?;
        let keep_first = start_pos == 0;

        // Read a limited section
        let mut section = vec![0u8; read_bytes as usize];
        let actually_read = rs.read(&mut section)?;
        section.truncate(actually_read);

        let mut cursor = io::Cursor::new(section);
        let (lines, has_more_in_window) =
            read_tail_lines_internal(&mut cursor, line_count, line_offset, keep_first)?;

        if lines.len() == line_count {
            let has_more = start_pos > 0 || has_more_in_window;
            if !has_more {
                return Ok((lines, STOP_REASON_BOF.to_string()));
            }
            return Ok((lines, String::new()));
        }

        if read_bytes >= read_limit || read_bytes >= total_size {
            if start_pos > 0 {
                return Ok((lines, STOP_REASON_READ_LIMIT.to_string()));
            }
            return Ok((lines, STOP_REASON_BOF.to_string()));
        }

        read_bytes = (read_bytes * 2).min(read_limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- read_lines ----

    #[test]
    fn test_read_lines_basic() {
        let data = b"line1\nline2\nline3\n";
        let (lines, reason) = read_lines(&data[..], 0, 0, 0).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(reason, STOP_REASON_EOF);
    }

    #[test]
    fn test_read_lines_with_limit() {
        let data = b"line1\nline2\nline3\n";
        let (lines, reason) = read_lines(&data[..], 2, 0, 0).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(reason, ""); // Hit line limit, not EOF
    }

    #[test]
    fn test_read_lines_with_skip() {
        let data = b"skip1\nskip2\nkeep1\nkeep2\n";
        let (lines, reason) = read_lines(&data[..], 0, 2, 0).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("keep1"));
        assert_eq!(reason, STOP_REASON_EOF);
    }

    #[test]
    fn test_read_lines_byte_limit() {
        let data = b"line1\nline2\nline3\n";
        let (lines, reason) = read_lines(&data[..], 0, 0, 12).unwrap();
        assert_eq!(reason, STOP_REASON_READ_LIMIT);
        assert!(lines.len() >= 1);
    }

    #[test]
    fn test_read_lines_empty() {
        let data = b"";
        let (lines, reason) = read_lines(&data[..], 0, 0, 0).unwrap();
        assert!(lines.is_empty());
        assert_eq!(reason, STOP_REASON_EOF);
    }

    // ---- read_last_n_line_offsets ----

    #[test]
    fn test_line_offsets_basic() {
        let data = b"line1\nline2\nline3\n";
        let mut cursor = io::Cursor::new(data.as_slice());
        let (offsets, total) = read_last_n_line_offsets(&mut cursor, 10, false).unwrap();
        // Without keep_first, offset 0 is not included.
        // After reading all lines, the final EOF offset is popped.
        assert_eq!(total, 2);
        assert_eq!(offsets.len(), 2);
        assert_eq!(offsets[0], 6); // "line2\n" starts at 6
        assert_eq!(offsets[1], 12); // "line3\n" starts at 12
    }

    #[test]
    fn test_line_offsets_keep_first() {
        let data = b"line1\nline2\nline3\n";
        let mut cursor = io::Cursor::new(data.as_slice());
        let (offsets, total) = read_last_n_line_offsets(&mut cursor, 10, true).unwrap();
        // keep_first adds offset 0 and total_lines=1 initially.
        // 3 lines read, pop last → total = 1 + 3 - 1 = 3
        assert_eq!(total, 3);
        assert_eq!(offsets[0], 0); // keep_first entry
        assert_eq!(offsets[1], 6);
        assert_eq!(offsets[2], 12);
    }

    #[test]
    fn test_line_offsets_max_lines() {
        let data = b"1\n2\n3\n4\n5\n";
        let mut cursor = io::Cursor::new(data.as_slice());
        let (offsets, total) = read_last_n_line_offsets(&mut cursor, 2, false).unwrap();
        // 5 lines read, pop last → total = 5 - 1 = 4
        assert_eq!(total, 4);
        assert_eq!(offsets.len(), 2); // Only last 2
    }

    // ---- read_tail_lines ----

    #[test]
    fn test_tail_lines_basic() {
        let data = b"line1\nline2\nline3\nline4\nline5\n";
        let mut cursor = io::Cursor::new(data.as_slice());
        let size = data.len() as u64;
        let (lines, reason) = read_tail_lines(&mut cursor, size, 3, 0, 1024 * 1024).unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("line3"));
        assert!(lines[1].contains("line4"));
        assert!(lines[2].contains("line5"));
        assert_eq!(reason, ""); // has_more = true
    }

    #[test]
    fn test_tail_lines_all() {
        let data = b"line1\nline2\n";
        let mut cursor = io::Cursor::new(data.as_slice());
        let size = data.len() as u64;
        let (lines, reason) = read_tail_lines(&mut cursor, size, 10, 0, 1024 * 1024).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(reason, STOP_REASON_BOF);
    }

    #[test]
    fn test_tail_lines_with_offset() {
        let data = b"line1\nline2\nline3\nline4\nline5\n";
        let mut cursor = io::Cursor::new(data.as_slice());
        let size = data.len() as u64;
        let (lines, _) = read_tail_lines(&mut cursor, size, 2, 1, 1024 * 1024).unwrap();
        assert_eq!(lines.len(), 2);
        // Skipping last 1 line (line5), reading 2 lines (line3, line4)
        assert!(lines[0].contains("line3"));
        assert!(lines[1].contains("line4"));
    }

    #[test]
    fn test_tail_lines_invalid_limit() {
        let data = b"line1\n";
        let mut cursor = io::Cursor::new(data.as_slice());
        let result = read_tail_lines(&mut cursor, data.len() as u64, 1, 0, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_tail_lines_empty() {
        let data = b"";
        let mut cursor = io::Cursor::new(data.as_slice());
        let (lines, reason) = read_tail_lines(&mut cursor, 0, 10, 0, 1024 * 1024).unwrap();
        assert!(lines.is_empty());
        assert_eq!(reason, STOP_REASON_BOF);
    }
}
