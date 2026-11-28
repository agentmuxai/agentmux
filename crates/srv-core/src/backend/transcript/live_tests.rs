// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;
use crate::backend::mps;
use crate::backend::storage::filestore::FileStore;
use crate::backend::transcript::append::append_transcript_with;

const ZONE: &str = "agent:def-live:current";

struct Env {
    block: String,
    fs: Arc<FileStore>,
    gfs: Arc<FileStore>,
}

fn env(block: &str) -> Env {
    Env { block: block.to_string(), fs: Arc::new(FileStore::open_in_memory().unwrap()), gfs: Arc::new(FileStore::open_in_memory().unwrap()) }
}

impl Env {
    /// Append `lines` as one call, as the stdout reader does for each line the CLI writes.
    fn write(&self, broker: Option<&mps::Broker>, lines: &[String]) {
        let data = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
        append_transcript_with(broker, &self.block, data.as_bytes(), Some(&self.fs), Some((&self.gfs, ZONE)), true, None, false);
    }

    fn text(&self, fs: &FileStore, zone: &str, name: &str) -> Option<String> {
        fs.read_file(zone, name).unwrap().map(|b| String::from_utf8(b).unwrap())
    }

    fn output(&self) -> String {
        self.text(&self.gfs, ZONE, "output").unwrap_or_default()
    }

    fn live(&self) -> String {
        self.text(&self.gfs, ZONE, LIVE_FILE).unwrap_or_default()
    }
}

fn line(v: Value) -> String {
    v.to_string()
}

fn ev(event: Value) -> String {
    line(json!({ "type": "stream_event", "session_id": "s", "parent_tool_use_id": null, "event": event }))
}

fn start(id: &str) -> String {
    ev(json!({ "type": "message_start", "message": { "id": id, "role": "assistant", "content": [] } }))
}

fn block(ix: u64, b: Value) -> String {
    ev(json!({ "type": "content_block_start", "index": ix, "content_block": b }))
}

fn text_delta(ix: u64, t: &str) -> String {
    ev(json!({ "type": "content_block_delta", "index": ix, "delta": { "type": "text_delta", "text": t } }))
}

fn stop_msg() -> String {
    ev(json!({ "type": "message_stop" }))
}

fn assistant(id: &str, text: &str) -> String {
    line(json!({ "type": "assistant", "session_id": "s", "message": { "id": id, "role": "assistant", "content": [{ "type": "text", "text": text }] } }))
}

fn user(t: &str) -> String {
    line(json!({ "type": "user", "session_id": "s", "message": { "role": "user", "content": t } }))
}

fn result() -> String {
    line(json!({ "type": "result", "session_id": "s", "subtype": "success" }))
}

fn init() -> String {
    line(json!({ "type": "system", "subtype": "init", "session_id": "s" }))
}

fn kinds(output: &str) -> Vec<String> {
    output.lines().map(|l| serde_json::from_str::<Value>(l).map(|v| v["type"].as_str().unwrap_or("").to_string()).unwrap_or_default()).collect()
}

/// Write a streamed reply one line at a time, as the CLI's stdout reader does.
fn stream_reply(e: &Env, id: &str, text: &str) {
    for l in [start(id), block(0, json!({ "type": "text", "text": "" })), text_delta(0, &text[..3]), text_delta(0, &text[3..]), assistant(id, text), stop_msg()] {
        e.write(None, &[l]);
    }
}

#[test]
fn a_streamed_reply_leaves_no_fragment_in_output_and_an_empty_live_file() {
    let e = env("blk-1");
    e.write(None, &[user("hello")]);
    stream_reply(&e, "m1", "marker-F-abc and the rest");
    e.write(None, &[result()]);
    assert_eq!(kinds(&e.output()), vec!["user", "assistant", "result"]);
    assert!(!e.output().contains("stream_event"));
    assert_eq!(e.live(), "", "the fragments left the live file once the message stopped");
    assert!(e.text(&e.fs, "blk-1", LIVE_FILE).is_none() && e.text(&e.fs, "blk-1", "output").unwrap().lines().count() == 3, "the block's own copy is the same");
}

#[test]
fn fragments_of_a_reply_in_progress_are_in_the_live_file_only() {
    let e = env("blk-2");
    for l in [start("m1"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "so far")] {
        e.write(None, &[l]);
    }
    assert_eq!(e.output(), "");
    assert_eq!(e.live().lines().count(), 3);
    assert!(e.live().contains("so far"));
    assert_eq!(e.text(&e.fs, "blk-2", LIVE_FILE).unwrap().lines().count(), 3, "and in the block's copy");
}

#[test]
fn a_reply_cut_off_at_the_turn_end_becomes_an_interrupted_line_before_the_boundary() {
    let e = env("blk-3");
    for l in [user("go"), start("m1"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "half an answer")] {
        e.write(None, &[l]);
    }
    e.write(None, &[result()]);
    assert_eq!(kinds(&e.output()), vec!["user", "assistant", "result"]);
    let v: Value = serde_json::from_str(e.output().lines().nth(1).unwrap()).unwrap();
    assert_eq!(v["interrupted"], true);
    assert_eq!(v["message"]["content"][0]["text"], "half an answer");
    assert_eq!(e.live(), "");
}

#[test]
fn a_fragment_that_cannot_be_proven_is_kept_in_output_at_the_turn_end() {
    let e = env("blk-4");
    let unknown = block(0, json!({ "type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": {} }));
    for l in [start("m1"), unknown.clone(), stop_msg()] {
        e.write(None, &[l]);
    }
    e.write(None, &[result()]);
    let kept = e.output();
    assert!(kept.contains("server_tool_use"), "nothing is lost for want of a proof");
    assert_eq!(kinds(&kept), vec!["stream_event", "stream_event", "stream_event", "result"]);
    assert_eq!(e.live(), "");
}

#[test]
fn a_kill_leaves_the_live_file_and_the_next_start_reassembles_it() {
    let e = env("blk-5");
    for l in [user("go"), start("m1"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "received before the kill")] {
        e.write(None, &[l]);
    }
    assert!(!e.live().is_empty());
    forget("blk-5"); // a new process: nothing in memory
    e.write(None, &[init()]);
    assert_eq!(kinds(&e.output()), vec!["user", "assistant", "system"]);
    assert!(e.output().contains("received before the kill"));
    assert_eq!(e.live(), "");
    assert!(e.text(&e.fs, "blk-5", LIVE_FILE).is_none());
}

#[test]
fn a_leftover_whose_complete_line_is_already_stored_is_dropped_without_a_copy() {
    let e = env("blk-6");
    for l in [start("m1"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "done text"), assistant("m1", "done text")] {
        e.write(None, &[l]);
    }
    // The message had not stopped: the fragments are still in the live file, the complete line in output.
    assert!(!e.live().is_empty());
    forget("blk-6");
    e.write(None, &[init()]);
    assert_eq!(kinds(&e.output()), vec!["assistant", "system"], "no interrupted line for a block that is stored whole");
    assert_eq!(e.live(), "");
}

#[test]
fn a_marker_carried_only_by_fragments_is_in_no_file_after_the_reply() {
    let e = env("blk-7");
    stream_reply(&e, "m1", "plain text");
    // F is in a delta that the complete line does not repeat; the reply completes by the next boundary.
    for l in [start("m2"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "MARKER-F-ONLY-IN-FRAGMENTS"), assistant("m2", "MARKER-F-ONLY-IN-FRAGMENTS"), stop_msg(), result()] {
        e.write(None, &[l]);
    }
    for (fs, zone) in [(&e.gfs, ZONE), (&e.fs, "blk-7")] {
        for f in fs.list_files(zone).unwrap() {
            let bytes = fs.read_file(zone, &f.name).unwrap().unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("stream_event"), "{} holds a fragment", f.name);
        }
    }
}

struct Recorder(Mutex<Vec<mps::WSFileEventData>>);

impl mps::WpsClient for Arc<Recorder> {
    fn send_event(&self, _route_id: &str, event: mps::MuxEvent) {
        self.0.lock().unwrap().push(serde_json::from_value(event.data.unwrap()).unwrap());
    }
}

#[test]
fn fragments_are_announced_with_no_position_and_complete_lines_with_one() {
    let e = env("blk-8");
    let broker = mps::Broker::new();
    let rec = Arc::new(Recorder(Mutex::new(Vec::new())));
    broker.set_client(Box::new(Arc::clone(&rec)));
    broker.subscribe("route", mps::SubscriptionRequest { event: mps::EVENT_BLOCK_FILE.to_string(), scopes: vec!["block:blk-8".to_string()], allscopes: false });
    for l in [start("m1"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "abcdef"), assistant("m1", "abcdef"), stop_msg()] {
        e.write(Some(&broker), &[l]);
    }
    let seen = rec.0.lock().unwrap();
    assert_eq!(seen.len(), 5);
    let with_pos = seen.iter().filter(|d| !d.pos.is_empty()).count();
    assert_eq!(with_pos, 1, "only the complete line is positioned");
    assert!(seen.iter().filter(|d| d.pos.is_empty()).all(|d| d.offset.is_none()));
}

#[test]
fn archiving_after_a_kill_keeps_the_leftover_with_its_own_conversation() {
    let e = env("blk-9");
    for l in [user("go"), start("m1"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "cut before the archive")] {
        e.write(None, &[l]);
    }
    forget("blk-9"); // the process was killed; nothing recovered the live file yet
    let archived = crate::backend::agent_session::archive::archive_session_in(&e.fs, Some(&e.gfs), "def-live").unwrap().unwrap();
    let held = e.text(&e.gfs, &archived.0, "output").unwrap();
    assert!(held.contains("cut before the archive") && held.contains("interrupted"), "{held}");
    assert!(e.text(&e.gfs, ZONE, LIVE_FILE).is_none(), "no leftover for the next conversation");
    e.write(None, &[user("new conversation")]);
    assert_eq!(kinds(&e.output()), vec!["user"], "the new conversation starts clean");
}

#[test]
fn a_cut_off_reply_the_shared_zone_failed_to_keep_is_recovered_from_the_other_copy() {
    let e = env("blk-10");
    for l in [user("go"), start("m1"), block(0, json!({ "type": "text", "text": "" })), text_delta(0, "kept only in the block's copy")] {
        e.write(None, &[l]);
    }
    e.gfs.delete_file(ZONE, LIVE_FILE).unwrap(); // the shared zone's live write had failed
    forget("blk-10");
    e.write(None, &[init()]);
    assert_eq!(kinds(&e.output()), vec!["user", "assistant", "system"]);
    assert!(e.output().contains("kept only in the block's copy"));
}

#[test]
fn settling_a_zone_with_no_live_file_is_a_success() {
    let e = env("blk-11");
    assert!(settle(&e.gfs, ZONE));
}

#[test]
fn a_write_without_an_agent_stream_is_not_split() {
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    let frag = start("m1");
    append_transcript_with(None, "term-1", format!("{frag}\n").as_bytes(), Some(&fs), None, false, None, false);
    assert!(fs.read_file("term-1", LIVE_FILE).unwrap().is_none());
    assert!(String::from_utf8(fs.read_file("term-1", "output").unwrap().unwrap()).unwrap().contains("stream_event"));
}
