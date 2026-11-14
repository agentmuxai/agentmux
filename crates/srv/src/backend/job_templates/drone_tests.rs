// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A template package that carries a Drone graph
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md, "What is a template?").

use serde_json::{json, Map, Value};

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
            node("worker", json!({ "kind": "spawn", "leader": "agenty", "brief": "x" })),
        ],
        "edges": []
    });
    let summary = graph::summarize(&parse(&manifest(json!({})), Some(&g), None).unwrap().graph.unwrap());
    assert_eq!((summary.nodes, summary.agents), (5, 2), "a Spawn node starts an agent, so the prompt counts it");
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
fn values_that_form_a_reference_with_the_text_next_to_them_are_refused() {
    let m = manifest(json!({
        "params": { "type": "object", "properties": { "a": { "type": "string" }, "b": { "type": "string" } }, "required": ["a", "b"] }
    }));
    let render = |body: &str, a: &str, b: &str| {
        let g = json!({ "nodes": [node("n", json!({ "kind": "message", "to": "x", "body": body }))], "edges": [] });
        let pkg = parse(&m, Some(&g), None).unwrap();
        let given = json!({ "a": a, "b": b }).as_object().cloned().unwrap();
        graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &given)
    };
    // Each value is clean alone; joined, they read as {{trigger.job.name}}.
    let err = render("{{param.a}}{{param.b}}", "{", "{trigger.job.name}}").unwrap_err();
    assert!(err.contains("forms {{"), "{err}");
    // A value that ends or starts with a brace next to a brace of the template.
    assert!(render("{{{param.a}}", "{trigger.job.name}}", "b").is_err(), "template brace + value brace");
    assert!(render("{{param.a}}{{param.b}}", "x{", "{y").is_err(), "value, value");
    // Braces that stay single, and the template's own references, are fine.
    let ok = render("{{param.a}} {x} {{ask.response}} {{param.b}}", "{a}", "{b}").unwrap();
    assert_eq!(ok.nodes[0].data["body"], "{a} {x} {{ask.response}} {b}");
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

// ── The built-in set ────────────────────────────────────────────────────────

/// Example values for each built-in's parameters.
fn example(id: &str) -> Map<String, Value> {
    let v = match id {
        "agentmux.scheduled-report" => json!({ "question": "What changed this week?", "owner": "loap" }),
        "agentmux.check-and-file-task" => json!({ "check": "Is the build green?" }),
        "agentmux.two-agent-handoff" => json!({ "first": "Name three colours.", "second": "Sort them.", "owner": "loap" }),
        other => panic!("no example for {other}"),
    };
    v.as_object().cloned().unwrap()
}

#[test]
fn the_built_in_drone_templates_are_valid_and_render_with_example_parameters() {
    let ids: Vec<&str> = DRONE_BUILTINS.iter().map(|b| b.id).collect();
    assert_eq!(ids, ["agentmux.scheduled-report", "agentmux.check-and-file-task", "agentmux.two-agent-handoff"]);
    for b in DRONE_BUILTINS {
        let pkg = package::load_embedded(b.id, b.files).unwrap_or_else(|e| panic!("{}: {e}", b.id));
        assert_eq!(pkg.manifest.kind, manifest::JobTemplateKind::Drone, "{}", b.id);
        assert!(b.files.iter().any(|(name, _)| *name == "README.md"), "{} says what to change first", b.id);
        let rendered = graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &example(b.id)).unwrap();
        graph::check(&rendered).unwrap();
        // No placeholder is left for the run to trip on.
        assert!(!serde_json::to_string(&rendered).unwrap().contains("{{param."), "{}", b.id);
        // A template runs on today's blocks and each node carries what its block reads.
        for node in &rendered.nodes {
            let needs: &[&str] = match node.data["kind"].as_str().unwrap() {
                "agent" => &["task"],
                "message" => &["to", "body"],
                "condition" => &["expr"],
                "task" => &["title"],
                other => panic!("{}: node {} is a {other} node", b.id, node.id),
            };
            for key in needs {
                assert!(node.data.get(*key).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()), "{}: node {} needs {key}", b.id, node.id);
            }
        }
        // Nothing is secret-looking or private, and nothing writes outside AgentMux.
        assert!(graph::summarize(pkg.graph.as_ref().unwrap()).writes_outside.is_empty(), "{}", b.id);
        assert!(pkg.manifest.secrets.is_empty());
    }
}

#[test]
fn only_a_problem_files_a_task_in_the_check_template() {
    let b = DRONE_BUILTINS.iter().find(|b| b.id == "agentmux.check-and-file-task").unwrap();
    let g = package::load_embedded(b.id, b.files).unwrap().graph.unwrap();
    let edge = g.edges.iter().find(|e| e.target == "file").unwrap();
    assert_eq!((edge.source.as_str(), edge.source_handle.as_deref()), ("problem", Some("true")), "the task is filed on the true branch only");
}

#[test]
fn two_checks_from_the_check_template_do_not_share_a_task() {
    let b = DRONE_BUILTINS.iter().find(|b| b.id == "agentmux.check-and-file-task").unwrap();
    let pkg = package::load_embedded(b.id, b.files).unwrap();
    let key = |check: &str| {
        let given = json!({ "check": check }).as_object().cloned().unwrap();
        let g = graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &given).unwrap();
        g.nodes.iter().find(|n| n.id == "file").unwrap().data["dedupe"].as_str().unwrap().to_string()
    };
    assert_ne!(key("Is the build green?"), key("Is the site up?"), "the check is part of the dedupe key");
    assert_eq!(key("Is the build green?"), key("Is the build green?"), "the same check repeats into one task");
}

#[test]
fn the_service_lists_the_built_in_drone_templates_for_approval_beside_the_job_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = JobTemplates::new(tmp.path().join("job-templates"), &tmp.path().join("data"), all_builtins());
    let listed = svc.list();
    assert_eq!(listed.len(), BUILTINS.len() + DRONE_BUILTINS.len());
    for b in DRONE_BUILTINS {
        let info = listed.iter().find(|t| t.id == b.id).unwrap();
        assert_eq!((info.state, info.source, info.error.clone()), (JobTemplateState::NeedsApproval, JobTemplateSource::Builtin, None));
        assert!(info.drone.is_some());
    }
}
