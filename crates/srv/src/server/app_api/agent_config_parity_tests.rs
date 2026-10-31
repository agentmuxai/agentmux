// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The native / container parity harness, step 1 of
//! docs/reports/REPORT_NATIVE_AND_CONTAINER_AGENTS_ONE_CONFIG_BEST_PRACTICES_2026_10_31.md §5.
//!
//! One fixture agent (`tests/fixtures/agent_config_parity/fixture.json`) is rendered the ways AgentMux launches it,
//! for a native and a container agent:
//! - `agent.open` ([`write_agent_config_files`]), which builds the files in Rust;
//! - the window's Launch (`writeagentconfig`): the files the window built (`window_files.golden.json`, checked
//!   against `buildConfigFiles` in `agent-config-parity.test.ts`) with what srv adds to them
//!   ([`crate::server::picker_config::picker_files`]);
//! - a container turn's environment ([`container_exec_env`]), against the same environment as a native agent gets.
//!
//! Every difference between two renderings must be in `allowlist.json`, with its reason, and every allowlist entry
//! must still be a difference. A new divergence fails here until someone writes down why it is wanted; a fixed one
//! fails until its entry is deleted. `PARITY_REPORT=1 cargo test ... agent_config_parity -- --nocapture` prints the
//! observed differences as allowlist entries.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use super::agent_open::write_agent_config_files;
use crate::backend::container_credential::container_exec_env;
use crate::backend::rpc_types::AgentConfigFile;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::store::{AgentContent, AgentDefinition, Store};
use crate::backend::storage::{McpServer, McpServerReach, Skill};

const FIXTURE: &str = include_str!("../../../tests/fixtures/agent_config_parity/fixture.json");
const WINDOW_FILES: &str = include_str!("../../../tests/fixtures/agent_config_parity/window_files.golden.json");
const ALLOWLIST: &str = include_str!("../../../tests/fixtures/agent_config_parity/allowlist.json");

#[derive(Deserialize)]
struct Fixture {
    agent: FixtureAgent,
    content: BTreeMap<String, String>,
    skills: Vec<FixtureSkill>,
    mcp_servers: Vec<FixtureMcp>,
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct FixtureAgent {
    id: String,
    name: String,
    slug: String,
    provider: String,
    working_directory: String,
}

#[derive(Deserialize)]
struct FixtureSkill {
    id: String,
    name: String,
    trigger: String,
    skill_type: String,
    description: String,
    content: String,
}

#[derive(Deserialize)]
struct FixtureMcp {
    id: String,
    name: String,
    transport: String,
    config: String,
    all_agents: bool,
    in_containers: bool,
}

#[derive(Deserialize)]
struct Allow {
    path: String,
    why: String,
    /// What differs inside the file: the JSON pointers of a JSON file, the heading lines of a Markdown one. Pins the
    /// difference, so a change inside an allowed file still fails.
    #[serde(default)]
    differs: BTreeSet<String>,
    #[serde(default)]
    only_on: Vec<String>,
}

#[derive(Deserialize)]
struct Allowlist {
    open_native_vs_container: Vec<Allow>,
    window_native_vs_container: Vec<Allow>,
    entry_points_native: Vec<Allow>,
    entry_points_container: Vec<Allow>,
    env_native_vs_container: Vec<Allow>,
}

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).unwrap()
}

fn allowlist() -> Allowlist {
    serde_json::from_str(ALLOWLIST).unwrap()
}

struct World {
    store: Arc<Store>,
    agent: AgentDefinition,
}

/// The fixture agent in a fresh store with Operator Config seeded, as a native or a container agent.
fn world(fx: &Fixture, container: bool) -> World {
    let store = Arc::new(Store::open_in_memory().unwrap());
    crate::backend::operator_config_seed::auto_seed_on_startup(&store);
    let mut agent = test_agent_def(
        &fx.agent.id,
        &fx.agent.name,
        &fx.agent.provider,
        if container { "container" } else { "host" },
        1_700_000_000_000,
        "",
    );
    agent.slug = fx.agent.slug.clone();
    agent.working_directory = fx.agent.working_directory.clone();
    store.agent_def_insert(&mut agent).unwrap();
    for (content_type, content) in &fx.content {
        let row = AgentContent {
            agent_id: agent.id.clone(),
            content_type: content_type.clone(),
            content: content.clone(),
            updated_at: 1_700_000_000_000,
        };
        store.agent_content_set(&row).unwrap();
    }
    for s in &fx.skills {
        let skill = Skill {
            id: s.id.clone(),
            name: s.name.clone(),
            trigger: s.trigger.clone(),
            skill_type: s.skill_type.clone(),
            description: s.description.clone(),
            content: s.content.clone(),
            is_global: false,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        store.skill_upsert_unique(&store, &agent.id, &skill, true).unwrap();
    }
    for m in &fx.mcp_servers {
        let server = McpServer {
            id: m.id.clone(),
            name: m.name.clone(),
            transport: m.transport.clone(),
            config: m.config.clone(),
            is_global: true,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        store.mcp_server_upsert_unique_global(&server).unwrap();
        store
            .mcp_server_set_reach(&m.id, McpServerReach { all_agents: m.all_agents, in_containers: m.in_containers }, 1)
            .unwrap();
    }
    World { store, agent }
}

/// A file's content as the comparison sees it: the working directory and per-agent signing keys are not content.
fn normalized(path: &str, content: &str, work_dir: &str) -> String {
    let content = if work_dir.is_empty() {
        content.to_string()
    } else {
        content.replace(work_dir, "<WORKDIR>").replace(&work_dir.replace('\\', "/"), "<WORKDIR>")
    };
    if path == "CLAUDE.md" {
        // Both launches write it through the ownership-aware writer, which puts its marker first.
        let marker = crate::backend::agent_config::CLAUDE_MD_MANAGED_MARKER;
        return content.strip_prefix(marker).unwrap_or(&content).trim_start_matches('\n').to_string();
    }
    if !path.ends_with(".json") {
        return content;
    }
    let Ok(mut json) = serde_json::from_str::<Value>(&content) else { return content };
    if path == ".mcp.json" {
        mask_keys(&mut json);
    }
    serde_json::to_string_pretty(&sorted(json)).unwrap()
}

/// `value` with every object's keys in order: key order is not content, and the two builders differ in it.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let ordered: BTreeMap<String, Value> = map.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            Value::Object(ordered.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn mask_keys(json: &mut Value) {
    if let Some(env) = json.pointer_mut("/mcpServers/agentmux/env").and_then(Value::as_object_mut) {
        for key in ["AGENTMUX_JEKT_KEY", "AGENTMUX_LAN_KEY"] {
            if let Some(v) = env.get_mut(key) {
                *v = Value::String("<KEY>".to_string());
            }
        }
    }
}

/// Files AgentMux keeps beside the agent's config to know what it wrote; not part of what the agent reads.
fn is_bookkeeping(path: &str) -> bool {
    path.starts_with(".agentmux") || path.rsplit('/').next().is_some_and(|name| name.starts_with(".agentmux-"))
}

fn read_tree(root: &Path, dir: &Path, work_dir: &str, out: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            read_tree(root, &path, work_dir, out);
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if !is_bookkeeping(&rel) {
                let content = normalized(&rel, &text, work_dir);
                out.insert(rel, content);
            }
        }
    }
}

/// What `agent.open` writes for the fixture agent.
fn render_open(fx: &Fixture, container: bool) -> BTreeMap<String, String> {
    let w = world(fx, container);
    let dir = tempfile::tempdir().unwrap();
    let work_dir = dir.path().to_str().unwrap();
    write_agent_config_files(&w.store, &w.store, &w.store, &w.agent, &w.agent.slug, work_dir).unwrap();
    let mut out = BTreeMap::new();
    read_tree(dir.path(), dir.path(), work_dir, &mut out);
    out
}

/// What the window's Launch (`writeagentconfig`) leaves to write for the fixture agent.
fn render_window(fx: &Fixture, container: bool) -> BTreeMap<String, String> {
    let w = world(fx, container);
    let files: Vec<AgentConfigFile> = serde_json::from_str(WINDOW_FILES).unwrap();
    let mode = if container { "container" } else { "host" };
    crate::server::picker_config::picker_files(files, &w.store, &w.store, &w.store, Some(&w.agent.id), mode, true)
        .into_iter()
        .map(|f| (f.path.clone(), normalized(&f.path, &f.content, "")))
        .collect()
}

/// The JSON pointers of every leaf in `value`, with the leaf.
fn leaves(prefix: &str, value: &Value, out: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(map) => map.iter().for_each(|(k, v)| leaves(&format!("{prefix}/{k}"), v, out)),
        Value::Array(items) => items.iter().enumerate().for_each(|(i, v)| leaves(&format!("{prefix}/{i}"), v, out)),
        leaf => {
            out.insert(prefix.to_string(), leaf.to_string());
        }
    }
}

/// What differs inside one file: the JSON pointers that are on one side only or hold other values, or, for text, the
/// heading lines on one side only.
fn fingerprint(path: &str, left: Option<&String>, right: Option<&String>) -> BTreeSet<String> {
    let (Some(left), Some(right)) = (left, right) else {
        return BTreeSet::from([if left.is_some() { "(only in the first)" } else { "(only in the second)" }.to_string()]);
    };
    if path.ends_with(".json") {
        let flatten = |text: &String| {
            let mut out = BTreeMap::new();
            leaves("", &serde_json::from_str::<Value>(text).unwrap_or(Value::Null), &mut out);
            out
        };
        let (l, r) = (flatten(left), flatten(right));
        return l.keys().chain(r.keys()).filter(|k| l.get(*k) != r.get(*k)).cloned().collect();
    }
    let headings = |text: &String| text.lines().filter(|l| l.starts_with('#')).map(str::to_string).collect::<BTreeSet<_>>();
    let (l, r) = (headings(left), headings(right));
    l.symmetric_difference(&r).cloned().collect()
}

/// Where two renderings differ: each path in one only, or in both with different content, with what differs in it.
fn differences(left: &BTreeMap<String, String>, right: &BTreeMap<String, String>) -> BTreeMap<String, BTreeSet<String>> {
    let out: BTreeMap<String, BTreeSet<String>> = left
        .keys()
        .chain(right.keys())
        .filter(|path| left.get(*path) != right.get(*path))
        .map(|path| (path.clone(), fingerprint(path, left.get(path), right.get(path))))
        .collect();
    if let Some(dir) = std::env::var_os("PARITY_REPORT").filter(|d| d.len() > 1) {
        // PARITY_REPORT=<dir> also writes each differing file's two renderings there, to `diff`.
        static CALL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = CALL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::fs::create_dir_all(&dir).unwrap();
        for path in out.keys() {
            let name = path.replace('/', "_");
            let dir = Path::new(&dir);
            std::fs::write(dir.join(format!("{n}-{name}.left")), left.get(path).map_or("", |s| s.as_str())).unwrap();
            std::fs::write(dir.join(format!("{n}-{name}.right")), right.get(path).map_or("", |s| s.as_str())).unwrap();
        }
    }
    out
}

/// The environment names a container turn changes against a native agent's: removed, added or changed.
fn env_differences(fx: &Fixture) -> BTreeMap<String, BTreeSet<String>> {
    let native: HashMap<String, String> = fx.env.clone().into_iter().collect();
    let container: HashMap<String, String> = container_exec_env("parity-block", &native).into_iter().collect();
    native
        .keys()
        .chain(container.keys())
        .filter(|name| native.get(*name) != container.get(*name))
        .map(|name| (name.clone(), BTreeSet::new()))
        .collect()
}

fn expected(entries: &[Allow]) -> BTreeMap<String, BTreeSet<String>> {
    entries
        .iter()
        .filter(|a| a.only_on.is_empty() || a.only_on.iter().any(|os| os == std::env::consts::OS))
        .map(|a| (a.path.clone(), a.differs.clone()))
        .collect()
}

fn report(what: &str, observed: &BTreeMap<String, BTreeSet<String>>) {
    if std::env::var_os("PARITY_REPORT").is_some() {
        eprintln!("PARITY {what}:");
        for (path, differs) in observed {
            let differs = serde_json::to_string(differs).unwrap();
            eprintln!("    {{ \"path\": {path:?}, \"differs\": {differs}, \"why\": \"\" }},");
        }
    }
}

fn check(what: &str, observed: BTreeMap<String, BTreeSet<String>>, allowed: &[Allow]) {
    report(what, &observed);
    assert!(allowed.iter().all(|a| !a.why.trim().is_empty()), "{what}: every allowlist entry needs a reason");
    let want = expected(allowed);
    let undeclared: Vec<_> = observed.iter().filter(|(path, _)| !want.contains_key(*path)).collect();
    let stale: Vec<_> = want.keys().filter(|path| !observed.contains_key(*path)).collect();
    let changed: Vec<_> =
        observed.iter().filter(|(path, differs)| want.get(*path).is_some_and(|w| w != *differs)).collect();
    assert!(
        undeclared.is_empty() && stale.is_empty() && changed.is_empty(),
        "{what}: differences with no allowlist entry (write down why they are wanted): {undeclared:?};          entries that are no longer differences (delete them): {stale:?};          files that differ in other places than their entry says (review, then update `differs`): {changed:?}"
    );
}

#[test]
fn the_fixture_renders_something() {
    // Zero cases is a failure, not a pass: an empty rendering would make every comparison below trivially equal.
    let fx = fixture();
    for container in [false, true] {
        for files in [render_open(&fx, container), render_window(&fx, container)] {
            assert!(files.contains_key("CLAUDE.md"), "no startup file rendered");
            assert!(files.contains_key(".mcp.json"), "no .mcp.json rendered");
        }
    }
}

#[test]
fn agent_open_native_vs_container_is_the_allowlist() {
    let fx = fixture();
    let observed = differences(&render_open(&fx, false), &render_open(&fx, true));
    check("open_native_vs_container", observed, &allowlist().open_native_vs_container);
}

#[test]
fn the_windows_launch_native_vs_container_is_the_allowlist() {
    let fx = fixture();
    let observed = differences(&render_window(&fx, false), &render_window(&fx, true));
    check("window_native_vs_container", observed, &allowlist().window_native_vs_container);
}

#[test]
fn the_two_entry_points_agree_for_a_native_agent_except_the_allowlist() {
    let fx = fixture();
    let observed = differences(&render_open(&fx, false), &render_window(&fx, false));
    check("entry_points_native", observed, &allowlist().entry_points_native);
}

#[test]
fn the_two_entry_points_agree_for_a_container_agent_except_the_allowlist() {
    let fx = fixture();
    let observed = differences(&render_open(&fx, true), &render_window(&fx, true));
    check("entry_points_container", observed, &allowlist().entry_points_container);
}

#[test]
fn a_container_turns_environment_differs_from_a_native_one_only_as_allowlisted() {
    let fx = fixture();
    check("env_native_vs_container", env_differences(&fx), &allowlist().env_native_vs_container);
}
