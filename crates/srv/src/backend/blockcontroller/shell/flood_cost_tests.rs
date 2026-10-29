// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a terminal flood costs srv per batch (task #7342). The batch is what the PTY flusher hands
//! `flush_pty_batch`: one filestore stat, one base64 + broadcast, one write transaction, the term cap's
//! bookkeeping. These tests drive that path with synthetic output of a given batch size and report
//! the time per megabyte and the log the store wrote; the ignored one is the measurement
//! (`cargo test --release -p agentmux-srv flood_cost -- --ignored --nocapture`).

use std::sync::Arc;
use std::time::Instant;

use super::lifecycle::flush_pty_batch;
use crate::backend::mps;
use crate::backend::storage::filestore::FileStore;

/// A client that does what the websocket one does to an event before the socket: serialize it.
struct SerializingClient;

impl mps::WpsClient for SerializingClient {
    fn send_event(&self, _route_id: &str, event: mps::MuxEvent) {
        let _ = serde_json::to_vec(&event);
    }
}

fn broker(subscribed: bool) -> mps::Broker {
    let broker = mps::Broker::new();
    broker.set_client(Box::new(SerializingClient));
    if subscribed {
        broker.subscribe("flood-route", mps::SubscriptionRequest { event: mps::EVENT_BLOCK_FILE.to_string(), scopes: vec![], allscopes: true });
    }
    broker
}

/// Output like a flood: `yes`-style short lines, in `batch`-sized pieces. Returns (seconds, batches, log bytes).
fn flood(total: usize, batch: usize, subscribed: bool, dir: &std::path::Path) -> (f64, usize, u64) {
    let path = dir.join(format!("flood-{batch}-{subscribed}.db"));
    let fs = Arc::new(FileStore::open(&path).expect("open filestore"));
    let broker = broker(subscribed);
    let line = b"y the quick brown fox jumps over the lazy dog 0123456789\r\n";
    let chunk: Vec<u8> = line.iter().copied().cycle().take(batch).collect();
    let mut line_buf = Vec::new();
    let batches = total / batch;
    let t0 = Instant::now();
    for _ in 0..batches {
        flush_pty_batch(&broker, "flood-block", &chunk, &[], Some(&fs), &mut line_buf, None);
    }
    let secs = t0.elapsed().as_secs_f64();
    let wal = std::fs::metadata(path.with_extension("db-wal")).map(|m| m.len()).unwrap_or(0);
    (secs, batches, wal)
}

#[test]
#[ignore = "a measurement, not a check"]
fn flood_cost_by_batch_size() {
    let dir = tempfile::tempdir().unwrap();
    let total = 64 << 20;
    println!("64 MB of output through flush_pty_batch (stat + base64 + broadcast + write + cap), release build:");
    println!("{:>10} {:>10} {:>12} {:>10} {:>14} {:>12}", "batch", "batches", "listener", "seconds", "ms per MB", "wal at end");
    for batch in [4096usize, 16 * 1024, 64 * 1024, 256 * 1024] {
        for subscribed in [false, true] {
            let (secs, batches, wal) = flood(total, batch, subscribed, dir.path());
            println!("{:>10} {:>10} {:>12} {:>10.2} {:>14.1} {:>12}", batch, batches, if subscribed { "serializing" } else { "none" }, secs, secs * 1000.0 / 64.0, wal);
        }
    }
}

#[test]
fn a_flood_written_in_batches_reads_back_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("whole.db");
    let fs = Arc::new(FileStore::open(&path).unwrap());
    let broker = broker(true);
    let mut line_buf = Vec::new();
    let mut sent = Vec::new();
    for i in 0..50usize {
        let piece: Vec<u8> = format!("line {i} {}\r\n", "x".repeat(300)).into_bytes();
        flush_pty_batch(&broker, "whole-block", &piece, &[], Some(&fs), &mut line_buf, None);
        sent.extend_from_slice(&piece);
    }
    assert_eq!(fs.read_file("whole-block", "term").unwrap().unwrap(), sent);
}
