// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent:open_tail`: the slice, where it sits, and that it reads through the
//! WAL reader pool only.
//! The open-first-screen design (#5156), under #5141.

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

const BLOCK: &str = "blk-open-tail";

/// A file-backed store, so it has the reader pool production has.
fn file_store(tag: &str) -> (Arc<FileStore>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("open-tail-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fs = Arc::new(FileStore::open(&dir.join("t.db")).unwrap());
    fs.make_file(BLOCK, "output", FileMeta::default(), FileOpts::default()).unwrap();
    (fs, dir)
}

fn mem_store() -> Arc<Store> {
    Arc::new(Store::open_in_memory().unwrap())
}

fn cmd(tail_bytes: Option<u64>) -> CommandAgentOpenTailData {
    CommandAgentOpenTailData {
        block_id: BLOCK.into(),
        filename: "output".into(),
        definition_id: None,
        tail_bytes,
        tail_turns: None,
        expect_gen: None,
    }
}

fn run(fs: &Arc<FileStore>, c: CommandAgentOpenTailData) -> AgentOpenTailResult {
    open_tail(fs, None, &mem_store(), c).unwrap()
}

fn line(i: usize, pad: usize) -> String {
    format!("{{\"type\":\"assistant\",\"i\":{i},\"pad\":\"{}\"}}", "x".repeat(pad))
}

fn user(i: usize) -> String {
    serde_json::json!({"type": "user", "message": {"role": "user", "content": format!("question {i}")}}).to_string()
}

/// Append `lines` (each a record) one by one, and return them.
fn seed(fs: &FileStore, lines: &[String]) {
    for l in lines {
        fs.append_lines(BLOCK, "output", format!("{l}\n").as_bytes()).unwrap();
    }
}

#[test]
fn the_slice_is_the_newest_lines_and_its_positions_name_where_it_sits() {
    let (fs, dir) = file_store("slice");
    let all: Vec<String> = (0..2000).map(|i| line(i, 60 + i % 40)).collect();
    seed(&fs, &all);
    let r = run(&fs, cmd(Some(8 * 1024)));
    let (start, end, total) = (r.start_line.unwrap(), r.end_line.unwrap(), r.total_lines.unwrap());
    assert_eq!(end, 2000, "the stream's lines at the snapshot");
    assert_eq!(total, 2000);
    assert_eq!(end, start + r.lines.len() as u64, "end_line == start_line + lines.length");
    assert_eq!(r.lines, all[start as usize..end as usize], "the lines are exactly those at those positions");
    assert!(r.lines.len() > 20 && r.lines.len() < 200, "about 8 KB of lines, not the file ({} lines)", r.lines.len());
    assert!(r.has_more_before.unwrap());
    assert_eq!(r.stream.as_deref(), Some("b:blk-open-tail"));
    assert_eq!(r.gen, Some(fs.line_state(BLOCK, "output").unwrap().unwrap().counted.unwrap().gen));
    // The bytes the offsets name are the lines.
    let whole = fs.read_file(BLOCK, "output").unwrap().unwrap();
    let slice = &whole[r.start_byte.unwrap() as usize..r.end_byte.unwrap() as usize];
    assert_eq!(slice, format!("{}\n", r.lines.join("\n")).as_bytes());
    assert_eq!(r.end_byte.unwrap(), whole.len() as u64);
    assert_eq!(r.size.unwrap(), whole.len() as u64);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_short_transcript_comes_back_whole_from_line_zero() {
    let (fs, dir) = file_store("short");
    let all: Vec<String> = (0..5).map(|i| line(i, 10)).collect();
    seed(&fs, &all);
    let r = run(&fs, cmd(None));
    assert_eq!((r.start_line, r.end_line), (Some(0), Some(5)));
    assert_eq!(r.lines, all);
    assert_eq!(r.has_more_before, Some(false));
    assert_eq!(r.start_byte, Some(0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_empty_transcript_is_an_empty_slice() {
    let (fs, dir) = file_store("empty");
    let r = run(&fs, cmd(None));
    assert!(r.lines.is_empty());
    assert_eq!((r.start_line, r.end_line, r.total_lines), (Some(0), Some(0), Some(0)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_newest_line_comes_back_even_when_it_is_larger_than_the_window() {
    let (fs, dir) = file_store("big");
    seed(&fs, &[line(0, 10), line(1, 10), line(2, 200_000)]);
    let r = run(&fs, cmd(Some(4 * 1024)));
    assert_eq!(r.lines.len(), 1, "only the newest, which alone exceeds the window");
    assert_eq!(r.lines[0].len(), line(2, 200_000).len());
    assert_eq!((r.start_line, r.end_line), (Some(2), Some(3)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn blank_lines_are_not_lines_and_do_not_shift_the_positions() {
    let (fs, dir) = file_store("blank");
    fs.append_lines(BLOCK, "output", b"{\"a\":1}\n\n   \n{\"b\":2}\n\n{\"c\":3}\n").unwrap();
    let r = run(&fs, cmd(None));
    assert_eq!(r.lines, vec!["{\"a\":1}", "{\"b\":2}", "{\"c\":3}"]);
    assert_eq!((r.start_line, r.end_line), (Some(0), Some(3)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_slice_starts_at_a_turn_start_like_read_range_does() {
    let (fs, dir) = file_store("turns");
    let mut all = Vec::new();
    for t in 0..40 {
        all.push(user(t));
        for k in 0..6 {
            all.push(line(t * 10 + k, 150));
        }
    }
    seed(&fs, &all);
    let r = run(&fs, cmd(Some(3 * 1024)));
    assert!(r.lines.first().unwrap().contains("\"type\":\"user\""), "opens on a user turn, not mid-turn: {}", &r.lines[0][..60.min(r.lines[0].len())]);
    assert_eq!(r.end_line.unwrap(), r.start_line.unwrap() + r.lines.len() as u64);
    assert_eq!(r.lines, all[r.start_line.unwrap() as usize..]);

    let mut c = cmd(Some(1 << 20));
    c.tail_turns = Some(2);
    let two = run(&fs, c);
    assert_eq!(two.lines.iter().filter(|l| l.contains("\"type\":\"user\"")).count(), 2, "the last two turns");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stamps_run_parallel_to_the_lines() {
    let (fs, dir) = file_store("stamps");
    for i in 0..300 {
        fs.append_lines_stamped(BLOCK, "output", format!("{}\n", line(i, 80)).as_bytes(), "output.tsidx").unwrap();
    }
    let r = run(&fs, cmd(Some(6 * 1024)));
    let stamps = r.stamps.expect("a sidecar exists");
    assert_eq!(stamps.len(), r.lines.len());
    assert!(stamps.iter().all(|s| *s > 0), "every line in the batch window has a receive time");
    assert!(stamps.windows(2).all(|w| w[0] <= w[1]), "receive times do not go backwards");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn no_sidecar_means_no_stamps() {
    let (fs, dir) = file_store("nostamps");
    seed(&fs, &[line(0, 10), line(1, 10)]);
    assert!(run(&fs, cmd(None)).stamps.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_counter_behind_the_file_is_caught_up_in_memory_and_nothing_is_written() {
    let (fs, dir) = file_store("behind");
    let all: Vec<String> = (0..500).map(|i| line(i, 50)).collect();
    seed(&fs, &all);
    // Roll the counter back to after the first 100 lines, as if a build that
    // doesn't maintain it appended the rest.
    let first100: usize = all[..100].iter().map(|l| l.len() + 1).sum();
    {
        let c = fs.lock_conn();
        c.execute(
            "UPDATE db_wave_file SET lines = 100, lines_size = ?1, lines_tail = ?1 WHERE zoneid = ?2 AND name = 'output'",
            rusqlite::params![first100 as i64, BLOCK],
        )
        .unwrap();
    }
    let before = fs.lock_conn().query_row("SELECT lines, lines_size FROM db_wave_file WHERE name = 'output'", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap();
    let r = run(&fs, cmd(Some(4 * 1024)));
    assert_eq!(r.end_line, Some(500), "the lines past the stale counter are counted");
    assert_eq!(r.lines, all[r.start_line.unwrap() as usize..500]);
    let after = fs.lock_conn().query_row("SELECT lines, lines_size FROM db_wave_file WHERE name = 'output'", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap();
    assert_eq!(before, after, "a read never writes the counter");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_transcript_without_a_usable_epoch_has_no_line_positions() {
    let (fs, dir) = file_store("uncounted");
    seed(&fs, &[line(0, 10), line(1, 10)]);
    fs.lock_conn().execute("UPDATE db_wave_file SET gen = NULL WHERE name = 'output'", []).unwrap();
    let r = run(&fs, cmd(None));
    assert!(r.stream.is_none() && r.gen.is_none() && r.start_line.is_none() && r.end_line.is_none());
    assert!(r.lines.is_empty());
    assert!(r.size.unwrap() > 0, "the size is still reported");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_replaced_generation_is_a_mismatch_not_other_lines() {
    let (fs, dir) = file_store("mismatch");
    seed(&fs, &[line(0, 10)]);
    let old = fs.line_state(BLOCK, "output").unwrap().unwrap().counted.unwrap().gen;
    fs.write_file(BLOCK, "output", format!("{}\n", line(9, 10)).as_bytes()).unwrap();
    let mut c = cmd(None);
    c.expect_gen = Some(old);
    let r = run(&fs, c);
    assert_eq!(r.gen_mismatch, Some(true));
    assert!(r.lines.is_empty());
    assert!(r.gen.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn it_never_waits_for_the_writer_connection() {
    let (fs, dir) = file_store("nolock");
    let all: Vec<String> = (0..300).map(|i| line(i, 60)).collect();
    seed(&fs, &all);
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let holder = {
        let fs = fs.clone();
        std::thread::spawn(move || {
            let _conn = fs.lock_conn(); // the writer, held for as long as the test needs
            held_tx.send(()).unwrap();
            let _ = done_rx.recv_timeout(std::time::Duration::from_secs(10));
        })
    };
    held_rx.recv().unwrap();
    let started = std::time::Instant::now();
    let r = run(&fs, cmd(Some(4 * 1024)));
    let took = started.elapsed();
    done_tx.send(()).unwrap();
    holder.join().unwrap();
    assert!(took < std::time::Duration::from_secs(2), "took {took:?} with the writer connection held");
    assert_eq!(r.end_line, Some(300));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_session_snapshot_comes_back_with_the_tail() {
    let (fs, dir) = file_store("overlay");
    seed(&fs, &[line(0, 10)]);
    crate::backend::agent_session::write_session_state(&fs, "agent-overlay-1", b"{\"overlay\":true}").unwrap();
    let mut c = cmd(None);
    c.definition_id = Some("agent-overlay-1".into());
    let r = run(&fs, c);
    assert_eq!(r.snapshot.as_deref(), Some("{\"overlay\":true}"));
    assert!(r.snapshot_modts.unwrap() > 0);
    let none = run(&fs, cmd(None));
    assert!(none.snapshot.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_the_transcript_has_a_tail() {
    let (fs, dir) = file_store("filename");
    let mut c = cmd(None);
    c.filename = "term".into();
    assert!(open_tail(&fs, None, &mem_store(), c).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

/// Through the RPC engine, as the pane calls it: the command is registered and
/// the answer has the field names the frontend's `OpenTailResult` reads.
#[tokio::test]
async fn the_command_is_registered_and_answers_with_the_frontends_field_names() {
    let state = crate::server::tests::test_state();
    let block = "blk-open-tail-rpc";
    state.filestore.make_file(block, "output", FileMeta::default(), FileOpts::default()).unwrap();
    for i in 0..30 {
        state.filestore.append_lines(block, "output", format!("{}\n", line(i, 40)).as_bytes()).unwrap();
    }
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, &state);
    engine.handle_message(RpcMessage {
        command: COMMAND_AGENT_OPEN_TAIL.to_string(),
        reqid: "req-1".to_string(),
        data: Some(serde_json::json!({"block_id": block, "filename": "output", "tail_bytes": 1024})),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(resp.error.is_empty(), "{}", resp.error);
    let v = resp.data.expect("result data");
    for key in ["lines", "stream", "gen", "start_line", "end_line", "total_lines", "start_byte", "end_byte", "size", "has_more_before"] {
        assert!(v.get(key).is_some(), "missing `{key}` in {v}");
    }
    assert_eq!(v["end_line"], serde_json::json!(30));
    assert_eq!(v["stream"], serde_json::json!("b:blk-open-tail-rpc"));
    assert_eq!(v["end_line"].as_u64().unwrap(), v["start_line"].as_u64().unwrap() + v["lines"].as_array().unwrap().len() as u64);
    assert!(v.get("gen_mismatch").is_none() && v.get("snapshot").is_none(), "absent fields are omitted");
}

#[test]
fn a_slice_with_no_turn_start_reaches_back_to_the_turn_that_began_before_it() {
    let (fs, dir) = file_store("longturn");
    let mut all = vec![user(0), line(0, 100)];
    // One long running turn: 600 records of about 1 KB each, no turn start among the last 64 KB.
    all.push(user(1));
    all.extend((0..600).map(|i| line(i, 1000)));
    seed(&fs, &all);
    let r = run(&fs, cmd(Some(64 * 1024)));
    assert!(r.lines[0].contains("\"question 1\""), "starts at the running turn's user message, not mid-turn: {}", &r.lines[0][..60]);
    assert_eq!(r.lines.len(), 601, "the whole running turn, which fits the restore budget");
    assert_eq!(r.end_line.unwrap(), r.start_line.unwrap() + r.lines.len() as u64);
    assert_eq!(r.lines, all[r.start_line.unwrap() as usize..]);
    assert!(r.has_more_before.unwrap());
    let whole = fs.read_file(BLOCK, "output").unwrap().unwrap();
    assert_eq!(&whole[r.start_byte.unwrap() as usize..r.end_byte.unwrap() as usize], format!("{}\n", r.lines.join("\n")).as_bytes());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_turn_longer_than_the_restore_budget_may_start_mid_turn() {
    let (fs, dir) = file_store("hugeturn");
    let mut all = vec![user(0)];
    all.extend((0..1800).map(|i| line(i, 1000))); // about 1.8 MB, no other turn start
    seed(&fs, &all);
    let r = run(&fs, cmd(Some(64 * 1024)));
    assert!(!r.lines[0].contains("\"type\":\"user\""), "no turn start within the budget: the slice stays the 64 KB one");
    assert!(r.lines.len() < 200, "{} lines", r.lines.len());
    assert_eq!(r.end_line.unwrap(), r.start_line.unwrap() + r.lines.len() as u64);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn start_and_end_byte_are_exact_utf8_file_offsets_for_multibyte_text() {
    let (fs, dir) = file_store("utf8");
    let all: Vec<String> = (0..400)
        .map(|i| format!("{{\"i\":{i},\"text\":\"日本語のテキスト 🎉🚀 ça va très bien {}\"}}", "é".repeat(i % 30)))
        .collect();
    seed(&fs, &all);
    let r = run(&fs, cmd(Some(2 * 1024)));
    let whole = fs.read_file(BLOCK, "output").unwrap().unwrap();
    let (start, end) = (r.start_byte.unwrap() as usize, r.end_byte.unwrap() as usize);
    let joined = format!("{}\n", r.lines.join("\n"));
    assert_eq!(&whole[start..end], joined.as_bytes(), "the bytes at the offsets are exactly the returned lines");
    assert_eq!(end - start, joined.len(), "end_byte - start_byte is the UTF-8 length of the lines and their newlines");
    assert!(joined.len() > joined.chars().count(), "the text really is multibyte");
    assert_eq!(r.size.unwrap() as usize, whole.len());
    assert_eq!(end, whole.len());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_huge_tail_bytes_is_clamped_to_the_restore_budget() {
    let (fs, dir) = file_store("clamp");
    let all: Vec<String> = (0..3000).map(|i| line(i, 1000)).collect(); // about 3 MB
    seed(&fs, &all);
    let r = run(&fs, cmd(Some(u64::MAX)));
    let bytes: usize = r.lines.iter().map(|l| l.len() + 1).sum();
    assert!(bytes <= (1 << 20) + 2048, "{bytes} bytes returned for tail_bytes = u64::MAX");
    assert!(r.lines.len() > 500);
    assert_eq!(r.end_line, Some(3000));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stamps_reach_back_past_a_long_sidecar_to_the_oldest_line() {
    let (fs, dir) = file_store("oldstamps");
    // Each line its own batch, so the sidecar outgrows any fixed tail window.
    for i in 0..4000 {
        // A second apart, so every line gets a record of its own.
        fs.append_lines_stamped_at(BLOCK, "output", format!("{}\n", line(i, 20)).as_bytes(), "output.tsidx", 1_000_000 + i as i64 * 1000).unwrap();
    }
    let sidecar = fs.stat(BLOCK, "output.tsidx").unwrap().unwrap().size;
    assert!(sidecar > 100 * 1024, "the sidecar is {sidecar} bytes");
    let r = run(&fs, cmd(Some(256 * 1024)));
    let stamps = r.stamps.expect("a sidecar exists");
    assert_eq!(stamps.len(), r.lines.len());
    assert!(r.lines.len() > 1500, "{} lines", r.lines.len());
    assert!(stamps.iter().all(|s| *s > 0), "even the oldest line of a long slice has its receive time");
    let _ = std::fs::remove_dir_all(dir);
}
