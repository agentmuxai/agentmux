// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The two small tasks around a shell's PTY: the blocking reader that turns its output
//! into chunks for the flusher, and the input task that feeds it keystrokes, resizes
//! and signals.

use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use portable_pty::PtySize;
use tokio::sync::mpsc;

use super::controller::ShellControllerInner;
use super::lifecycle::PtyReadChunk;
use super::pty::PTY_READ_BUF_SIZE;
use crate::backend::mps;

/// Reads the PTY on a blocking thread and sends each read, with OSC sequences stripped
/// for an agent pane, to the flusher. Ends at EOF, dropping `pty_tx` to close the channel.
pub(super) fn spawn_pty_reader(
    reader: Box<dyn std::io::Read + Send>,
    pty_tx: mpsc::Sender<PtyReadChunk>,
    inner_read: Arc<Mutex<ShellControllerInner>>,
    broker_read: Option<Arc<mps::Broker>>,
    is_agent_read: bool,
    block_id_read: String,
) {
    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buf = [0u8; PTY_READ_BUF_SIZE];

        // OSC extractor: only for agent panes. Terminal panes forward
        // raw bytes to xterm.js which handles OSC natively — stripping
        // there would suppress the native window-title update. Agent
        // panes don't use xterm.js; OSC bytes in FileStore corrupt the
        // document renderer, so we extract and strip them here. Kept
        // in THIS (per-read) loop rather than the flusher: it is
        // cheap, and its own internal state must see every read in
        // order exactly as they arrive — moving it to the batched side
        // would change nothing about correctness but would mean
        // holding it across an .await, so it stays here for simplicity.
        let mut osc_extractor: Option<crate::backend::osc_extractor::OscExtractor> =
            if is_agent_read {
                Some(crate::backend::osc_extractor::OscExtractor::new())
            } else {
                None
            };

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    inner_read.lock().unwrap().last_pty_output = Some(Instant::now());
                    if broker_read.is_some() {
                        let raw = &buf[..n];

                        // Extract OSC sequences from agent-pane output.
                        // `cleaned` has OSC bytes removed; `osc_events` carries
                        // any normalised Claude Code title strings found in this chunk.
                        let mut cleaned_storage: Vec<u8> = Vec::new();
                        let mut osc_events: Vec<crate::backend::osc_extractor::OscEvent> =
                            Vec::new();
                        if let Some(ref mut ext) = osc_extractor {
                            let (cleaned, evs) = ext.feed(raw);
                            cleaned_storage = cleaned;
                            osc_events = evs;
                        }
                        let chunk: &[u8] = if osc_extractor.is_some() {
                            &cleaned_storage
                        } else {
                            raw
                        };

                        // `blocking_send`, not `send` — this closure runs
                        // on a `spawn_blocking` thread, not async, and a
                        // bounded channel's plain `send` is itself async.
                        // Blocks THIS (already-dedicated-to-blocking)
                        // thread once the channel is full, which is
                        // exactly the backpressure PTY_CHANNEL_CAPACITY's
                        // doc comment describes — never the async runtime.
                        //
                        // Best-effort beyond that: if the flusher task is
                        // gone (block torn down mid-read), there is
                        // nothing useful to do with this chunk — the PTY
                        // read loop itself must keep draining so the
                        // child process's stdout never backs up.
                        let _ = pty_tx.blocking_send(PtyReadChunk {
                            ack: None,
                            data: chunk.to_vec(),
                            osc_events,
                        });
                    }
                }
                Err(e) => {
                    tracing::debug!("PTY read error for {}: {}", block_id_read, e);
                    break;
                }
            }
        }
        // `pty_tx` drops here, closing the channel — the flusher below
        // sees `recv()` return `None` once it has drained everything
        // already sent, flushes any final partial batch, and exits.
        // (Still true with the barrier handle in play: that one is a
        // `WeakSender`, which does not keep the channel open. It must stay
        // weak — see its own comment above.)
    });
}

/// Routes the input channel to the PTY writer, resizes and signals. Owns the writer and the
/// master: dropping them closes the PTY, which makes the child exit.
pub(super) fn spawn_input_task(
    writer: Box<dyn std::io::Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    input_rx: mpsc::UnboundedReceiver<crate::backend::blockcontroller::BlockInputUnion>,
) {
    tokio::spawn(async move {
        let mut writer = writer;
        let mut input_rx = input_rx;
        while let Some(input) = input_rx.recv().await {
            if let Some(data) = input.input_data {
                use std::io::Write;
                if let Err(e) = writer.write_all(&data) {
                    tracing::debug!("PTY write error: {}", e);
                    break;
                }
            }
            if let Some(ref size) = input.term_size {
                let pty_size = PtySize {
                    rows: size.rows as u16,
                    cols: size.cols as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                };
                if let Err(e) = master.resize(pty_size) {
                    tracing::debug!("PTY resize error: {}", e);
                }
            }
            if input.sig_name.is_some() {
                // Drop writer + master to close PTY, which terminates the child
                break;
            }
        }
        // writer and master drop here → PTY closes → child gets EOF/terminates
    });
}
