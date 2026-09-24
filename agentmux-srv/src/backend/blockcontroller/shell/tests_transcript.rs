// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Transcript appends are written first and published with their positions
//! (`file_ops::append_transcript`, Phase 5a-3).

use std::sync::{Arc, Barrier, Mutex};

use base64::Engine as _;

use super::file_ops::append_transcript;
use super::handle_append_block_file;
use crate::backend::mps;
use crate::backend::storage::filestore::FileStore;

const ZONE: &str = "agent:def-5a3:current";

/// Records each event with the block file's size at the moment it arrived
/// (`publish` delivers synchronously), so a test can see whether the write
/// had already landed.
struct Recorder {
    fs: Arc<FileStore>,
    block_id: String,
    seen: Mutex<Vec<(mps::WSFileEventData, i64)>>,
}

impl mps::WpsClient for Arc<Recorder> {
    fn send_event(&self, _route_id: &str, event: mps::MuxEvent) {
        let data: mps::WSFileEventData = serde_json::from_value(event.data.unwrap()).unwrap();
        let size = self.fs.line_state(&self.block_id, "output").unwrap().map_or(-1, |s| s.size);
        self.seen.lock().unwrap().push((data, size));
    }
}

fn setup(block_id: &str) -> (mps::Broker, Arc<Recorder>, Arc<FileStore>, Arc<FileStore>) {
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    let gfs = Arc::new(FileStore::open_in_memory().unwrap());
    let broker = mps::Broker::new();
    let rec = Arc::new(Recorder { fs: fs.clone(), block_id: block_id.to_string(), seen: Mutex::new(Vec::new()) });
    broker.set_client(Box::new(Arc::clone(&rec)));
    broker.subscribe(
        "route",
        mps::SubscriptionRequest {
            event: mps::EVENT_BLOCK_FILE.to_string(),
            scopes: vec![format!("block:{block_id}")],
            allscopes: false,
        },
    );
    (broker, rec, fs, gfs)
}

fn pos_of<'a>(ev: &'a mps::WSFileEventData, prefix: &str) -> &'a mps::StreamPos {
    ev.pos.iter().find(|p| p.stream.starts_with(prefix)).unwrap_or_else(|| panic!("no {prefix} position in {ev:?}"))
}

fn gen_of(fs: &FileStore, zone: &str) -> String {
    fs.line_state(zone, "output").unwrap().unwrap().counted.unwrap().gen
}

#[test]
fn an_event_carries_each_streams_position_and_the_exact_offset() {
    let block = "blk-positions";
    let (broker, rec, fs, gfs) = setup(block);
    // Another block of the same agent already wrote to the global zone, so
    // the two streams number the same records differently.
    gfs.make_file(ZONE, "output", Default::default(), Default::default()).unwrap();
    gfs.append_lines(ZONE, "output", b"{\"other\":0}\n").unwrap();

    append_transcript(&broker, block, b"{\"a\":1}\n{\"b\":2}\n", Some(&fs), Some((&gfs, ZONE)), true);
    append_transcript(&broker, block, b"{\"c\":3}\n", Some(&fs), Some((&gfs, ZONE)), true);

    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let (e0, e1) = (&seen[0].0, &seen[1].0);
    let (b0, g0, b1, g1) = (pos_of(e0, "b:"), pos_of(e0, "g:"), pos_of(e1, "b:"), pos_of(e1, "g:"));
    assert_eq!((b0.stream.as_str(), b0.line, b0.lines), ("b:blk-positions", 0, 2));
    assert_eq!((g0.stream.as_str(), g0.line, g0.lines), ("g:agent:def-5a3:current", 1, 3));
    assert_eq!((b1.line, b1.lines, g1.line, g1.lines), (2, 3, 3, 4));
    assert_eq!(b0.gen, gen_of(&fs, block));
    assert_eq!(g0.gen, gen_of(&gfs, ZONE));
    assert_eq!((e0.offset, e1.offset), (Some(0), Some(16)));
    let data = base64::engine::general_purpose::STANDARD.decode(&e1.data64).unwrap();
    assert_eq!(data, b"{\"c\":3}\n");
}

#[test]
fn the_event_goes_out_after_the_record_is_written() {
    let block = "blk-after";
    let (broker, rec, fs, gfs) = setup(block);
    append_transcript(&broker, block, b"{\"a\":1}\n", Some(&fs), Some((&gfs, ZONE)), true);
    append_transcript(&broker, block, b"{\"b\":22}\n", Some(&fs), Some((&gfs, ZONE)), true);
    let seen = rec.seen.lock().unwrap();
    // The block file already held each record when its event arrived.
    assert_eq!(seen.iter().map(|(_, size)| *size).collect::<Vec<_>>(), vec![8, 17]);
}

#[test]
fn one_blocks_events_leave_in_line_order_under_concurrent_writers() {
    const THREADS: usize = 6;
    const EACH: usize = 40;
    let block = "blk-order";
    let (broker, rec, fs, gfs) = setup(block);
    let broker = Arc::new(broker);
    let start = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let (broker, fs, gfs, start) = (broker.clone(), fs.clone(), gfs.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                for i in 0..EACH {
                    let line = format!("{{\"t\":{t},\"i\":{i}}}\n");
                    append_transcript(&broker, block, line.as_bytes(), Some(&fs), Some((&gfs, ZONE)), true);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), THREADS * EACH);
    let stored = fs.read_bytes_db(block, "output", 0, fs.line_state(block, "output").unwrap().unwrap().size).unwrap();
    let stored: Vec<&str> = std::str::from_utf8(&stored).unwrap().lines().collect();
    for (k, (ev, _)) in seen.iter().enumerate() {
        let (b, g) = (pos_of(ev, "b:"), pos_of(ev, "g:"));
        // Publish order is line order, in both streams.
        assert_eq!((b.line, g.line), (k as u64, k as u64), "event {k} out of order");
        // And the line each event names holds that event's record.
        let data = base64::engine::general_purpose::STANDARD.decode(&ev.data64).unwrap();
        assert_eq!(stored[b.line as usize].as_bytes(), &data[..data.len() - 1]);
    }
}

#[test]
fn a_torn_last_line_is_closed_and_keeps_its_index() {
    let block = "blk-torn";
    let (broker, rec, fs, _gfs) = setup(block);
    fs.make_file(block, "output", Default::default(), Default::default()).unwrap();
    fs.append_data(block, "output", b"{\"a\":1}\n{\"par").unwrap();
    append_transcript(&broker, block, b"{\"b\":2}\n", Some(&fs), None, true);
    let seen = rec.seen.lock().unwrap();
    let b = pos_of(&seen[0].0, "b:");
    assert_eq!((b.line, b.lines), (2, 3));
    let size = fs.line_state(block, "output").unwrap().unwrap().size;
    assert_eq!(fs.read_bytes_db(block, "output", 0, size).unwrap(), b"{\"a\":1}\n{\"par\n{\"b\":2}\n");
    // The event's offset is where its own bytes start: past the `\n` that
    // closed the torn line (review of #3635).
    let ev = &seen[0].0;
    let data = base64::engine::general_purpose::STANDARD.decode(&ev.data64).unwrap();
    assert_eq!(ev.offset, Some(14));
    assert_eq!(fs.read_bytes_db(block, "output", 14, data.len() as i64).unwrap(), data);
}

#[test]
fn an_event_carries_exactly_the_bytes_written_at_its_offset() {
    let block = "blk-exact";
    let (broker, rec, fs, _gfs) = setup(block);
    append_transcript(&broker, block, b"{\"a\":1}\n", Some(&fs), None, true);
    // Blank lines, CRLF-only lines and an unterminated last line are
    // normalized away before the write; the event carries what was written.
    append_transcript(&broker, block, b"\n{\"b\":2}\n \r\n\n{\"c\":3}", Some(&fs), None, true);
    // Nothing but blanks: nothing written, nothing announced.
    append_transcript(&broker, block, b"\n  \r\n", Some(&fs), None, true);

    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let ev = &seen[1].0;
    let data = base64::engine::general_purpose::STANDARD.decode(&ev.data64).unwrap();
    assert_eq!(data, b"{\"b\":2}\n{\"c\":3}\n");
    let at = ev.offset.unwrap() as i64;
    assert_eq!(fs.read_bytes_db(block, "output", at, data.len() as i64).unwrap(), data);
    assert_eq!(at + data.len() as i64, fs.line_state(block, "output").unwrap().unwrap().size);
}

#[test]
fn terminal_data_is_still_published_first_and_without_positions() {
    let block = "blk-term";
    let (broker, rec, fs, _gfs) = setup(block);
    handle_append_block_file(&broker, block, "term", b"$ ls\r\n", Some(&fs), None);
    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].0.pos.is_empty());
    assert_eq!(seen[0].0.offset, Some(0));
}

/// How long a transcript event now waits for its writes: the whole
/// `append_transcript` call, block file and global zone both file-backed
/// (WAL, like production). The spec's gate (§6.3.7 item 5) is p99 added
/// latency ≤ 2 ms on the local store. Not pass/fail; run with
/// `cargo test --bin agentmux-srv --release -- --ignored transcript_event_latency --nocapture`.
#[test]
#[ignore]
fn transcript_event_latency() {
    const N: usize = 3000;
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(FileStore::open(&dir.path().join("filestore.db")).unwrap());
    let gfs = Arc::new(FileStore::open(&dir.path().join("global.db")).unwrap());
    let broker = mps::Broker::new();
    let line = format!("{{\"type\":\"assistant\",\"text\":\"{}\"}}\n", "x".repeat(900));
    let mut local_only = Vec::with_capacity(N);
    let mut both = Vec::with_capacity(N);
    // Before 5a-3: the event went out first, then four separate commits —
    // block line, block stamp, global line, global stamp — which the stdout
    // reader waited for before reading its next line.
    let mut old_writes = Vec::with_capacity(N);
    for name in ["output", "output.tsidx"] {
        for (store, zone) in [(&fs, "bench-old"), (&gfs, "bench-old-global")] {
            store.make_file(zone, name, Default::default(), Default::default()).unwrap();
        }
    }
    let stamp = b"{\"off\":0,\"ms\":0}\n";
    for _ in 0..N {
        let t = std::time::Instant::now();
        fs.append_data("bench-old", "output", line.as_bytes()).unwrap();
        fs.append_data("bench-old", "output.tsidx", stamp).unwrap();
        gfs.append_data("bench-old-global", "output", line.as_bytes()).unwrap();
        gfs.append_data("bench-old-global", "output.tsidx", stamp).unwrap();
        old_writes.push(t.elapsed().as_micros());
    }
    for _ in 0..N {
        let t = std::time::Instant::now();
        append_transcript(&broker, "bench-local", line.as_bytes(), Some(&fs), None, true);
        local_only.push(t.elapsed().as_micros());
        let t = std::time::Instant::now();
        append_transcript(&broker, "bench-both", line.as_bytes(), Some(&fs), Some((&gfs, ZONE)), true);
        both.push(t.elapsed().as_micros());
    }
    for (label, mut us) in [
        ("old: 4 separate commits (reader blocked, event already out)", old_writes),
        ("new: block file only", local_only),
        ("new: block file + global zone (event waits)", both),
    ] {
        us.sort_unstable();
        let p = |q: f64| us[((us.len() as f64 * q) as usize).min(us.len() - 1)];
        println!("{label}: n={N} p50={}us p90={}us p99={}us max={}us", p(0.5), p(0.9), p(0.99), us[us.len() - 1]);
    }
}

#[test]
fn without_a_filestore_the_event_still_goes_out_without_positions() {
    let block = "blk-nofs";
    let (broker, rec, _fs, _gfs) = setup(block);
    append_transcript(&broker, block, b"{\"a\":1}\n", None, None, false);
    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].0.pos.is_empty());
    assert_eq!(seen[0].0.offset, None);
}
