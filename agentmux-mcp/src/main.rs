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

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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

const WHOAMI_TOOL: &str = r#"{
  "name": "WhoAmI",
  "description": "Return your own place in the AgentMux UI: your block (pane), tab, window, and workspace ids plus their names. Use it to discover the targets for naming/layout verbs (e.g. before SetWindowName). Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const SET_WINDOW_NAME_TOOL: &str = r#"{
  "name": "SetWindowName",
  "description": "Set the name of your AgentMux window as it appears in the OS taskbar / window title (e.g. \"Starter Workspace\" → a label you choose). Defaults to your own window. Useful for making an instance easy to identify. Name is trimmed and clamped to 64 characters.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name": { "type": "string", "description": "The window display name to show in the taskbar / title bar" }
    },
    "required": ["name"]
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
                let set_window_name: Value =
                    serde_json::from_str(SET_WINDOW_NAME_TOOL).expect("static json");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": [shell, shell_stop, open_editor, send_message, discover_agents, whoami, set_window_name] }
                })
            }
            "tools/call" => {
                match call_tool(&params, &local_url, &auth_key, &block_id, &client).await {
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

async fn call_tool(
    params: &Value,
    local_url: &str,
    auth_key: &str,
    block_id: &str,
    client: &reqwest::Client,
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
        "SetWindowName" => {
            let new_name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            if block_id.is_empty() {
                anyhow::bail!(
                    "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
                     — cannot resolve this agent's window. Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/window/name", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&json!({ "block_id": block_id, "name": new_name }))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("set window name failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let applied = result
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(new_name);
            Ok(format!("Window name set to \"{applied}\""))
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}
