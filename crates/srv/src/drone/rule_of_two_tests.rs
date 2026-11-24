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
fn a_read_that_sends_a_credential_may_not_follow_an_address_from_outside() {
    let read = |extra: Value| {
        let mut data = json!({ "kind": "api", "method": "GET", "url": "{{trigger.url}}" });
        data.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        graph(&[("get", data)], &[])
    };
    assert!(check(&read(json!({})), &webhook()).is_empty(), "a plain read may follow an address it was given");
    assert_eq!(broken(&read(json!({ "headers": { "Authorization": "Bearer abc" } })), &webhook()), ["get"]);
    assert_eq!(broken(&read(json!({ "headers": { "X-Thing": "{{env.AGENTMUX_DR_TOKEN}}" } })), &webhook()), ["get"]);
    assert_eq!(broken(&read(json!({ "headers": { "Accept": "text/plain", "Cookie": "a=b" } })), &webhook()), ["get"]);
    assert!(check(&read(json!({ "headers": { "Accept": "text/plain" } })), &webhook()).is_empty(), "a header that carries no secret is not one");
}

#[test]
fn the_address_ends_at_the_first_slash_outside_a_reference() {
    assert_eq!(address_of("https://api.example.com/pulls/1"), "https://api.example.com");
    assert_eq!(address_of("https://{{trigger.host}}/x?y=1"), "https://{{trigger.host}}");
    assert_eq!(address_of("https://{{trigger.host | default:a/b}}/x"), "https://{{trigger.host | default:a/b}}");
    assert_eq!(address_of("https://{{trigger.host | default:a/b"), "https://{{trigger.host | default:a/b");
    let post = graph(&[("post", json!({ "kind": "api", "method": "POST", "url": "https://{{trigger.host | default:a/b}}/x" }))], &[]);
    let found = check(&post, &webhook());
    assert!(found.iter().any(|f| f.why.contains("the address a request goes to")), "{found:?}");
}

#[test]
fn the_scheme_is_found_outside_a_reference() {
    assert_eq!(address_of("{{trigger.url | default:https://api.example.com/path}}"), "{{trigger.url | default:https://api.example.com/path}}");
    let get = graph(
        &[("get", json!({ "kind": "api", "method": "GET", "url": "{{trigger.url | default:https://api.example.com/path}}", "headers": { "Authorization": "Bearer abc" } }))],
        &[],
    );
    assert_eq!(broken(&get, &webhook()), ["get"]);
}

/// A graph with one Drone block that calls `callee` with `inputs`.
fn caller(inputs: Value) -> DroneGraph {
    graph(&[("call", json!({ "kind": "drone", "drone": "sub", "inputs": inputs }))], &[])
}

#[test]
fn a_graph_moved_into_a_drone_it_calls_is_still_refused() {
    let fetch = json!({ "kind": "api", "method": "GET", "url": "https://ci.example.com/log" });
    let post = json!({ "kind": "api", "method": "POST", "url": "https://api.example.com/comments", "body": "{{fetch.body}}" });
    let sub = graph(&[("fetch", fetch), ("post", post)], &[("fetch", "post")]);
    let load = |id: &str, _v: Option<i64>| (id == "sub").then(|| ("Comment".to_string(), sub.clone()));
    let calling = caller(json!({}));
    assert!(check(&calling, &webhook()).is_empty(), "on its own the calling graph has nothing to refuse");
    let found = check_tree(&calling, &webhook(), &load);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].why.contains("post:") && found[0].why.contains("Comment"), "{found:?}");
    assert!(refuse(&calling, &webhook(), &load).is_err());
    assert!(calls(&calling, "sub", &load) && !calls(&calling, "other", &load));
    assert!(refuse(&calling, &webhook(), &|_, _| None).is_ok(), "a callee that is not there is for the runner to say");
}

#[test]
fn what_a_caller_passes_in_is_clean_in_the_drone_it_calls() {
    let rerun = json!({ "kind": "api", "method": "POST", "url": "https://ci.example.com/runs/{{trigger.run}}/rerun" });
    let sub = graph(&[("rerun", rerun)], &[]);
    let load = |id: &str, _v: Option<i64>| (id == "sub").then(|| ("Rerun".to_string(), sub.clone()));
    assert!(check_tree(&caller(json!({ "run": "42" })), &webhook(), &load).is_empty());
    let found = check_tree(&caller(json!({})), &webhook(), &load);
    assert_eq!(found.len(), 1, "a name the caller did not pass is not clean: {found:?}");
}

#[test]
fn an_error_edge_out_of_an_approval_is_not_an_approval() {
    // A no and a silence fail the Approval, and only its `error` edge takes them: the write on it runs when nobody approved.
    let mut g = graph(&[("ok", approval()), ("post", comment("{{trigger.title}}"))], &[("ok", "post")]);
    g.edges[0].source_handle = Some("error".to_string());
    assert_eq!(broken(&g, &webhook()), ["post"]);
    g.edges[0].source_handle = None;
    assert!(check(&g, &webhook()).is_empty(), "the edge a yes takes does clear it");
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
    let keyed = |url: &str| graph(&[("g", json!({ "kind": "api", "method": "GET", "url": url, "auth": { "vault": "token", "as": "bearer" } }))], &[]);
    assert!(check(&get("https://api.example.com/items/{{trigger.id}}?q={{trigger.q}}"), &webhook()).is_empty());
    assert!(check(&get("{{trigger.url}}"), &webhook()).is_empty(), "a plain read with no credential may follow an address it was given");
    assert_eq!(broken(&keyed("https://{{trigger.host}}/x"), &webhook()), ["g"], "a credential goes only where the graph says");
    assert_eq!(broken(&keyed("{{trigger.url}}"), &webhook()), ["g"]);
    let post = graph(&[("g", json!({ "kind": "api", "method": "POST", "url": "https://{{trigger.host}}/x" }))], &[]);
    assert!(broken(&post, &webhook()).contains(&"g".to_string()), "a write goes only where the graph says");
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
    assert_eq!(broken(&agent(json!({ "allow_commands": ["./release-check {{trigger.tag}}"] })), &webhook()), ["a"], "a list of command lines lets it run them");
    assert_eq!(broken(&agent(json!({ "allow_commands": "./release-check" })), &webhook()), ["a"]);
    assert!(check(&agent(json!({ "allow_commands": [] })), &webhook()).is_empty(), "an empty list lets it run nothing");
    assert!(check(&agent(json!({ "allow_commands": false })), &webhook()).is_empty());
}

#[test]
fn a_graph_of_inside_nodes_and_a_timer_passes_and_the_refusal_says_every_node() {
    let quiet = graph(&[("t", json!({ "kind": "task", "title": "Look at the board", "target": "opaz" }))], &[]);
    assert!(refuse(&quiet, &fields_of(None, &Map::new()), &|_, _| None).is_ok());
    let two = graph(&[("a", comment("{{trigger.x}}")), ("b", comment("{{trigger.y}}"))], &[]);
    let said = refuse(&two, &webhook(), &|_, _| None).unwrap_err();
    assert!(said.starts_with("this drone cannot be turned on") && said.contains("a:") && said.contains("b:"), "{said}");
}

/// Every drone AgentMux ships, filled in with example values, is turned on without a refusal: the rule that guards a
/// person's drone does not refuse the templates (the merge of `verify-release` has an Approval on its only path).
#[test]
fn every_shipped_drone_template_passes_the_check_it_is_turned_on_under() {
    use crate::backend::job_templates::{graph as tgraph, manifest, package, DRONE_BUILTINS};
    let example = |id: &str| -> Map<String, Value> {
        let v = match id {
            "agentmux.scheduled-report" => json!({ "question": "What changed?", "owner": "loap" }),
            "agentmux.check-and-file-task" => json!({ "check": "Is the build green?" }),
            "agentmux.two-agent-handoff" => json!({ "first": "a", "second": "b", "owner": "loap" }),
            "agentmux.after-upgrade-check" => json!({ "leader": "agenty", "steward": "masty" }),
            "agentmux.triage-failed-nightly" => json!({ "repo": "acme/site", "leader": "agenty", "triager": "masty" }),
            "agentmux.rerun-flaky-job" => json!({ "repo": "acme/site", "workflow": "CI", "leader": "agenty" }),
            "agentmux.merged-pr-still-claimed" => json!({ "repo": "acme/site", "steward": "masty" }),
            "agentmux.verify-release" => json!({ "repo": "acme/site", "held_pr": 42, "leader": "agenty" }),
            other => panic!("no example for {other}"),
        };
        v.as_object().cloned().unwrap()
    };
    for b in DRONE_BUILTINS {
        let pkg = package::load_embedded(b.id, b.files).unwrap();
        let given = example(b.id);
        let g = tgraph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &given).unwrap();
        let params = manifest::resolve_params(&pkg.manifest, &given).unwrap();
        let mut trigger = pkg.manifest.trigger.clone().unwrap_or(Value::Null);
        tgraph::render_value(&mut trigger, &params).unwrap();
        let source = trigger.get("source").and_then(Value::as_str);
        let filter = trigger.get("filter").and_then(Value::as_object).cloned().unwrap_or_default();
        let found = check(&g, &fields_of(source, &filter));
        assert!(found.is_empty(), "{}: {found:?}", b.id);
    }
}
