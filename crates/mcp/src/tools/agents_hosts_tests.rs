// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn ok_reply(host: &str, answer: Value) -> PeerReply {
    PeerReply { host: host.to_string(), state: "ok", detail: None, version: Some("0.60.37".to_string()), as_of_ms: Some(1_000), answer: Some(answer) }
}

/// A machine that did not answer, and what the LAN poll says of it.
fn away() -> (PeerReply, Value) {
    let lan = json!({ "version": "0.60.30", "agents": ["Old"], "agent_status": { "old": "idle" }, "agent_providers": { "old": "claude" } });
    (PeerReply::failed("away", "unreachable", "away did not answer in 3 seconds".to_string()), lan)
}

#[test]
fn a_peer_is_asked_by_its_name_and_by_channel_and_install_when_it_has_them() {
    assert_eq!(target_of(&json!({ "hostname": " box ", "channel": "", "install": "x" })), "box");
    assert_eq!(target_of(&json!({ "hostname": "box", "channel": "stable" })), "box/stable");
    assert_eq!(target_of(&json!({ "hostname": "box", "channel": "dev", "install": "windows-store" })), "box/dev/windows-store");
}

#[test]
fn host_all_asks_each_instance_once_and_nobody_without_a_name() {
    let local = json!({ "peers": [
        { "hostname": "a", "channel": "stable", "instance_id": "i1" },
        { "hostname": "a", "channel": "stable", "instance_id": "i1" },
        { "hostname": "a", "channel": "dev", "instance_id": "i2" },
        { "hostname": "", "channel": "", "instance_id": "i3" },
        { "hostname": "b" },
    ] });
    let names: Vec<String> = peers_of(&local).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["a/stable", "a/dev", "b"]);
    assert!(peers_of(&json!({})).is_empty());
}

#[test]
fn an_answer_is_ok_or_says_whether_the_peer_is_too_old_not_trusting_or_out_of_reach() {
    let done = json!({ "status": "done", "result": { "version": "0.60.37", "as_of_ms": 5, "answer": { "agents": [] } } });
    let ok = reply_of("there", &done);
    assert_eq!((ok.state, ok.version.as_deref(), ok.as_of_ms), ("ok", Some("0.60.37"), Some(5)));
    assert!(ok.answer.is_some());
    let old = reply_of("old", &json!({ "status": "refused", "refused_reason": "unknown_op", "result": null }));
    assert_eq!(old.state, "needs_upgrade");
    assert!(old.detail.unwrap().contains("newer version"));
    assert_eq!(reply_of("older", &json!({ "status": "refused", "refused_reason": "peer_cannot_do_remote_ops" })).state, "needs_upgrade");
    let trust = reply_of("new", &json!({ "status": "refused", "refused_reason": "needs_certificate" }));
    assert_eq!(trust.state, "refused");
    assert!(trust.detail.unwrap().contains("Approve this install"));
    let down = reply_of("gone", &json!({ "status": "failed", "result": { "error": "couldn't reach that machine" } }));
    assert_eq!((down.state, down.answer), ("unreachable", None));
}

#[test]
fn a_merged_list_says_which_machine_each_agent_is_on_and_lists_a_silent_peer_by_liveness_only() {
    let local = json!({
        "host": { "hostname": "here", "version": "0.60.37", "agents": [{ "name": "Loap", "model_label": "Sonnet 5.5" }], "page": { "total": 1, "returned": 1 } },
        "peers": [{ "hostname": "there" }],
    });
    let there = ok_reply("there", json!({ "agents": [{ "name": "Wren", "model_label": "Opus 5.5" }], "page": { "total": 1 } }));
    let (down, lan) = away();
    let merged = merge_list(Some(local), &[(there, Value::Null), (down, lan)], 2_500);
    let rows = merged["host"]["agents"].as_array().unwrap();
    let who: Vec<String> = rows.iter().map(|r| format!("{}@{}", r["name"].as_str().unwrap(), r["host"].as_str().unwrap())).collect();
    assert_eq!(who, ["Loap@here", "Wren@there", "Old@away"]);
    assert_eq!(rows[2]["facts"], "liveness_only");
    assert_eq!((rows[2]["state"].as_str(), rows[2]["provider"].as_str()), (Some("idle"), Some("claude")));
    assert!(rows[2].get("model_label").is_none(), "nothing is guessed for a machine that gave no facts");
    assert_eq!(merged["host"]["page"]["total"], 3);
    assert!(merged.get("peers").is_none());
    let hosts = merged["hosts"].as_array().unwrap();
    let states: Vec<&str> = hosts.iter().map(|h| h["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["ok", "ok", "unreachable"]);
    assert_eq!(hosts[0]["this_machine"], true);
    assert_eq!((hosts[1]["reachable"].clone(), hosts[1]["age_ms"].clone()), (json!(true), json!(1_500)));
    assert_eq!((hosts[2]["reachable"].clone(), hosts[2]["version"].as_str()), (json!(false), Some("0.60.30")));
    assert!(hosts[2]["error"].as_str().unwrap().contains("did not answer"));
}

#[test]
fn one_named_peer_is_asked_alone() {
    let there = ok_reply("there", json!({ "agents": [{ "name": "Wren" }], "page": { "total": 1 } }));
    let merged = merge_list(None, &[(there, Value::Null)], 1_000);
    assert_eq!(merged["host"]["agents"][0], json!({ "name": "Wren", "host": "there" }));
    assert_eq!(merged["hosts"].as_array().unwrap().len(), 1, "this machine is not part of an answer about another");
}

#[test]
fn a_line_has_its_machine_as_the_second_column() {
    assert_eq!(line_with_host("Loap | working | Sonnet 5.5", "here"), "Loap | here | working | Sonnet 5.5");
    assert_eq!(line_with_host("Loap", "here"), "Loap | here");
}

#[test]
fn a_merged_status_adds_up_the_counts_and_merges_the_groups() {
    let local = json!({
        "host": "here", "total": 2, "returned": 2,
        "lines": ["Loap | working (turn 4m) | Sonnet 5.5 high", "Wren | idle | Opus 5.5"],
        "counts": { "by_state": { "idle": 1, "working": 1 }, "by_model": { "Sonnet 5.5": 1, "Opus 5.5": 1 }, "by_host": { "here": 2 } },
        "group_by": "model",
        "groups": [{ "key": "Opus 5.5", "count": 1, "agents": ["Wren"] }, { "key": "Sonnet 5.5", "count": 1, "agents": ["Loap"] }],
    });
    let there = ok_reply("there", json!({
        "host": "there", "total": 1, "returned": 1, "lines": ["Pip | idle | Sonnet 5.5"],
        "counts": { "by_state": { "idle": 1 }, "by_model": { "Sonnet 5.5": 1 }, "by_host": { "there": 1 } },
        "group_by": "model", "groups": [{ "key": "Sonnet 5.5", "count": 1, "agents": ["Pip"] }],
    }));
    let (down, lan) = away();
    let merged = merge_status(Some(local), &[(there, Value::Null), (down, lan)], 2_000);
    let lines: Vec<&str> = merged["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
    assert_eq!(lines, ["Loap | here | working (turn 4m) | Sonnet 5.5 high", "Wren | here | idle | Opus 5.5", "Pip | there | idle | Sonnet 5.5", "Old | away | idle (liveness only)"]);
    assert_eq!((merged["total"].clone(), merged["returned"].clone()), (json!(4), json!(4)));
    assert_eq!(merged["counts"]["by_state"], json!({ "idle": 2, "working": 1 }));
    assert_eq!(merged["counts"]["by_model"], json!({ "Opus 5.5": 1, "Sonnet 5.5": 2 }));
    assert_eq!(merged["counts"]["by_host"], json!({ "away": 1, "here": 2, "there": 1 }));
    let groups = merged["groups"].as_array().unwrap();
    assert_eq!(groups[1], json!({ "key": "Sonnet 5.5", "count": 2, "agents": ["Loap@here", "Pip@there"] }));
    assert_eq!(merged["group_by"], "model");
    assert_eq!(merged["hosts"].as_array().unwrap().len(), 3);
}
