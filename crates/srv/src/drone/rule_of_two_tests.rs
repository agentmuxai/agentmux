// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Map, Value};

use super::*;
use crate::backend::jobs::definition::event_source;
use crate::drone::payload_schema::fields_of;

/// A graph of `nodes` (each `(id, data)`) joined by `edges` (each `(from, to)`).
fn graph(nodes: &[(&str, Value)], edges: &[(&str, &str)]) -> DroneGraph {
    let nodes: Vec<Value> = nodes.iter().map(|(id, data)| json!({ "id": id, "position": { "x": 0, "y": 0 }, "data": data })).collect();
    let edges: Vec<Value> = edges.iter().map(|(a, b)| json!({ "id": format!("{a}-{b}"), "source": a, "target": b })).collect();
    serde_json::from_value(json!({ "nodes": nodes, "edges": edges })).unwrap()
}

/// What a webhook drone reads: every field of the body is outside text.
fn webhook() -> Vec<PayloadField> {
    fields_of(Some(event_source::WEBHOOK_LOCAL), &Map::new())
}

fn comment(body: &str) -> Value {
    json!({ "kind": "api", "method": "POST", "url": "https://api.example.com/pulls/1/comments", "body": body })
}

fn approval() -> Value {
    json!({ "kind": "approval", "title": "Post the comment" })
}

fn broken(g: &DroneGraph, fields: &[PayloadField]) -> Vec<String> {
    check(g, fields).into_iter().map(|f| f.node).collect()
}

#[test]
fn a_write_that_reads_the_trigger_needs_an_approval_on_the_way() {
    let bare = graph(&[("post", comment("{{trigger.title}}"))], &[]);
    assert_eq!(broken(&bare, &webhook()), ["post"]);
    let why = &check(&bare, &webhook())[0].why;
    assert!(why.contains("{{trigger.title}}") && why.contains("Approval"), "{why}");

    let approved = graph(&[("ok", approval()), ("post", comment("{{trigger.title}}"))], &[("ok", "post")]);
    assert!(check(&approved, &webhook()).is_empty(), "an Approval before the write clears it");
}

#[test]
fn a_write_from_a_log_through_an_agent_is_refused_unless_a_person_approves() {
    let nodes = [
        ("fetch", json!({ "kind": "api", "method": "GET", "url": "https://ci.example.com/log" })),
        ("agent", json!({ "kind": "agent", "task": "Say what failed in {{fetch.body}}" })),
        ("post", comment("{{agent.response}}")),
    ];
    let open = graph(&nodes, &[("fetch", "agent"), ("agent", "post")]);
    assert_eq!(broken(&open, &[]), ["post"], "the log and the agent's reply are outside text; reading them is not a write");

    let mut guarded = nodes.to_vec();
    guarded.push(("ok", approval()));
    let g = graph(&guarded, &[("fetch", "agent"), ("agent", "ok"), ("ok", "post")]);
    assert!(check(&g, &[]).is_empty());
}

#[test]
fn an_approval_on_only_one_of_two_paths_does_not_clear_the_other() {
    let g = graph(
        &[("fetch", json!({ "kind": "api", "url": "https://a.example.com" })), ("ok", approval()), ("post", comment("{{fetch.body}}"))],
        &[("fetch", "ok"), ("ok", "post"), ("fetch", "post")],
    );
    assert_eq!(broken(&g, &[]), ["post"]);
}

#[test]
fn a_write_whose_inputs_are_all_typed_clean_needs_no_approval() {
    // Acceptance (b): run a CI job again by its id. The id is a field the trigger declares clean.
    let rerun = json!({ "kind": "api", "method": "POST", "url": "https://ci.example.com/jobs/{{trigger.run_id}}/rerun" });
    let g = graph(&[("rerun", rerun)], &[]);
    let timer = fields_of(None, &Map::new());
    assert!(check(&g, &timer).is_empty(), "run_id is made by AgentMux");
    let text = graph(&[("rerun", json!({ "kind": "api", "method": "POST", "url": "https://ci.example.com/x", "body": "{{trigger.note}}" }))], &[]);
    assert_eq!(broken(&text, &timer), ["rerun"], "a field the sender chose is outside text");
}

#[test]
fn outside_text_may_go_into_a_message_as_data_but_never_choose_where_it_goes() {
    let msg = |to: &str, body: &str| graph(&[("m", json!({ "kind": "message", "to": to, "body": body }))], &[]);
    assert!(check(&msg("opaz", "A pull request said {{trigger.title}}"), &webhook()).is_empty());
    assert_eq!(broken(&msg("{{trigger.agent}}", "hi"), &webhook()), ["m"]);

    let task = graph(&[("t", json!({ "kind": "task", "title": "x", "target": "{{trigger.owner}}" }))], &[]);
    assert_eq!(broken(&task, &webhook()), ["t"]);
}

#[test]
fn outside_text_cannot_be_the_host_or_a_header_name_but_can_be_in_the_path() {
    let get = |url: &str| graph(&[("g", json!({ "kind": "api", "method": "GET", "url": url }))], &[]);
    assert!(check(&get("https://api.example.com/items/{{trigger.id}}?q={{trigger.q}}"), &webhook()).is_empty());
    assert_eq!(broken(&get("https://{{trigger.host}}/x"), &webhook()), ["g"]);
    assert_eq!(broken(&get("{{trigger.url}}"), &webhook()), ["g"]);
    let header = graph(&[("g", json!({ "kind": "api", "url": "https://a.example.com", "headers": { "{{trigger.name}}": "x" } }))], &[]);
    assert_eq!(broken(&header, &webhook()), ["g"]);
    let cleared = graph(&[("ok", approval()), ("g", json!({ "kind": "api", "url": "{{trigger.url}}" }))], &[("ok", "g")]);
    assert!(check(&cleared, &webhook()).is_empty(), "a person read the card");
}

#[test]
fn a_clean_value_or_a_condition_result_is_not_outside_text() {
    let g = graph(
        &[
            ("c", json!({ "kind": "condition", "expr": "{{trigger.title}} == 'x'" })),
            ("post", comment("{{c.result}} and {{env.NAME}} and just text")),
        ],
        &[("c", "post")],
    );
    assert!(check(&g, &webhook()).is_empty());
}

#[test]
fn outside_text_carries_through_a_response_and_a_for_each() {
    let g = graph(
        &[
            ("r", json!({ "kind": "response", "template": "{{trigger.title}}" })),
            ("post", comment("{{r.text}}")),
        ],
        &[("r", "post")],
    );
    assert_eq!(broken(&g, &webhook()), ["post"]);

    let each = graph(
        &[
            ("fetch", json!({ "kind": "api", "url": "https://a.example.com" })),
            ("loop", json!({ "kind": "foreach", "items": "{{fetch.body.items}}", "body": "post" })),
            ("post", comment("{{item}}")),
        ],
        &[("fetch", "loop")],
    );
    assert_eq!(broken(&each, &[]), ["post"], "the body reads items that came from an API");
}

#[test]
fn a_command_and_a_callee_count_as_writes_and_a_read_only_agent_does_not() {
    let run = graph(&[("sh", json!({ "kind": "run", "command": "echo {{trigger.title}}" }))], &[]);
    assert_eq!(broken(&run, &webhook()), ["sh"]);
    let call = graph(&[("call", json!({ "kind": "drone", "drone": "other", "inputs": { "text": "{{trigger.body}}" } }))], &[]);
    assert_eq!(broken(&call, &webhook()), ["call"]);
    let agent = |extra: Value| {
        let mut data = json!({ "kind": "agent", "task": "Look at {{trigger.title}}" });
        data.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        graph(&[("a", data)], &[])
    };
    assert!(check(&agent(json!({})), &webhook()).is_empty(), "a native agent with read-only tools reads outside; it does not write");
    assert_eq!(broken(&agent(json!({ "allow_commands": true })), &webhook()), ["a"]);
}

#[test]
fn a_graph_of_inside_nodes_and_a_timer_passes_and_the_refusal_says_every_node() {
    let quiet = graph(&[("t", json!({ "kind": "task", "title": "Look at the board", "target": "opaz" }))], &[]);
    assert!(refuse(&quiet, &fields_of(None, &Map::new())).is_ok());
    let two = graph(&[("a", comment("{{trigger.x}}")), ("b", comment("{{trigger.y}}"))], &[]);
    let said = refuse(&two, &webhook()).unwrap_err();
    assert!(said.starts_with("this drone cannot be turned on") && said.contains("a:") && said.contains("b:"), "{said}");
}
