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
/// prefix is the program and the server's command line, joined unquoted, is its one argument.
fn through_prefix(entry: &Value, prefix: &str) -> Option<Value> {
    let command = entry.get("command")?.as_str()?;
    let mut line = command.to_string();
    for arg in entry.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        line.push(' ');
        line.push_str(arg);
    }
    let mut config = json!({ "command": prefix, "args": [line] });
    if let Some(env) = entry.get("env") {
        config["env"] = env.clone();
    }
    Some(config)
}

/// Test the `agentmux` server of the agent about to launch, once per agent per build, in the
/// background. Called with the environment the CLI will get; without a shell prefix there is nothing
/// to test that the server's own start would not show.
pub fn launch(env: &HashMap<String, String>, block_id: &str, block_meta: &crate::backend::obj::MetaMapType) {
    let Some(prefix) = env.get(PREFIX_ENV).filter(|p| !p.is_empty()).cloned() else { return };
    let agent = env.get("AGENTMUX_AGENT_ID").cloned().unwrap_or_default();
    let cwd = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
    if agent.is_empty() || cwd.is_empty() {
        return;
    }
    if !tested().lock().unwrap().insert(format!("{agent}@{}", env!("CARGO_PKG_VERSION"))) {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let (block_id, path) = (block_id.to_string(), Path::new(&cwd).join(".mcp.json"));
    runtime.spawn(async move {
        let result = run(&path, &prefix).await;
        match &result {
            Ok(()) => tracing::info!(block_id = %block_id, "mcp launch self-test: the agentmux server answers through the shell prefix"),
            Err(e) => tracing::warn!(block_id = %block_id, "mcp launch self-test failed: {e}"),
        }
        crate::backend::mcp_health::on_selftest(&block_id, result);
    });
}

/// Probe the `agentmux` entry of the `.mcp.json` at `path` through `prefix`.
async fn run(path: &Path, prefix: &str) -> Result<(), String> {
    let text = tokio::fs::read_to_string(path).await.map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let entry = entry_of(&text).ok_or_else(|| ".mcp.json has no agentmux entry".to_string())?;
    let config = through_prefix(&entry, prefix).ok_or_else(|| "the agentmux entry has no command".to_string())?;
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
        let config = through_prefix(&entry, "/t/agentmux-bashwrap").unwrap();
        assert_eq!(config["command"], "/t/agentmux-bashwrap");
        assert_eq!(config["args"], json!(["agentmux-mcp --stdio x"]), "one argument: the command line");
        assert_eq!(config["env"]["AGENTMUX_AGENT_ID"], "manoz");
        assert_eq!(through_prefix(&json!({ "command": "agentmux-mcp" }), "p").unwrap()["args"], json!(["agentmux-mcp"]));
        assert!(through_prefix(&json!({ "args": [] }), "p").is_none(), "no command, nothing to start");
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
        assert!(run(&path, "p").await.unwrap_err().contains("cannot read"));
        std::fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();
        assert!(run(&path, "p").await.unwrap_err().contains("no agentmux entry"));
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
        assert_eq!(run(&dir.join(".mcp.json"), &prefix).await, Ok(()));
        write("agentmux-mcp-that-does-not-exist-anywhere");
        assert!(run(&dir.join(".mcp.json"), &prefix).await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
