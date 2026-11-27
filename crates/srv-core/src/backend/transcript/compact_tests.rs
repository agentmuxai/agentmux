// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

fn line(v: Value) -> String {
    v.to_string()
}

fn ev(parent: Value, event: Value) -> String {
    line(json!({ "type": "stream_event", "session_id": "sess", "parent_tool_use_id": parent, "event": event }))
}

fn start(id: &str) -> String {
    ev(Value::Null, json!({ "type": "message_start", "message": { "id": id, "role": "assistant", "content": [] } }))
}

fn block_start(ix: u64, block: Value) -> String {
    ev(Value::Null, json!({ "type": "content_block_start", "index": ix, "content_block": block }))
}

fn delta(ix: u64, d: Value) -> String {
    ev(Value::Null, json!({ "type": "content_block_delta", "index": ix, "delta": d }))
}

fn text_delta(ix: u64, t: &str) -> String {
    delta(ix, json!({ "type": "text_delta", "text": t }))
}

fn stop(ix: u64) -> String {
    ev(Value::Null, json!({ "type": "content_block_stop", "index": ix }))
}

fn msg_end() -> Vec<String> {
    vec![ev(Value::Null, json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" } })), ev(Value::Null, json!({ "type": "message_stop" }))]
}

fn assistant(id: &str, blocks: Vec<Value>) -> String {
    line(json!({ "type": "assistant", "session_id": "sess", "message": { "id": id, "role": "assistant", "content": blocks } }))
}

fn user(text: &str) -> String {
    line(json!({ "type": "user", "session_id": "sess", "message": { "role": "user", "content": text } }))
}

fn tool_result(tool: &str, out: &str) -> String {
    line(json!({ "type": "user", "session_id": "sess", "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": tool, "content": out }] } }))
}

fn result() -> String {
    line(json!({ "type": "result", "session_id": "sess", "subtype": "success", "usage": { "output_tokens": 3 } }))
}

fn init() -> String {
    line(json!({ "type": "system", "subtype": "init", "session_id": "sess" }))
}

/// One streamed text reply and its complete line.
fn text_reply(id: &str, text: &str) -> Vec<String> {
    let mut v = vec![start(id), block_start(0, json!({ "type": "text", "text": "" }))];
    let half = text.len() / 2;
    v.push(text_delta(0, &text[..half]));
    v.push(text_delta(0, &text[half..]));
    v.push(stop(0));
    v.push(assistant(id, vec![json!({ "type": "text", "text": text })]));
    v.extend(msg_end());
    v
}

fn tool_reply(id: &str, tool: &str, input: &Value) -> Vec<String> {
    let raw = input.to_string();
    let (a, b) = raw.split_at(raw.len() / 3);
    vec![
        start(id),
        block_start(0, json!({ "type": "tool_use", "id": tool, "name": "Bash", "input": {} })),
        delta(0, json!({ "type": "input_json_delta", "partial_json": a })),
        delta(0, json!({ "type": "input_json_delta", "partial_json": b })),
        stop(0),
        assistant(id, vec![json!({ "type": "tool_use", "id": tool, "name": "Bash", "input": input })]),
    ]
}

fn t_event(which: &str) -> Value {
    match which {
        "start" => json!({ "type": "message_start", "message": { "id": "m9", "role": "assistant", "content": [] } }),
        "block" => json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
        _ => json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "hi" } }),
    }
}

fn kinds(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| parse(l).map_or("unparsed".to_string(), |v| line_type(&v).to_string())).collect()
}

fn check(old: &[String], end: WindowEnd) -> (Vec<String>, Plan) {
    let (new, plan) = compact_lines(old, end);
    verify(old, &new).unwrap_or_else(|e| panic!("verify failed: {e}"));
    (new, plan)
}

#[test]
fn a_streamed_text_reply_keeps_its_complete_line_and_loses_its_fragments() {
    let mut old = vec![user("hello")];
    old.extend(text_reply("m1", "the answer is 42"));
    old.push(result());
    let (new, plan) = check(&old, WindowEnd::Open);
    assert_eq!(kinds(&new), vec!["user", "assistant", "result"]);
    assert_eq!(new[0], old[0]);
    assert_eq!(new[2], old.last().unwrap().as_str());
    // The complete line is kept byte for byte.
    assert!(old.contains(&new[1]));
    assert_eq!(plan.stats.dropped_fragments, old.len() - 3);
    assert_eq!(plan.stats.interrupted_lines, 0);
    assert!(no_fragments(&new).is_ok());
}

#[test]
fn a_tool_call_whose_input_arrived_in_pieces_is_proven_against_its_complete_line() {
    let input = json!({ "command": "ls -la", "description": "list" });
    let mut old = vec![user("go")];
    old.extend(tool_reply("m1", "toolu_1", &input));
    old.extend(msg_end());
    old.push(tool_result("toolu_1", "file-a\nfile-b"));
    old.push(result());
    let (new, _) = check(&old, WindowEnd::Open);
    assert_eq!(kinds(&new), vec!["user", "assistant", "user", "result"]);
    // The tool input and the tool output are both still there, whole.
    assert!(new[1].contains("ls -la"));
    assert!(new[2].contains("file-a\\nfile-b"));
}

#[test]
fn thinking_with_its_signature_and_one_complete_line_per_block() {
    let old: Vec<String> = vec![
        start("m1"),
        block_start(0, json!({ "type": "thinking", "thinking": "", "signature": "" })),
        delta(0, json!({ "type": "thinking_delta", "thinking": "let me " })),
        delta(0, json!({ "type": "thinking_delta", "thinking": "think" })),
        delta(0, json!({ "type": "signature_delta", "signature": "SIG==" })),
        stop(0),
        assistant("m1", vec![json!({ "type": "thinking", "thinking": "let me think", "signature": "SIG==" })]),
        block_start(1, json!({ "type": "text", "text": "" })),
        text_delta(1, "done"),
        stop(1),
        assistant("m1", vec![json!({ "type": "text", "text": "done" })]),
        result(),
    ];
    let (new, plan) = check(&old, WindowEnd::Open);
    assert_eq!(kinds(&new), vec!["assistant", "assistant", "result"]);
    assert_eq!(plan.stats.unproven_fragments, 0);
}

#[test]
fn a_reply_cut_off_before_its_complete_line_becomes_one_interrupted_line() {
    let old: Vec<String> = vec![
        user("go"),
        start("m1"),
        block_start(0, json!({ "type": "text", "text": "" })),
        text_delta(0, "half an ans"),
        text_delta(0, "wer"),
        block_start(1, json!({ "type": "tool_use", "id": "toolu_9", "name": "Edit", "input": {} })),
        delta(1, json!({ "type": "input_json_delta", "partial_json": "{\"file\":\"a" })),
        init(),
        user("again"),
    ];
    let (new, plan) = check(&old, WindowEnd::Open);
    assert_eq!(kinds(&new), vec!["user", "assistant", "system", "user"]);
    let v: Value = serde_json::from_str(&new[1]).unwrap();
    assert_eq!(v["interrupted"], true);
    assert_eq!(v["message"]["id"], "m1");
    assert_eq!(v["message"]["content"][0]["text"], "half an answer");
    assert_eq!(v["message"]["content"][1]["name"], "Edit");
    assert_eq!(v["message"]["content"][1]["partial_input_json"], "{\"file\":\"a");
    assert_eq!(plan.stats.interrupted_lines, 1);
    assert_eq!(v["session_id"], "sess");
    assert_eq!(plan.synth[0].after, 6, "after the message's last fragment");
}

#[test]
fn a_message_that_may_still_complete_is_pending_in_an_open_window_and_interrupted_at_the_end() {
    let old: Vec<String> = vec![start("m1"), block_start(0, json!({ "type": "text", "text": "" })), text_delta(0, "so far")];
    let (open, p) = check(&old, WindowEnd::Open);
    assert_eq!(open, old, "an open window leaves a message without a boundary alone");
    assert_eq!(p.stats.pending_fragments, 3);
    let (fin, p) = check(&old, WindowEnd::Final);
    assert_eq!(kinds(&fin), vec!["assistant"]);
    assert_eq!(p.stats.interrupted_lines, 1);
}

#[test]
fn a_fragment_that_differs_from_its_complete_line_is_not_lost() {
    let mut old = vec![start("m1"), block_start(0, json!({ "type": "text", "text": "" })), text_delta(0, "one thing"), stop(0)];
    old.push(assistant("m1", vec![json!({ "type": "text", "text": "another thing" })]));
    old.push(result());
    let (new, plan) = check(&old, WindowEnd::Open);
    // The block has no counterpart, the turn is over: it is interrupted, with its fragments' own text; the complete line stays.
    assert!(new.iter().any(|l| l.contains("another thing")));
    assert!(new.iter().any(|l| l.contains("one thing") && l.contains("interrupted")));
    assert_eq!(plan.stats.unproven_fragments, 0);
    assert!(no_fragments(&new).is_ok());
}

#[test]
fn what_the_module_cannot_reassemble_is_kept_whole() {
    let old: Vec<String> = vec![
        start("m1"),
        block_start(0, json!({ "type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": {} })),
        delta(0, json!({ "type": "input_json_delta", "partial_json": "{}" })),
        stop(0),
        assistant("m1", vec![json!({ "type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": {} })]),
        start("m2"),
        block_start(0, json!({ "type": "text", "text": "" })),
        delta(0, json!({ "type": "citations_delta", "citation": {} })),
        stop(0),
        assistant("m2", vec![json!({ "type": "text", "text": "" })]),
        result(),
    ];
    let (new, plan) = check(&old, WindowEnd::Final);
    assert_eq!(new, old, "an unknown block or delta kind keeps every fragment");
    assert_eq!(plan.stats.unproven_fragments, 8);
    assert_eq!(no_fragments(&new), Err(8));
}

#[test]
fn a_user_role_stream_message_is_not_an_assistant_reply_and_stays() {
    let old: Vec<String> = vec![
        ev(Value::Null, json!({ "type": "message_start", "message": { "id": "u1", "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t", "content": "x" }] } })),
        ev(Value::Null, json!({ "type": "message_stop" })),
        result(),
    ];
    let (new, plan) = check(&old, WindowEnd::Final);
    assert_eq!(new, old);
    assert_eq!(plan.stats.unproven_fragments, 2);
}

#[test]
fn fragments_with_no_message_start_are_orphans_and_stay() {
    let old: Vec<String> = vec![text_delta(0, "tail of something"), stop(0), user("hi")];
    let (new, plan) = check(&old, WindowEnd::Final);
    assert_eq!(new, old);
    assert_eq!(plan.stats.orphan_fragments, 2);
}

#[test]
fn a_subagent_stream_interleaved_with_the_main_one_is_told_apart_by_its_parent_tool_use() {
    let sub = |e: Value| ev(json!("toolu_parent"), e);
    let old: Vec<String> = vec![
        start("main1"),
        sub(json!({ "type": "message_start", "message": { "id": "sub1", "role": "assistant", "content": [] } })),
        block_start(0, json!({ "type": "text", "text": "" })),
        sub(json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })),
        text_delta(0, "main text"),
        sub(json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "sub text" } })),
        assistant("sub1", vec![json!({ "type": "text", "text": "sub text" })]),
        assistant("main1", vec![json!({ "type": "text", "text": "main text" })]),
        result(),
    ];
    let (new, plan) = check(&old, WindowEnd::Open);
    assert_eq!(kinds(&new), vec!["assistant", "assistant", "result"]);
    assert_eq!(plan.stats.dropped_fragments, 6);
}

#[test]
fn every_other_line_is_kept_byte_for_byte_including_replays_and_unknown_kinds() {
    let replay = line(json!({ "type": "user", "isReplay": true, "uuid": "u-card", "message": { "role": "user", "content": "<local-command-stdout>Compacted</local-command-stdout>" } }));
    let old: Vec<String> = vec![
        "not json at all".to_string(),
        "{\"type\":\"user\" ".to_string(),
        replay.clone(),
        line(json!({ "type": "a_future_kind", "payload": [1, 2, 3] })),
        line(json!({ "type": "system", "subtype": "compact_boundary", "compact_metadata": { "trigger": "auto" } })),
        line(json!({ "type": "rate_limit_event", "info": {} })),
        line(json!({ "type": "assistant", "partial": true, "message": { "id": "m1", "content": [{ "type": "text", "text": "snapshot" }] } })),
        "".to_string(),
        result(),
    ];
    let (new, plan) = check(&old, WindowEnd::Final);
    assert_eq!(new, old);
    assert_eq!(plan.stats.kept, old.len());
    assert!(DROP_RULES.iter().all(|r| r.line_type == "stream_event"), "only fragments are ever named");
}

#[test]
fn compacting_a_compact_window_changes_nothing() {
    let mut old = vec![user("a")];
    old.extend(text_reply("m1", "first reply"));
    old.extend(tool_reply("m2", "toolu_2", &json!({ "command": "echo hi" })));
    old.push(tool_result("toolu_2", "hi"));
    old.push(start("m3"));
    old.push(block_start(0, json!({ "type": "text", "text": "" })));
    old.push(text_delta(0, "cut off"));
    old.push(result());
    let (once, _) = check(&old, WindowEnd::Final);
    let (twice, plan) = check(&once, WindowEnd::Final);
    assert_eq!(once, twice);
    assert_eq!(plan.stats.dropped_fragments, 0);
}

#[test]
fn the_checker_refuses_each_kind_of_wrong_result() {
    let mut old = vec![user("hello")];
    old.extend(text_reply("m1", "the answer"));
    old.push(result());
    let (good, _) = check(&old, WindowEnd::Open);

    // L1: a line reordered.
    let mut reordered = good.clone();
    reordered.swap(0, 1);
    assert!(matches!(verify(&old, &reordered), Err(CheckFailure::L1 { .. })));
    // L1: a line altered by one byte.
    let mut altered = good.clone();
    altered[0].push(' ');
    assert!(matches!(verify(&old, &altered), Err(CheckFailure::L1 { .. })));
    // L1: a complete line dropped.
    let mut missing = good.clone();
    missing.remove(1);
    assert!(matches!(verify(&old, &missing), Err(CheckFailure::L1 { .. })));
    // L2/L3: a fragment dropped that the complete line does not cover.
    let mut lossy_old = old.clone();
    lossy_old.insert(4, text_delta(0, " and more"));
    let lossy_new = good.clone();
    assert!(matches!(verify(&lossy_old, &lossy_new), Err(CheckFailure::L2 { .. })));
    // L3: a session id that only the dropped fragments carried.
    let only_frag = |t: &str| line(json!({ "type": "stream_event", "session_id": "only-in-fragments", "parent_tool_use_id": null, "event": t_event(t) }));
    let sess_old: Vec<String> = vec![
        only_frag("start"),
        only_frag("block"),
        only_frag("delta"),
        line(json!({ "type": "assistant", "message": { "id": "m9", "content": [{ "type": "text", "text": "hi" }] } })),
        result(),
    ];
    let (sess_new, _) = compact_lines(&sess_old, WindowEnd::Open);
    assert!(matches!(verify(&sess_old, &sess_new), Err(CheckFailure::L3 { .. })), "the session id would be in no kept line");
    // The right answer passes.
    assert_eq!(verify(&old, &good), Ok(()));
}

#[test]
fn the_checker_refuses_an_interrupted_line_that_does_not_match_its_fragments() {
    let old: Vec<String> = vec![start("m1"), block_start(0, json!({ "type": "text", "text": "" })), text_delta(0, "received text"), result()];
    let (good, _) = check(&old, WindowEnd::Open);
    assert_eq!(kinds(&good), vec!["assistant", "result"]);
    let forged = good[0].replace("received text", "invented text");
    let bad = vec![forged, good[1].clone()];
    assert!(matches!(verify(&old, &bad), Err(CheckFailure::L2 { .. })));
    // A kept fragment that the old window did not have is refused too.
    let mut extra = good.clone();
    extra.insert(0, text_delta(0, "never there"));
    assert!(matches!(verify(&old, &extra), Err(CheckFailure::L1 { .. })));
}

#[test]
fn a_survivor_fragment_fails_l4() {
    let lines = vec![user("x"), text_delta(0, "left over")];
    assert_eq!(no_fragments(&lines), Err(1));
    assert_eq!(no_fragments(&[user("x")]), Ok(()));
}
