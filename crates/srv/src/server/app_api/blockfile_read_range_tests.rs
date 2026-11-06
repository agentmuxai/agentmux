// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `blockfile:read_range` through the RPC engine: the windowing, the three places lines come from
//! (the `output.idx` index, the whole file, the broker's ring buffer), the generation guard and the
//! tail trims.

use super::*;
use crate::backend::mps::{MuxEvent, EVENT_BLOCK_FILE};
use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};
use base64::Engine as _;
use serde_json::{json, Value};

async fn read_range(state: &AppState, data: Value) -> Value {
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    engine.handle_message(RpcMessage {
        command: COMMAND_BLOCKFILE_READ_RANGE.to_string(),
        reqid: "req-1".to_string(),
        data: Some(data),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(resp.error.is_empty(), "{}", resp.error);
    resp.data.expect("result data")
}

fn req(block: &str, file: &str, offset: u64, limit: u64) -> Value {
    json!({"block_id": block, "filename": file, "offset": offset, "limit": limit})
}

fn seed(fs: &FileStore, zone: &str, file: &str, opts: FileOpts, body: &[u8]) {
    fs.make_file(zone, file, FileMeta::default(), opts).unwrap();
    fs.append_lines(zone, file, body).unwrap();
}

fn lines(v: &Value) -> Vec<&str> {
    v["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect()
}

fn user(text: &str) -> String {
    json!({"type": "user", "message": {"content": text}}).to_string()
}

fn transcript(rows: &[String]) -> Vec<u8> {
    rows.iter().flat_map(|r| format!("{r}\n").into_bytes()).collect()
}

fn ring_event(block: &str, file: &str, data: &[u8]) -> MuxEvent {
    MuxEvent {
        event: EVENT_BLOCK_FILE.to_string(),
        scopes: vec![format!("block:{block}")],
        sender: String::new(),
        persist: 100,
        data: Some(json!({"filename": file, "data64": base64::engine::general_purpose::STANDARD.encode(data)})),
    }
}

#[tokio::test]
async fn the_index_serves_a_window_of_the_non_blank_lines_and_the_total() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-window";
    seed(&state.filestore, block, "output", FileOpts::default(), b"{\"n\":0}\n\n   \n{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n{\"n\":4}\n");

    let r = read_range(&state, req(block, "output", 1, 2)).await;
    assert_eq!(lines(&r), ["{\"n\":1}", "{\"n\":2}"]);
    assert_eq!(r["total"], json!(5));
    if let Some(stamps) = r.get("stamps") {
        assert_eq!(stamps.as_array().unwrap().len(), 2, "one stamp per returned line");
    }
    assert!(r.get("offset").is_none(), "only a tail read names where lines start");

    let all = read_range(&state, req(block, "output", 0, 50_000)).await;
    assert_eq!(lines(&all).len(), 5, "a limit past the cap still returns what there is");

    let past = read_range(&state, req(block, "output", 9, 5)).await;
    assert_eq!((lines(&past).len(), past["total"].clone()), (0, json!(5)));
    let none = read_range(&state, req(block, "output", 0, 0)).await;
    assert_eq!((lines(&none).len(), none["total"].clone()), (0, json!(5)));
}

#[tokio::test]
async fn a_file_that_is_not_the_transcript_is_read_whole_and_named_by_no_stream() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-notes";
    seed(&state.filestore, block, "notes", FileOpts::default(), b"first\nsecond\n\nthird\nfourth\n");

    let r = read_range(&state, req(block, "notes", 1, 2)).await;
    assert_eq!(lines(&r), ["second", "third"]);
    assert_eq!(r["total"], json!(4), "blank lines are not counted");
    assert!(r.get("stream").is_none() && r.get("gen").is_none() && r.get("stamps").is_none());

    let past = read_range(&state, req(block, "notes", 10, 5)).await;
    assert_eq!((lines(&past).len(), past["total"].clone()), (0, json!(4)));

    // Only a counted transcript has a generation to expect.
    let mut expecting = req(block, "notes", 0, 5);
    expecting["expect_gen"] = json!("some-gen");
    let mismatch = read_range(&state, expecting).await;
    assert_eq!(mismatch["gen_mismatch"], json!(true));
    assert_eq!(lines(&mismatch).len(), 0);
}

#[tokio::test]
async fn a_circular_output_is_read_whole_not_through_the_index() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-circular";
    let opts = FileOpts { circular: true, maxsize: 1 << 20, ..Default::default() };
    seed(&state.filestore, block, "output", opts, b"one\ntwo\nthree\n");
    let r = read_range(&state, req(block, "output", 1, 5)).await;
    assert_eq!(lines(&r), ["two", "three"]);
    assert_eq!(r["total"], json!(3));
}

#[tokio::test]
async fn with_no_file_the_brokers_ring_buffer_answers() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-ring";
    state.broker.publish(ring_event(block, "output", b"alpha\nbeta\n"));
    state.broker.publish(ring_event(block, "other", b"not this file\n"));
    state.broker.publish(ring_event("blk-rr-other", "output", b"not this block\n"));
    state.broker.publish(ring_event(block, "output", b"\ngamma\n"));
    state.broker.publish(MuxEvent {
        data: Some(json!({"filename": "output", "data64": "!!not base64!!"})),
        ..ring_event(block, "output", b"")
    });

    let r = read_range(&state, req(block, "output", 1, 5)).await;
    assert_eq!(lines(&r), ["beta", "gamma"]);
    assert_eq!(r["total"], json!(3));

    let nothing = read_range(&state, req("blk-rr-empty", "output", 0, 5)).await;
    assert_eq!((lines(&nothing).len(), nothing["total"].clone()), (0, json!(0)));
}

#[tokio::test]
async fn an_empty_file_falls_back_to_the_ring_buffer_too() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-emptyfile";
    state.filestore.make_file(block, "tty", FileMeta::default(), FileOpts::default()).unwrap();
    state.broker.publish(ring_event(block, "tty", b"from the ring\n"));
    let r = read_range(&state, req(block, "tty", 0, 5)).await;
    assert_eq!(lines(&r), ["from the ring"]);
}

#[tokio::test]
async fn tail_turns_keeps_the_last_turns_and_says_where_they_start() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-turns";
    let rows = [
        user("first question"),
        json!({"type": "assistant", "n": 1}).to_string(),
        user("second question"),
        json!({"type": "assistant", "n": 2}).to_string(),
        json!({"type": "assistant", "n": 3}).to_string(),
    ];
    seed(&state.filestore, block, "output", FileOpts::default(), &transcript(&rows));

    let mut last = req(block, "output", 0, 100);
    last["tail_turns"] = json!(1);
    let r = read_range(&state, last).await;
    assert_eq!(lines(&r), [rows[2].as_str(), rows[3].as_str(), rows[4].as_str()]);
    assert_eq!(r["offset"], json!(2));
    assert_eq!(r["total"], json!(5));
    assert!(r.get("stream").is_some(), "the read still names the generation it saw");

    let mut both = req(block, "output", 0, 100);
    both["tail_turns"] = json!(5);
    let whole = read_range(&state, both).await;
    assert_eq!((lines(&whole).len(), whole["offset"].clone()), (5, json!(0)), "fewer turns than asked keeps everything");
}

#[tokio::test]
async fn tail_bytes_keeps_the_newest_lines_that_fit_and_starts_at_a_turn_when_it_can() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-bytes";
    let rows = [
        user("first question"),
        json!({"type": "assistant", "n": 1}).to_string(),
        user("second question"),
        json!({"type": "assistant", "n": 2}).to_string(),
    ];
    seed(&state.filestore, block, "output", FileOpts::default(), &transcript(&rows));

    let mut one = req(block, "output", 0, 100);
    one["tail_bytes"] = json!(1);
    let r = read_range(&state, one).await;
    assert_eq!(lines(&r), [rows[3].as_str()], "the newest line always stays");
    assert_eq!(r["offset"], json!(3));

    let mut fits = req(block, "output", 0, 100);
    fits["tail_bytes"] = json!(rows[2].len() + rows[3].len() + 2);
    let r = read_range(&state, fits).await;
    assert_eq!(lines(&r), [rows[2].as_str(), rows[3].as_str()]);
    assert_eq!(r["offset"], json!(2));

    let mut roomy = req(block, "output", 0, 100);
    roomy["tail_bytes"] = json!(1_000_000);
    let r = read_range(&state, roomy).await;
    assert_eq!((lines(&r).len(), r["offset"].clone()), (4, json!(0)));
}

#[tokio::test]
async fn a_read_that_expects_the_generation_it_saw_is_served() {
    let state = crate::server::tests::test_state();
    let block = "blk-rr-expect";
    seed(&state.filestore, block, "output", FileOpts::default(), b"{\"a\":1}\n{\"b\":2}\n");
    let gen = state.filestore.line_state(block, "output").unwrap().unwrap().counted.unwrap().gen;
    let mut expecting = req(block, "output", 0, 10);
    expecting["expect_gen"] = json!(gen);
    let r = read_range(&state, expecting).await;
    assert_eq!(lines(&r), ["{\"a\":1}", "{\"b\":2}"]);
    assert_eq!(r["gen"], json!(gen));
    assert!(r.get("gen_mismatch").is_none());
}
