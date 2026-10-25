// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Transcript appends are written first and published with their positions
//! (`transcript::append::append_transcript`, Phase 5a-3).

use std::sync::{Arc, Barrier, Mutex};

use base64::Engine as _;

use crate::backend::transcript::append::append_transcript;
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

/// The broker argument of `append_transcript`, from a `Broker` or an
/// `Arc<Broker>` alike.
fn at(b: &mps::Broker) -> Option<&mps::Broker> {
    Some(b)
}

fn pos_of<'a>(ev: &'a mps::WSFileEventData, prefix: &str) -> &'a mps::StreamPos {
    ev.pos.iter().find(|p| p.stream.starts_with(prefix)).unwrap_or_else(|| panic!("no {prefix} position in {ev:?}"))
}

fn mem_store() -> crate::backend::storage::store::Store {
    crate::backend::storage::store::Store::open_in_memory().unwrap()
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

    append_transcript(at(&broker), block, b"{\"a\":1}\n{\"b\":2}\n", Some(&fs), Some((&gfs, ZONE)), true, None);
    append_transcript(at(&broker), block, b"{\"c\":3}\n", Some(&fs), Some((&gfs, ZONE)), true, None);

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
    append_transcript(at(&broker), block, b"{\"a\":1}\n", Some(&fs), Some((&gfs, ZONE)), true, None);
    append_transcript(at(&broker), block, b"{\"b\":22}\n", Some(&fs), Some((&gfs, ZONE)), true, None);
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
                    append_transcript(at(&broker), block, line.as_bytes(), Some(&fs), Some((&gfs, ZONE)), true, None);
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
    append_transcript(at(&broker), block, b"{\"b\":2}\n", Some(&fs), None, true, None);
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
    append_transcript(at(&broker), block, b"{\"a\":1}\n", Some(&fs), None, true, None);
    // Blank lines, CRLF-only lines and an unterminated last line are
    // normalized away before the write; the event carries what was written.
    append_transcript(at(&broker), block, b"\n{\"b\":2}\n \r\n\n{\"c\":3}", Some(&fs), None, true, None);
    // Nothing but blanks: nothing written, nothing announced.
    append_transcript(at(&broker), block, b"\n  \r\n", Some(&fs), None, true, None);

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
        append_transcript(at(&broker), "bench-local", line.as_bytes(), Some(&fs), None, true, None);
        local_only.push(t.elapsed().as_micros());
        let t = std::time::Instant::now();
        append_transcript(at(&broker), "bench-both", line.as_bytes(), Some(&fs), Some((&gfs, ZONE)), true, None);
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
fn a_replaced_or_deleted_transcript_is_announced_with_its_new_generation() {
    let block = "blk-changed";
    let (broker, rec, fs, _gfs) = setup(block);
    fs.make_file(block, "output", Default::default(), Default::default()).unwrap();
    fs.write_file(block, "output", b"{\"a\":1}\n{\"b\":2}\n").unwrap();
    crate::backend::transcript::append::publish_transcript_changed(&broker, block, mps::FILE_OP_REPLACE, &fs, &mem_store());
    fs.delete_files(block, &["output"]).unwrap();
    crate::backend::transcript::append::publish_transcript_changed(&broker, block, mps::FILE_OP_DELETE, &fs, &mem_store());

    let seen = rec.seen.lock().unwrap();
    let (replaced, deleted) = (&seen[0].0, &seen[1].0);
    assert_eq!(replaced.fileop, "replace");
    let b = pos_of(replaced, "b:");
    assert_eq!((b.line, b.lines), (0, 2));
    assert_eq!(deleted.fileop, "delete");
    assert!(deleted.pos.is_empty());
}

#[test]
fn an_append_during_a_replace_is_published_after_it() {
    // Review of #3636: a restore running while the agent writes must not let
    // an append's write and event interleave with the replace and its event.
    let block = "blk-order-replace";
    let (broker, rec, fs, _gfs) = setup(block);
    let broker = Arc::new(broker);
    append_transcript(at(&broker), block, b"{\"old\":1}\n", Some(&fs), None, true, None);

    let (holding, held) = std::sync::mpsc::channel();
    let replacer = {
        let (broker, fs) = (broker.clone(), fs.clone());
        std::thread::spawn(move || {
            crate::backend::transcript::append::with_transcript_order(block, || {
                holding.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(150));
                fs.write_file(block, "output", b"{\"new\":1}\n").unwrap();
                crate::backend::transcript::append::publish_transcript_changed(&broker, block, mps::FILE_OP_REPLACE, &fs, &mem_store());
            })
        })
    };
    held.recv().unwrap();
    // Waits for the replace to finish, then lands in the new generation.
    append_transcript(at(&broker), block, b"{\"new\":2}\n", Some(&fs), None, true, None);
    replacer.join().unwrap();

    let seen = rec.seen.lock().unwrap();
    let ops: Vec<&str> = seen.iter().map(|(e, _)| e.fileop.as_str()).collect();
    assert_eq!(ops, ["append", "replace", "append"]);
    let after = pos_of(&seen[2].0, "b:");
    assert_eq!((after.line, after.gen.clone()), (1, gen_of(&fs, block)));
    assert_eq!(pos_of(&seen[1].0, "b:").gen, after.gen);
}

#[test]
fn a_persisted_user_line_is_announced_as_an_echo_with_its_position() {
    // Phase 5a-3c: the stdin user line used to be written without any event,
    // leaving a hole in the pane's line sequence. It is now announced, marked
    // as an echo so the pane adds no second node for it.
    let block = "blk-echo";
    let (broker, rec, fs, _gfs) = setup(block);
    append_transcript(at(&broker), block, b"{\"type\":\"assistant\"}\n", Some(&fs), None, true, None);
    crate::backend::transcript::append::persist_user_line(Some(&broker), block, b"{\"type\":\"user\"}\n", Some(&fs), None);

    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let (reply, echo) = (&seen[0].0, &seen[1].0);
    assert_eq!(reply.echo, None);
    assert_eq!(echo.echo.as_deref(), Some("stdin"));
    let b = pos_of(echo, "b:");
    assert_eq!((b.line, b.lines), (1, 2), "the echo fills line 1: no gap after the reply");
}

#[test]
fn a_message_no_pane_typed_is_announced_as_a_plain_record_the_pane_shows() {
    // A jekt to a Codex agent: no composer made a node for it, so it is not an
    // echo. It still fills its line, so the stream has no gap.
    let block = "blk-injected";
    let (broker, rec, fs, _gfs) = setup(block);
    append_transcript(at(&broker), block, b"{\"type\":\"assistant\"}\n", Some(&fs), None, true, None);
    crate::backend::transcript::append::persist_injected_user_line(Some(&broker), block, b"{\"type\":\"user\"}\n", Some(&fs), None);

    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].0.echo, None);
    let b = pos_of(&seen[1].0, "b:");
    assert_eq!((b.line, b.lines), (1, 2));
}

#[test]
fn an_echo_is_mirrored_with_positions_in_both_streams() {
    // `persist_user_line` resolves the process-wide global store; drive the
    // same echo append against a test global store to check the mirror side.
    let block = "blk-echo-global";
    let (broker, rec, fs, gfs) = setup(block);
    append_transcript(at(&broker), block, b"{\"type\":\"assistant\"}\n", Some(&fs), Some((&gfs, ZONE)), true, None);
    append_transcript(at(&broker), block, b"{\"type\":\"user\"}\n", Some(&fs), Some((&gfs, ZONE)), true, Some("stdin"));
    append_transcript(at(&broker), block, b"{\"type\":\"assistant\"}\n", Some(&fs), Some((&gfs, ZONE)), true, None);

    let seen = rec.seen.lock().unwrap();
    let echoes: Vec<Option<&str>> = seen.iter().map(|(e, _)| e.echo.as_deref()).collect();
    assert_eq!(echoes, [None, Some("stdin"), None]);
    for (k, (ev, _)) in seen.iter().enumerate() {
        let (b, g) = (pos_of(ev, "b:"), pos_of(ev, "g:"));
        assert_eq!((b.line, g.line), (k as u64, k as u64), "event {k}: the echo leaves no gap in either stream");
    }
    let stored = gfs.read_bytes_db(ZONE, "output", 0, gfs.line_state(ZONE, "output").unwrap().unwrap().size).unwrap();
    assert_eq!(std::str::from_utf8(&stored).unwrap().lines().nth(1), Some("{\"type\":\"user\"}"));
}

#[test]
fn without_a_filestore_the_event_still_goes_out_without_positions() {
    let block = "blk-nofs";
    let (broker, rec, _fs, _gfs) = setup(block);
    append_transcript(at(&broker), block, b"{\"a\":1}\n", None, None, false, None);
    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].0.pos.is_empty());
    assert_eq!(seen[0].0.offset, None);
}

// ---- single copy: SPEC_AGENT_TRANSCRIPT_SINGLE_COPY_2026_10_19.md step B ----

use crate::backend::transcript::append::append_transcript_with;

fn has_output(fs: &FileStore, zone: &str) -> bool {
    fs.stat(zone, "output").unwrap().is_some_and(|f| f.size > 0)
}

#[test]
fn with_single_copy_a_defined_agent_is_written_to_its_zone_only() {
    let block = "blk-single";
    let (broker, rec, fs, gfs) = setup(block);
    append_transcript_with(at(&broker), block, b"{\"a\":1}\n{\"b\":2}\n", Some(&fs), Some((&gfs, ZONE)), true, None, true);
    append_transcript_with(at(&broker), block, b"{\"c\":3}\n", Some(&fs), Some((&gfs, ZONE)), true, None, true);
    assert!(!has_output(&fs, block), "nothing in the block's own store");
    assert!(fs.stat(block, "output.tsidx").unwrap().is_none(), "nor its sidecar");
    assert_eq!(gfs.read_file(ZONE, "output").unwrap().unwrap(), b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n");
    assert!(gfs.stat(ZONE, "output.tsidx").unwrap().is_some(), "the zone keeps its sidecar");
    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (ev, _) in seen.iter() {
        assert!(ev.pos.iter().all(|p| p.stream.starts_with("g:")), "only the zone's position: {ev:?}");
        assert_eq!(ev.offset, None);
    }
    let (e0, e1) = (&seen[0].0, &seen[1].0);
    assert_eq!((pos_of(e0, "g:").line, pos_of(e0, "g:").lines), (0, 2));
    assert_eq!((pos_of(e1, "g:").line, pos_of(e1, "g:").lines), (2, 3));
}

#[test]
fn without_the_setting_both_copies_are_written_as_before() {
    let block = "blk-both";
    let (broker, rec, fs, gfs) = setup(block);
    append_transcript_with(at(&broker), block, b"{\"a\":1}\n", Some(&fs), Some((&gfs, ZONE)), true, None, false);
    assert!(has_output(&fs, block) && has_output(&gfs, ZONE));
    let seen = rec.seen.lock().unwrap();
    assert_eq!(seen[0].0.offset, Some(0));
    assert_eq!(seen[0].0.pos.len(), 2);
}

#[test]
fn a_quick_launch_pane_keeps_its_own_copy_with_the_setting_on() {
    let block = "blk-quick";
    let (broker, _rec, fs, gfs) = setup(block);
    let provider_zone = "agent:claude:current";
    append_transcript_with(at(&broker), block, b"{\"a\":1}\n", Some(&fs), Some((&gfs, provider_zone)), true, None, true);
    assert!(has_output(&fs, block), "the provider's zone is shared by every pane: the block keeps its copy");
    assert!(has_output(&gfs, provider_zone));
}

#[test]
fn a_writer_with_no_global_zone_keeps_writing_the_block_copy() {
    // A fenced writer passes no global zone.
    let block = "blk-fenced";
    let (broker, rec, fs, _gfs) = setup(block);
    append_transcript_with(at(&broker), block, b"{\"a\":1}\n", Some(&fs), None, true, None, true);
    assert!(has_output(&fs, block));
    assert_eq!(rec.seen.lock().unwrap()[0].0.offset, Some(0));
}

#[test]
fn a_failed_global_write_falls_back_to_the_block_copy_so_the_line_is_not_lost() {
    let block = "blk-fallback";
    let dir = std::env::temp_dir().join(format!("single-copy-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("g.db");
    drop(FileStore::open(&db).unwrap());
    let read_only = Arc::new(FileStore::open_read_only(&db).unwrap());
    let (broker, rec, fs, _) = setup(block);
    append_transcript_with(at(&broker), block, b"{\"a\":1}\n", Some(&fs), Some((&read_only, ZONE)), true, None, true);
    assert!(has_output(&fs, block), "the global store refused the write: the line went to the block's copy");
    let seen = rec.seen.lock().unwrap();
    assert!(seen[0].0.pos.iter().any(|p| p.stream.starts_with("b:")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_replace_announcement_names_the_zone_when_the_block_has_no_copy() {
    use crate::backend::transcript::source::TranscriptSource;
    let block = "blk-announce";
    let (broker, rec, fs, gfs) = setup(block);
    gfs.make_file(ZONE, "output", Default::default(), Default::default()).unwrap();
    gfs.append_lines(ZONE, "output", b"{\"a\":1}\n{\"b\":2}\n").unwrap();
    let source = TranscriptSource { fs: &gfs, zone: ZONE.to_string(), global: true, own: None };
    crate::backend::transcript::append::publish_transcript_changed_from(&broker, block, mps::FILE_OP_REPLACE, &fs, &source);
    let seen = rec.seen.lock().unwrap();
    let ev = &seen[0].0;
    assert_eq!(ev.fileop, mps::FILE_OP_REPLACE);
    let p = pos_of(ev, "g:");
    assert_eq!((p.stream.as_str(), p.line, p.lines), ("g:agent:def-5a3:current", 0, 2));
    assert_eq!(p.gen, gen_of(&gfs, ZONE));
}
