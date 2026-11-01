// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The launch self-test of an agent's `agentmux` MCP server (`SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md`
//! §2 and §4.1, signal C): once per agent per AgentMux build, start the server the way Claude Code will,
//! through the shell prefix and from the entry in the agent's `.mcp.json`, and speak the MCP handshake
//! to it (`mcp_probe`). The bug this is for (v0.59.19) started fine directly and not through the
//! prefix, so a probe of the bare entry would have said it was healthy.
//!
//! It runs in the background while the agent launches and changes nothing about the launch. A failure
//! is logged and kept for `agentmux-mcp doctor` (`mcp_health::Snapshot::selftest`).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

/// The env var Claude Code reads for its shell prefix.
const PREFIX_ENV: &str = "CLAUDE_CODE_SHELL_PREFIX";

/// Agents already tested by this build: `<agent>@<version>`.
fn tested() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

/// The `agentmux` entry of a `.mcp.json`.
fn entry_of(mcp_json: &str) -> Option<Value> {
    let doc: Value = serde_json::from_str(mcp_json).ok()?;
    doc.pointer("/mcpServers/agentmux").cloned()
}

/// The probe's config for `entry` started through `prefix` the way the CLI starts a stdio server: the
/// prefix is the program and the server's command line, joined unquoted, is its one argument. The
/// server gets the entry's `env` and the agent's `PATH` (`path`), which has the AgentMux tools directory
/// the bare `agentmux-mcp` command is found in; srv's own `PATH` may not. An `env` of the entry wins.
fn through_prefix(entry: &Value, prefix: &str, path: Option<&str>) -> Option<Value> {
    let command = entry.get("command")?.as_str()?;
    let mut line = command.to_string();
    for arg in entry.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        line.push(' ');
        line.push_str(arg);
    }
    let mut env = entry.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(path) = path {
        env.entry("PATH").or_insert_with(|| json!(path));
    }
    Some(json!({ "command": prefix, "args": [line], "env": env }))
}

/// Test the `agentmux` server of the agent about to launch, once per agent per build, in the
/// background. Called with the environment the CLI will get; without a shell prefix there is nothing
/// to test that the server's own start would not show.
pub fn launch(env: &HashMap<String, String>, block_id: &str, block_meta: &crate::backend::obj::MetaMapType) {
    // A container agent has the prefix stripped (`agent_env_scope`) and starts its server inside the image.
    if crate::backend::obj::meta_get_string(block_meta, "agentMode", "host") == "container" {
        return;
    }
    let Some(prefix) = env.get(PREFIX_ENV).filter(|p| !p.is_empty()).cloned() else { return };
    let agent = env.get("AGENTMUX_AGENT_ID").cloned().unwrap_or_default();
    let cwd = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
    if agent.is_empty() || cwd.is_empty() {
        return;
    }
    let key = format!("{agent}@{}", env!("CARGO_PKG_VERSION"));
    if !tested().lock().unwrap().insert(key.clone()) {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tested().lock().unwrap().remove(&key);
        return;
    };
    let (block_id, path, agent_path) = (block_id.to_string(), mcp_json_path(&cwd), env.get("PATH").cloned());
    runtime.spawn(async move {
        let result = run(&path, &prefix, agent_path.as_deref()).await;
        match &result {
            Ok(()) => tracing::info!(block_id = %block_id, "mcp launch self-test: the agentmux server answers through the shell prefix"),
            Err(e) => {
                tracing::warn!(block_id = %block_id, "mcp launch self-test failed: {e}");
                // Not final: the agent's next launch tests it again.
                tested().lock().unwrap().remove(&key);
            }
        }
        crate::backend::mcp_health::on_selftest(&block_id, result);
    });
}

/// The agent's `.mcp.json`: `cmd:cwd` may be a default folder stored as `~/.agentmux/agents/<slug>`.
fn mcp_json_path(cwd: &str) -> std::path::PathBuf {
    crate::backend::base::expand_home_dir_safe(cwd).join(".mcp.json")
}

/// Probe the `agentmux` entry of the `.mcp.json` at `path` through `prefix`.
async fn run(path: &Path, prefix: &str, agent_path: Option<&str>) -> Result<(), String> {
    let text = tokio::fs::read_to_string(path).await.map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let entry = entry_of(&text).ok_or_else(|| ".mcp.json has no agentmux entry".to_string())?;
    let config = through_prefix(&entry, prefix, agent_path).ok_or_else(|| "the agentmux entry has no command".to_string())?;
    let probed = crate::backend::mcp_probe::probe("stdio", &config.to_string()).await;
    match probed.status {
        crate::backend::mcp_probe::ProbeStatus::Connected => Ok(()),
        _ => Err(probed.error.unwrap_or_else(|| format!("{:?}", probed.status))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_started_the_way_the_cli_starts_it() {
        let entry = json!({ "command": "agentmux-mcp", "args": ["--stdio", "x"], "env": { "AGENTMUX_AGENT_ID": "manoz" } });
        let config = through_prefix(&entry, "/t/agentmux-bashwrap", None).unwrap();
        assert_eq!(config["command"], "/t/agentmux-bashwrap");
        assert_eq!(config["args"], json!(["agentmux-mcp --stdio x"]), "one argument: the command line");
        assert_eq!(config["env"]["AGENTMUX_AGENT_ID"], "manoz");
        assert_eq!(through_prefix(&json!({ "command": "agentmux-mcp" }), "p", None).unwrap()["args"], json!(["agentmux-mcp"]));
        assert!(through_prefix(&json!({ "args": [] }), "p", None).is_none(), "no command, nothing to start");
    }

    #[test]
    fn the_server_gets_the_agents_path_unless_its_entry_sets_one() {
        let bare = json!({ "command": "agentmux-mcp", "env": { "AGENTMUX_AGENT_ID": "manoz" } });
        let config = through_prefix(&bare, "p", Some("/agent/tools:/usr/bin")).unwrap();
        assert_eq!(config["env"]["PATH"], "/agent/tools:/usr/bin", "the bare command is found where the CLI finds it");
        assert_eq!(config["env"]["AGENTMUX_AGENT_ID"], "manoz");
        let own = json!({ "command": "x", "env": { "PATH": "/entry/own" } });
        assert_eq!(through_prefix(&own, "p", Some("/agent/tools")).unwrap()["env"]["PATH"], "/entry/own");
    }

    #[test]
    fn a_container_agent_is_not_tested() {
        let env = HashMap::from([("CLAUDE_CODE_SHELL_PREFIX".to_string(), "/w".to_string()), ("AGENTMUX_AGENT_ID".to_string(), "a-container-agent".to_string())]);
        let meta: crate::backend::obj::MetaMapType = HashMap::from([("agentMode".to_string(), json!("container")), ("cmd:cwd".to_string(), json!("/work"))]);
        launch(&env, "b", &meta);
        assert!(!tested().lock().unwrap().iter().any(|k| k.starts_with("a-container-agent@")), "no test was started for it");
    }

    #[test]
    fn a_default_workspace_stored_with_a_tilde_is_found() {
        let path = mcp_json_path("~/.agentmux/agents/x-0930a");
        assert!(!path.starts_with("~"), "{}", path.display());
        assert_eq!(path.file_name().unwrap(), ".mcp.json");
        assert!(path.parent().unwrap().ends_with("x-0930a"), "{}", path.display());
        assert_eq!(mcp_json_path("/work/a"), Path::new("/work/a").join(".mcp.json"));
    }

    #[test]
    fn only_the_agentmux_entry_is_tested() {
        let doc = r#"{"mcpServers":{"github":{"command":"gh-mcp"},"agentmux":{"command":"agentmux-mcp"}}}"#;
        assert_eq!(entry_of(doc).unwrap()["command"], "agentmux-mcp");
        assert!(entry_of(r#"{"mcpServers":{"github":{"command":"x"}}}"#).is_none());
        assert!(entry_of("not json").is_none());
    }

    #[tokio::test]
    async fn a_file_that_is_missing_or_has_no_entry_is_an_error_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("agentmux-selftest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".mcp.json");
        assert!(run(&path, "p", None).await.unwrap_err().contains("cannot read"));
        std::fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();
        assert!(run(&path, "p", None).await.unwrap_err().contains("no agentmux entry"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The real wrapper as the prefix and a stub server behind it: the handshake goes through, and a
    /// server that cannot start is an error. Skipped when `agentmux-bashwrap` is not built next to srv.
    #[tokio::test]
    async fn a_server_behind_the_real_prefix_answers_and_one_that_cannot_start_does_not() {
        let exe = if cfg!(windows) { "agentmux-bashwrap.exe" } else { "agentmux-bashwrap" };
        let wrapper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug").join(exe);
        let has_node = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("node").is_file() || d.join("node.exe").is_file()));
        if !wrapper.is_file() || !has_node {
            eprintln!("selftest wrapper test: agentmux-bashwrap or node missing: skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("agentmux-selftest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let stub = dir.join("stub.js");
        std::fs::write(
            &stub,
            r#"const rl = require("readline").createInterface({ input: process.stdin });
rl.on("line", (l) => { let m; try { m = JSON.parse(l); } catch { return; }
  if (m.method === "initialize") console.log(JSON.stringify({ jsonrpc: "2.0", id: m.id, result: { protocolVersion: "2024-11-05", capabilities: {}, serverInfo: { name: "stub", version: "1" } } }));
  if (m.method === "tools/list") console.log(JSON.stringify({ jsonrpc: "2.0", id: m.id, result: { tools: [] } })); });"#,
        )
        .unwrap();
        let write = |command: &str| {
            let doc = json!({ "mcpServers": { "agentmux": { "command": command, "args": [] } } });
            std::fs::write(dir.join(".mcp.json"), doc.to_string()).unwrap();
        };
        let prefix = wrapper.to_string_lossy().to_string();
        write(&format!("node {}", stub.to_string_lossy().replace('\\', "/")));
        assert_eq!(run(&dir.join(".mcp.json"), &prefix, None).await, Ok(()));
        write("agentmux-mcp-that-does-not-exist-anywhere");
        assert!(run(&dir.join(".mcp.json"), &prefix, None).await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
