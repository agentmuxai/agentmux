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

/// Counts the events a batch publishes, and serializes each as the websocket client does.
struct CountingClient(Arc<std::sync::atomic::AtomicUsize>);

impl mps::WpsClient for CountingClient {
    fn send_event(&self, _route_id: &str, event: mps::MuxEvent) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = serde_json::to_vec(&event);
    }
}

/// CPU time this process has used so far, in ms.
fn cpu_ms() -> u64 {
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), true, sysinfo::ProcessRefreshKind::nothing().with_cpu());
    sys.process(pid).map(|p| p.accumulated_cpu_time()).unwrap_or(0)
}

/// A real flood: a child process writes `FLOOD_MB` megabytes (default 64) to a real PTY (ConPTY on Windows),
/// read by the same loop and flusher the shell controller runs, into a real filestore. Prints the wall time,
/// the CPU this process spent, the size of the reads and of the batches, and whether the log holds what was
/// read (it can't past the term cap). A measurement, not a check:
/// `cargo test -p agentmux-srv real_pty_flood -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement, not a check"]
async fn real_pty_flood() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::Read;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mb: usize = std::env::var("FLOOD_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("flood.txt");
    {
        use std::io::Write;
        let line = b"y the quick brown fox jumps over the lazy dog 0123456789\r\n";
        let block: Vec<u8> = line.iter().copied().cycle().take(1 << 20).collect();
        let mut f = std::io::BufWriter::new(std::fs::File::create(&src).unwrap());
        for _ in 0..mb {
            f.write_all(&block).unwrap();
        }
    }
    let fs = Arc::new(FileStore::open(&dir.path().join("pty.db")).unwrap());
    let events = Arc::new(AtomicUsize::new(0));
    let broker = Arc::new(mps::Broker::new());
    broker.set_client(Box::new(CountingClient(events.clone())));
    broker.subscribe("flood-route", mps::SubscriptionRequest { event: mps::EVENT_BLOCK_FILE.to_string(), scopes: vec![], allscopes: true });

    let pair = native_pty_system().openpty(PtySize { rows: 50, cols: 200, pixel_width: 0, pixel_height: 0 }).unwrap();
    #[cfg(windows)]
    let mut cmd = CommandBuilder::new("cmd");
    #[cfg(windows)]
    cmd.args(["/C", "type", &src.to_string_lossy()]);
    #[cfg(unix)]
    let mut cmd = CommandBuilder::new("cat");
    #[cfg(unix)]
    cmd.arg(&src);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(super::pty::PTY_CHANNEL_CAPACITY);
    let flusher = tokio::spawn(super::lifecycle::run_pty_output_flusher(rx, Some(broker.clone()), "pty-flood".to_string(), Some(fs.clone()), false));

    let cpu0 = cpu_ms();
    let t0 = Instant::now();
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    // The read loop of `start()`: 4 KB reads, one chunk each.
    let read = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; super::pty::PTY_READ_BUF_SIZE];
        let (mut bytes, mut reads) = (0usize, 0usize);
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    bytes += n;
                    reads += 1;
                    let _ = tx.blocking_send(super::lifecycle::PtyReadChunk { data: buf[..n].to_vec(), osc_events: Vec::new(), ack: None });
                }
            }
        }
        (bytes, reads)
    });
    // ConPTY's host outlives the child, so the reader never sees EOF: end it once the child is gone and
    // the output has stopped coming.
    let waited = tokio::task::spawn_blocking(move || child.wait()).await.unwrap().unwrap();
    assert!(waited.success());
    let mut last = 0usize;
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let n = events.load(Ordering::Relaxed);
        if n == last && n > 0 {
            break;
        }
        last = n;
    }
    drop(pair.master);
    let (bytes, reads) = tokio::time::timeout(std::time::Duration::from_secs(30), read).await.expect("the reader ended").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), flusher).await.expect("the flusher ended").unwrap();
    let wall = t0.elapsed().as_secs_f64();
    let cpu = (cpu_ms() - cpu0) as f64 / 1000.0;
    let batches = events.load(Ordering::Relaxed);
    let logged = fs.read_file("pty-flood", "term").unwrap().map(|b| b.len()).unwrap_or(0);
    println!("real PTY flood: {mb} MB file; read {bytes} bytes in {reads} reads (avg {} B), {batches} batches (avg {} B)", bytes / reads.max(1), bytes / batches.max(1));
    println!("wall {wall:.1} s, this process's CPU {cpu:.1} s ({:.2} s per wall second, {:.0} ms per MB read), {:.2} MB/s", cpu / wall, cpu * 1000.0 / (bytes as f64 / 1e6), bytes as f64 / 1e6 / wall);
    println!("log holds {logged} bytes of {bytes} read (the term cap keeps the newest 50 MB)");
    assert!(bytes > mb << 20, "ConPTY/PTY output is at least the file");
}
