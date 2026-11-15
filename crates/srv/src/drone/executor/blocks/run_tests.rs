// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Value};

use super::*;
use crate::drone::types::NodePosition;

fn node(data: Value) -> FlowNode {
    FlowNode { id: "r1".to_string(), position: NodePosition::default(), data, node_type: None }
}

fn scope_with(block: &str, value: Value, bits: (bool, bool)) -> ExecutionScope {
    let mut s = ExecutionScope::new();
    s.outputs.insert(block.to_string(), value);
    s.bits.insert(block.to_string(), bits);
    s
}

fn done(code: i64, stdout: &str, stderr: &str) -> OneShotOutcome {
    OneShotOutcome { exit_code: code, stdout: stdout.into(), stderr: stderr.into(), truncated: false, duration_ms: 12 }
}

#[test]
fn a_command_is_a_program_and_its_arguments_run_as_given() {
    let n = node(json!({ "command": ["scripts/verify.sh", "--tag", "v1"], "env": { "MODE": "ci" }, "cwd": "/work" }));
    let (one, ok) = compose(&n, &ExecutionScope::new()).unwrap();
    assert_eq!(one.argv, ["scripts/verify.sh", "--tag", "v1"]);
    assert_eq!(one.env, [("MODE".to_string(), "ci".to_string())]);
    assert_eq!(one.cwd.as_deref(), Some("/work"));
    assert_eq!(ok, [0]);
    assert!(one.image.contains("agent-base"), "the base image by default: {}", one.image);
}

#[test]
fn a_command_that_is_not_a_list_of_text_is_refused() {
    for bad in [json!({}), json!({ "command": "ls -l" }), json!({ "command": [] }), json!({ "command": [""] }), json!({ "command": ["ls", 3] }), json!({ "command": [{"a": 1}] })] {
        assert!(parse(&node(bad.clone())).is_err(), "{bad}");
    }
    assert!(parse(&node(json!({ "command": "ls -l" }))).unwrap_err().contains("must be a list"));
}

#[test]
fn a_shell_string_is_not_split_or_interpreted() {
    // One item is one argument, whatever is in it.
    let n = node(json!({ "command": ["echo", "a; rm -rf / | cat $(x)"] }));
    let (one, _) = compose(&n, &ExecutionScope::new()).unwrap();
    assert_eq!(one.argv, ["echo", "a; rm -rf / | cat $(x)"]);
}

#[test]
fn a_clean_value_is_filled_in() {
    let scope = scope_with("a", json!({ "tag": "v1.2.3" }), (false, false));
    let (one, _) = compose(&node(json!({ "command": ["release", "{{a.tag}}"], "env": { "TAG": "{{a.tag}}" } })), &scope).unwrap();
    assert_eq!(one.argv, ["release", "v1.2.3"]);
    assert_eq!(one.env[0].1, "v1.2.3");
}

#[test]
fn text_from_outside_never_reaches_the_command_line_the_environment_or_the_directory() {
    let scope = scope_with("api", json!({ "name": "x; rm -rf /" }), (true, false));
    for data in [
        json!({ "command": ["echo", "{{api.name}}"] }),
        json!({ "command": ["{{api.name}}"] }),
        json!({ "command": ["echo", "a {{api.name}} b"] }),
        json!({ "command": ["echo"], "env": { "X": "{{api.name}}" } }),
        json!({ "command": ["echo"], "cwd": "/w/{{api.name}}" }),
    ] {
        let err = compose(&node(data.clone()), &scope).unwrap_err();
        assert!(err.contains("never built from outside text"), "{data}: {err}");
    }
}

#[test]
fn a_value_worked_out_from_outside_text_is_refused_too() {
    let scope = scope_with("n", json!({ "count": 3 }), (false, true));
    let err = compose(&node(json!({ "command": ["head", "-n", "{{n.count}}", "f"] })), &scope).unwrap_err();
    assert!(err.contains("worked out from text from outside"), "{err}");
}

#[test]
fn what_started_the_run_is_outside_text() {
    let scope = ExecutionScope::with_trigger(json!({ "ref": "main" }));
    assert!(compose(&node(json!({ "command": ["git", "checkout", "{{trigger.ref}}"] })), &scope).is_err());
}

#[test]
fn a_host_command_and_the_things_not_given_yet_are_refused_with_a_reason() {
    let host = parse(&node(json!({ "command": ["ls"], "where": "host" }))).unwrap_err();
    assert!(host.contains("named allowance") && host.contains("`where: container`"), "{host}");
    assert!(parse(&node(json!({ "command": ["ls"], "where": "elsewhere" }))).is_err());
    assert!(parse(&node(json!({ "command": ["ls"], "where": "container" }))).is_ok());
    for key in ["mounts", "hosts", "network", "vault", "secrets", "stdin"] {
        let err = parse(&node(json!({ "command": ["ls"], key: [] }))).unwrap_err();
        assert!(err.contains(key) && err.contains("not supported yet"), "{key}: {err}");
    }
}

#[test]
fn the_image_is_written_in_the_graph() {
    assert!(parse(&node(json!({ "command": ["ls"], "image": "{{a.img}}" }))).is_err());
    assert_eq!(parse(&node(json!({ "command": ["ls"], "image": "alpine:3" }))).unwrap().image.as_deref(), Some("alpine:3"));
}

#[test]
fn an_environment_name_must_be_a_name_and_a_value_text() {
    assert!(parse(&node(json!({ "command": ["ls"], "env": { "A B": "x" } }))).is_err());
    assert!(parse(&node(json!({ "command": ["ls"], "env": { "1A": "x" } }))).is_err());
    assert!(parse(&node(json!({ "command": ["ls"], "env": { "A": 3 } }))).is_err());
    assert!(parse(&node(json!({ "command": ["ls"], "env": ["A"] }))).is_err());
}

#[test]
fn the_time_limit_has_a_ceiling_of_an_hour() {
    assert!(parse(&node(json!({ "command": ["ls"], "timeout_seconds": 3600 }))).is_ok());
    assert!(parse(&node(json!({ "command": ["ls"], "timeout_seconds": 3601 }))).unwrap_err().contains("at most 3600"));
}

#[test]
fn an_exit_code_is_a_failure_unless_the_node_says_it_is_ok() {
    assert!(outcome_of(&done(0, "", ""), &[0]).is_ok());
    let err = outcome_of(&done(2, "", "boom\n  at line 3\n"), &[0]).unwrap_err();
    assert_eq!(err, "run block: the command exited with code 2: boom at line 3");
    assert!(outcome_of(&done(1, "no diff", ""), &[0, 1]).is_ok(), "grep-style codes");
    assert!(outcome_of(&done(0, "", ""), &[1]).is_err(), "only the codes named are ok");
    assert!(parse(&node(json!({ "command": ["ls"], "ok_exit_codes": [] }))).is_err());
    assert!(parse(&node(json!({ "command": ["ls"], "ok_exit_codes": ["0"] }))).is_err());
}

#[test]
fn the_output_carries_the_code_the_time_and_the_streams() {
    let out = outcome_of(&done(0, "hi\n", "warn\n"), &[0]).unwrap();
    assert_eq!(out, json!({ "exit_code": 0, "stdout": "hi\n", "stderr": "warn\n", "duration_ms": 12, "truncated": false }));
}

#[test]
fn a_long_error_is_cut_to_its_end() {
    let long = format!("{} the end", "x".repeat(1000));
    let err = outcome_of(&done(1, "", &long), &[0]).unwrap_err();
    assert!(err.ends_with("the end") && err.contains('…'), "{err}");
    assert!(err.chars().count() < 400, "{}", err.chars().count());
}
