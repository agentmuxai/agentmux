// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A template package that carries a Drone graph
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md, "What is a template?").

use serde_json::{json, Value};

use super::tests::manifest_json;
use super::*;

fn manifest(extra: Value) -> Value {
    let mut m = manifest_json("acme.echo", json!({ "kind": "drone", "action": null, "outputs": [] }));
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        if v.is_null() {
            m.as_object_mut().unwrap().remove(&k);
        } else {
            m[k] = v;
        }
    }
    m
}

fn node(id: &str, data: Value) -> Value {
    json!({ "id": id, "position": { "x": 0, "y": 0 }, "data": data })
}

fn graph() -> Value {
    json!({
        "nodes": [
            node("ask", json!({ "kind": "agent", "task": "Summarise the week." })),
            node("tell", json!({ "kind": "message", "to": "owner", "body": "{{ask.response}}" })),
        ],
        "edges": [{ "id": "e1", "source": "ask", "target": "tell" }]
    })
}

fn parse(m: &Value, graph: Option<&Value>, brief: Option<&str>) -> Result<package::Package, String> {
    let mut files = std::collections::BTreeMap::new();
    files.insert("template.json".to_string(), serde_json::to_vec(m).unwrap());
    if let Some(g) = graph {
        files.insert("graph.json".to_string(), serde_json::to_vec(g).unwrap());
    }
    if let Some(b) = brief {
        files.insert("brief.md".to_string(), b.as_bytes().to_vec());
    }
    package::parse("acme.echo", &files)
}

#[test]
fn a_drone_template_carries_a_graph_and_the_graph_is_in_its_hash() {
    let pkg = parse(&manifest(json!({})), Some(&graph()), None).unwrap();
    assert_eq!(pkg.manifest.kind, manifest::JobTemplateKind::Drone);
    assert_eq!(pkg.graph.as_ref().map(|g| g.nodes.len()), Some(2));
    let mut other = graph();
    other["nodes"][0]["data"]["task"] = json!("Summarise the month.");
    assert_ne!(pkg.hash, parse(&manifest(json!({})), Some(&other), None).unwrap().hash, "a changed graph is a changed template");
}

#[test]
fn the_graph_goes_with_the_drone_kind_and_with_nothing_else() {
    let refused = |m: Value, graph: Option<&Value>, brief: Option<&str>, why: &str| {
        let err = parse(&m, graph, brief).unwrap_err();
        assert!(err.contains(why), "{err}");
    };
    refused(manifest(json!({})), None, None, "graph.json");
    refused(manifest(json!({})), Some(&graph()), Some("a brief"), "no brief.md");
    refused(manifest(json!({ "action": { "kind": "message" } })), Some(&graph()), None, "no action");
    refused(manifest_json("acme.echo", json!({})), Some(&graph()), Some("a brief"), "only a Drone template");
    refused(manifest(json!({ "owner": "loap" })), Some(&graph()), None, "belong to a cloud template");
}

#[test]
fn a_graph_this_build_cannot_hold_is_refused_and_says_where() {
    let refused = |g: Value, why: &str| {
        let err = parse(&manifest(json!({})), Some(&g), None).unwrap_err();
        assert!(err.contains(why), "{err}");
    };
    refused(json!({ "nodes": [], "edges": [] }), "no nodes");
    refused(json!({ "nodes": [node("a", json!({ "kind": "agent" })), node("a", json!({ "kind": "agent" }))], "edges": [] }), "twice");
    // A node kind a later AgentMux has: named, so the author knows which one.
    refused(json!({ "nodes": [node("loop1", json!({ "kind": "for_each" }))], "edges": [] }), "\"loop1\" is a \"for_each\" node");
    refused(json!({ "nodes": [node("a", json!({ "task": "x" }))], "edges": [] }), "\"a\" is a \"\" node");
    let mut dangling = graph();
    dangling["edges"] = json!([{ "id": "e9", "source": "ask", "target": "nowhere" }]);
    refused(dangling, "\"nowhere\"");
    let many: Vec<Value> = (0..=graph::MAX_NODES).map(|i| node(&format!("n{i}"), json!({ "kind": "response" }))).collect();
    refused(json!({ "nodes": many, "edges": [] }), "at most");
    assert!(parse(&manifest(json!({})), Some(&json!("not a graph")), None).unwrap_err().contains("graph.json"));
}

#[test]
fn the_prompt_lists_what_the_graph_does_outside_agentmux() {
    let g = json!({
        "nodes": [
            node("ask", json!({ "kind": "agent", "task": "x" })),
            node("read", json!({ "kind": "api", "url": "https://example.com/a" })),
            node("post", json!({ "kind": "api", "method": "post", "url": "https://example.com/hook" })),
            node("note", json!({ "kind": "task", "title": "t" })),
        ],
        "edges": []
    });
    let summary = graph::summarize(&parse(&manifest(json!({})), Some(&g), None).unwrap().graph.unwrap());
    assert_eq!((summary.nodes, summary.agents), (4, 1));
    assert_eq!(summary.writes_outside, vec!["post: POST https://example.com/hook".to_string()], "a plain read and an inside write are not listed");
}

#[test]
fn a_drone_template_installs_lists_and_waits_for_approval_like_a_job_one() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("src").join("acme.echo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("template.json"), serde_json::to_vec(&manifest(json!({}))).unwrap()).unwrap();
    std::fs::write(dir.join("graph.json"), serde_json::to_vec(&graph()).unwrap()).unwrap();
    let svc = JobTemplates::new(tmp.path().join("job-templates"), &tmp.path().join("data"), &[]);
    assert_eq!(svc.install(&dir, false).unwrap(), "acme.echo");
    let info = svc.list().into_iter().find(|t| t.id == "acme.echo").unwrap();
    assert_eq!((info.state, info.error), (JobTemplateState::NeedsApproval, None));
    assert_eq!(info.drone.map(|d| d.nodes), Some(2), "the prompt carries the graph's summary");
    svc.approve("acme.echo", &info.hash, "").unwrap();
}

fn param_graph() -> Value {
    json!({
        "nodes": [node("tell", json!({ "kind": "message", "to": "{{param.who}}", "body": "Look at {{param.repo}} and {{ask.response}}" }))],
        "edges": []
    })
}

fn param_manifest() -> Value {
    manifest(json!({
        "params": { "type": "object", "properties": { "repo": { "type": "string" }, "who": { "type": "string", "default": "loap" } }, "required": ["repo"] }
    }))
}

#[test]
fn a_graph_may_use_only_the_parameters_the_template_declares() {
    assert!(parse(&param_manifest(), Some(&param_graph()), None).is_ok());
    let err = parse(&manifest(json!({})), Some(&param_graph()), None).unwrap_err();
    assert!(err.contains("{{param.who}}") && err.contains("isn't one of its parameters"), "{err}");
    let typo = json!({ "nodes": [node("a", json!({ "kind": "message", "to": "x", "body": "{{param.repo" }))], "edges": [] });
    assert!(parse(&param_manifest(), Some(&typo), None).unwrap_err().contains("node \"a\""));
}

#[test]
fn rendering_fills_the_parameters_once_and_leaves_the_drones_own_references() {
    let pkg = parse(&param_manifest(), Some(&param_graph()), None).unwrap();
    let given = json!({ "repo": "acme/site" }).as_object().cloned().unwrap();
    let g = graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &given).unwrap();
    assert_eq!(g.nodes[0].data["to"], "loap", "a default fills in");
    assert_eq!(g.nodes[0].data["body"], "Look at acme/site and {{ask.response}}");
    // A value that opens a reference would be read by the drone at run time.
    let hostile = json!({ "repo": "{{trigger.job.name}}" }).as_object().cloned().unwrap();
    assert!(graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &hostile).unwrap_err().contains("can't contain"));
    assert!(graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &Default::default()).unwrap_err().contains("required"));
}

#[test]
fn a_drone_template_declares_no_secrets_yet() {
    let m = manifest(json!({
        "params": { "type": "object", "properties": { "t": { "type": "string", "format": "credential" } } },
        "secrets": [{ "name": "TOKEN", "param": "t" }]
    }));
    assert!(parse(&m, Some(&graph()), None).unwrap_err().contains("secrets"));
}

#[test]
fn a_job_from_a_drone_template_names_the_drone_it_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("src").join("acme.echo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("template.json"), serde_json::to_vec(&param_manifest()).unwrap()).unwrap();
    std::fs::write(dir.join("graph.json"), serde_json::to_vec(&param_graph()).unwrap()).unwrap();
    let svc = JobTemplates::new(tmp.path().join("job-templates"), &tmp.path().join("data"), &[]);
    svc.install(&dir, false).unwrap();
    let hash = svc.get("acme.echo").unwrap().0.hash;
    svc.approve("acme.echo", &hash, "").unwrap();

    let make = |action: Value| json!({ "jobVersion": 1, "template": { "id": "acme.echo" }, "params": { "repo": "a/b" }, "action": action, "owner": "uid-a" });
    let apply = |mut def: Value| crate::backend::jobs::from_template::apply_with(&mut def, None, &svc).map(|()| def);
    let err = apply(make(json!({ "kind": "message", "to": "x" }))).unwrap_err();
    assert!(err.contains("make its drone first"), "{err}");
    let def = apply(make(json!({ "kind": "drone", "drone_id": "d-1" }))).unwrap();
    assert_eq!(def["action"], json!({ "kind": "drone", "drone_id": "d-1" }), "no brief is written into a drone action");
    assert_eq!(def["trigger"]["kind"], "cron", "the template's trigger default is filled in");
    assert_eq!(def["template"]["hash"], hash.as_str());
}
