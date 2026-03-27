// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Multi-buffer byte reader for log file viewing.
//! Port of Go's `pkg/util/logview/multibuf.go`.
//!
//! Provides buffered random-access byte reading for files using a
//! multi-buffer sliding window. Maintains up to 2 adjacent buffers
//! of configurable size, rebuffering as needed. Supports line-level
//! navigation (next/previous line).

#![allow(dead_code)]

use std::io::{self, Read, Seek, SeekFrom};

/// Error indicating beginning-of-file was reached.
#[derive(Debug, Clone, PartialEq)]
pub struct BofError;

impl std::fmt::Display for BofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "beginning of file")
    }
}

impl std::error::Error for BofError {}

/// A buffered random-access byte reader for files.
///
/// Uses a sliding window of up to 2 adjacent buffers for locality.
pub struct MultiBufferByteGetter<R: Read + Seek> {
    reader: R,
    offset: i64,
    eof: bool,
    buffers: Vec<Vec<u8>>,
    buf_size: i64,
}

impl<R: Read + Seek> MultiBufferByteGetter<R> {
    /// Create a new MultiBufferByteGetter.
    ///
    /// `buf_size` is the size of each buffer partition in bytes.
    pub fn new(reader: R, buf_size: i64) -> Self {
        Self {
            reader,
            offset: 0,
            eof: false,
            buffers: Vec::new(),
            buf_size,
        }
    }

    /// Get the byte at the given file offset.
    ///
    /// Returns `io::ErrorKind::UnexpectedEof` if at end of file.
    pub fn get_byte(&mut self, offset: i64) -> io::Result<u8> {
        if let Some(b) = self.read_from_buffer(offset) {
            return Ok(b);
        }
        if self.eof && offset >= self.offset + self.total_buf_size() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"));
        }
        self.rebuffer(offset)?;
        match self.read_from_buffer(offset) {
            Some(b) => Ok(b),
            None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF")),
        }
    }

    /// Find the start of the next line after `offset`.
    ///
    /// Scans forward from `offset` to find the next `\n`, then returns
    /// the offset of the character after it.
    ///
    /// Returns `UnexpectedEof` if at end of file.
    pub fn next_line(&mut self, mut offset: i64) -> io::Result<i64> {
        loop {
            let b = self.get_byte(offset)?;
            if b == b'\n' {
                break;
            }
            offset += 1;
        }
        // Check if there's a next character
        match self.get_byte(offset + 1) {
            Ok(_) => Ok(offset + 1),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"))
            }
            Err(e) => Err(e),
        }
    }

    /// Find the start of the previous line before `offset`.
    ///
    /// Scans backward to find the preceding `\n` and returns the
    /// offset of the character after it.
    ///
    /// Returns an error with `BofError` if at beginning of file.
    pub fn prev_line(&mut self, offset: i64) -> Result<i64, Box<dyn std::error::Error>> {
        if offset == 0 {
            return Err(Box::new(BofError));
        }
        let mut pos = offset - 2;
        loop {
            if pos < 0 {
                break;
            }
            let b = self.get_byte(pos)?;
            if b == b'\n' {
                break;
            }
            pos -= 1;
        }
        Ok(pos + 1)
    }

    /// Read a byte from the current buffer if it's within range.
    fn read_from_buffer(&self, offset: i64) -> Option<u8> {
        if offset < self.offset || offset >= self.offset + self.total_buf_size() {
            return None;
        }
        let buf_idx = ((offset - self.offset) / self.buf_size) as usize;
        let buf_offset = ((offset - self.offset) % self.buf_size) as usize;
        if buf_idx < self.buffers.len() && buf_offset < self.buffers[buf_idx].len() {
            Some(self.buffers[buf_idx][buf_offset])
        } else {
            None
        }
    }

    /// Total bytes across all buffers.
    fn total_buf_size(&self) -> i64 {
        self.buffers.iter().map(|b| b.len() as i64).sum()
    }

    /// Read a new buffer partition from the file.
    fn rebuffer(&mut self, new_offset: i64) -> io::Result<()> {
        let part_num = new_offset / self.buf_size;
        let part_offset = part_num * self.buf_size;

        self.reader.seek(SeekFrom::Start(part_offset as u64))?;
        let mut new_buf = vec![0u8; self.buf_size as usize];
        let n = self.reader.read(&mut new_buf)?;
        let is_eof = n < self.buf_size as usize;
        new_buf.truncate(n);

        let (new_buffers, new_base_offset) = if !self.buffers.is_empty() && self.buffers.len() < 2 {
            let first_buf_part = self.offset / self.buf_size;
            // Inclusive last partition (not exclusive-end)
            let last_buf_part = first_buf_part + self.buffers.len() as i64 - 1;

            if first_buf_part == part_num + 1 {
                // New buffer is directly before existing: [new, old[0]]
                (vec![new_buf, self.buffers[0].clone()], part_offset)
            } else if last_buf_part == part_num - 1 {
                // New buffer is directly after existing: [old[0], new]
                (vec![self.buffers[0].clone(), new_buf], self.offset)
            } else {
                (vec![new_buf], part_offset)
            }
        } else {
            (vec![new_buf], part_offset)
        };

        self.buffers = new_buffers;
        self.offset = new_base_offset;
        self.eof = is_eof;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_getter(data: &[u8], buf_size: i64) -> MultiBufferByteGetter<Cursor<Vec<u8>>> {
        MultiBufferByteGetter::new(Cursor::new(data.to_vec()), buf_size)
    }

    #[test]
    fn test_get_byte_basic() {
        let mut g = make_getter(b"hello world", 64);
        assert_eq!(g.get_byte(0).unwrap(), b'h');
        assert_eq!(g.get_byte(5).unwrap(), b' ');
        assert_eq!(g.get_byte(10).unwrap(), b'd');
    }

    #[test]
    fn test_get_byte_eof() {
        let mut g = make_getter(b"abc", 64);
        assert!(g.get_byte(3).is_err());
    }

    #[test]
    fn test_get_byte_rebuffer() {
        let data = b"0123456789abcdef";
        let mut g = make_getter(data, 4); // Small buffer to force rebuffering
        assert_eq!(g.get_byte(0).unwrap(), b'0');
        assert_eq!(g.get_byte(8).unwrap(), b'8');
        assert_eq!(g.get_byte(15).unwrap(), b'f');
    }

    #[test]
    fn test_next_line() {
        let mut g = make_getter(b"line1\nline2\nline3\n", 64);
        assert_eq!(g.next_line(0).unwrap(), 6); // After "line1\n"
        assert_eq!(g.next_line(6).unwrap(), 12); // After "line2\n"
    }

    #[test]
    fn test_next_line_eof() {
        let mut g = make_getter(b"line1\n", 64);
        // Scanning from 0 finds \n at 5, then checks offset 6 → EOF
        assert!(g.next_line(0).is_err());
    }

    #[test]
    fn test_prev_line() {
        let mut g = make_getter(b"line1\nline2\nline3\n", 64);
        assert_eq!(g.prev_line(12).unwrap(), 6); // Before "line3\n", back to "line2\n"
        assert_eq!(g.prev_line(6).unwrap(), 0); // Before "line2\n", back to start
    }

    #[test]
    fn test_prev_line_bof() {
        let mut g = make_getter(b"line1\n", 64);
        let result = g.prev_line(0);
        assert!(result.is_err());
        assert!(result.unwrap_err().downcast::<BofError>().is_ok());
    }

    #[test]
    fn test_sequential_lines() {
        let data = b"aaa\nbbb\nccc\n";
        let mut g = make_getter(data, 64);

        // Navigate forward
        let line2 = g.next_line(0).unwrap();
        assert_eq!(line2, 4);
        let line3 = g.next_line(line2).unwrap();
        assert_eq!(line3, 8);

        // Navigate backward
        let back_to_2 = g.prev_line(line3).unwrap();
        assert_eq!(back_to_2, 4);
        let back_to_1 = g.prev_line(back_to_2).unwrap();
        assert_eq!(back_to_1, 0);
    }

    #[test]
    fn test_small_buffer_navigation() {
        let data = b"line1\nline2\nline3\nline4\nline5\n";
        let mut g = make_getter(data, 4); // Very small buffer

        // Navigate through all lines
        let mut offset = 0i64;
        let mut count = 0;
        while let Ok(next) = g.next_line(offset) {
            offset = next;
            count += 1;
        }
        assert_eq!(count, 4); // 5 lines, 4 transitions
    }

    #[test]
    fn test_empty_file() {
        let mut g = make_getter(b"", 64);
        assert!(g.get_byte(0).is_err());
    }
}
