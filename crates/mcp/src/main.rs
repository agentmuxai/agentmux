// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! agentmux-mcp — MCP stdio server that exposes AgentMux's MuxAPI tools
//! (`Shell`, `SendMessage`, memory, panes, …) to Claude.
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
//!   AGENTMUX_LOCAL_URL    — sidecar HTTP base URL
//!   AGENTMUX_AUTH_KEY     — X-AuthKey header secret
//!   AGENTMUX_BLOCKID      — block UUID for shell event scoping (preferred).
//!                           Injected by agent_handlers/input.rs into every persistent
//!                           subprocess env; inherited by this MCP subprocess.
//!   AGENTMUX_AGENT_BUS_ID — MuxBus routing identifier (fallback only).
//!                           Often set to the agent type string (e.g. "claude")
//!                           in .mcp.json, NOT the block UUID — do not use it
//!                           as the shell scope unless AGENTMUX_BLOCKID is absent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentmux_common::api_types::{
    InjectRequest, PaneOpenRequest, PaneOpenResponse, PtyShellCreateRequest, PtyShellCreateResponse,
    PtyShellInputRequest, PtyShellInputResponse, PtyShellReadRequest, PtyShellReadResponse,
    PtyShellResizeRequest, PtyShellResizeResponse,
    PtyShellStatusRequest, PtyShellStatusResponse, PtyShellStopRequest, PtyShellStopResponse,
    ShellCreateRequest, ShellCreateResponse, ShellInputFailure, ShellInputRequest,
    ShellInputResponse, ShellStatusRequest, ShellStatusResponse, ShellStopRequest,
    ShellStopResponse, TabActivateRequest, TabNameRequest, TabNewRequest,
    UiBrowserDispatchKeyRequest, UiBrowserEvalRequest, UiBrowserFocusElementRequest,
    UiBrowserFocusInfoRequest, UiBrowserHistoryRequest, UiBrowserNavigateRequest,
    UiBrowserOpenRequest, UiClickRequest, UiPressKeysRequest, UiQueryRequest, UiRunCommandRequest,
    UiScreenshotRequest, UiScreenshotResponse, UiShortcutsListRequest, WindowFocusRequest,
    WindowNameRequest, WorkspaceNameRequest, PaneTitleRequest, ClosePaneRequest, QuitSelfRequest,
    RegisterDevServerRequest, RegisterDevServerResponse,
};
use agentmux_common::AUTH_KEY_HEADER;
use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::task::JoinHandle;

/// Metadata kept alongside each running loop's task handle.
struct LoopEntry {
    handle: JoinHandle<()>,
    prompt: String,
    target: String,
    interval_secs: u64,
    max_iterations: Option<u64>,
    fire_count: Arc<AtomicU64>,
    started_at: u64,
}

/// In-process registry of running loops, keyed by loop_id. Lives for this MCP
/// process's lifetime (== the agent session), so loops are reaped automatically
/// when the agent pane closes.
type LoopRegistry = Mutex<HashMap<String, LoopEntry>>;

mod doctor;
mod self_keys;
mod tool_schemas;
use tool_schemas::*;
mod window_capture;
use window_capture::*;

#[tokio::main]
async fn main() {
    // `agentmux-mcp doctor [--heal]`, from the agent's shell: why its AgentMux
    // tools aren't connected, with no MCP session (doctor.rs).
    if std::env::args().nth(1).as_deref() == Some("doctor") {
        std::process::exit(doctor::run(std::env::args().skip(2).collect()).await);
    }
    let local_url = std::env::var("AGENTMUX_LOCAL_URL").unwrap_or_default();
    let auth_key = std::env::var("AGENTMUX_AUTH_KEY").unwrap_or_default();
    // AGENTMUX_BLOCKID is the canonical block UUID injected by agent_handlers/input.rs
    // into the persistent subprocess env. It is what the frontend subscribes to
    // for shell_node_create events (`block:<uuid>`), so it MUST be used here.
    //
    // AGENTMUX_AGENT_BUS_ID is the MuxBus routing identifier — a different
    // concept. In existing .mcp.json files it is often set to "claude" (the
    // agent type, not the block UUID), which was accidentally being used as the
    // shell scope and caused shell_node_create to publish under `block:claude`
    // instead of the real pane UUID — making the ActivityDock never receive
    // shell events. Prefer AGENTMUX_BLOCKID; fall back to AGENTMUX_AGENT_BUS_ID
    // only for older deployments that pre-date AGENTMUX_BLOCKID injection.
    let block_id = {
        let blockid = std::env::var("AGENTMUX_BLOCKID").unwrap_or_default();
        if blockid.is_empty() {
            std::env::var("AGENTMUX_AGENT_BUS_ID").unwrap_or_default()
        } else {
            blockid
        }
    };

    // Identity M4a (spec §6.5.3): every request carries this agent's token,
    // inherited from the spawn environment (never from `.mcp.json`), so srv
    // can attribute it to the agent's UID. Attribution only — srv never
    // refuses a request for lacking it. Absent for a tokenless spawn.
    let mut default_headers = reqwest::header::HeaderMap::new();
    if let Some(token) = std::env::var("AGENTMUX_AGENT_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .and_then(|t| reqwest::header::HeaderValue::from_str(t.trim()).ok())
    {
        let mut token = token;
        token.set_sensitive(true);
        default_headers.insert("X-Agent-Token", token);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .default_headers(default_headers)
        .build()
        .expect("http client");

    // Identity M4d-3: fetch this agent's signing keys from srv now, so the
    // first signed send doesn't wait on it (self_keys.rs).
    {
        let (client, local_url, auth_key) = (client.clone(), local_url.clone(), auth_key.clone());
        tokio::spawn(async move { self_keys::refresh_if_stale(&client, &local_url, &auth_key).await });
    }

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
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": advertised_tools() }
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

mod srv_http;
mod identity;
mod tool_helpers;
mod tools;
pub(crate) use identity::*;
use srv_http::*;
use tool_helpers::*;

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

    // Identity M4d-3: every tool may sign (a jekt, a UI-automation proof), so
    // the fetched keys are checked here, once, before any of them runs.
    self_keys::refresh_if_stale(client, local_url, auth_key).await;
    tools::call(
        name,
        &arguments,
        &tools::ToolCtx {
            local_url,
            auth_key,
            block_id,
            client,
            loops,
            loop_counter,
        },
    )
    .await
}

#[cfg(test)]
#[path = "tests/main_tests.rs"]
mod tests;

/// `agentmux_common::checkout`: this test binary must hold this checkout's
/// code, not another clone's from a shared target directory.
#[cfg(test)]
mod checkout_guard {
    #[test]
    fn this_test_binary_was_built_from_this_checkout() {
        agentmux_common::checkout::check(env!("CARGO_MANIFEST_DIR")).unwrap();
    }
}
