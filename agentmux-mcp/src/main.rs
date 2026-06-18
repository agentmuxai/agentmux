// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! agentmux-mcp — MCP stdio server that exposes the `Shell` tool to Claude.
//!
//! Claude Code launches this binary as an MCP server (via `.mcp.json`'s
//! `"command": "agentmux-mcp"` entry, auto-injected by agent_config.rs).
//! Communication is JSON-RPC 2.0 over stdin/stdout.
//!
//! The `Shell` tool starts a persistent shell process on agentmux-srv and
//! returns immediately with a `shell_id`. Output streams live in the
//! conversation as a `ShellNode` row without blocking the agent.
//!
//! Env vars (inherited from agentmux-srv's agent env injection):
//!   AGENTMUX_LOCAL_URL   — sidecar HTTP base URL
//!   AGENTMUX_AUTH_KEY    — X-AuthKey header secret
//!   AGENTMUX_AGENT_BUS_ID — block UUID for event scoping (preferred)
//!   AGENTMUX_BLOCKID      — block UUID, used as the fallback source for the
//!                           bus id. websocket.rs reliably injects this onto the
//!                           agent process env (it holds exactly the block UUID
//!                           the MCP server needs), and the MCP subprocess —
//!                           spawned by the agent CLI — inherits it. The
//!                           production `.mcp.json` path never injects
//!                           AGENTMUX_AGENT_BUS_ID, so this fallback is what
//!                           makes the Shell tool functional.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::task::JoinHandle;

/// In-process registry of running loops (loop_id → task handle), used by
/// `Loop` (insert) and `LoopStop` (remove + abort). Loops live in this MCP
/// process, so they stop automatically when the agent pane / session ends.
type LoopRegistry = Mutex<HashMap<String, JoinHandle<()>>>;

const SHELL_TOOL: &str = r#"{
  "name": "Shell",
  "description": "Start a long-running shell process. Returns immediately with a shell_id. Output streams live in the conversation document. Use for build systems, watchers, dev servers — anything that should run in the background without blocking the conversation. Stop it later with ShellStop(shell_id) — never use kill/taskkill on a shell you started.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "cmd":   { "type": "string",  "description": "Command to run (passed to sh -c / cmd /C)" },
      "cwd":   { "type": "string",  "description": "Working directory (defaults to agent workdir)" },
      "title": { "type": "string",  "description": "Display label shown in the conversation row (defaults to cmd)" },
      "env":   { "type": "object",  "description": "Extra environment variables", "additionalProperties": { "type": "string" } }
    },
    "required": ["cmd"]
  }
}"#;

const SHELL_STOP_TOOL: &str = r#"{
  "name": "ShellStop",
  "description": "Stop a running shell started by Shell(). Pass the shell_id returned by Shell. Tree-kills the whole process group (e.g. `task dev` and its child task.exe/node processes), so prefer this over kill/taskkill — those can hit other agents' or the host's processes by name.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "shell_id": { "type": "string", "description": "The shell_id returned by a prior Shell() call" }
    },
    "required": ["shell_id"]
  }
}"#;

const SEND_MESSAGE_TOOL: &str = r#"{
  "name": "SendMessage",
  "description": "Send a message to another agent by name. The message is injected as input into the target agent's active conversation. Use for agent-to-agent coordination — handoff, task delegation, status notifications. Delivery is best-effort and tries local → LAN → cloud in order. Returns once delivery is confirmed or all tiers have failed.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "to":      { "type": "string", "description": "Name of the target agent (its AGENTMUX_AGENT_ID value)" },
      "message": { "type": "string", "description": "Message text to inject into the target agent's conversation" }
    },
    "required": ["to", "message"]
  }
}"#;

const DISCOVER_AGENTS_TOOL: &str = r#"{
  "name": "DiscoverAgents",
  "description": "List the agents and instances reachable from here across the muxbus delivery tiers (host, LAN, cloud), so you can pick a valid target before SendMessage. Returns JSON: host.addressable (agents reachable right now via local delivery), host.agents (this host's agent directory, each with an `addressable` flag), lan (LAN peers and the agents on them), and wan.subscribed_agents (cloud-subscribed agents). Addressing is by agent name, case-insensitive. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const SET_ACTIVE_TAB_TOOL: &str = r#"{
  "name": "SetActiveTab",
  "description": "Switch the active (foreground) tab to the given tab_id within its workspace. Get tab ids from Layout(query:\"tabs\") or Layout(query:\"layout\").",
  "inputSchema": {
    "type": "object",
    "properties": {
      "tab_id": { "type": "string", "description": "The tab to make active" }
    },
    "required": ["tab_id"]
  }
}"#;

const NEW_TAB_TOOL: &str = r#"{
  "name": "NewTab",
  "description": "Open a new tab in your own workspace and switch to it. Optionally name it; otherwise AgentMux auto-names it.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name": { "type": "string", "description": "Optional name for the new tab" }
    }
  }
}"#;

const FOCUS_WINDOW_TOOL: &str = r#"{
  "name": "FocusWindow",
  "description": "Bring an AgentMux window to the foreground. Defaults to your own window. Pass window_id (from Layout(query:\"windows\") or Layout(query:\"layout\")) to focus a specific one.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "window_id": { "type": "string", "description": "Optional window to focus; defaults to your own" }
    }
  }
}"#;

// Consolidated read/introspection verb — replaces the former GetLayout /
// ListWindows / ListWorkspaces / ListTabs tools (one tool, `query` selects the
// view). `WhoAmI` stays its own no-arg tool (the spec's "foundation" self-context
// call). See SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md §4.6 / §10.
const LAYOUT_TOOL: &str = r#"{
  "name": "Layout",
  "description": "Read the AgentMux UI structure around you. `query` selects what to return: \"layout\" (the full tree — every window with its workspace, tabs, and panes [block_id, view, title], and which tab is active), \"windows\" (window_id, display name, assigned workspace), \"workspaces\" (workspace_id, name, tab count, active tab), or \"tabs\" (tabs in your own workspace: tab_id, name, pane count). Read-only; use before naming or focusing things. For your OWN ids (block/tab/window/workspace), use WhoAmI instead.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "enum": ["layout", "windows", "workspaces", "tabs"], "description": "What to return (default: layout)" }
    }
  }
}"#;

const WHOAMI_TOOL: &str = r#"{
  "name": "WhoAmI",
  "description": "Return your own place in the AgentMux UI: your block (pane), tab, window, and workspace ids plus their names. Use it to discover the targets for naming/layout verbs (e.g. before SetName). Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

// Consolidated naming verb — replaces the former SetWindowName / SetTabName /
// SetPaneTitle / SetWorkspaceName tools (one tool, `target` selects which UI
// element). All default to the caller's own element and are non-destructive.
// See SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md §4.3 / §10.
const SET_NAME_TOOL: &str = r#"{
  "name": "SetName",
  "description": "Rename one of your own AgentMux UI elements. `target` selects which: \"window\" (the OS taskbar / window-title name; clamped to 64 chars), \"tab\" (the tab-bar label), \"pane\" (this conversation pane's header title), or \"workspace\" (the workspace name — also shown in the window/taskbar title when no explicit window name is set). Defaults to your own element; trimmed; non-window names clamp to 128 chars.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "target": { "type": "string", "enum": ["window", "tab", "pane", "workspace"], "description": "Which UI element to rename" },
      "name": { "type": "string", "description": "The new name/title to display" }
    },
    "required": ["target", "name"]
  }
}"#;

const OPEN_EDITOR_TOOL: &str = r#"{
  "name": "OpenEditor",
  "description": "Open a file in an AgentMux editor pane next to this conversation. Use when you want the user to see a file you're discussing or editing. Pass an absolute host path. Fire-and-forget: returns once the pane is opened.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file":  { "type": "string", "description": "Absolute path to the file to open" },
      "title": { "type": "string", "description": "Optional tab/pane title (defaults to the file name)" },
      "split": { "type": "string", "enum": ["right", "left", "down", "up"], "description": "Where to place the new pane relative to this agent pane (default: right). Ignored when floating is true." },
      "collapse_tree": { "type": "boolean", "description": "Open the editor with its file-tree sidebar collapsed (just the file, no explorer). Default: false (tree expanded)." },
      "floating": { "type": "boolean", "description": "Open the file in a floating window (a chromeless pane over the app) instead of a docked split. Default: false." }
    },
    "required": ["file"]
  }
}"#;

const LOOP_TOOL: &str = r#"{
  "name": "Loop",
  "description": "Run a prompt or slash command on a recurring interval by re-injecting it into a conversation. AgentMux's analogue of Claude's /loop. Returns immediately with a loop_id; the prompt is injected on a fixed schedule until you stop it with LoopStop(loop_id). Use for polling/babysitting tasks ('check the deploy every 5m', 'keep running /babysit-prs'). Loops stop automatically when the agent pane closes. Do NOT use for one-off tasks.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "prompt":    { "type": "string", "description": "The prompt or slash command to inject each interval (e.g. 'check the PR status' or '/babysit-prs')" },
      "interval":  { "type": "string", "description": "How often to run: a number with optional unit s/m/h (e.g. '30s', '5m', '1h'). A bare number is minutes. Default '10m'. Minimum 10s." },
      "to":        { "type": "string", "description": "Target agent name (its AGENTMUX_AGENT_ID) to inject into. Defaults to this agent itself (a self-loop)." },
      "immediate": { "type": "boolean", "description": "Run once immediately on start in addition to every interval. Default false (first run after one interval)." }
    },
    "required": ["prompt"]
  }
}"#;

const LOOP_STOP_TOOL: &str = r#"{
  "name": "LoopStop",
  "description": "Stop a recurring loop started by Loop(). Pass the loop_id it returned. Loops also stop automatically when the agent pane closes.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "loop_id": { "type": "string", "description": "The loop_id returned by a prior Loop() call" }
    },
    "required": ["loop_id"]
  }
}"#;

#[tokio::main]
async fn main() {
    let local_url = std::env::var("AGENTMUX_LOCAL_URL").unwrap_or_default();
    let auth_key = std::env::var("AGENTMUX_AUTH_KEY").unwrap_or_default();
    // The "bus id" is the agent's block UUID, used to scope shell events to the
    // conversation pane. AGENTMUX_AGENT_BUS_ID is the preferred source, but the
    // production .mcp.json path never injects it (build_mcp_config is called with
    // an empty bus id and build_config_files_with_bus has no call site). Fall
    // back to AGENTMUX_BLOCKID, which holds the same block UUID and is reliably
    // set on the agent process env (websocket.rs) and inherited by this MCP
    // subprocess. Without this fallback the Shell tool always bailed.
    let block_id = {
        let bus = std::env::var("AGENTMUX_AGENT_BUS_ID").unwrap_or_default();
        if bus.is_empty() {
            std::env::var("AGENTMUX_BLOCKID").unwrap_or_default()
        } else {
            bus
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("http client");

    // Running loops, keyed by loop_id. Lives for this MCP process's lifetime
    // (== the agent session), so loops are reaped when the agent pane closes.
    let loops: LoopRegistry = Mutex::new(HashMap::new());
    let loop_counter = AtomicU64::new(0);

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Notifications have no id — no response expected.
        if id.is_null() {
            continue;
        }

        let response = match method {
            "initialize" => {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "agentmux-mcp", "version": env!("CARGO_PKG_VERSION") }
                    }
                })
            }
            "tools/list" => {
                let shell: Value = serde_json::from_str(SHELL_TOOL).expect("static json");
                let shell_stop: Value = serde_json::from_str(SHELL_STOP_TOOL).expect("static json");
                let open_editor: Value = serde_json::from_str(OPEN_EDITOR_TOOL).expect("static json");
                let send_message: Value = serde_json::from_str(SEND_MESSAGE_TOOL).expect("static json");
                let discover_agents: Value =
                    serde_json::from_str(DISCOVER_AGENTS_TOOL).expect("static json");
                let whoami: Value = serde_json::from_str(WHOAMI_TOOL).expect("static json");
                let layout: Value = serde_json::from_str(LAYOUT_TOOL).expect("static json");
                let set_name: Value = serde_json::from_str(SET_NAME_TOOL).expect("static json");
                let set_active_tab: Value =
                    serde_json::from_str(SET_ACTIVE_TAB_TOOL).expect("static json");
                let new_tab: Value = serde_json::from_str(NEW_TAB_TOOL).expect("static json");
                let focus_window: Value =
                    serde_json::from_str(FOCUS_WINDOW_TOOL).expect("static json");
                let loop_tool: Value = serde_json::from_str(LOOP_TOOL).expect("static json");
                let loop_stop: Value = serde_json::from_str(LOOP_STOP_TOOL).expect("static json");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": [shell, shell_stop, open_editor, send_message, discover_agents, whoami, layout, set_name, set_active_tab, new_tab, focus_window, loop_tool, loop_stop] }
                })
            }
            "tools/call" => {
                match call_tool(&params, &local_url, &auth_key, &block_id, &client, &loops, &loop_counter).await {
                    Ok(content) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": [{"type": "text", "text": content}], "isError": false }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": [{"type": "text", "text": e.to_string()}], "isError": true }
                    }),
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }),
        };

        let resp_str = serde_json::to_string(&response).unwrap_or_default();
        let _ = stdout.write_all(resp_str.as_bytes()).await;
        let _ = stdout.write_all(b"\n").await;
        let _ = stdout.flush().await;
    }
}

/// Guard for the self-scoped agent-API verbs: they need the sidecar URL +
/// auth key to reach it, and the caller's block id to resolve "my own"
/// tab/pane/window/workspace.
fn require_agent_env(local_url: &str, auth_key: &str, block_id: &str) -> Result<()> {
    if local_url.is_empty() || auth_key.is_empty() {
        anyhow::bail!(
            "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
             Is this agent pane opened via AgentMux?"
        );
    }
    if block_id.is_empty() {
        anyhow::bail!(
            "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
             — cannot resolve this agent's context. Is this agent pane opened via AgentMux?"
        );
    }
    Ok(())
}

async fn call_tool(
    params: &Value,
    local_url: &str,
    auth_key: &str,
    block_id: &str,
    client: &reqwest::Client,
    loops: &LoopRegistry,
    loop_counter: &AtomicU64,
) -> Result<String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(json!({}));

    match name {
        "Shell" => {
            let cmd = arguments
                .get("cmd")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: cmd"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            if block_id.is_empty() {
                anyhow::bail!(
                    "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
                     — cannot associate the shell with a conversation pane. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let title = arguments
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(cmd);
            let cwd = arguments.get("cwd").and_then(|v| v.as_str());
            let env = arguments.get("env").cloned();

            let url = format!(
                "{}/api/v1/shell/create",
                local_url.trim_end_matches('/')
            );
            let mut body = json!({
                "agent_block_id": block_id,
                "cmd": cmd,
                "title": title,
            });
            if let Some(cwd) = cwd {
                body["cwd"] = json!(cwd);
            }
            if let Some(env) = env {
                body["env"] = env;
            }

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/create failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let shell_id = result
                .get("shell_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            Ok(shell_id.to_string())
        }
        "ShellStop" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/shell/stop", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&json!({ "shell_id": shell_id }))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/stop failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let stopped = result.get("stopped").and_then(|v| v.as_bool()).unwrap_or(false);
            Ok(if stopped {
                format!("stopped shell {shell_id}")
            } else {
                format!("shell {shell_id} was not running (unknown or already exited)")
            })
        }
        "OpenEditor" => {
            let file = arguments
                .get("file")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: file"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let split = arguments
                .get("split")
                .and_then(|v| v.as_str())
                .filter(|s| matches!(*s, "right" | "left" | "down" | "up"))
                .unwrap_or("right");

            let url = format!("{}/api/v1/pane/open", local_url.trim_end_matches('/'));
            let mut body = json!({
                "view": "editor",
                "file": file,
                "focus": true,
            });
            // Place the editor relative to the calling agent pane when we know
            // its block id (AGENTMUX_BLOCKID); otherwise the sidecar inserts it
            // at the tab root.
            if !block_id.is_empty() {
                body["split_direction"] = json!(split);
                body["split_reference_block_id"] = json!(block_id);
            }
            if let Some(title) = arguments.get("title").and_then(|v| v.as_str()) {
                body["title"] = json!(title);
            }
            // `collapse_tree: true` → open with the file-tree sidebar collapsed.
            // Maps to the editor's `tree_expanded` meta (collapsed == not expanded).
            if arguments.get("collapse_tree").and_then(|v| v.as_bool()) == Some(true) {
                body["tree_expanded"] = json!(false);
            }
            // `floating: true` → open in a floating window instead of a docked split.
            if arguments.get("floating").and_then(|v| v.as_bool()) == Some(true) {
                body["floating"] = json!(true);
            }

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("pane.open failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let block = result
                .get("block_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            Ok(format!("Opened {file} in editor pane (block {block})"))
        }
        "SendMessage" => {
            let to = arguments
                .get("to")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to"))?;
            let message = arguments
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let source_agent = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());

            let url = format!(
                "{}/agentmux/reactive/inject",
                local_url.trim_end_matches('/')
            );
            let mut body = json!({
                "target_agent": to,
                "message": message,
            });
            if let Some(src) = source_agent {
                body["source_agent"] = json!(src);
            }

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("inject failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
                Ok(format!("Message sent to {to}"))
            } else {
                let err = result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                anyhow::bail!("Message delivery failed: {err}")
            }
        }
        "DiscoverAgents" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "WhoAmI" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            if block_id.is_empty() {
                anyhow::bail!(
                    "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
                     — cannot resolve this agent's pane. Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/self", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("block_id", block_id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("self lookup failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "SetName" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: target"))?;
            let new_name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;
            require_agent_env(local_url, auth_key, block_id)?;

            // window/tab/workspace POST {"name": …} to /…/name; pane POSTs
            // {"title": …} to /pane/title. `label` is the human-facing echo.
            let (path, field, label) = match target {
                "window" => ("window/name", "name", "Window name"),
                "tab" => ("tab/name", "name", "Tab name"),
                "workspace" => ("workspace/name", "name", "Workspace name"),
                "pane" => ("pane/title", "title", "Pane title"),
                other => anyhow::bail!(
                    "invalid target '{other}' — expected one of: window, tab, pane, workspace"
                ),
            };
            let url = format!("{}/api/v1/{path}", local_url.trim_end_matches('/'));
            let mut body = json!({ "block_id": block_id });
            body[field] = json!(new_name);
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("{label} change failed: HTTP {status} — {text}");
            }
            // Surface the server-applied value (e.g. a window name clamped to 64
            // chars) when the endpoint echoes it back; fall back to the request.
            // Restores the pre-consolidation SetWindowName behavior generically.
            let applied = resp
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v.get(field).and_then(|s| s.as_str()).map(str::to_string))
                .unwrap_or_else(|| new_name.to_string());
            Ok(format!("{label} set to \"{applied}\""))
        }
        "Layout" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("layout");
            let path = match query {
                "layout" => "layout",
                "windows" => "windows",
                "workspaces" => "workspaces",
                "tabs" => "tabs",
                other => anyhow::bail!(
                    "invalid query '{other}' — expected one of: layout, windows, workspaces, tabs"
                ),
            };
            let url = format!("{}/api/v1/{path}", local_url.trim_end_matches('/'));
            let mut reqb = client.get(&url).header("X-AuthKey", auth_key);
            // The "tabs" query scopes to the caller's own workspace when we know it.
            if query == "tabs" && !block_id.is_empty() {
                reqb = reqb.query(&[("block_id", block_id)]);
            }
            let resp = reqb
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("Layout({query}) failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "SetActiveTab" => {
            let tab_id = arguments
                .get("tab_id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: tab_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/tab/activate", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&json!({ "tab_id": tab_id }))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("set active tab failed: HTTP {status} — {text}");
            }
            Ok(format!("Switched to tab {tab_id}"))
        }
        "NewTab" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let name = arguments.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let url = format!("{}/api/v1/tab/new", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&json!({ "block_id": block_id, "name": name }))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("new tab failed: HTTP {status} — {text}");
            }
            Ok(if name.is_empty() {
                "Opened a new tab".to_string()
            } else {
                format!("Opened new tab \"{name}\"")
            })
        }
        "FocusWindow" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let window_id = arguments.get("window_id").and_then(|v| v.as_str()).unwrap_or("");
            let url = format!("{}/api/v1/window/focus", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&json!({ "block_id": block_id, "window_id": window_id }))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("focus window failed: HTTP {status} — {text}");
            }
            Ok("Focused window".to_string())
        }
        "Loop" => {
            let prompt = arguments
                .get("prompt")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: prompt"))?
                .to_string();

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            // Target: explicit `to`, else self (this agent's AGENTMUX_AGENT_ID).
            let self_id = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());
            let target = arguments
                .get("to")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| self_id.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no loop target: pass `to`, or ensure AGENTMUX_AGENT_ID is set for a self-loop"
                    )
                })?;

            let interval_str = arguments
                .get("interval")
                .and_then(|v| v.as_str())
                .unwrap_or("10m")
                .to_string();
            let interval = parse_interval(&interval_str)?;
            let immediate = arguments
                .get("immediate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let n = loop_counter.fetch_add(1, Ordering::Relaxed) + 1;
            let loop_id = format!("loop-{n}");

            // Each loop is an independent task that re-injects the prompt on the
            // fixed interval (fire-and-forget; no idle-wait — InjectionRequest
            // .wait_for_idle is dead scaffolding that srv never reads).
            let interval_display = format_duration(interval);
            let url = format!("{}/agentmux/reactive/inject", local_url.trim_end_matches('/'));
            let task_client = client.clone();
            let task_auth = auth_key.to_string();
            let task_target = target.clone();
            let task_source = self_id;
            let handle = tokio::spawn(async move {
                if !immediate {
                    tokio::time::sleep(interval).await;
                }
                loop {
                    let mut body = json!({
                        "target_agent": task_target,
                        "message": prompt,
                    });
                    if let Some(src) = &task_source {
                        body["source_agent"] = json!(src);
                    }
                    let _ = task_client
                        .post(&url)
                        .header("X-AuthKey", &task_auth)
                        .json(&body)
                        .send()
                        .await;
                    tokio::time::sleep(interval).await;
                }
            });

            loops
                .lock()
                .unwrap()
                .insert(loop_id.clone(), handle);

            Ok(format!(
                "Started {loop_id}: injecting to '{target}' every {interval_display}\
                 {}. Stop with LoopStop({loop_id}).",
                if immediate { " (first run now)" } else { "" }
            ))
        }
        "LoopStop" => {
            let loop_id = arguments
                .get("loop_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: loop_id"))?;

            let removed = loops.lock().unwrap().remove(loop_id);
            match removed {
                Some(handle) => {
                    handle.abort();
                    Ok(format!("stopped {loop_id}"))
                }
                None => Ok(format!("{loop_id} was not running (unknown or already stopped)")),
            }
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

/// Parse a loop interval string into a `Duration`. Accepts a number with an
/// optional unit suffix: `s` (seconds), `m` (minutes), `h` (hours); a bare
/// number is treated as minutes. Clamped to [10s, 24h].
fn parse_interval(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("interval is empty");
    }
    let (num_part, mult_secs) = if let Some(n) = s.strip_suffix('s').or_else(|| s.strip_suffix('S')) {
        (n, 1.0_f64)
    } else if let Some(n) = s.strip_suffix('m').or_else(|| s.strip_suffix('M')) {
        (n, 60.0)
    } else if let Some(n) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
        (n, 3600.0)
    } else {
        (s, 60.0) // bare number → minutes
    };
    let val: f64 = num_part
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid interval '{s}' (expected e.g. 30s, 5m, 1h)"))?;
    if !val.is_finite() || val <= 0.0 {
        anyhow::bail!("interval must be a positive number: '{s}'");
    }
    let secs = (val * mult_secs).round() as u64;
    Ok(Duration::from_secs(secs.clamp(10, 24 * 3600)))
}

/// Human-readable representation of a clamped Duration — used in the Loop
/// success message so the reported cadence matches the actual run cadence.
fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s % 3600 == 0 {
        format!("{}h", s / 3600)
    } else if s % 60 == 0 {
        format!("{}m", s / 60)
    } else {
        format!("{}s", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tool advertised by `tools/list` must be valid JSON with a `name`
    /// and `inputSchema` — the server `expect("static json")`s these at runtime,
    /// so a malformed const would panic on the first `tools/list`. Also pins the
    /// tool count (11 original + 2 loop tools = 13). See
    /// SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md §10.
    #[test]
    fn all_tool_defs_are_valid_json_with_names() {
        let defs = [
            SHELL_TOOL,
            SHELL_STOP_TOOL,
            OPEN_EDITOR_TOOL,
            SEND_MESSAGE_TOOL,
            DISCOVER_AGENTS_TOOL,
            WHOAMI_TOOL,
            LAYOUT_TOOL,
            SET_NAME_TOOL,
            SET_ACTIVE_TAB_TOOL,
            NEW_TAB_TOOL,
            FOCUS_WINDOW_TOOL,
            LOOP_TOOL,
            LOOP_STOP_TOOL,
        ];
        assert_eq!(defs.len(), 13, "tools/list advertises 13 tools (11 original + Loop + LoopStop)");
        for d in defs {
            let v: Value = serde_json::from_str(d).expect("tool def must be valid JSON");
            assert!(
                v.get("name").and_then(|n| n.as_str()).is_some(),
                "tool def missing name: {d}"
            );
            assert!(v.get("inputSchema").is_some(), "tool def missing inputSchema");
        }
    }

    /// The two consolidated verbs must expose their discriminator enums so the
    /// model can pick the sub-action (replaces the former one-tool-per-verb set).
    #[test]
    fn consolidated_tools_expose_their_discriminators() {
        let layout: Value = serde_json::from_str(LAYOUT_TOOL).unwrap();
        let query = layout["inputSchema"]["properties"]["query"]["enum"]
            .as_array()
            .expect("Layout.query.enum is an array");
        assert_eq!(query.len(), 4, "Layout.query folds the 4 read verbs");

        let set_name: Value = serde_json::from_str(SET_NAME_TOOL).unwrap();
        let target = set_name["inputSchema"]["properties"]["target"]["enum"]
            .as_array()
            .expect("SetName.target.enum is an array");
        assert_eq!(target.len(), 4, "SetName.target folds the 4 naming verbs");
        let required = set_name["inputSchema"]["required"]
            .as_array()
            .expect("SetName.required is an array");
        assert_eq!(required.len(), 2, "SetName requires both target and name");
    }

    #[test]
    fn parse_interval_handles_units() {
        assert_eq!(parse_interval("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_interval("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_interval("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_interval("10").unwrap(), Duration::from_secs(600)); // bare = minutes
    }

    #[test]
    fn parse_interval_clamps_minimum() {
        assert_eq!(parse_interval("1s").unwrap(), Duration::from_secs(10)); // clamp to 10s
    }
}
