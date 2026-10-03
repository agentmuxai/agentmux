// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! agentmux-mcp — MCP stdio server that exposes AgentMux's App API tools
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
                let open_files: Value = serde_json::from_str(OPEN_FILES_TOOL).expect("static json");
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
                let conn_list: Value = serde_json::from_str(CONN_LIST_TOOL).expect("static json");
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
                    "result": { "tools": [shell, shell_stop, shell_input, shell_status, pty_shell, pty_shell_input, pty_shell_resize, pty_shell_read, pty_shell_status, pty_shell_stop, conn_list, open_editor, open_media, open_files, send_message, discover_agents, get_agent_transcript, list_conversations, search_history, supervisor_nudge, whoami, layout, set_name, set_active_tab, new_tab, focus_window, ui_screenshot, ui_click, ui_query, close_pane, quit_self, register_dev_server, browser_navigate, browser_back, browser_forward, browser_reload, browser_eval, browser_dispatch_key, browser_focus_element, browser_focus_info, capture_window, discover_windows, fleet_list, fleet_broadcast, fleet_bulk_stop, open_agent, loop_tool, loop_stop, loop_list, cron_create, cron_delete, cron_list, cron_pause, cron_resume, work_enqueue, work_claim, work_heartbeat, work_complete, work_release, work_list, memory_list, memory_read, memory_write, memory_history, memory_diff, memory_revert, global_memory_list, global_memory_read, global_memory_write, global_memory_remove, global_memory_history, global_memory_diff, global_memory_revert, preset_list, preset_get, identity_accounts, identity_validate] }
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
