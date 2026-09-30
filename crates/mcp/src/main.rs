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
//!   AGENTMUX_LOCAL_URL    — sidecar HTTP base URL
//!   AGENTMUX_AUTH_KEY     — X-AuthKey header secret
//!   AGENTMUX_BLOCKID      — block UUID for shell event scoping (preferred).
//!                           Injected by agent_handlers.rs into every persistent
//!                           subprocess env; inherited by this MCP subprocess.
//!   AGENTMUX_AGENT_BUS_ID — MuxBus routing identifier (fallback only).
//!                           Often set to the agent type string (e.g. "claude")
//!                           in .mcp.json, NOT the block UUID — do not use it
//!                           as the shell scope unless AGENTMUX_BLOCKID is absent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agentmux_common::api_types::{
    InjectRequest, PaneOpenRequest, PaneOpenResponse, PtyShellCreateRequest, PtyShellCreateResponse,
    PtyShellInputRequest, PtyShellInputResponse, PtyShellReadRequest, PtyShellReadResponse,
    PtyShellResizeRequest, PtyShellResizeResponse,
    PtyShellStatusRequest, PtyShellStatusResponse, PtyShellStopRequest, PtyShellStopResponse,
    ShellCreateRequest, ShellCreateResponse, ShellInputFailure, ShellInputRequest,
    ShellInputResponse, ShellStatusRequest, ShellStatusResponse, ShellStopRequest,
    ShellStopResponse, TabActivateRequest, TabNameRequest, TabNewRequest,
    UiBrowserDispatchKeyRequest, UiBrowserEvalRequest, UiBrowserFocusElementRequest,
    UiBrowserFocusInfoRequest, UiBrowserHistoryRequest, UiBrowserNavigateRequest, UiClickRequest,
    UiQueryRequest, UiScreenshotRequest, UiScreenshotResponse, WindowFocusRequest,
    WindowNameRequest, WorkspaceNameRequest, PaneTitleRequest, ClosePaneRequest, QuitSelfRequest,
    RegisterDevServerRequest, RegisterDevServerResponse,
};
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

mod tool_schemas;
use tool_schemas::*;
mod window_capture;
use window_capture::*;

#[tokio::main]
async fn main() {
    let local_url = std::env::var("AGENTMUX_LOCAL_URL").unwrap_or_default();
    let auth_key = std::env::var("AGENTMUX_AUTH_KEY").unwrap_or_default();
    // AGENTMUX_BLOCKID is the canonical block UUID injected by agent_handlers.rs
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
                let shell_input: Value = serde_json::from_str(SHELL_INPUT_TOOL).expect("static json");
                let shell_status: Value = serde_json::from_str(SHELL_STATUS_TOOL).expect("static json");
                let pty_shell: Value = serde_json::from_str(PTY_SHELL_TOOL).expect("static json");
                let pty_shell_input: Value =
                    serde_json::from_str(PTY_SHELL_INPUT_TOOL).expect("static json");
                let pty_shell_resize: Value =
                    serde_json::from_str(PTY_SHELL_RESIZE_TOOL).expect("static json");
                let pty_shell_read: Value =
                    serde_json::from_str(PTY_SHELL_READ_TOOL).expect("static json");
                let pty_shell_status: Value =
                    serde_json::from_str(PTY_SHELL_STATUS_TOOL).expect("static json");
                let pty_shell_stop: Value =
                    serde_json::from_str(PTY_SHELL_STOP_TOOL).expect("static json");
                let open_editor: Value = serde_json::from_str(OPEN_EDITOR_TOOL).expect("static json");
                let open_media: Value = serde_json::from_str(OPEN_MEDIA_TOOL).expect("static json");
                let send_message: Value = serde_json::from_str(SEND_MESSAGE_TOOL).expect("static json");
                let discover_agents: Value =
                    serde_json::from_str(DISCOVER_AGENTS_TOOL).expect("static json");
                let get_agent_transcript: Value =
                    serde_json::from_str(GET_AGENT_TRANSCRIPT_TOOL).expect("static json");
                let list_conversations: Value =
                    serde_json::from_str(LIST_CONVERSATIONS_TOOL).expect("static json");
                let search_history: Value =
                    serde_json::from_str(SEARCH_HISTORY_TOOL).expect("static json");
                let supervisor_nudge: Value =
                    serde_json::from_str(SUPERVISOR_NUDGE_TOOL).expect("static json");
                let whoami: Value = serde_json::from_str(WHOAMI_TOOL).expect("static json");
                let layout: Value = serde_json::from_str(LAYOUT_TOOL).expect("static json");
                let set_name: Value = serde_json::from_str(SET_NAME_TOOL).expect("static json");
                let set_active_tab: Value =
                    serde_json::from_str(SET_ACTIVE_TAB_TOOL).expect("static json");
                let new_tab: Value = serde_json::from_str(NEW_TAB_TOOL).expect("static json");
                let focus_window: Value =
                    serde_json::from_str(FOCUS_WINDOW_TOOL).expect("static json");
                let ui_screenshot: Value =
                    serde_json::from_str(UI_SCREENSHOT_TOOL).expect("static json");
                let ui_click: Value = serde_json::from_str(UI_CLICK_TOOL).expect("static json");
                let ui_query: Value = serde_json::from_str(UI_QUERY_TOOL).expect("static json");
                let close_pane: Value =
                    serde_json::from_str(CLOSE_PANE_TOOL).expect("static json");
                let quit_self: Value = serde_json::from_str(QUIT_SELF_TOOL).expect("static json");
                let register_dev_server: Value =
                    serde_json::from_str(REGISTER_DEV_SERVER_TOOL).expect("static json");
                let browser_navigate: Value =
                    serde_json::from_str(BROWSER_NAVIGATE_TOOL).expect("static json");
                let browser_back: Value =
                    serde_json::from_str(BROWSER_BACK_TOOL).expect("static json");
                let browser_forward: Value =
                    serde_json::from_str(BROWSER_FORWARD_TOOL).expect("static json");
                let browser_reload: Value =
                    serde_json::from_str(BROWSER_RELOAD_TOOL).expect("static json");
                let browser_eval: Value =
                    serde_json::from_str(BROWSER_EVAL_TOOL).expect("static json");
                let browser_dispatch_key: Value =
                    serde_json::from_str(BROWSER_DISPATCH_KEY_TOOL).expect("static json");
                let browser_focus_element: Value =
                    serde_json::from_str(BROWSER_FOCUS_ELEMENT_TOOL).expect("static json");
                let browser_focus_info: Value =
                    serde_json::from_str(BROWSER_FOCUS_INFO_TOOL).expect("static json");
                let capture_window: Value =
                    serde_json::from_str(CAPTURE_WINDOW_TOOL).expect("static json");
                let discover_windows: Value =
                    serde_json::from_str(DISCOVER_WINDOWS_TOOL).expect("static json");
                let fleet_list: Value = serde_json::from_str(FLEET_LIST_TOOL).expect("static json");
                let fleet_broadcast: Value =
                    serde_json::from_str(FLEET_BROADCAST_TOOL).expect("static json");
                let fleet_bulk_stop: Value =
                    serde_json::from_str(FLEET_BULK_STOP_TOOL).expect("static json");
                let open_agent: Value =
                    serde_json::from_str(OPEN_AGENT_TOOL).expect("static json");
                let loop_tool: Value = serde_json::from_str(LOOP_TOOL).expect("static json");
                let loop_stop: Value = serde_json::from_str(LOOP_STOP_TOOL).expect("static json");
                let loop_list: Value = serde_json::from_str(LOOP_LIST_TOOL).expect("static json");
                let work_enqueue: Value = serde_json::from_str(WORK_ENQUEUE_TOOL).expect("static json");
                let work_claim: Value = serde_json::from_str(WORK_CLAIM_TOOL).expect("static json");
                let work_heartbeat: Value = serde_json::from_str(WORK_HEARTBEAT_TOOL).expect("static json");
                let work_complete: Value = serde_json::from_str(WORK_COMPLETE_TOOL).expect("static json");
                let work_release: Value = serde_json::from_str(WORK_RELEASE_TOOL).expect("static json");
                let work_list: Value = serde_json::from_str(WORK_LIST_TOOL).expect("static json");
                let cron_create: Value = serde_json::from_str(CRON_CREATE_TOOL).expect("static json");
                let cron_delete: Value = serde_json::from_str(CRON_DELETE_TOOL).expect("static json");
                let cron_list: Value = serde_json::from_str(CRON_LIST_TOOL).expect("static json");
                let cron_pause: Value = serde_json::from_str(CRON_PAUSE_TOOL).expect("static json");
                let cron_resume: Value = serde_json::from_str(CRON_RESUME_TOOL).expect("static json");
                let memory_list: Value = serde_json::from_str(MEMORY_LIST_TOOL).expect("static json");
                let memory_read: Value = serde_json::from_str(MEMORY_READ_TOOL).expect("static json");
                let memory_write: Value = serde_json::from_str(MEMORY_WRITE_TOOL).expect("static json");
                let memory_history: Value = serde_json::from_str(MEMORY_HISTORY_TOOL).expect("static json");
                let memory_diff: Value = serde_json::from_str(MEMORY_DIFF_TOOL).expect("static json");
                let memory_revert: Value = serde_json::from_str(MEMORY_REVERT_TOOL).expect("static json");
                let global_memory_list: Value = serde_json::from_str(GLOBAL_MEMORY_LIST_TOOL).expect("static json");
                let global_memory_read: Value = serde_json::from_str(GLOBAL_MEMORY_READ_TOOL).expect("static json");
                let global_memory_write: Value = serde_json::from_str(GLOBAL_MEMORY_WRITE_TOOL).expect("static json");
                let global_memory_remove: Value = serde_json::from_str(GLOBAL_MEMORY_REMOVE_TOOL).expect("static json");
                let global_memory_history: Value = serde_json::from_str(GLOBAL_MEMORY_HISTORY_TOOL).expect("static json");
                let global_memory_diff: Value = serde_json::from_str(GLOBAL_MEMORY_DIFF_TOOL).expect("static json");
                let global_memory_revert: Value = serde_json::from_str(GLOBAL_MEMORY_REVERT_TOOL).expect("static json");
                let preset_list: Value = serde_json::from_str(PRESET_LIST_TOOL).expect("static json");
                let preset_get: Value = serde_json::from_str(PRESET_GET_TOOL).expect("static json");
                let identity_accounts: Value =
                    serde_json::from_str(IDENTITY_ACCOUNTS_TOOL).expect("static json");
                let identity_validate: Value =
                    serde_json::from_str(IDENTITY_VALIDATE_TOOL).expect("static json");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": [shell, shell_stop, shell_input, shell_status, pty_shell, pty_shell_input, pty_shell_resize, pty_shell_read, pty_shell_status, pty_shell_stop, open_editor, open_media, send_message, discover_agents, get_agent_transcript, list_conversations, search_history, supervisor_nudge, whoami, layout, set_name, set_active_tab, new_tab, focus_window, ui_screenshot, ui_click, ui_query, close_pane, quit_self, register_dev_server, browser_navigate, browser_back, browser_forward, browser_reload, browser_eval, browser_dispatch_key, browser_focus_element, browser_focus_info, capture_window, discover_windows, fleet_list, fleet_broadcast, fleet_bulk_stop, open_agent, loop_tool, loop_stop, loop_list, cron_create, cron_delete, cron_list, cron_pause, cron_resume, work_enqueue, work_claim, work_heartbeat, work_complete, work_release, work_list, memory_list, memory_read, memory_write, memory_history, memory_diff, memory_revert, global_memory_list, global_memory_read, global_memory_write, global_memory_remove, global_memory_history, global_memory_diff, global_memory_revert, preset_list, preset_get, identity_accounts, identity_validate] }
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
            let cwd = arguments.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
            let env = arguments
                .get("env")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            let url = format!(
                "{}/api/v1/shell/create",
                local_url.trim_end_matches('/')
            );
            let capture_stdin = arguments.get("capture_stdin").and_then(|v| v.as_bool());
            let req = ShellCreateRequest {
                agent_block_id: block_id.to_string(),
                cmd: cmd.to_string(),
                title: Some(title.to_string()),
                cwd,
                env,
                capture_stdin,
            };

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/create failed: HTTP {status} — {text}");
            }

            let result: ShellCreateResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(result.shell_id)
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
                .json(&ShellStopRequest { shell_id: shell_id.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/stop failed: HTTP {status} — {text}");
            }

            let result: ShellStopResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.stopped {
                format!("stopped shell {shell_id}")
            } else {
                format!("shell {shell_id} was not running (unknown or already exited)")
            })
        }
        "ShellInput" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let text = arguments
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: text"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/shell/input", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&ShellInputRequest { shell_id: shell_id.to_string(), text: text.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/input failed: HTTP {status} — {body}");
            }

            let result: ShellInputResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.written {
                format!("wrote to shell {shell_id}")
            } else {
                match result.reason {
                    Some(ShellInputFailure::StdinNotCaptured) => format!(
                        "shell {shell_id} is running but was started without capture_stdin=true — \
                         its stdin is /dev/null. Recreate the shell with Shell(..., capture_stdin=true) \
                         to send input."
                    ),
                    Some(ShellInputFailure::WriteFailed) => format!(
                        "shell {shell_id} closed its stdin — input discarded"
                    ),
                    Some(ShellInputFailure::NotRunning) | None => format!(
                        "shell {shell_id} is not running — input discarded"
                    ),
                }
            })
        }
        "ShellStatus" => {
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

            let url = format!("{}/api/v1/shell/status", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&ShellStatusRequest { shell_id: shell_id.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/status failed: HTTP {status} — {body}");
            }

            let result: ShellStatusResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.running {
                format!("shell {shell_id} is running ({} lines so far)", result.line_count)
            } else {
                let code = result.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
                format!("shell {shell_id} has exited — exit_code: {code}, {} lines total", result.line_count)
            })
        }
        "PtyShell" => {
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

            let cwd = arguments.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
            let rows = arguments.get("rows").and_then(|v| v.as_u64()).map(|n| n as u16);
            let cols = arguments.get("cols").and_then(|v| v.as_u64()).map(|n| n as u16);

            let url = format!("{}/api/v1/ptyshell/create", local_url.trim_end_matches('/'));
            let req = PtyShellCreateRequest { agent_block_id: block_id.to_string(), cwd, rows, cols };
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/create failed: HTTP {status} — {text}");
            }

            let result: PtyShellCreateResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(result.shell_id)
        }
        "PtyShellInput" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let text = arguments
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: text"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/input", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&PtyShellInputRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                    text: text.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/input failed: HTTP {status} — {body}");
            }

            let result: PtyShellInputResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.written {
                format!("wrote to shell {shell_id}")
            } else {
                format!(
                    "shell {shell_id}: write failed — {}",
                    result.error.unwrap_or_else(|| "unknown reason".to_string())
                )
            })
        }
        "PtyShellResize" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let rows = arguments
                .get("rows")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: rows"))? as u16;
            let cols = arguments
                .get("cols")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: cols"))? as u16;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/resize", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&PtyShellResizeRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                    rows,
                    cols,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/resize failed: HTTP {status} — {body}");
            }

            let result: PtyShellResizeResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.resized {
                format!("resized shell {shell_id} to {rows}x{cols}")
            } else {
                format!(
                    "shell {shell_id}: resize failed — {}",
                    result.error.unwrap_or_else(|| "unknown reason".to_string())
                )
            })
        }
        "PtyShellRead" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let tail_lines = arguments.get("tail_lines").and_then(|v| v.as_u64()).map(|n| n as u32);

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/read", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&PtyShellReadRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                    tail_lines,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/read failed: HTTP {status} — {body}");
            }

            let result: PtyShellReadResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let prefix = if result.truncated { "[...output truncated...]\n" } else { "" };
            Ok(format!("{prefix}{}", result.content))
        }
        "PtyShellStatus" => {
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

            let url = format!("{}/api/v1/ptyshell/status", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&PtyShellStatusRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/status failed: HTTP {status} — {body}");
            }

            let result: PtyShellStatusResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.running {
                format!("shell {shell_id} is running")
            } else {
                let code = result.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
                format!("shell {shell_id} is not running — exit_code: {code}")
            })
        }
        "PtyShellStop" => {
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

            let url = format!("{}/api/v1/ptyshell/stop", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&PtyShellStopRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/stop failed: HTTP {status} — {body}");
            }

            let result: PtyShellStopResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.released {
                format!("released the agent lock on shell {shell_id} (the shell itself keeps running)")
            } else {
                format!("shell {shell_id}: no active agent lock to release (unrecognized id, not yours, or already unlocked)")
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
            // Place the editor relative to the calling agent pane when we know
            // its block id (AGENTMUX_BLOCKID); otherwise the sidecar inserts it
            // at the tab root.
            let (split_direction, split_reference_block_id) = if block_id.is_empty() {
                (None, None)
            } else {
                (Some(split.to_string()), Some(block_id.to_string()))
            };
            let req = PaneOpenRequest {
                view: "editor".to_string(),
                file: Some(file.to_string()),
                focus: Some(true),
                split_direction,
                split_reference_block_id,
                title: arguments.get("title").and_then(|v| v.as_str()).map(str::to_string),
                // `collapse_tree: true` → open with the file-tree sidebar collapsed.
                // Maps to the editor's `tree_expanded` meta (collapsed == not expanded).
                tree_expanded: if arguments.get("collapse_tree").and_then(|v| v.as_bool()) == Some(true) {
                    Some(false)
                } else {
                    None
                },
                // `floating: true` → open in a floating window instead of a docked split.
                floating: if arguments.get("floating").and_then(|v| v.as_bool()) == Some(true) {
                    Some(true)
                } else {
                    None
                },
                url: None,
                cwd: None,
                tab_id: None,
                // Reuse an already-open Editor pane in this agent's own tab
                // instead of always spawning a new one — the explicit opt-in
                // only OpenEditor sets (see reuse_editor_pane's doc comment on
                // PaneOpenRequest for why this can't be inferred from
                // split_reference_block_id alone).
                reuse_editor_pane: Some(true),
            };

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("pane.open failed: HTTP {status} — {text}");
            }

            let result: PaneOpenResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(format!("Opened {file} in editor pane (block {})", result.block_id))
        }
        "OpenMedia" => {
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
            // Place the media pane relative to the calling agent pane when we
            // know its block id (AGENTMUX_BLOCKID); otherwise the sidecar
            // inserts it at the tab root.
            let (split_direction, split_reference_block_id) = if block_id.is_empty() {
                (None, None)
            } else {
                (Some(split.to_string()), Some(block_id.to_string()))
            };
            let req = PaneOpenRequest {
                view: "media".to_string(),
                file: Some(file.to_string()),
                focus: Some(true),
                split_direction,
                split_reference_block_id,
                title: arguments.get("title").and_then(|v| v.as_str()).map(str::to_string),
                // The Media pane has no file-tree sidebar, unlike Editor.
                tree_expanded: None,
                // `floating: true` → open in a floating window instead of a docked split.
                floating: if arguments.get("floating").and_then(|v| v.as_bool()) == Some(true) {
                    Some(true)
                } else {
                    None
                },
                url: None,
                cwd: None,
                tab_id: None,
                reuse_editor_pane: None, // view != "editor" — irrelevant here
            };

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("pane.open failed: HTTP {status} — {text}");
            }

            let result: PaneOpenResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(format!("Opened {file} in media pane (block {})", result.block_id))
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
            let req = sign_outgoing_jekt(source_agent.as_deref(), to, message)
                .into_request(to.to_string(), message.to_string(), source_agent);

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&req)
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
                // `success: true` spans two very different outcomes and the old
                // message conflated them, which made agent-to-agent delivery
                // unfalsifiable from the sender side: a name that exists on no
                // machine anywhere reported exactly the same string as a
                // message injected into a live conversation. Verified against a
                // running srv — `SendMessage(to="definitely-not-a-real-agent-xyz123")`
                // returned "Message sent to ...", and the server log showed
                // "cloud relay: queued for WAN delivery" for it, identical to a
                // real remote agent.
                //
                // The response already carries the distinction: the handler sets
                // `block_id` to the receiving block on local/host delivery
                // (backend/reactive/handler.rs), while the cloud-relay path
                // leaves it `None` because no receiver has seen the message yet
                // (server/reactive.rs, `try_cloud_relay` — "Queued is not
                // delivered"). Report which one happened.
                if result.get("block_id").and_then(|v| v.as_str()).is_some() {
                    // srv queues a message while the target's process is
                    // starting up, restarting or stopping
                    // (SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md §2.1) and
                    // says so with `deferred`; "injected" would be untrue.
                    if result.get("deferred").and_then(|v| v.as_bool()) == Some(true) {
                        Ok(deferred_delivery_text(&to))
                    } else {
                        Ok(format!("Delivered to {to} — injected into their conversation."))
                    }
                } else {
                    Ok(format!(
                        "QUEUED for {to} via the cloud relay — NOT yet delivered. \
                         The relay accepted it; their AgentMux picks it up on its next \
                         sync, which never happens if that instance is offline. You get \
                         this same result for an agent name that does not exist anywhere, \
                         so check the spelling against DiscoverAgents if you expected \
                         local delivery."
                    ))
                }
            } else if result.get("held").and_then(|v| v.as_bool()) == Some(true) {
                // SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md: the target is a
                // known agent that is not running anywhere srv can reach, so
                // srv kept the message and delivers it when the agent starts.
                Ok(format!(
                    "HELD for {to} — not delivered yet. {to} is not running; this AgentMux \
                     instance (channel) keeps the message and delivers it when {to} starts \
                     here, for up to 24 hours. Do not resend it."
                ))
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
        "FleetList" => {
            // Thin fleet-framed alias of DiscoverAgents — identical call,
            // see this tool's own doc comment for why a separate tool
            // exists despite reusing the exact same endpoint.
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
        "FleetBroadcast" => {
            let targets = arguments
                .get("targets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: targets"))?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            if targets.is_empty() {
                anyhow::bail!("targets must be a non-empty array of block_id (host/cross-channel) or agent name (LAN/WAN) strings");
            }
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

            // `/agentmux/reactive/inject`'s `target_agent` resolves by
            // registered AGENT NAME only (`agent_to_block`,
            // crates/srv/src/backend/reactive/handler.rs) — never by
            // block_id, even though this tool's own advertised contract
            // (FLEET_BROADCAST_TOOL) is block_id values from FleetList, to
            // stay consistent with FleetBulkStop's targeting scheme. The
            // WS-RPC path (`fleet_broadcast_impl`, agentmux-srv) already
            // resolves block_id -> agent_id via `get_agent_by_block` before
            // injecting; this MCP path talks to srv over plain HTTP with no
            // access to that in-process registry, so it resolves the same
            // way DiscoverAgents/FleetList already do: read
            // `/agentmux/discovery`'s `host.addressable` (each entry already
            // carries both `agent_id` and a live `block_id`) and map through
            // it before signing/injecting (Codex P1 + reagent P0, PR #2687
            // review — every advertised call failed "agent not found"
            // without this).
            //
            // REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md:
            // `host.addressable` alone missed `host.cross_channel` entries
            // (a different channel on this SAME host — those genuinely have
            // a `block_id` in this host's namespace, discovery just wasn't
            // being read for it) and LAN/WAN entries (which have NO
            // `block_id` at all — only an agent name, since they're not
            // local blocks). Both are folded in below: `cross_channel`
            // extends the same block_id->name map (identical shape), and
            // any target string that doesn't match ANY known block_id falls
            // through to being used AS a literal agent name — safe because
            // `/agentmux/reactive/inject`'s own cross-tier cascade
            // (cross-channel -> LAN -> WAN muxbus relay,
            // `server/reactive.rs`) already resolves by name across every
            // tier; an invalid name just fails the same "not found" way it
            // always did.
            let discovery_url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let discovery_resp = client
                .get(&discovery_url)
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("discovery request failed: {e}"))?;
            if !discovery_resp.status().is_success() {
                let status = discovery_resp.status();
                let text = discovery_resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }
            let discovery: Value = discovery_resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("discovery response parse failed: {e}"))?;
            let block_to_agent = build_block_to_agent_map(&discovery);

            // Same signed single-target delivery SendMessage uses, looped
            // once per target — only this process holds AGENTMUX_JEKT_KEY,
            // so per-message signing can only happen here (see this
            // tool's own doc comment). Never a single aggregate result:
            // per-target success/failure is collected below regardless of
            // how many targets fail. Sent in chunks with a pause between
            // them — `/agentmux/reactive/inject` shares ReactiveHandler's
            // global rate limiter (10/sec, hard reset per second, not a
            // smooth refill — crates/srv/src/backend/reactive/mod.rs's
            // RATE_LIMIT_MAX), and a tight loop past ~10 targets would
            // otherwise deterministically fail the tail of any larger
            // broadcast with "rate limit exceeded" (Codex P1, same review).
            // Mirrors `fleet_broadcast_impl`'s own chunking constants —
            // kept in sync by hand since this is a separate process/crate
            // with no dependency on agentmux-srv internals.
            const BROADCAST_CHUNK_SIZE: usize = 10;
            const BROADCAST_CHUNK_PAUSE: std::time::Duration = std::time::Duration::from_millis(1100);

            let url = format!("{}/agentmux/reactive/inject", local_url.trim_end_matches('/'));
            let mut succeeded: Vec<String> = Vec::new();
            let mut failed: Vec<serde_json::Value> = Vec::new();
            for (chunk_idx, chunk) in targets.chunks(BROADCAST_CHUNK_SIZE).enumerate() {
                if chunk_idx > 0 {
                    tokio::time::sleep(BROADCAST_CHUNK_PAUSE).await;
                }
                for target in chunk {
                    let target = target.clone();
                    // LAN/WAN discovery entries carry no block_id at all
                    // (they're not local blocks) — a target that doesn't
                    // match any known block_id is used AS the agent name
                    // directly, letting `/agentmux/reactive/inject`'s own
                    // cross-tier cascade attempt it. This can never make a
                    // genuinely-wrong block_id succeed silently: it still
                    // fails, just via the inject endpoint's own "agent not
                    // found" rather than this pre-check.
                    let target_agent = block_to_agent.get(&target).cloned().unwrap_or_else(|| target.clone());
                    let req = sign_outgoing_jekt(source_agent.as_deref(), &target_agent, message)
                        .into_request(target_agent, message.to_string(), source_agent.clone());
                    let outcome = async {
                        let resp = client
                            .post(&url)
                            .header("X-AuthKey", auth_key)
                            .json(&req)
                            .send()
                            .await
                            .map_err(|e| format!("request failed: {e}"))?;
                        if !resp.status().is_success() {
                            let status = resp.status();
                            let text = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {status} — {text}"));
                        }
                        let result: Value = resp
                            .json()
                            .await
                            .map_err(|e| format!("response parse failed: {e}"))?;
                        if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
                            Ok(())
                        } else {
                            Err(result
                                .get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown error")
                                .to_string())
                        }
                    }
                    .await;
                    match outcome {
                        Ok(()) => succeeded.push(target),
                        Err(error) => failed.push(json!({ "id": target, "error": error })),
                    }
                }
            }

            let result = json!({ "succeeded": succeeded, "failed": failed });
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "OpenAgent" => {
            let agent_id = arguments
                .get("agent_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent_id"))?;
            let tab_id = arguments.get("tab_id").and_then(|v| v.as_str()).map(str::to_string);
            let focus = arguments.get("focus").and_then(|v| v.as_bool());

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/agent/open", local_url.trim_end_matches('/'));
            let body = json!({
                "agent_id": agent_id,
                "tab_id": tab_id,
                "focus": focus,
            });
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
                anyhow::bail!("agent open failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "FleetBulkStop" => {
            let targets = arguments
                .get("targets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: targets"))?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            if targets.is_empty() {
                anyhow::bail!("targets must be a non-empty array of block_id strings");
            }
            let signal = arguments.get("signal").and_then(|v| v.as_str()).map(str::to_string);
            let staged = arguments.get("staged").cloned();

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/fleet/bulk-stop", local_url.trim_end_matches('/'));
            let mut body = json!({ "targets": targets, "signal": signal });
            if let Some(staged) = staged {
                body["staged"] = staged;
            }
            // Names this agent on the user's banner and in the audit log.
            if let Ok(auth) = sign_ui_automation_auth() {
                body["auth"] = serde_json::to_value(auth).unwrap_or(Value::Null);
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
                anyhow::bail!("fleet bulk-stop failed: HTTP {status} — {text}");
            }

            let mut result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            if result["status"] == "pending_user_override" {
                // Each running target's user had 15 s to keep it (§6.5). srv
                // runs the windows in parallel, so waiting on them one after
                // another costs no extra time.
                let pending = result["pending"].as_array().cloned().unwrap_or_default();
                let mut outcomes = Vec::with_capacity(pending.len());
                for p in &pending {
                    let request_id = p["request_id"].as_str().unwrap_or_default();
                    let now = await_shutdown(client, local_url, auth_key, request_id, true).await;
                    outcomes.push((p["block_id"].as_str().unwrap_or_default().to_string(), now));
                }
                result = merge_bulk_stop_outcomes(result, outcomes);
            }

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "SearchHistory" => {
            // Whose history this is, srv takes from this process's
            // `X-Agent-Token` (sent on every request, see `main`), never from a
            // name; the tool exposes no `agent` parameter on purpose — see
            // SEARCH_HISTORY_TOOL's comment. The name is sent only for srv's
            // actor counters, so a process without one still searches.
            let slug = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let tool = arguments.get("tool").and_then(|v| v.as_str());
            if query.trim().is_empty() && tool.is_none() {
                anyhow::bail!(
                    "query must be non-empty unless `tool` is given (an empty query with no \
                     tool filter would match everything and return a truncated firehose)"
                );
            }

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!(
                "{}/agentmux/reactive/history/search",
                local_url.trim_end_matches('/')
            );
            let mut query_params: Vec<(&str, String)> = vec![("query", query)];
            if !slug.trim().is_empty() {
                query_params.push(("agent", slug));
            }
            if let Some(t) = tool {
                query_params.push(("tool", t.to_string()));
            }
            if let Some(r) = arguments.get("role").and_then(|v| v.as_str()) {
                query_params.push(("role", r.to_string()));
            }
            for (key, arg) in [
                ("since", "since"),
                ("until", "until"),
                ("max_sessions", "max_sessions"),
                ("limit", "limit"),
            ] {
                if let Some(n) = arguments.get(arg).and_then(|v| v.as_i64()) {
                    query_params.push((key, n.to_string()));
                }
            }
            if arguments.get("include_inferred").and_then(|v| v.as_bool()) == Some(true) {
                query_params.push(("include_inferred", "true".to_string()));
            }

            let body = srv_get_json(client, &url, auth_key, &query_params, "history search").await?;
            Ok(serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()))
        }
        "GetAgentTranscript" => {
            let agent = arguments
                .get("agent")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent"))?;
            let max_lines = arguments.get("max_lines").and_then(|v| v.as_u64());

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!(
                "{}/agentmux/reactive/transcript",
                local_url.trim_end_matches('/')
            );
            let mut query: Vec<(&str, String)> = vec![("agent", agent.to_string())];
            if let Some(n) = max_lines {
                query.push(("max_lines", n.to_string()));
            }

            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&query)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("transcript fetch failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "ListConversations" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!(
                "{}/api/v1/muxspect/conversations",
                local_url.trim_end_matches('/')
            );
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("conversations fetch failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "SupervisorNudge" => {
            let target_agent = arguments
                .get("target_agent")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: target_agent"))?;
            let action = arguments
                .get("action")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;
            if action != "nudge" && action != "decline" {
                anyhow::bail!("invalid action: {action} (expected \"nudge\" or \"decline\")");
            }
            let reason = arguments.get("reason").and_then(|v| v.as_str());

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
                "{}/agentmux/reactive/supervisor-decision",
                local_url.trim_end_matches('/')
            );
            let body = json!({
                "target_agent": target_agent,
                "action": action,
                "reason": reason,
                "source_agent": source_agent,
            });

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            let status = resp.status();
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            if !status.is_success() {
                let err = result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                anyhow::bail!("supervisor decision rejected: HTTP {status} — {err}");
            }

            // The route always returns HTTP 200 for a request that passed
            // the entitlement/ceiling gates — actual delivery success is
            // only reflected in the response body's `success` field (a
            // failed nudge still gets logged and returned as 200/success:
            // false, same as SendMessage's InjectionResponse). Checking
            // HTTP status alone previously reported a failed delivery to
            // the calling Supervisor as success (reagentx P2 on PR #2557).
            if result.get("success").and_then(|v| v.as_bool()) != Some(true) {
                let err = result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                anyhow::bail!("supervisor decision delivery failed: {err}");
            }

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
            let target_id = arguments
                .get("target_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            // block_id is only needed for own-context resolution; when
            // target_id is explicit we can reach any element without it.
            if target_id.is_none() && block_id.is_empty() {
                anyhow::bail!(
                    "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
                     — pass target_id to name a specific element, or open this \
                     agent in an AgentMux pane to default to your own."
                );
            }

            // window/tab/workspace POST {"name": …} to /…/name; pane POSTs
            // {"title": …} to /pane/title. `label` and `resp_field` are the
            // human-facing echo and the response key used to surface the
            // server-applied value (e.g. a window name clamped to 64 chars).
            let own_block = if block_id.is_empty() { None } else { Some(block_id.to_string()) };
            let (path, label, resp_field, body) = match target {
                "window" => ("window/name", "Window name", "name", serde_json::to_value(WindowNameRequest {
                    block_id: own_block,
                    name: new_name.to_string(),
                    window_id: target_id,
                })?),
                "tab" => ("tab/name", "Tab name", "name", serde_json::to_value(TabNameRequest {
                    block_id: own_block,
                    tab_id: target_id,
                    name: new_name.to_string(),
                })?),
                "workspace" => ("workspace/name", "Workspace name", "name", serde_json::to_value(WorkspaceNameRequest {
                    block_id: own_block,
                    workspace_id: target_id,
                    name: new_name.to_string(),
                })?),
                "pane" => ("pane/title", "Pane title", "title", serde_json::to_value(PaneTitleRequest {
                    block_id: target_id.or(own_block),
                    title: new_name.to_string(),
                })?),
                other => anyhow::bail!(
                    "invalid target '{other}' — expected one of: window, tab, pane, workspace"
                ),
            };
            let url = format!("{}/api/v1/{path}", local_url.trim_end_matches('/'));
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
            let applied = resp
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v.get(resp_field).and_then(|s| s.as_str()).map(str::to_string))
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
                .json(&TabActivateRequest { tab_id: tab_id.to_string() })
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
                .json(&TabNewRequest {
                    block_id: Some(block_id.to_string()),
                    workspace_id: None,
                    name: if name.is_empty() { None } else { Some(name.to_string()) },
                })
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
                .json(&WindowFocusRequest {
                    block_id: Some(block_id.to_string()),
                    window_id: if window_id.is_empty() { None } else { Some(window_id.to_string()) },
                })
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
        "UIScreenshot" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let url = format!("{}/api/v1/ui/screenshot", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&UiScreenshotRequest { auth })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("screenshot failed: HTTP {status} — {text}");
            }
            let result: UiScreenshotResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(format!(
                "Screenshot saved to {} — use Read on that path to view it yourself, or OpenMedia to show it to the user.",
                result.path
            ))
        }
        "CaptureWindow" => {
            // `index` stays an Option all the way into capture_window_impl —
            // NOT defaulted to 0 here. Defaulting it here is exactly what
            // let an ambiguous match silently capture the wrong (and once,
            // a genuinely unrelated/sensitive) window with no warning —
            // see docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md
            // §1. Losing "the caller didn't specify an index at all" vs.
            // "the caller explicitly asked for index 0" was the bug.
            let title_contains = arguments
                .get("title_contains")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let index = arguments
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let pid = arguments
                .get("pid")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            let query_desc = match (pid, title_contains.as_deref()) {
                (Some(p), _) => format!("pid={p}"),
                (None, Some(t)) => format!("title_contains={t:?}"),
                (None, None) => "<no target given>".to_string(),
            };

            // Audit trail, not a capability gate — reagent P1 (PR #2709
            // round 4): a real per-agent opt-in gate is a bigger feature
            // (settings storage + enforcement, no existing mechanism
            // anywhere in the codebase to build on — confirmed by the
            // spec this tool's own doc comment already cites) than fits
            // reactively on this PR, and this tool's actual residual risk
            // after rounds 2-3's scoping is disclosure across a human
            // boundary (a different AgentMux instance's window can belong
            // to a different OS user on a shared machine), not a technical
            // one — logging every call (who, what was requested, what
            // happened) is the honest, shippable Phase-1 answer while the
            // real gate is tracked separately (operator-confirmed).
            let mut resolved: Option<(CaptureTier, String)> = None;
            let outcome = capture_window_impl(title_contains.as_deref(), index, pid, &mut resolved);
            audit_log_capture_window(&query_desc, &outcome, &resolved);
            return outcome.map(|c| c.message);
        }
        "DiscoverWindows" => {
            let include_self = arguments
                .get("include_self")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // Non-AgentMux windows are enumerated now (they're T4 capture
            // targets) but stay OUT of the default listing: ordinary discovery
            // shouldn't disclose the titles of a user's unrelated applications
            // — their browser tabs, their password manager — as a side effect
            // of looking for AgentMux windows.
            let include_foreign = arguments
                .get("include_foreign")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let windows = enumerate_agentmux_windows()?;
            let list: Vec<Value> = windows
                .iter()
                // These two filters decide WHICH windows are listed. What each
                // listed entry may say about itself — in particular that a
                // withheld window is redacted rather than omitted — belongs to
                // `window_listing_entry` below, which documents that rationale
                // rather than repeating it here (reagentx P2 on PR #2845: an
                // earlier version of this comment still described a
                // `tier.allowed()` filter that has since been replaced, and
                // contradicted the code under it).
                .filter(|w| include_self || !w.is_self)
                .filter(|w| include_foreign || w.is_agentmux)
                .map(window_listing_entry)
                .collect();
            // reagent P1 on PR #2810: exe_path (embeds the OS username) for
            // OTHER instances/users on a shared machine is the same
            // disclosure-across-a-human-boundary risk CaptureWindow already
            // logs — this tool must too, not just the tool that follows it.
            audit_log_discover_windows(include_self, include_foreign, &list);
            return Ok(serde_json::to_string_pretty(&json!({ "windows": list }))
                .unwrap_or_else(|_| "{\"windows\":[]}".to_string()));
        }
        "UIClick" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: selector"))?;
            let url = format!("{}/api/v1/ui/click", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&UiClickRequest {
                    auth,
                    selector: selector.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("click failed: HTTP {status} — {text}");
            }
            Ok(format!("Clicked {selector:?}"))
        }
        "UIQuery" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: selector"))?;
            let limit = arguments.get("limit").and_then(|v| v.as_u64()).map(|n| n as u32);
            let url = format!("{}/api/v1/ui/query", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&UiQueryRequest {
                    auth,
                    selector: selector.to_string(),
                    limit,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("query failed: HTTP {status} — {text}");
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let matches = body
                .get("data")
                .and_then(|d| d.get("matches"))
                .cloned()
                .unwrap_or_else(|| json!([]));
            Ok(serde_json::to_string_pretty(&matches).unwrap_or_else(|_| matches.to_string()))
        }
        "BrowserNavigate" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let url = arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: url"))?;
            let req_url = format!("{}/api/v1/ui/browser/navigate", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserNavigateRequest { auth, url: url.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("navigate failed: HTTP {status} — {text}");
            }
            Ok(format!("Navigated to {url:?}"))
        }
        "BrowserBack" | "BrowserForward" | "BrowserReload" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let ignore_cache = arguments.get("ignore_cache").and_then(|v| v.as_bool());
            let route = match name {
                "BrowserBack" => "back",
                "BrowserForward" => "forward",
                _ => "reload",
            };
            let req_url = format!("{}/api/v1/ui/browser/{route}", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserHistoryRequest { auth, ignore_cache })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("{route} failed: HTTP {status} — {text}");
            }
            Ok(format!("Browser {route} ok"))
        }
        "BrowserEval" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let script = arguments
                .get("script")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: script"))?;
            let await_promise = arguments.get("await_promise").and_then(|v| v.as_bool());
            let req_url = format!("{}/api/v1/ui/browser/eval", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserEvalRequest {
                    auth,
                    script: script.to_string(),
                    await_promise,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("eval failed: HTTP {status} — {text}");
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let data = body.get("data").cloned().unwrap_or(json!({}));
            Ok(serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string()))
        }
        "BrowserDispatchKey" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments.get("selector").and_then(|v| v.as_str()).map(str::to_string);
            let text = arguments.get("text").and_then(|v| v.as_str()).map(str::to_string);
            let key = arguments.get("key").and_then(|v| v.as_str()).map(str::to_string);
            if text.is_some() == key.is_some() {
                anyhow::bail!("BrowserDispatchKey requires exactly one of `text` or `key`");
            }
            let req_url = format!("{}/api/v1/ui/browser/dispatch_key", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserDispatchKeyRequest { auth, selector, text, key })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("dispatch_key failed: HTTP {status} — {text}");
            }
            Ok("Dispatched".to_string())
        }
        "BrowserFocusElement" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: selector"))?;
            let req_url = format!("{}/api/v1/ui/browser/focus_element", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserFocusElementRequest { auth, selector: selector.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("focus_element failed: HTTP {status} — {text}");
            }
            Ok(format!("Focused {selector:?}"))
        }
        "BrowserFocusInfo" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let req_url = format!("{}/api/v1/ui/browser/focus_info", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserFocusInfoRequest { auth })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("focus_info failed: HTTP {status} — {text}");
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let focused = body
                .get("data")
                .and_then(|d| d.get("focused"))
                .cloned()
                .unwrap_or(Value::Null);
            Ok(serde_json::to_string_pretty(&focused).unwrap_or_else(|_| focused.to_string()))
        }
        "ClosePane" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let target_block_id = arguments.get("block_id").and_then(|v| v.as_str()).map(str::to_string);
            let reason = arguments.get("reason").and_then(|v| v.as_str()).map(str::to_string);
            let url = format!("{}/api/v1/agent/pane/close", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&ClosePaneRequest { auth, block_id: target_block_id.clone(), reason })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("close failed: HTTP {status} — {text}");
            }
            if resp.status().as_u16() == 202 {
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                let request_id = body["request_id"].as_str().unwrap_or_default();
                return match target_block_id.as_deref() {
                    // Another agent's pane: its user had 15 s to keep it (§6.5).
                    Some(b) => {
                        let now = await_shutdown(client, local_url, auth_key, request_id, true).await;
                        close_pane_outcome(b, &now)
                    }
                    // Yourself (§7): as QuitSelf, done once it's proceeding —
                    // the quit waits for this very turn to end.
                    None => {
                        let now = await_shutdown(client, local_url, auth_key, request_id, false).await;
                        match now["status"].as_str().unwrap_or("") {
                            "" | "pending" => anyhow::bail!("ClosePane: no answer on the pending shutdown {request_id}"),
                            state => quit_self_outcome(state, &now),
                        }
                    }
                };
            }
            match target_block_id {
                Some(b) => Ok(format!("Closed pane {b:?}")),
                // srv's only 200 for the no-argument form (§7): a quit already
                // scheduled or under way.
                None => quit_self_result(200, &Value::Null),
            }
        }
        "QuitSelf" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let field = |k: &str| {
                arguments
                    .get(k)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: {k}"))
            };
            let (reason, user_instruction) = (field("reason")?, field("user_instruction")?);
            let auth = sign_ui_automation_auth()?;
            let url = format!("{}/api/v1/agent/self/quit", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&QuitSelfRequest { auth, reason, user_instruction })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if status.as_u16() != 202 || body["status"] != "pending_user_override" {
                return quit_self_result(status.as_u16(), &body);
            }
            // Not the user's own ask: the user has 15 s to keep this agent
            // (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5). Poll for the answer —
            // one long request would outlast the client's timeout.
            let request_id = body["request_id"].as_str().unwrap_or_default();
            // `proceeding` is final for QuitSelf: the shutdown waits for this
            // very turn to end, so waiting on here would hold it up.
            let now = await_shutdown(client, local_url, auth_key, request_id, false).await;
            match now["status"].as_str().unwrap_or("") {
                "" | "pending" => anyhow::bail!("QuitSelf: no answer on the pending shutdown {request_id}"),
                state => quit_self_outcome(state, &now),
            }
        }
        "RegisterDevServer" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let project = arguments
                .get("project")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: project"))?
                .to_string();
            let port = arguments
                .get("port")
                .and_then(|v| v.as_u64())
                .filter(|p| *p > 0 && *p <= u16::MAX as u64)
                .ok_or_else(|| anyhow::anyhow!("missing or invalid required parameter: port (must be 1-65535)"))?
                as u16;
            let url = format!(
                "{}/api/v1/agent/dev_server/register",
                local_url.trim_end_matches('/')
            );
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&RegisterDevServerRequest { auth, project, port })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("RegisterDevServer failed: HTTP {status} — {text}");
            }
            let body: RegisterDevServerResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(format!("Registered. Browse to: {}", body.url))
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
            let max_iterations: Option<u64> = arguments
                .get("max_iterations")
                .and_then(|v| v.as_u64())
                .filter(|&n| n > 0);

            let n = loop_counter.fetch_add(1, Ordering::Relaxed) + 1;
            let loop_id = format!("loop-{n}");

            let interval_display = format_duration(interval);
            let url = format!("{}/agentmux/reactive/inject", local_url.trim_end_matches('/'));
            let task_client = client.clone();
            let task_auth = auth_key.to_string();
            let task_target = target.clone();
            let task_source = self_id;
            let task_prompt = prompt.clone();
            let fire_count = Arc::new(AtomicU64::new(0));
            let task_fire_count = Arc::clone(&fire_count);
            let task_max = max_iterations;

            let handle = tokio::spawn(async move {
                if !immediate {
                    tokio::time::sleep(interval).await;
                }
                loop {
                    let req = sign_outgoing_jekt(task_source.as_deref(), &task_target, &task_prompt)
                        .into_request(task_target.clone(), task_prompt.clone(), task_source.clone());
                    let _ = task_client
                        .post(&url)
                        .header("X-AuthKey", &task_auth)
                        .json(&req)
                        .send()
                        .await;
                    let fired = task_fire_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if let Some(max) = task_max {
                        if fired >= max {
                            break;
                        }
                    }
                    tokio::time::sleep(interval).await;
                }
            });

            let started_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            loops.lock().unwrap().insert(loop_id.clone(), LoopEntry {
                handle,
                prompt,
                target: target.clone(),
                interval_secs: interval.as_secs(),
                max_iterations,
                fire_count,
                started_at,
            });

            let cap_note = match max_iterations {
                Some(n) => format!(", auto-stops after {n} fires"),
                None => String::new(),
            };
            Ok(format!(
                "Started {loop_id}: injecting to '{target}' every {interval_display}{cap_note}\
                 {}. Stop with LoopStop({loop_id}) or use LoopList() to see all running loops.",
                if immediate { ", first run now" } else { "" }
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
                Some(entry) => {
                    entry.handle.abort();
                    Ok(format!(
                        "stopped {loop_id} (fired {} time(s))",
                        entry.fire_count.load(Ordering::Relaxed)
                    ))
                }
                None => Ok(format!("{loop_id} was not running (unknown or already stopped)")),
            }
        }
        "LoopList" => {
            let reg = loops.lock().unwrap();
            if reg.is_empty() {
                return Ok("No loops running in this session.".to_string());
            }
            let mut lines = vec![format!("{} loop(s) in this session:", reg.len())];
            for (id, entry) in reg.iter() {
                let fired = entry.fire_count.load(Ordering::Relaxed);
                let status = match entry.max_iterations {
                    Some(max) if fired >= max => format!("DONE ({fired}/{max})"),
                    Some(max) => format!("running ({fired}/{max})"),
                    None => format!("running ({fired} fired, unlimited)"),
                };
                let interval = format_duration(Duration::from_secs(entry.interval_secs));
                let prompt_preview: String = entry.prompt.chars().take(60).collect();
                let age_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
                    .saturating_sub(entry.started_at);
                lines.push(format!(
                    "  {id}  every={interval}  to='{}'  status={status}  age={age_secs}s  prompt='{prompt_preview}'",
                    entry.target,
                ));
            }
            Ok(lines.join("\n"))
        }
        // ── Muxqueue ────────────────────────────────────────────────────
        // All six post/get to /agentmux/work* on the local srv, same auth
        // header as the cron arms below.
        "WorkEnqueue" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let title = arguments.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: title"))?;
            let payload = arguments.get("payload").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: payload"))?;
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty()).unwrap_or_default();
            let target_agent = arguments.get("target_agent").and_then(|v| v.as_str()).unwrap_or("");
            // Identity M3: resolve the typed target NOW, at the boundary, and
            // carry its uid; an ambiguous name is handed back with candidates.
            let target_agent_uid = if target_agent.is_empty() {
                None
            } else {
                resolve_agent_name_at_boundary(&client, local_url, auth_key, target_agent).await?
            };

            let url = format!("{}/agentmux/work", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "title": title,
                "payload": payload,
                "kind": arguments.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                "target_agent": target_agent,
                "target_agent_uid": target_agent_uid.unwrap_or_default(),
                "target_group": arguments.get("target_group").and_then(|v| v.as_str()).unwrap_or(""),
                "priority": arguments.get("priority").and_then(|v| v.as_i64()).unwrap_or(0),
                "not_before": arguments.get("not_before").and_then(|v| v.as_i64()),
                "max_attempts": arguments.get("max_attempts").and_then(|v| v.as_i64()).filter(|&n| n > 0),
                "created_by": self_id,
            });
            let resp = client.post(&url).header("X-AuthKey", auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("work enqueue request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("WorkEnqueue failed: HTTP {status} — {text}");
            }
            let id = serde_json::from_str::<Value>(&text).ok()
                .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            Ok(format!("Enqueued work item {id}: {title}"))
        }
        "WorkClaim" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("AGENTMUX_AGENT_ID is not set — cannot claim work without an agent identity"))?;

            // Identity M1b/M3: carry this agent's UID (AGENTMUX_AGENT_UID, set
            // by srv at spawn since M1a). It is the only way into a
            // UID-addressed item. Empty when absent (pre-M1a spawn,
            // continuation resume, quick-launch pane) — srv then takes the
            // UID from this block's row, and counts the fallback.
            let self_uid = std::env::var("AGENTMUX_AGENT_UID").ok().filter(|s| !s.is_empty()).unwrap_or_default();

            let url = format!("{}/agentmux/work/claim", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "agent_id": self_id,
                "agent_uid": self_uid,
                // Identity M3: when no UID is carried, srv takes it from the
                // row on this block rather than deriving one from the name.
                "block_id": block_id,
                "kind": arguments.get("kind").and_then(|v| v.as_str()),
                "lease_ms": arguments.get("lease_ms").and_then(|v| v.as_i64()).filter(|&n| n > 0),
                // Group membership is resolved server-side-of-this-call by the
                // caller in the general design; the MCP path has no cheap way
                // to know this agent's groups yet, so it claims only untargeted
                // and self-targeted work. Group-targeted claiming arrives with
                // the group lookup, not before — better to under-claim than to
                // silently ignore a group restriction.
                "groups": [],
            });
            let resp = client.post(&url).header("X-AuthKey", auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("work claim request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("WorkClaim failed: HTTP {status} — {text}");
            }
            let v: Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
            if !v.get("claimed").and_then(|x| x.as_bool()).unwrap_or(false) {
                return Ok("No work available on the queue right now.".to_string());
            }
            let attempt = v.get("attempt").and_then(|x| x.as_i64()).unwrap_or(0);
            let item = v.get("item").cloned().unwrap_or(serde_json::json!({}));
            let id = item.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("");
            let payload = item.get("payload").and_then(|x| x.as_str()).unwrap_or("");
            // Carry forward why a PREVIOUS holder handed this back (Codex P2 on
            // PR #2902). WorkRelease's description promises the next claimant
            // sees the reason; dropping it here broke that promise and let
            // agents re-hit a known blocker with no warning. `attempt > 1` is
            // exactly "someone has held this before me".
            let prior = item.get("result").and_then(|x| x.as_str()).unwrap_or("");
            let handback = if attempt > 1 && !prior.is_empty() {
                format!(
                    "\n\nNOTE — a previous agent held this and handed it back \
                     (attempt {} of this item). Their reason: {prior}\n\
                     Read that before repeating their approach.",
                    attempt - 1
                )
            } else {
                String::new()
            };
            Ok(format!(
                "Claimed work item {id} (attempt {attempt}) — pass attempt={attempt} to \
                 WorkHeartbeat/WorkComplete/WorkRelease for this item.\n\n\
                 Title: {title}\n\n{payload}{handback}"
            ))
        }
        "WorkHeartbeat" | "WorkComplete" | "WorkRelease" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("AGENTMUX_AGENT_ID is not set"))?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let attempt = arguments.get("attempt").and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: attempt (from the WorkClaim response)"))?;

            let (segment, result_text) = match name {
                "WorkHeartbeat" => ("heartbeat", String::new()),
                "WorkComplete" => (
                    "complete",
                    // Enforced at runtime as well as in the schema (Codex P2 on
                    // PR #2902): completing with no result marks the item done
                    // forever while destroying the only record that the work
                    // happened. Better to reject the call than to accept a
                    // silent hole in the audit trail.
                    arguments
                        .get("result")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "WorkComplete requires a non-empty 'result' — it is the only \
                                 record of what this item accomplished once it is marked done"
                            )
                        })?
                        .to_string(),
                ),
                _ => (
                    "release",
                    arguments.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                ),
            };

            let url = format!("{}/agentmux/work/{}/{}", local_url.trim_end_matches('/'), id, segment);
            let body = serde_json::json!({
                "agent_id": self_id,
                "attempt": attempt,
                "result": result_text,
                "lease_ms": arguments.get("lease_ms").and_then(|v| v.as_i64()).filter(|&n| n > 0),
            });
            let resp = client.post(&url).header("X-AuthKey", auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("work {segment} request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::CONFLICT {
                // The fence rejected this call. Say so in the terms the agent
                // can act on, rather than surfacing a raw 409 — the recovery is
                // always the same: claim again.
                anyhow::bail!(
                    "This claim is no longer yours — the lease expired and the item was \
                     reclaimed, or another agent holds it now. Call WorkClaim again if you \
                     still want work; do NOT retry with the old attempt number."
                );
            }
            if !status.is_success() {
                anyhow::bail!("{name} failed: HTTP {status} — {text}");
            }
            Ok(match segment {
                "heartbeat" => format!("Lease extended on {id}."),
                "complete" => format!("Completed {id}."),
                _ => {
                    // A release on the FINAL allowed attempt parks the item as
                    // `failed` rather than reopening it, so reporting "back to
                    // the queue" unconditionally would tell the caller another
                    // agent can pick it up when nobody ever will (Codex P2 on
                    // PR #2902). The server reports the resulting state; trust
                    // it rather than re-deriving the attempts rule here.
                    let resulting = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| v.get("state").and_then(|s| s.as_str()).map(str::to_string))
                        .unwrap_or_default();
                    if resulting == "failed" {
                        format!(
                            "Released {id}, and it has now used its final attempt — the item is \
                             parked as FAILED and will not be offered to any agent again. If it \
                             still needs doing, enqueue a fresh item (ideally with what you \
                             learned about why it kept failing)."
                        )
                    } else {
                        format!("Released {id} back to the queue for another agent.")
                    }
                }
            })
        }
        "WorkList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/agentmux/work", local_url.trim_end_matches('/'));
            let state = arguments.get("state").and_then(|v| v.as_str()).unwrap_or("");
            let limit = arguments.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            // Built via reqwest's own query serializer, NOT string
            // concatenation (reagent P2 on PR #2902): `state` is a free-form
            // string in this tool's schema, not a validated enum, so a value
            // containing `&`, `#`, or `%` would otherwise corrupt the query
            // rather than being sent as the literal the caller intended.
            let resp = client
                .get(&url)
                .query(&[("state", state), ("limit", &limit.to_string())])
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("work list request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("WorkList failed: HTTP {status} — {text}");
            }
            let v: Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
            let items = v.get("items").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            if items.is_empty() {
                return Ok("Queue is empty.".to_string());
            }
            let mut out = format!("{} work item(s):\n", items.len());
            for it in &items {
                let id = it.get("id").and_then(|x| x.as_str()).unwrap_or("");
                let st = it.get("state").and_then(|x| x.as_str()).unwrap_or("");
                let title = it.get("title").and_then(|x| x.as_str()).unwrap_or("");
                let holder = it.get("claimed_by").and_then(|x| x.as_str()).unwrap_or("");
                let who = if holder.is_empty() { String::new() } else { format!(" [{holder}]") };
                out.push_str(&format!("  {id}  {st}{who}  {title}\n"));
                // `result` is the completion trace for a done item, and the
                // reason for a failed/released one — the very thing
                // WorkComplete calls "the only record". Omitting it here left
                // that record unreachable through the only agent-facing read
                // tool (Codex P2 on PR #2902).
                let result = it.get("result").and_then(|x| x.as_str()).unwrap_or("");
                if !result.is_empty() {
                    out.push_str(&format!("      -> {result}\n"));
                }
            }
            Ok(out)
        }
        "CronCreate" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let name = arguments.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;
            let expression = arguments.get("expression").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: expression"))?;
            let prompt = arguments.get("prompt").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: prompt"))?;
            let target = arguments.get("to").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to (target agent id)"))?;
            let max_fires = arguments.get("max_fires").and_then(|v| v.as_i64()).filter(|&n| n > 0);
            let max_age_secs = arguments.get("max_age_secs").and_then(|v| v.as_i64()).filter(|&n| n > 0);
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty()).unwrap_or_default();
            // Identity M3 (spec §5.4): resolve the target while the author is
            // present; the job then fires by uid, never by resolving a name.
            let target_uid = resolve_agent_name_at_boundary(&client, local_url, auth_key, target).await?;

            let url = format!("{}/agentmux/cron", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "name": name, "expression": expression, "prompt": prompt,
                "target": target, "target_uid": target_uid.unwrap_or_default(),
                "created_by": self_id, "max_fires": max_fires,
                "max_age_secs": max_age_secs,
            });
            let resp = client.post(&url).header("X-AuthKey", auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("cron create request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("CronCreate failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let job = &v["job"];
            Ok(format!(
                "Created cron job '{}' (id={})\nExpression: {} UTC\nNext fire: {}\nTarget: {}",
                job["name"].as_str().unwrap_or(name),
                job["id"].as_str().unwrap_or("?"),
                expression,
                job["next_fire"].as_str().unwrap_or("unknown"),
                target,
            ))
        }
        "CronDelete" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let url = format!("{}/agentmux/cron/{}", local_url.trim_end_matches('/'), id);
            let resp = client.delete(&url).header("X-AuthKey", auth_key).send().await
                .map_err(|e| anyhow::anyhow!("cron delete request failed: {e}"))?;
            let status = resp.status();
            if status.as_u16() == 404 {
                return Ok(format!("Job '{id}' not found (already deleted or wrong id)"));
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("CronDelete failed: HTTP {status} — {text}");
            }
            Ok(format!("Deleted cron job {id}"))
        }
        "CronList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/agentmux/cron", local_url.trim_end_matches('/'));
            let resp = client.get(&url).header("X-AuthKey", auth_key).send().await
                .map_err(|e| anyhow::anyhow!("cron list request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("CronList failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let jobs = v["jobs"].as_array().cloned().unwrap_or_default();
            if jobs.is_empty() {
                return Ok("No cron jobs configured.".to_string());
            }
            let mut lines = vec![format!("{} cron job(s):", jobs.len())];
            for j in &jobs {
                let status_str = if j["enabled"].as_bool().unwrap_or(false) { "enabled" } else { "paused" };
                let next = j["next_fire"].as_str().unwrap_or("—");
                let fires = j["fire_count"].as_i64().unwrap_or(0);
                let max = j["max_fires"].as_i64().map(|n| format!("/{n}")).unwrap_or_default();
                let age_bound = j["expires_in_secs"].as_i64().map(|n| format!("  expires_in={n}s")).unwrap_or_default();
                lines.push(format!(
                    "  {}  {}  [{}]  fires={fires}{max}  next={}  expr='{}'{age_bound}",
                    j["id"].as_str().unwrap_or("?"),
                    j["name"].as_str().unwrap_or("?"),
                    status_str,
                    next,
                    j["expression"].as_str().unwrap_or("?"),
                ));
            }
            Ok(lines.join("\n"))
        }
        "CronPause" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            cron_set_enabled(client, local_url, auth_key, id, "pause").await
        }
        "CronResume" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            cron_set_enabled(client, local_url, auth_key, id, "resume").await
        }
        "MemoryList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/memory/list", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("agent_id", agent_id.as_str())])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("memory/list failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "MemoryRead" => {
            let filename = arguments
                .get("filename")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: filename"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/memory/read", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("agent_id", agent_id.as_str()), ("filename", filename)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("memory/read failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            // Surface the file content directly when present; fall back to the raw body.
            Ok(result
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                }))
        }
        "MemoryWrite" => {
            let filename = arguments
                .get("filename")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: filename"))?;
            let content = arguments
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/memory/write", local_url.trim_end_matches('/'));
            let mut body = json!({
                "agent_id": agent_id,
                "filename": filename,
                "content": content,
            });
            // Pass provenance through verbatim when the caller supplied it —
            // advisory metadata for the version history, see
            // SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.1.
            if let Some(provenance) = arguments.get("provenance") {
                body["provenance"] = provenance.clone();
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
                anyhow::bail!("memory/write failed: HTTP {status} — {text}");
            }
            Ok(format!("Wrote memory file \"{filename}\""))
        }
        "MemoryHistory" => {
            let filename = arguments
                .get("filename")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: filename"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/memory/history", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("agent_id", agent_id.as_str()), ("filename", filename)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("memory/history failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "MemoryDiff" => {
            let from_version_id = arguments
                .get("from_version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: from_version_id"))?;
            let to_version_id = arguments
                .get("to_version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to_version_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/memory/diff", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("agent_id", agent_id.as_str()), ("from_version_id", from_version_id), ("to_version_id", to_version_id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("memory/diff failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(result
                .get("diff")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                }))
        }
        "MemoryRevert" => {
            let filename = arguments
                .get("filename")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: filename"))?;
            let target_version_id = arguments
                .get("target_version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: target_version_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/memory/revert", local_url.trim_end_matches('/'));
            let body = json!({
                "agent_id": agent_id,
                "filename": filename,
                "target_version_id": target_version_id,
            });
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
                anyhow::bail!("memory/revert failed: HTTP {status} — {text}");
            }
            Ok(format!("Reverted \"{filename}\" to version {target_version_id}"))
        }
        "GlobalMemoryList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/list", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/list failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "GlobalMemoryRead" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/read", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("id", id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/read failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(result
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                }))
        }
        "GlobalMemoryWrite" => {
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;
            let content = arguments
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/globalmemory/write", local_url.trim_end_matches('/'));
            let mut body = json!({
                "agent_id": agent_id,
                "name": name,
                "content": content,
            });
            if let Some(id) = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                body["id"] = json!(id);
            }
            // Pass provenance through verbatim when the caller supplied it —
            // same advisory-metadata pattern as MemoryWrite above.
            if let Some(provenance) = arguments.get("provenance") {
                body["provenance"] = provenance.clone();
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
                anyhow::bail!("globalmemory/write failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let id = result.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Wrote Global Memory entry \"{name}\" (id: {id})"))
        }
        "GlobalMemoryRemove" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/remove", local_url.trim_end_matches('/'));
            let body = json!({ "id": id });
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
                anyhow::bail!("globalmemory/remove failed: HTTP {status} — {text}");
            }
            Ok(format!("Removed Global Memory entry {id}"))
        }
        "GlobalMemoryHistory" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/history", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("id", id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/history failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "GlobalMemoryDiff" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let from_version_id = arguments
                .get("from_version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: from_version_id"))?;
            let to_version_id = arguments
                .get("to_version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to_version_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/diff", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("id", id), ("from_version_id", from_version_id), ("to_version_id", to_version_id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/diff failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(result
                .get("diff")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                }))
        }
        "GlobalMemoryRevert" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let version_id = arguments
                .get("version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: version_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/globalmemory/revert", local_url.trim_end_matches('/'));
            let body = json!({
                "agent_id": agent_id,
                "id": id,
                "version_id": version_id,
            });
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
                anyhow::bail!("globalmemory/revert failed: HTTP {status} — {text}");
            }
            Ok(format!("Reverted Global Memory entry {id} to version {version_id}"))
        }
        "PresetList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/preset/list", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("preset/list failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "PresetGet" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/preset/get", local_url.trim_end_matches('/'));
            // id/name are both optional; with neither set the server returns the
            // agent's own bound preset ("self"), resolved from agent_id.
            let mut query: Vec<(&str, &str)> = vec![("agent_id", agent_id.as_str())];
            if let Some(pid) = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                query.push(("id", pid));
            }
            if let Some(pname) = arguments.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                query.push(("name", pname));
            }
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&query)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("preset/get failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "IdentityAccounts" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/identity/accounts", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("agent_id", agent_id.as_str())])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("identity/accounts failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "IdentityValidate" => {
            let account_id = arguments
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: account_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/identity/validate", local_url.trim_end_matches('/'));
            let body = json!({
                "agent_id": agent_id,
                "account_id": account_id,
            });
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
                anyhow::bail!("identity/validate failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

#[cfg(test)]
#[path = "tests/main_tests.rs"]
mod tests;
