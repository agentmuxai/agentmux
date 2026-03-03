// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Thread-safe byte buffer.
//! Port of Go's `pkg/util/syncbuf/syncbuf.go`.
//!
//! A `SyncBuffer` wraps a `Vec<u8>` behind a mutex and implements `Write`.

#![allow(dead_code)]

use std::io::{self, Write};
use std::sync::Mutex;

/// A thread-safe byte buffer.
pub struct SyncBuffer {
    buf: Mutex<Vec<u8>>,
}

impl Default for SyncBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncBuffer {
    /// Create a new empty buffer.
    pub fn new() -> Self {
        Self {
            buf: Mutex::new(Vec::new()),
        }
    }

    /// Get the buffer contents as a string.
    pub fn to_string_lossy(&self) -> String {
        let buf = self.buf.lock().unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Get the buffer contents as bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.buf.lock().unwrap().clone()
    }

    /// Get the current length in bytes.
    pub fn len(&self) -> usize {
        self.buf.lock().unwrap().len()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.lock().unwrap().is_empty()
    }
}

impl Write for SyncBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self.buf.lock().unwrap();
        inner.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Allow writing via shared reference (for thread-safe use).
impl SyncBuffer {
    /// Write data via shared reference (thread-safe).
    pub fn write_shared(&self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self.buf.lock().unwrap();
        inner.extend_from_slice(buf);
        Ok(buf.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let buf = SyncBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.to_string_lossy(), "");
    }

    #[test]
    fn test_write() {
        let mut buf = SyncBuffer::new();
        buf.write_all(b"hello").unwrap();
        assert_eq!(buf.to_string_lossy(), "hello");
        assert_eq!(buf.len(), 5);
    }

    #[test]
    fn test_multiple_writes() {
        let mut buf = SyncBuffer::new();
        buf.write_all(b"hello ").unwrap();
        buf.write_all(b"world").unwrap();
        assert_eq!(buf.to_string_lossy(), "hello world");
    }

    #[test]
    fn test_write_shared() {
        let buf = SyncBuffer::new();
        buf.write_shared(b"shared write").unwrap();
        assert_eq!(buf.to_string_lossy(), "shared write");
    }

    #[test]
    fn test_to_bytes() {
        let buf = SyncBuffer::new();
        buf.write_shared(b"\x00\x01\x02").unwrap();
        assert_eq!(buf.to_bytes(), vec![0, 1, 2]);
    }

    #[test]
    fn test_default() {
        let buf = SyncBuffer::default();
        assert!(buf.is_empty());
    }
}
