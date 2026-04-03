// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RPC I/O adapters for different transport modes.
//! Port of Go's `pkg/wshutil/wshrpcio.go`.
//!
//! Provides adapters to convert between:
//! - Stream (JSON lines) ↔ message channels
//! - PTY (OSC-wrapped) ↔ message channels
//! - WebSocket (JSON packets) ↔ message channels

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use tokio::sync::mpsc;
use super::osc::encode_wave_osc_bytes;

/// Read JSON lines from a stream and send them to a channel.
///
/// Each line is sent as a separate message. Blocks until the reader is exhausted.
pub fn adapt_stream_to_msg_ch(
    input: impl Read + Send + 'static,
    output: mpsc::Sender<Vec<u8>>,
) -> std::thread::JoinHandle<Result<(), String>> {
    std::thread::spawn(move || {
        let reader = BufReader::new(input);
        for line in reader.lines() {
            let line = line.map_err(|e| format!("read error: {}", e))?;
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            output
                .blocking_send(trimmed.into_bytes())
                .map_err(|e| format!("channel send error: {}", e))?;
        }
        Ok(())
    })
}

/// Read messages from a channel and write them as JSON lines to a stream.
///
/// Each message is followed by a newline. Blocks until the channel is closed.
pub async fn adapt_output_ch_to_stream(
    mut output_ch: mpsc::Receiver<Vec<u8>>,
    mut output: impl Write,
) -> Result<(), String> {
    while let Some(msg) = output_ch.recv().await {
        output
            .write_all(&msg)
            .map_err(|e| format!("write error: {}", e))?;
        output
            .write_all(b"\n")
            .map_err(|e| format!("newline write error: {}", e))?;
        output.flush().map_err(|e| format!("flush error: {}", e))?;
    }
    Ok(())
}

/// Read messages from a channel and write them as OSC-escaped sequences to a PTY.
///
/// Each message is wrapped in the appropriate OSC escape sequence.
pub async fn adapt_msg_ch_to_pty(
    mut output_ch: mpsc::Receiver<Vec<u8>>,
    osc_esc: &str,
    mut output: impl Write,
) -> Result<(), String> {
    if osc_esc.len() != 5 {
        return Err("osc_esc must be 5 characters".to_string());
    }
    while let Some(msg) = output_ch.recv().await {
        let encoded =
            encode_wave_osc_bytes(osc_esc, &msg)?;
        output
            .write_all(&encoded)
            .map_err(|e| format!("write error: {}", e))?;
        output.flush().map_err(|e| format!("flush error: {}", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn test_adapt_stream_to_msg_ch() {
        let data = b"{\"cmd\":\"test\"}\n{\"cmd\":\"hello\"}\n";
        let (tx, mut rx) = mpsc::channel(10);

        let handle = adapt_stream_to_msg_ch(Cursor::new(data.to_vec()), tx);

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(String::from_utf8(msg1).unwrap(), "{\"cmd\":\"test\"}");

        let msg2 = rx.recv().await.unwrap();
        assert_eq!(String::from_utf8(msg2).unwrap(), "{\"cmd\":\"hello\"}");

        handle.join().unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_adapt_output_ch_to_stream() {
        let (tx, rx) = mpsc::channel(10);
        let mut output = Vec::new();

        tx.send(b"{\"result\":\"ok\"}".to_vec()).await.unwrap();
        drop(tx); // close channel

        adapt_output_ch_to_stream(rx, &mut output).await.unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "{\"result\":\"ok\"}\n");
    }
}
