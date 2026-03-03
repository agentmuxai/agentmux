// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Utility data structures.
//! Port of Go's pkg/utilds/.
//!
//! Provides a line-buffered reader with circular buffer and callbacks.

#![allow(dead_code)]

use std::io::{self, BufRead, BufReader, Read};
use std::sync::Mutex;

// ---- Constants ----

/// Default maximum number of lines in the buffer.
const DEFAULT_MAX_LINES: usize = 1000;

/// Callback type for line notifications.
type LineCallback = Box<dyn Fn(&str) + Send>;

// ---- ReaderLineBuffer ----

/// A line-buffered reader backed by a circular buffer.
///
/// Reads lines from an `io::Read` source, stores them in a fixed-size
/// circular buffer, and optionally calls a callback for each line.
pub struct ReaderLineBuffer<R: Read> {
    inner: Mutex<ReaderLineBufferInner<R>>,
}

struct ReaderLineBufferInner<R: Read> {
    reader: BufReader<R>,
    lines: Vec<String>,
    max_lines: usize,
    total_line_count: usize,
    done: bool,
    line_callback: Option<LineCallback>,
}

impl<R: Read> ReaderLineBuffer<R> {
    /// Create a new ReaderLineBuffer wrapping the given reader.
    ///
    /// `max_lines` specifies the circular buffer capacity.
    /// If 0, defaults to 1000.
    pub fn new(reader: R, max_lines: usize) -> Self {
        let max = if max_lines == 0 {
            DEFAULT_MAX_LINES
        } else {
            max_lines
        };
        Self {
            inner: Mutex::new(ReaderLineBufferInner {
                reader: BufReader::new(reader),
                lines: Vec::with_capacity(max),
                max_lines: max,
                total_line_count: 0,
                done: false,
                line_callback: None,
            }),
        }
    }

    /// Set a callback that is invoked for each line read.
    pub fn set_line_callback<F>(&self, callback: F)
    where
        F: Fn(&str) + Send + 'static,
    {
        self.inner.lock().unwrap().line_callback = Some(Box::new(callback));
    }

    /// Read the next line from the underlying reader.
    ///
    /// Returns `Ok(line)` for each line, or `Err(io::ErrorKind::UnexpectedEof)`
    /// when the reader is exhausted.
    pub fn read_line(&self) -> io::Result<String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.done {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "reader done"));
        }

        let mut line = String::new();
        match inner.reader.read_line(&mut line) {
            Ok(0) => {
                inner.done = true;
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"))
            }
            Ok(_) => {
                // Strip trailing newline
                let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                let owned = trimmed.to_string();
                inner.add_line(&owned);
                Ok(owned)
            }
            Err(e) => {
                inner.done = true;
                Err(e)
            }
        }
    }

    /// Read all remaining lines until EOF.
    pub fn read_all(&self) {
        loop {
            if self.read_line().is_err() {
                break;
            }
        }
    }

    /// Check if the reader has been fully consumed.
    pub fn is_done(&self) -> bool {
        self.inner.lock().unwrap().done
    }

    /// Get a copy of all buffered lines.
    pub fn get_lines(&self) -> Vec<String> {
        self.inner.lock().unwrap().lines.clone()
    }

    /// Get the number of lines currently in the buffer.
    pub fn get_line_count(&self) -> usize {
        self.inner.lock().unwrap().lines.len()
    }

    /// Get the total number of lines read (may exceed buffer capacity).
    pub fn get_total_line_count(&self) -> usize {
        self.inner.lock().unwrap().total_line_count
    }
}

impl<R: Read> ReaderLineBufferInner<R> {
    fn add_line(&mut self, line: &str) {
        if self.lines.len() >= self.max_lines {
            self.lines.remove(0);
        }
        self.lines.push(line.to_string());
        self.total_line_count += 1;

        if let Some(ref cb) = self.line_callback {
            cb(line);
        }
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Arc;

    #[test]
    fn test_read_single_line() {
        let data = Cursor::new("hello\n");
        let buf = ReaderLineBuffer::new(data, 10);

        let line = buf.read_line().unwrap();
        assert_eq!(line, "hello");
        assert_eq!(buf.get_line_count(), 1);
        assert_eq!(buf.get_total_line_count(), 1);
    }

    #[test]
    fn test_read_multiple_lines() {
        let data = Cursor::new("line1\nline2\nline3\n");
        let buf = ReaderLineBuffer::new(data, 10);

        assert_eq!(buf.read_line().unwrap(), "line1");
        assert_eq!(buf.read_line().unwrap(), "line2");
        assert_eq!(buf.read_line().unwrap(), "line3");
        assert!(buf.read_line().is_err());
        assert!(buf.is_done());
    }

    #[test]
    fn test_read_all() {
        let data = Cursor::new("a\nb\nc\n");
        let buf = ReaderLineBuffer::new(data, 10);

        buf.read_all();

        assert!(buf.is_done());
        let lines = buf.get_lines();
        assert_eq!(lines, vec!["a", "b", "c"]);
        assert_eq!(buf.get_total_line_count(), 3);
    }

    #[test]
    fn test_circular_buffer() {
        let data = Cursor::new("1\n2\n3\n4\n5\n");
        let buf = ReaderLineBuffer::new(data, 3);

        buf.read_all();

        let lines = buf.get_lines();
        assert_eq!(lines, vec!["3", "4", "5"]); // Only last 3
        assert_eq!(buf.get_line_count(), 3);
        assert_eq!(buf.get_total_line_count(), 5);
    }

    #[test]
    fn test_default_max_lines() {
        let data = Cursor::new("test\n");
        let buf = ReaderLineBuffer::new(data, 0); // 0 = default

        buf.read_all();
        // Should use DEFAULT_MAX_LINES (1000), not crash
        assert_eq!(buf.get_total_line_count(), 1);
    }

    #[test]
    fn test_line_callback() {
        let data = Cursor::new("hello\nworld\n");
        let buf = ReaderLineBuffer::new(data, 10);

        let collected = Arc::new(Mutex::new(Vec::<String>::new()));
        let collected_clone = collected.clone();
        buf.set_line_callback(move |line| {
            collected_clone.lock().unwrap().push(line.to_string());
        });

        buf.read_all();

        let lines = collected.lock().unwrap();
        assert_eq!(*lines, vec!["hello", "world"]);
    }

    #[test]
    fn test_empty_input() {
        let data = Cursor::new("");
        let buf = ReaderLineBuffer::new(data, 10);

        assert!(buf.read_line().is_err());
        assert!(buf.is_done());
        assert_eq!(buf.get_line_count(), 0);
    }

    #[test]
    fn test_no_trailing_newline() {
        let data = Cursor::new("no newline at end");
        let buf = ReaderLineBuffer::new(data, 10);

        let line = buf.read_line().unwrap();
        assert_eq!(line, "no newline at end");
    }

    #[test]
    fn test_windows_line_endings() {
        let data = Cursor::new("line1\r\nline2\r\n");
        let buf = ReaderLineBuffer::new(data, 10);

        buf.read_all();
        let lines = buf.get_lines();
        assert_eq!(lines, vec!["line1", "line2"]);
    }

    #[test]
    fn test_get_lines_returns_copy() {
        let data = Cursor::new("a\nb\n");
        let buf = ReaderLineBuffer::new(data, 10);

        buf.read_all();
        let lines1 = buf.get_lines();
        let lines2 = buf.get_lines();
        assert_eq!(lines1, lines2);
    }
}
