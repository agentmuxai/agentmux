// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! CLI stdin reader for WSH command-line interface.
//! Port of Go's `pkg/wshutil/wshcmdreader.go`.
//!
//! Reads JSON-RPC messages from stdin for the `wsh` CLI tool.
//! Supports both single-shot commands and interactive mode.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read};
use tokio::sync::mpsc;

/// CLI stdin reader that parses JSON-RPC messages from standard input.
pub struct CmdReader {
    /// Channel to send parsed messages.
    pub msg_tx: mpsc::Sender<Vec<u8>>,
}

impl CmdReader {
    /// Create a new CmdReader with the given message sender.
    pub fn new(msg_tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self { msg_tx }
    }

    /// Read a single JSON message from stdin (blocking).
    pub fn read_single_message(input: impl Read) -> Result<Vec<u8>, String> {
        let mut reader = BufReader::new(input);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("read error: {}", e))?;

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err("empty input".to_string());
        }

        // Validate JSON
        serde_json::from_str::<serde_json::Value>(trimmed)
            .map_err(|e| format!("invalid JSON: {}", e))?;

        Ok(trimmed.as_bytes().to_vec())
    }

    /// Start reading JSON lines from stdin in a background thread.
    /// Each line is sent to the message channel.
    /// Returns a handle to the reader thread.
    pub fn start_reading(
        &self,
        input: impl Read + Send + 'static,
    ) -> std::thread::JoinHandle<Result<(), String>> {
        let tx = self.msg_tx.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(input);
            for line in reader.lines() {
                let line = line.map_err(|e| format!("read error: {}", e))?;
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }

                // Validate JSON before sending
                if serde_json::from_str::<serde_json::Value>(&trimmed).is_err() {
                    tracing::warn!("skipping invalid JSON line: {}", &trimmed[..trimmed.len().min(100)]);
                    continue;
                }

                tx.blocking_send(trimmed.into_bytes())
                    .map_err(|e| format!("channel send error: {}", e))?;
            }
            Ok(())
        })
    }

    /// Read all available input from stdin and return as a single message.
    /// Used for piped input (non-interactive).
    pub fn read_all(input: impl Read) -> Result<Vec<u8>, String> {
        let mut buf = String::new();
        let mut reader = BufReader::new(input);
        reader
            .read_to_string(&mut buf)
            .map_err(|e| format!("read error: {}", e))?;

        let trimmed = buf.trim();
        if trimmed.is_empty() {
            return Err("empty input".to_string());
        }

        Ok(trimmed.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_single_message() {
        let input = Cursor::new(b"{\"command\":\"test\"}\n");
        let msg = CmdReader::read_single_message(input).unwrap();
        assert_eq!(String::from_utf8(msg).unwrap(), "{\"command\":\"test\"}");
    }

    #[test]
    fn test_read_single_message_invalid_json() {
        let input = Cursor::new(b"not json\n");
        let result = CmdReader::read_single_message(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn test_read_single_message_empty() {
        let input = Cursor::new(b"\n");
        let result = CmdReader::read_single_message(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_all() {
        let input = Cursor::new(b"{\"data\": \"hello world\"}");
        let msg = CmdReader::read_all(input).unwrap();
        assert_eq!(
            String::from_utf8(msg).unwrap(),
            "{\"data\": \"hello world\"}"
        );
    }

    #[tokio::test]
    async fn test_start_reading() {
        let (tx, mut rx) = mpsc::channel(10);
        let reader = CmdReader::new(tx);

        let input = Cursor::new(b"{\"cmd\":\"a\"}\n{\"cmd\":\"b\"}\nnot-json\n{\"cmd\":\"c\"}\n");
        let handle = reader.start_reading(input);

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(String::from_utf8(msg1).unwrap(), "{\"cmd\":\"a\"}");

        let msg2 = rx.recv().await.unwrap();
        assert_eq!(String::from_utf8(msg2).unwrap(), "{\"cmd\":\"b\"}");

        // "not-json" is skipped
        let msg3 = rx.recv().await.unwrap();
        assert_eq!(String::from_utf8(msg3).unwrap(), "{\"cmd\":\"c\"}");

        handle.join().unwrap().unwrap();
    }
}
