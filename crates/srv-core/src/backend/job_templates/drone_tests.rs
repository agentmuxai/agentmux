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
    // A call to another drone: what that drone does could not be shown in the prompt.
    refused(json!({ "nodes": [node("call", json!({ "kind": "drone", "drone": "other" }))], "edges": [] }), "a template cannot");
    refused(json!({ "nodes": [node("go", json!({ "kind": "run", "command": ["ls"] }))], "edges": [] }), "runs a command, and a template cannot");
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
        "agentmux.after-upgrade-check" => json!({ "leader": "agenty", "steward": "masty" }),
        "agentmux.triage-failed-nightly" => json!({ "repo": "acme/site", "leader": "agenty", "triager": "masty" }),
        "agentmux.rerun-flaky-job" => json!({ "repo": "acme/site", "workflow": "CI", "leader": "agenty" }),
        "agentmux.merged-pr-still-claimed" => json!({ "repo": "acme/site", "steward": "masty" }),
        "agentmux.verify-release" => json!({ "repo": "acme/site", "held_pr": 42, "leader": "agenty" }),
        other => panic!("no example for {other}"),
    };
    v.as_object().cloned().unwrap()
}

#[test]
fn the_built_in_drone_templates_are_valid_and_render_with_example_parameters() {
    let ids: Vec<&str> = DRONE_BUILTINS.iter().map(|b| b.id).collect();
    assert_eq!(
        ids,
        [
            "agentmux.scheduled-report",
            "agentmux.check-and-file-task",
            "agentmux.two-agent-handoff",
            "agentmux.after-upgrade-check",
            "agentmux.triage-failed-nightly",
            "agentmux.rerun-flaky-job",
            "agentmux.merged-pr-still-claimed",
            "agentmux.verify-release"
        ]
    );
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
                "app_query" => &["query"],
                "notify" => &["title"],
                "approval" => &["title"],
                "api" => &["url"],
                "transform" => &["input"],
                other => panic!("{}: node {} is a {other} node", b.id, node.id),
            };
            for key in needs {
                assert!(node.data.get(*key).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()), "{}: node {} needs {key}", b.id, node.id);
            }
        }
        // Nothing is secret-looking or private, and nothing writes outside AgentMux unless an Approval is on every path to it.
        let writes = graph::summarize(pkg.graph.as_ref().unwrap()).writes_outside;
        assert!(writes.is_empty() || graph::approval_guards_every_write(pkg.graph.as_ref().unwrap()), "{}: {writes:?}", b.id);
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

#[test]
fn the_after_upgrade_check_tells_the_leader_when_all_is_up_and_files_a_task_when_not() {
    let b = DRONE_BUILTINS.iter().find(|b| b.id == "agentmux.after-upgrade-check").unwrap();
    let pkg = package::load_embedded(b.id, b.files).unwrap();
    let g = graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &example(b.id)).unwrap();

    // It starts on an upgrade only, after the agents have had time to come back, and the filter is one the app events accept.
    let t = pkg.manifest.trigger.clone().unwrap();
    assert_eq!((t["kind"].clone(), t["source"].clone(), t["delay"].clone()), (json!("event"), json!("app_event"), json!(5)));
    crate::backend::jobs::app_events::validate_filter(t["filter"].as_object().unwrap()).unwrap();
    assert_eq!(t["filter"]["upgraded"], json!(true), "a plain restart starts nothing");

    // The branches: all well tells the leader and nothing else; not well files the task, then notifies.
    let to = |from: &str, handle: Option<&str>| g.edges.iter().filter(|e| e.source == from && e.source_handle.as_deref() == handle).map(|e| e.target.clone()).collect::<Vec<_>>();
    assert_eq!(to("healthy", Some("true")), ["allclear"]);
    assert_eq!(to("healthy", Some("false")), ["file"]);
    assert_eq!(to("file", None), ["alert"]);
    assert!(to("allclear", None).is_empty());
    let data = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().data.clone();
    assert_eq!((data("allclear")["to"].clone(), data("file")["target"].clone()), (json!("agenty"), json!("masty")), "the parameters land where the messages go");
}

// ── A trigger default that names the template's parameters ──────────────────

#[test]
fn a_template_trigger_may_use_its_parameters_and_only_those() {
    let m = |url: &str| manifest(json!({ "trigger": { "kind": "event", "source": "poll", "filter": { "query": "api", "every_minutes": 15, "url": url } } }));
    let mp = |url: &str| {
        let mut v = m(url);
        v["params"] = json!({ "type": "object", "properties": { "repo": { "type": "string" }, "who": { "type": "string", "default": "loap" } }, "required": ["repo"] });
        v
    };
    assert!(parse(&mp("https://api.github.com/repos/{{param.repo}}/runs"), Some(&param_graph()), None).is_ok());
    let err = parse(&m("https://api.github.com/repos/{{param.nope}}/runs"), Some(&graph()), None).unwrap_err();
    assert!(err.contains("the trigger uses {{param.nope}}") && err.contains("isn't one of its parameters"), "{err}");
}

#[test]
fn a_trigger_default_takes_each_parameter_once_and_never_reads_a_value_for_placeholders() {
    let params = json!({ "repo": "acme/site" }).as_object().cloned().unwrap();
    let mut t = json!({ "kind": "event", "filter": { "url": "https://api.github.com/repos/{{param.repo}}/runs", "n": 5 } });
    graph::render_value(&mut t, &params).unwrap();
    assert_eq!(t["filter"]["url"], "https://api.github.com/repos/acme/site/runs");
    assert_eq!(t["filter"]["n"], 5, "a number is left as it is");
    let hostile = json!({ "repo": "a/b{{trigger.item.x}}" }).as_object().cloned().unwrap();
    assert!(graph::render_value(&mut json!({ "u": "{{param.repo}}" }), &hostile).unwrap_err().contains("can't contain"));
}

// ── The acceptance drones (a), (b) and (d), in their local forms ─────────────

fn rendered(id: &str) -> (package::Package, crate::backend::storage::drone_types::DroneGraph, Value) {
    let b = DRONE_BUILTINS.iter().find(|b| b.id == id).unwrap();
    let pkg = package::load_embedded(b.id, b.files).unwrap();
    let given = example(id);
    let g = graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &given).unwrap();
    let params = manifest::resolve_params(&pkg.manifest, &given).unwrap();
    let mut trigger = pkg.manifest.trigger.clone().unwrap();
    graph::render_value(&mut trigger, &params).unwrap();
    (pkg, g, trigger)
}

fn edges_from(g: &crate::backend::storage::drone_types::DroneGraph, from: &str, handle: Option<&str>) -> Vec<String> {
    g.edges.iter().filter(|e| e.source == from && e.source_handle.as_deref() == handle).map(|e| e.target.clone()).collect()
}

#[test]
fn the_acceptance_drones_poll_the_one_repository_their_calls_name_and_the_poll_accepts_the_trigger() {
    for id in ["agentmux.triage-failed-nightly", "agentmux.rerun-flaky-job", "agentmux.merged-pr-still-claimed", "agentmux.verify-release"] {
        let (_pkg, g, trigger) = rendered(id);
        assert_eq!((trigger["kind"].clone(), trigger["source"].clone()), (json!("event"), json!("poll")), "{id}");
        let filter = trigger["filter"].as_object().unwrap();
        crate::backend::jobs::poll::validate_filter(filter).unwrap_or_else(|e| panic!("{id}: {e}"));
        let url = filter["url"].as_str().unwrap();
        assert!(url.starts_with("https://api.github.com/repos/acme/site/"), "{id}: {url}");
        assert_eq!(filter["auth"], json!({ "vault": "api-key-github", "as": "bearer" }), "{id}: the entry the parameter names");
        // Every call of the graph goes to the same site and repository, with the same entry.
        for n in g.nodes.iter().filter(|n| n.data["kind"] == "api" && n.data.get("auth").is_some()) {
            assert!(n.data["url"].as_str().unwrap().starts_with("https://api.github.com/repos/acme/site/"), "{id}: {}", n.id);
            assert_eq!(n.data["auth"], filter["auth"], "{id}: {}", n.id);
        }
    }
}

#[test]
fn the_nightly_triage_spends_an_agent_only_on_a_failure_and_files_a_task_only_when_it_is_sure() {
    let (pkg, g, trigger) = rendered("agentmux.triage-failed-nightly");
    assert_eq!(pkg.manifest.outputs, ["message:owner", "kanban_task"]);
    assert_eq!(trigger["filter"]["url"], "https://api.github.com/repos/acme/site/actions/runs?event=schedule&status=failure&per_page=10", "scheduled runs that failed");
    assert_eq!(trigger["filter"]["items"], "workflow_runs");
    let data = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().data.clone();
    assert_eq!(graph::summarize(&g).agents, 1);
    // The agent's answer is four short fields, and only a closed set says how sure it is.
    assert_eq!(data("triage")["output_schema"]["confidence"], json!({ "enum": ["high", "medium", "low"] }));
    assert_eq!(edges_from(&g, "found", Some("true")), ["loglink"]);
    assert!(edges_from(&g, "found", Some("false")).is_empty(), "no failed job in the run: it ends");
    assert_eq!(edges_from(&g, "sure", Some("true")), ["file"]);
    assert_eq!(edges_from(&g, "file", None), ["tell"]);
    // The task goes to the person named, one task for the same workflow in the same repository.
    assert_eq!((data("file")["target"].clone(), data("tell")["to"].clone()), (json!("masty"), json!("agenty")));
    assert!(data("file")["dedupe"].as_str().unwrap().contains("acme/site"));
    // The log's address is read by a call that has no credential, so the token never goes to the log's host.
    assert!(data("log").get("auth").is_none() && data("loglink").get("auth").is_some());
}

#[test]
fn the_flaky_job_check_tells_once_and_only_for_the_named_workflow_on_its_first_attempt_and_writes_nothing_outside() {
    let (pkg, g, trigger) = rendered("agentmux.rerun-flaky-job");
    // It reads untrusted text and uses a credential, so it may not write outside AgentMux (the Rule of Two):
    // the re-run is left to the person or agent the line goes to.
    assert_eq!(pkg.manifest.outputs, ["message:owner"]);
    assert!(pkg.manifest.outside_writes().is_empty());
    assert_eq!(trigger["filter"]["id"], "id", "a run is told once, by its id");
    let data = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().data.clone();
    assert_eq!(data("flaky")["expr"], "{{trigger.item.run_attempt}} == 1 && {{trigger.item.name}} == 'CI'");
    assert_eq!(edges_from(&g, "flaky", Some("true")), ["tell"]);
    assert!(edges_from(&g, "flaky", Some("false")).is_empty());
    assert!(data("tell")["body"].as_str().unwrap().contains("re-run its failed jobs"));
    assert!(graph::summarize(&g).writes_outside.is_empty());
    assert_eq!(graph::summarize(&g).agents, 0, "no agent reads the run");
}

// ── A credential is found in the graph and the trigger, not in what the manifest declares ──

/// A poll trigger and a graph of one POST, each with a Vault entry as given, and the outputs declared.
fn with_credential(trigger_vault: Option<&str>, node_vault: Option<&str>, outputs: Value, policy: Value) -> Result<package::Package, String> {
    let mut filter = json!({ "query": "api", "every_minutes": 10, "url": "https://api.github.com/repos/a/b/actions/runs" });
    if let Some(v) = trigger_vault {
        filter["auth"] = json!({ "vault": v, "as": "bearer" });
    }
    let mut call = json!({ "kind": "api", "url": "https://api.github.com/repos/a/b/x", "method": "POST" });
    if let Some(v) = node_vault {
        call["auth"] = json!({ "vault": v, "as": "bearer" });
    }
    let m = manifest(json!({
        "params": { "type": "object", "properties": { "credential": { "type": "string" } } },
        "trigger": { "kind": "event", "source": "poll", "filter": filter },
        "outputs": outputs,
        "policy": policy,
    }));
    parse(&m, Some(&json!({ "nodes": [node("call", call)], "edges": [] })), None)
}

#[test]
fn a_credential_in_the_trigger_or_a_node_counts_for_the_rule_of_two_whatever_secrets_says() {
    let outside = json!(["ci_rerun"]);
    // Untrusted (a poll), a credential, and a write outside: refused, naming the three and what to do.
    for (trigger, node, where_) in [(Some("api-key-github"), None, "in the trigger, a literal id"), (None, Some("{{param.credential}}"), "in a node, through a parameter")] {
        let err = with_credential(trigger, node, outside.clone(), Value::Null).unwrap_err();
        assert!(err.contains("reads untrusted input") && err.contains("uses a credential") && err.contains("writes outside (ci_rerun)"), "{where_}: {err}");
        assert!(err.contains("remove one of the three"), "{where_}: {err}");
    }
    // No credential anywhere: it passes, with the same outputs.
    assert!(with_credential(None, None, outside.clone(), Value::Null).is_ok());
    // A credential and untrusted input but no write outside: it passes.
    assert!(with_credential(Some("api-key-github"), Some("api-key-github"), json!(["message:owner"]), Value::Null).is_ok());
    // An approval per run lets all three stand.
    assert!(with_credential(Some("api-key-github"), None, outside, json!({ "approval": "required" })).is_ok());
}

#[test]
fn the_prompt_says_where_a_template_reads_a_credential() {
    let pkg = with_credential(Some("api-key-github"), Some("{{param.credential}}"), json!(["message:owner"]), Value::Null).unwrap();
    assert!(pkg.manifest.secrets.is_empty(), "the manifest declares none");
    assert_eq!(
        graph::credentials(pkg.graph.as_ref().unwrap(), pkg.manifest.trigger.as_ref()),
        ["trigger: api-key-github", "node call: {{param.credential}}"]
    );
    let none = with_credential(None, None, json!(["message:owner"]), Value::Null).unwrap();
    assert!(graph::credentials(none.graph.as_ref().unwrap(), none.manifest.trigger.as_ref()).is_empty());
}

#[test]
fn the_built_in_drones_that_poll_with_a_token_are_shown_with_it_and_any_that_writes_outside_has_an_approval_first() {
    for b in DRONE_BUILTINS {
        let pkg = package::load_embedded(b.id, b.files).unwrap_or_else(|e| panic!("{}: {e}", b.id));
        let credentials = graph::credentials(pkg.graph.as_ref().unwrap(), pkg.manifest.trigger.as_ref());
        let polls_with_token =
            ["agentmux.triage-failed-nightly", "agentmux.rerun-flaky-job", "agentmux.merged-pr-still-claimed", "agentmux.verify-release"].contains(&b.id);
        assert_eq!(!credentials.is_empty(), polls_with_token, "{}: {credentials:?}", b.id);
        // Reading untrusted text with a credential and writing outside is for a person to decide, node by node: only a graph
        // that puts an Approval on every path to its writes may.
        if !credentials.is_empty() && !pkg.manifest.outside_writes().is_empty() {
            assert!(graph::approval_guards_every_write(pkg.graph.as_ref().unwrap()), "{}: it reads untrusted text and uses a credential", b.id);
        }
    }
}

#[test]
fn the_merged_pull_request_check_waits_half_an_hour_and_tells_only_when_a_task_is_still_claimed() {
    let (pkg, g, trigger) = rendered("agentmux.merged-pr-still-claimed");
    assert_eq!(pkg.manifest.outputs, ["message:owner"]);
    assert_eq!((trigger["delay"].clone(), trigger["filter"]["id"].clone()), (json!(30), json!("number")));
    let data = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().data.clone();
    assert_eq!(data("claimed")["pr"], "{{trigger.item.html_url}}");
    assert_eq!(data("claimed")["state"], "claimed");
    assert_eq!(edges_from(&g, "merged", Some("true")), ["claimed"], "a pull request closed without a merge ends here");
    assert_eq!(edges_from(&g, "found", Some("true")), ["tell"]);
    assert!(graph::summarize(&g).writes_outside.is_empty());
    assert_eq!(data("tell")["to"], "masty");
}

#[test]
fn a_job_made_from_a_template_gets_the_trigger_with_its_parameters_filled_in() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("src").join("acme.echo");
    std::fs::create_dir_all(&dir).unwrap();
    let mut m = param_manifest();
    m["trigger"] = json!({ "kind": "event", "source": "poll", "filter": { "query": "api", "every_minutes": 15, "url": "https://api.github.com/repos/{{param.repo}}/actions/runs" } });
    std::fs::write(dir.join("template.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    std::fs::write(dir.join("graph.json"), serde_json::to_vec(&param_graph()).unwrap()).unwrap();
    let svc = JobTemplates::new(tmp.path().join("job-templates"), &tmp.path().join("data"), &[]);
    svc.install(&dir, false).unwrap();
    let hash = svc.get("acme.echo").unwrap().0.hash;
    svc.approve("acme.echo", &hash, "").unwrap();

    let make = |trigger: Value| {
        let mut def = json!({ "jobVersion": 1, "template": { "id": "acme.echo" }, "params": { "repo": "a/b" }, "action": { "kind": "drone", "drone_id": "d-1" }, "owner": "uid-a" });
        if !trigger.is_null() {
            def["trigger"] = trigger;
        }
        crate::backend::jobs::from_template::apply_with(&mut def, None, &svc).map(|()| def)
    };
    let def = make(Value::Null).unwrap();
    assert_eq!(def["trigger"]["filter"]["url"], "https://api.github.com/repos/a/b/actions/runs", "the poll asks about the repository the drone's calls name");
    // A trigger the job names itself is its own: nothing is filled in.
    let own = make(json!({ "kind": "manual" })).unwrap();
    assert_eq!(own["trigger"], json!({ "kind": "manual" }));
}

// ── Where a parameter lands decides what it may hold ─────────────────────────

#[test]
fn a_parameter_put_into_an_address_is_a_name_or_owner_slash_name() {
    let pkg = |id: &str| {
        let b = DRONE_BUILTINS.iter().find(|b| b.id == id).unwrap();
        package::load_embedded(b.id, b.files).unwrap()
    };
    let id = "agentmux.triage-failed-nightly";
    let p = pkg(id);
    let with = |repo: &str| {
        let mut given = example(id);
        given.insert("repo".into(), json!(repo));
        let g = graph::render(p.graph.as_ref().unwrap(), &p.manifest, &given);
        let params = manifest::resolve_params(&p.manifest, &given).unwrap();
        let mut trigger = p.manifest.trigger.clone().unwrap();
        let t = graph::render_value(&mut trigger, &params);
        (g.map(|_| ()), t)
    };
    assert!(with("acme/site").0.is_ok() && with("acme/site").1.is_ok());
    assert!(with("a-b.c_d/e-f.g_h").0.is_ok());
    for bad in ["acme/site?x=1", "acme/site#frag", "acme//site", "acme/site/actions", "../site", "acme/..", "acme/site%2F..", "a b/c", "acme/site@evil", "acme:80/site", "", "/site", "acme/"] {
        let (g, t) = with(bad);
        assert!(g.unwrap_err().contains("owner/name"), "{bad:?} in the graph");
        assert!(t.unwrap_err().contains("owner/name"), "{bad:?} in the trigger");
    }
}

#[test]
fn a_parameter_put_into_a_condition_holds_no_quote_mark_or_backslash() {
    let b = DRONE_BUILTINS.iter().find(|b| b.id == "agentmux.rerun-flaky-job").unwrap();
    let pkg = package::load_embedded(b.id, b.files).unwrap();
    let with = |workflow: &str| {
        let mut given = example(b.id);
        given.insert("workflow".into(), json!(workflow));
        graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &given)
    };
    assert!(with("CI").is_ok() && with("Build and test (linux)").is_ok());
    for bad in ["CI' || 'x' == 'x", "CI\" || \"x", "CI\\", "CI`x"] {
        assert!(with(bad).unwrap_err().contains("no quote mark or backslash"), "{bad:?}");
    }
    // Words in an agent's task or a message are prose, not an expression: a quote there is fine.
    let b = DRONE_BUILTINS.iter().find(|b| b.id == "agentmux.check-and-file-task").unwrap();
    let pkg = package::load_embedded(b.id, b.files).unwrap();
    let given = json!({ "check": "Is the 'main' build green?" }).as_object().cloned().unwrap();
    assert!(graph::render(pkg.graph.as_ref().unwrap(), &pkg.manifest, &given).is_ok());
}

// ── Verify a release (acceptance drone c) ──

#[test]
fn verify_release_merges_only_behind_an_approval_and_the_template_guard_says_so() {
    let (pkg, g, trigger) = rendered("agentmux.verify-release");
    assert_eq!(trigger["delay"], 150, "two and a half hours after the release is published");
    assert_eq!(pkg.manifest.outputs, ["message:owner", "pull_request"]);
    // The one write is the merge, and the Approval is on its only path.
    assert_eq!(graph::summarize(&g).writes_outside, ["merge: PUT https://api.github.com/repos/acme/site/pulls/42/merge"]);
    assert!(graph::approval_guards_every_write(&g));
    assert_eq!(edges_from(&g, "approve", None), ["merge"]);
    assert_eq!(edges_from(&g, "found", Some("true")), ["tell"]);
    assert_eq!(edges_from(&g, "found", Some("false")), ["missing"]);
    // The release is fetched again after the delay, and every check reads that answer, not the event's old copy.
    assert_eq!(edges_from(&g, "release", None), ["published"]);
    assert_eq!(edges_from(&g, "published", Some("false")), ["skipped"]);
    for id in ["published", "manifest"] {
        let data = serde_json::to_string(&g.nodes.iter().find(|n| n.id == id).unwrap().data).unwrap();
        assert!(data.contains("release.body") && !data.contains("trigger.item"), "{id}: {data}");
    }
    // The merge reads nothing from the release: only the template's own values.
    let merge = g.nodes.iter().find(|n| n.id == "merge").unwrap();
    assert!(!serde_json::to_string(&merge.data).unwrap().contains("trigger"), "{}", merge.data);
    // The title is the template's own words.
    let approve = g.nodes.iter().find(|n| n.id == "approve").unwrap();
    assert!(!approve.data["title"].as_str().unwrap().contains("trigger"));
}

#[test]
fn an_approval_that_is_not_on_every_path_to_a_write_does_not_satisfy_the_template_guard() {
    let node = |id: &str, data: Value| json!({ "id": id, "position": { "x": 0, "y": 0 }, "data": data });
    let put = json!({ "kind": "api", "method": "PUT", "url": "https://api.github.com/x" });
    let g = |edges: Value| -> crate::backend::storage::drone_types::DroneGraph {
        serde_json::from_value(json!({
            "nodes": [node("a", json!({ "kind": "condition", "expr": "1 == 1" })), node("ok", json!({ "kind": "approval", "title": "t" })), node("w", put.clone())],
            "edges": edges,
        }))
        .unwrap()
    };
    assert!(graph::approval_guards_every_write(&g(json!([{ "id": "1", "source": "a", "target": "ok" }, { "id": "2", "source": "ok", "target": "w" }]))));
    assert!(!graph::approval_guards_every_write(&g(json!([{ "id": "1", "source": "a", "target": "ok" }, { "id": "2", "source": "a", "target": "w" }]))), "a second path to the write");
    assert!(!graph::approval_guards_every_write(&g(json!([{ "id": "1", "source": "a", "target": "w" }]))), "no approval at all");
    assert!(!graph::approval_guards_every_write(&g(json!([]))), "an unconnected write is reachable from its own start");
}
