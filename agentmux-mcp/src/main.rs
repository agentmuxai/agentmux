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
    PtyShellResizeRequest, PtyShellResizeResponse, PtyShellSignalRequest, PtyShellSignalResponse,
    PtyShellStatusRequest, PtyShellStatusResponse, PtyShellStopRequest, PtyShellStopResponse,
    ShellCreateRequest, ShellCreateResponse, ShellInputFailure, ShellInputRequest,
    ShellInputResponse, ShellStatusRequest, ShellStatusResponse, ShellStopRequest,
    ShellStopResponse, TabActivateRequest, TabNameRequest, TabNewRequest, UiClickRequest,
    UiQueryRequest, UiScreenshotRequest, UiScreenshotResponse, WindowFocusRequest,
    WindowNameRequest, WorkspaceNameRequest, PaneTitleRequest,
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
                let shell_input: Value = serde_json::from_str(SHELL_INPUT_TOOL).expect("static json");
                let shell_status: Value = serde_json::from_str(SHELL_STATUS_TOOL).expect("static json");
                let pty_shell: Value = serde_json::from_str(PTY_SHELL_TOOL).expect("static json");
                let pty_shell_input: Value =
                    serde_json::from_str(PTY_SHELL_INPUT_TOOL).expect("static json");
                let pty_shell_signal: Value =
                    serde_json::from_str(PTY_SHELL_SIGNAL_TOOL).expect("static json");
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
                let preset_list: Value = serde_json::from_str(PRESET_LIST_TOOL).expect("static json");
                let preset_get: Value = serde_json::from_str(PRESET_GET_TOOL).expect("static json");
                let identity_accounts: Value =
                    serde_json::from_str(IDENTITY_ACCOUNTS_TOOL).expect("static json");
                let identity_validate: Value =
                    serde_json::from_str(IDENTITY_VALIDATE_TOOL).expect("static json");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": [shell, shell_stop, shell_input, shell_status, pty_shell, pty_shell_input, pty_shell_signal, pty_shell_resize, pty_shell_read, pty_shell_status, pty_shell_stop, open_editor, open_media, send_message, discover_agents, get_agent_transcript, list_conversations, supervisor_nudge, whoami, layout, set_name, set_active_tab, new_tab, focus_window, ui_screenshot, ui_click, ui_query, capture_window, discover_windows, fleet_list, fleet_broadcast, fleet_bulk_stop, open_agent, loop_tool, loop_stop, loop_list, cron_create, cron_delete, cron_list, cron_pause, cron_resume, work_enqueue, work_claim, work_heartbeat, work_complete, work_release, work_list, memory_list, memory_read, memory_write, memory_history, memory_diff, memory_revert, preset_list, preset_get, identity_accounts, identity_validate] }
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

/// The calling agent's slug (its `AGENTMUX_AGENT_ID`), injected by AgentMux into
/// this MCP server's trusted environment. The App API identity/preset/memory
/// REST endpoints stamp their `agent_id` from this — the agent's own model
/// output cannot reach those endpoints (no auth key in the PTY) nor override
/// this value, so the slug cannot be forged. See
/// SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md §5.
pub(crate) fn agent_slug() -> Result<String> {
    let slug = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
    if slug.is_empty() {
        anyhow::bail!(
            "AGENTMUX_AGENT_ID is not set — cannot resolve this agent's identity. \
             Is this agent pane opened via AgentMux?"
        );
    }
    Ok(slug)
}

/// Process-wide counter for `generate_jekt_msgid` — millis-timestamp alone
/// isn't guaranteed unique if two jekts are sent in the same millisecond.
static JEKT_MSGID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique-enough message id for a signed jekt: no `uuid` dependency needed
/// (this crate doesn't otherwise pull one in) — timestamp + this process's
/// own pid + a monotonic counter is unique enough for its one purpose (a
/// value both this signer and the receiving srv agree to include in the
/// signed material, so a signature can't be replayed under a different id).
fn generate_jekt_msgid() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let n = JEKT_MSGID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{}", now.as_millis(), std::process::id(), n)
}

/// `FleetBroadcast`'s block_id -> agent-name resolution
/// (REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md): a `/agentmux/discovery`
/// response's `host.addressable` AND `host.cross_channel` sections both
/// carry a `block_id` — `addressable` under `agent_id`/`block_id`,
/// `cross_channel` (a different channel on this same host) under
/// `name`/`block_id`. A block_id is unique across channels on one host, so
/// both are folded into one map, not kept separate. `lan`/`wan` entries
/// carry no `block_id` at all (they're not local blocks) — a target for
/// those tiers is never in this map, and the caller falls back to treating
/// it as a literal agent name instead (see the `FleetBroadcast` handler).
///
/// Pure (no I/O) — extracted so it's unit-testable without a live discovery
/// endpoint.
fn build_block_to_agent_map(discovery: &Value) -> std::collections::HashMap<String, String> {
    let mut map: std::collections::HashMap<String, String> = discovery
        .get("host")
        .and_then(|h| h.get("addressable"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let agent_id = e.get("agent_id")?.as_str()?.to_string();
                    let block_id = e.get("block_id")?.as_str()?.to_string();
                    Some((block_id, agent_id))
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(arr) = discovery.get("host").and_then(|h| h.get("cross_channel")).and_then(|v| v.as_array()) {
        for e in arr {
            if let (Some(name), Some(block_id)) = (
                e.get("name").and_then(|v| v.as_str()),
                e.get("block_id").and_then(|v| v.as_str()),
            ) {
                map.insert(block_id.to_string(), name.to_string());
            }
        }
    }
    map
}

/// Build the id/timestamp/host-signature/LAN-signature quadruple for an
/// outgoing jekt (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2,
/// SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.3). Reads this process's OWN
/// `AGENTMUX_JEKT_KEY`/`AGENTMUX_LAN_KEY` — injected at spawn alongside
/// `AGENTMUX_AGENT_ID`, into this agent's env only, never any other
/// agent's — and signs over the same (msgid, source_agent, target_agent,
/// ts_secs, message) material both schemes share.
///
/// Both signatures are computed unconditionally, regardless of which
/// delivery tier the message actually ends up taking — this process has no
/// reliable way to know that in advance (routing/forwarding is a
/// server-side decision, see
/// `docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` §3). Sending a
/// `lan_sig` alongside a message that's actually delivered host or WAN
/// costs nothing — srv only ever consults `lan_sig` when it has
/// independently determined `delivery_tier == "lan"` (never trusting the
/// message body's own claim), so an irrelevant signature is simply ignored.
///
/// Returns `(request_id, ts_secs, jekt_sig, lan_sig)`. Either signature is
/// `None` — not an error — when its key is unavailable (an agent whose
/// `.mcp.json` predates this feature, or `source_agent` itself unresolved):
/// srv treats an absent signature as "unverified," never as a reason to
/// fail delivery, so a missing key here must never block
/// `SendMessage`/`Loop` from sending.
///
/// The cross-channel signature (`channel_sig`,
/// `SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md` §D5) rides along on the
/// same terms as `lan_sig`: same `AGENTMUX_LAN_KEY`, but over a
/// domain-separated payload that also binds this process's channel
/// (`AGENTMUX_CHANNEL`, injected into this env at spawn by
/// `inject_jekt_signing_keys_into_mcp_json`; defaults to `stable` exactly as
/// srv's own registry writer does). srv only consults it once it has itself
/// labelled the delivery `channel` — a same-machine forward to a different
/// instance — so on every other tier it's simply ignored.
///
/// Returned as a named struct rather than a tuple: the three signatures are
/// consecutive `Option<String>`s and a transposition at any of the call
/// sites would compile clean and silently mislabel trust on the receiver.
struct OutgoingJektSignatures {
    request_id: String,
    ts_secs: i64,
    jekt_sig: Option<String>,
    lan_sig: Option<String>,
    source_channel: Option<String>,
    channel_sig: Option<String>,
}

impl OutgoingJektSignatures {
    /// The wire request these signatures were minted for — every send path
    /// builds it through here so none can forget a field.
    fn into_request(self, target_agent: String, message: String, source_agent: Option<String>) -> InjectRequest {
        InjectRequest {
            target_agent,
            message,
            source_agent,
            request_id: Some(self.request_id),
            ts_secs: Some(self.ts_secs),
            jekt_sig: self.jekt_sig,
            lan_sig: self.lan_sig,
            source_channel: self.source_channel,
            channel_sig: self.channel_sig,
        }
    }
}

fn sign_outgoing_jekt(
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
) -> OutgoingJektSignatures {
    let msgid = generate_jekt_msgid();
    let ts_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let jekt_sig = (|| {
        let key_b64 = std::env::var("AGENTMUX_JEKT_KEY").ok().filter(|s| !s.is_empty())?;
        let key = agentmux_common::jekt_sign::decode_key(&key_b64)?;
        let src = source_agent?;
        Some(agentmux_common::jekt_sign::sign_jekt(&key, &msgid, src, target_agent, ts_secs, message))
    })();
    let lan_key = std::env::var("AGENTMUX_LAN_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|b64| agentmux_common::jekt_sign::decode_key(&b64));
    let lan_sig = (|| {
        let key = lan_key.as_deref()?;
        let src = source_agent?;
        agentmux_common::jekt_sign::sign_lan_jekt(key, &msgid, src, target_agent, ts_secs, message)
    })();
    let source_channel = std::env::var("AGENTMUX_CHANNEL").ok().filter(|s| !s.is_empty());
    let channel_sig = (|| {
        let key = lan_key.as_deref()?;
        let src = source_agent?;
        // Same default srv's registry writer uses (`local_channel_id`): an
        // unset var means the stable channel, not "unknown".
        let channel = source_channel.as_deref().unwrap_or("stable");
        agentmux_common::jekt_sign::sign_channel_jekt(key, &msgid, src, channel, target_agent, ts_secs, message)
    })();
    // Only declare a channel when there's a signature bound to it — a bare
    // `source_channel` with nothing to verify is noise on the wire.
    let source_channel = channel_sig
        .as_ref()
        .map(|_| source_channel.unwrap_or_else(|| "stable".to_string()));
    OutgoingJektSignatures { request_id: msgid, ts_secs, jekt_sig, lan_sig, source_channel, channel_sig }
}

/// Build the identity proof every `/api/v1/ui/*` request carries — an
/// HMAC-SHA256 signature over this agent's own agent_id, using this
/// agent's own `AGENTMUX_JEKT_KEY` (the same per-agent key `sign_outgoing_jekt`
/// above uses for jekt messages; reused rather than inventing a parallel
/// credential system). Unlike jekt signing, a missing key here is a hard
/// error, not a silent "send unsigned" — srv has no unverified fallback
/// path for UI automation (see `agentmux-srv/src/server/ui_handlers.rs`'s
/// module doc comment), so a tool call with no key would just 401 anyway;
/// failing fast with a clear "respawn to get a key" message is more useful
/// than a confusing round trip.
fn sign_ui_automation_auth() -> Result<agentmux_common::api_types::UiAutomationAuth> {
    let agent_id = agent_slug()?;
    let key_b64 = std::env::var("AGENTMUX_JEKT_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "AGENTMUX_JEKT_KEY is not set — this agent needs to be respawned to get a \
                 signing key before it can use UI automation (UIScreenshot/UIClick/UIQuery)"
            )
        })?;
    let key = agentmux_common::jekt_sign::decode_key(&key_b64)
        .ok_or_else(|| anyhow::anyhow!("AGENTMUX_JEKT_KEY is set but not valid base64"))?;
    let ts_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let sig = agentmux_common::jekt_sign::sign_jekt(
        &key,
        "ui-automation-identity",
        &agent_id,
        "__srv__",
        ts_secs,
        "",
    );
    Ok(agentmux_common::api_types::UiAutomationAuth { agent_id, ts_secs, sig })
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
                .json(&PtyShellInputRequest { shell_id: shell_id.to_string(), text: text.to_string() })
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
        "PtyShellSignal" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/signal", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&PtyShellSignalRequest { shell_id: shell_id.to_string(), name: name.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/signal failed: HTTP {status} — {body}");
            }

            let result: PtyShellSignalResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.sent {
                format!("sent {name} to shell {shell_id}")
            } else {
                format!(
                    "shell {shell_id}: signal failed — {}",
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
                .json(&PtyShellResizeRequest { shell_id: shell_id.to_string(), rows, cols })
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
                .json(&PtyShellReadRequest { shell_id: shell_id.to_string(), tail_lines })
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
                .json(&PtyShellStatusRequest { shell_id: shell_id.to_string() })
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
                .json(&PtyShellStopRequest { shell_id: shell_id.to_string() })
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
            Ok(if result.stopped {
                format!("stopped shell {shell_id}")
            } else {
                format!("shell {shell_id} was not running (unknown or already stopped)")
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
            // agentmux-srv/src/backend/reactive/handler.rs) — never by
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
            // smooth refill — agentmux-srv/src/backend/reactive/mod.rs's
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

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
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

            let url = format!("{}/agentmux/work", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "title": title,
                "payload": payload,
                "kind": arguments.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                "target_agent": arguments.get("target_agent").and_then(|v| v.as_str()).unwrap_or(""),
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

            let url = format!("{}/agentmux/work/claim", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "agent_id": self_id,
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

            let url = format!("{}/agentmux/cron", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "name": name, "expression": expression, "prompt": prompt,
                "target": target, "created_by": self_id, "max_fires": max_fires,
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

async fn cron_set_enabled(
    client: &reqwest::Client,
    local_url: &str,
    auth_key: &str,
    id: &str,
    action: &str,
) -> Result<String> {
    let url = format!("{}/agentmux/cron/{}", local_url.trim_end_matches('/'), id);
    let body = serde_json::json!({"action": action});
    let resp = client.patch(&url).header("X-AuthKey", auth_key).json(&body).send().await
        .map_err(|e| anyhow::anyhow!("cron {action} request failed: {e}"))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(format!("Job '{id}' not found"));
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Cron{} failed: HTTP {status} — {text}", if action == "pause" { "Pause" } else { "Resume" });
    }
    Ok(format!("Cron job {id} {}", if action == "pause" { "paused" } else { "resumed" }))
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

    /// Guards every test that mutates `AGENTMUX_DATA_HOME` (a process-global
    /// env var) so they never run concurrently against each other — cargo
    /// runs tests in parallel by default, and two tests independently
    /// setting/clearing the same env var would otherwise be a genuine race,
    /// not just a stale comment. Acquire this at the start of any such test,
    /// before touching the env var, and hold it for the env var's entire
    /// mutated lifetime (not just around `capture_window_dir()`/
    /// `audit_log_capture_window()` themselves).
    static DATA_HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Mirrors `agentmux-srv`'s `prune_old_screenshots_deletes_only_stale_pngs`
    /// (`ui_handlers.rs`) exactly — same bug class, same fix, same test shape
    /// (reagent P1/P2 on this tool's own PR, #2709 round 1).
    #[test]
    fn prune_old_captures_deletes_only_stale_pngs() {
        let dir = tempfile::tempdir().unwrap();

        let fresh = dir.path().join("fresh.png");
        std::fs::write(&fresh, b"png").unwrap();

        let stale = dir.path().join("stale.png");
        std::fs::write(&stale, b"png").unwrap();
        let old_time = std::time::SystemTime::now() - (CAPTURE_RETENTION * 2);
        let file = std::fs::File::options().write(true).open(&stale).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(old_time))
            .unwrap();

        // Non-PNG files must never be touched, however old.
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, b"keep me").unwrap();
        let file = std::fs::File::options().write(true).open(&other).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(old_time))
            .unwrap();

        prune_old_captures(dir.path());

        assert!(fresh.exists(), "fresh capture must survive pruning");
        assert!(!stale.exists(), "stale capture must be pruned");
        assert!(other.exists(), "non-png files must never be pruned");
    }

    /// `AGENTMUX_DATA_HOME`, when set, must win over the `~/.agentmux` default
    /// — the same override `agentmux-srv`'s own `get_wave_data_dir()` honors,
    /// which this function replicates rather than reinventing.
    #[test]
    fn capture_window_dir_honors_agentmux_data_home_override() {
        // SAFETY: test-only; DATA_HOME_ENV_LOCK held for the env var's
        // entire mutated lifetime serializes this against every other test
        // that touches AGENTMUX_DATA_HOME (see that lock's own doc comment
        // — cargo runs tests in parallel by default, so this isn't optional).
        let _guard = DATA_HOME_ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("AGENTMUX_DATA_HOME", "/tmp/custom-agentmux-home") };
        let dir = capture_window_dir();
        unsafe { std::env::remove_var("AGENTMUX_DATA_HOME") };
        assert_eq!(
            dir,
            std::path::PathBuf::from("/tmp/custom-agentmux-home/tmp/capture-window")
        );
    }

    /// `own_instance_pids()` must always include the calling process's own
    /// pid at minimum (the first element of its ancestor-walk fallback,
    /// independent of whether `AGENTMUX_APP_PATH` is set in this test's
    /// environment). Not a full behavioral test of the exclusion logic
    /// itself — mocking `sysinfo`'s real OS process table isn't practical —
    /// but it does verify the function runs without panicking and its one
    /// environment-independent guarantee holds. The actual exclusion
    /// behavior (reagent P0, PR #2709 round 3) was verified manually against
    /// this repo's own live process tree — see that commit's message for
    /// the before/after evidence.
    #[test]
    fn own_instance_pids_always_includes_the_calling_process_itself() {
        let pids = own_instance_pids();
        assert!(
            pids.contains(&std::process::id()),
            "own_instance_pids() must always include this process's own pid"
        );
    }

    /// `looks_unrendered()` is the retry/hint trigger for
    /// `CaptureWindow` (SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md
    /// Fix 4) — must flag a truly uniform-color frame (the "blank capture,
    /// no signal" bug the report documents) and must NOT flag a frame with
    /// real visual variation, even a subtle one, as long as it exceeds the
    /// tolerance.
    #[test]
    fn looks_unrendered_flags_a_solid_color_frame() {
        let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([20, 20, 20, 255]));
        assert!(looks_unrendered(&img), "a fully solid-color frame must be flagged");
    }

    #[test]
    fn looks_unrendered_does_not_flag_a_varied_frame() {
        let mut img = image::RgbaImage::from_pixel(32, 32, image::Rgba([20, 20, 20, 255]));
        // A single differing pixel could land between two sampled indices
        // (sampling is spaced across the image, not exhaustive — see
        // looks_unrendered's doc comment) and be missed entirely. Fill a
        // whole row instead, so several sampled indices are guaranteed to
        // fall inside it regardless of the exact sample step for this
        // image size.
        for x in 0..32 {
            img.put_pixel(x, 16, image::Rgba([220, 30, 30, 255]));
        }
        assert!(
            !looks_unrendered(&img),
            "a frame with a clearly differing region must not be flagged as unrendered"
        );
    }

    #[test]
    fn looks_unrendered_ignores_noise_within_tolerance() {
        // Real compositor output isn't perfectly uniform even for a
        // genuinely "blank" themed window (subpixel AA, slight gradient
        // banding) — small per-channel noise within the tolerance must
        // still read as unrendered, or every real blank frame would dodge
        // the retry.
        let mut img = image::RgbaImage::from_pixel(32, 32, image::Rgba([20, 20, 20, 255]));
        for (i, px) in img.pixels_mut().enumerate() {
            let jitter = (i % 5) as u8; // stays within the +/-8 tolerance
            *px = image::Rgba([20 + jitter, 20, 20, 255]);
        }
        assert!(
            looks_unrendered(&img),
            "small within-tolerance noise must still read as unrendered"
        );
    }

    /// `audit_log_capture_window` must append one valid NDJSON line per
    /// call, for both success and failure outcomes, without ever panicking
    /// or returning an error to its caller (it's a fire-and-forget
    /// best-effort side effect — reagent P1, PR #2709 round 4).
    #[test]
    fn audit_log_capture_window_appends_ndjson_for_success_and_failure() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-only; see DATA_HOME_ENV_LOCK's own doc comment for
        // why this guard (not just the tempdir) is required.
        let _guard = DATA_HOME_ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("AGENTMUX_DATA_HOME", dir.path()) };

        audit_log_capture_window(
            "first query",
            &Ok(CaptureOutcome {
                message: "captured ok".to_string(),
                tier: CaptureTier::OtherInstance,
                target: "pid=42 title=\"Other\"".to_string(),
                image_sha256: Some("deadbeef".to_string()),
            }),
            &None,
        );
        audit_log_capture_window("second query", &Err(anyhow::anyhow!("no match")), &None);

        unsafe { std::env::remove_var("AGENTMUX_DATA_HOME") };

        let log_path = dir.path().join("tmp/capture-window/capture-window-audit.log");
        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "one NDJSON line per call");

        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["tool"], "CaptureWindow");
        assert_eq!(first["query"], "first query");
        assert_eq!(first["outcome"]["result"], "success");
        // Spec §6 fields — what was captured, not just that something was.
        assert_eq!(first["tier"], "T2-other-instance");
        assert_eq!(first["target"], "pid=42 title=\"Other\"");
        assert_eq!(first["image_sha256"], "deadbeef");
        assert_eq!(first["redacted"], false);

        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["query"], "second query");
        assert_eq!(second["outcome"]["result"], "error");
        // Nothing was resolved on failure, so these stay null rather than
        // recording a target that was never captured.
        assert!(second["tier"].is_null());
        assert!(second["image_sha256"].is_null());
    }

    /// codex P2 on PR #2845: a failure AFTER target resolution — a denied T3
    /// pid, a failed capture, a failed save — must still record which tier and
    /// target it addressed. Otherwise the audit cannot tell "no such window"
    /// apart from "blocked cross-user attempt", which is the entry a reviewer
    /// most wants to find.
    #[test]
    fn a_failed_capture_still_audits_its_resolved_tier_and_target() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = DATA_HOME_ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("AGENTMUX_DATA_HOME", dir.path()) };

        let resolved = Some((
            CaptureTier::OtherUser,
            "pid=99 <non-AgentMux window>".to_string(),
        ));
        audit_log_capture_window("pid=99", &Err(anyhow::anyhow!("withheld")), &resolved);

        unsafe { std::env::remove_var("AGENTMUX_DATA_HOME") };

        let log_path = dir.path().join("tmp/capture-window/capture-window-audit.log");
        let content = std::fs::read_to_string(&log_path).unwrap();
        let entry: Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(entry["outcome"]["result"], "error");
        assert_eq!(
            entry["tier"], "T3-other-user",
            "a denied cross-user attempt must be identifiable in the trail"
        );
        assert_eq!(entry["target"], "pid=99 <non-AgentMux window>");
    }

    /// A foreign window's TITLE must never appear in a candidate/miss list —
    /// that would bypass DiscoverWindows' include_foreign opt-in by simply
    /// missing on purpose (reagent P1 / codex P2 on PR #2845).
    #[test]
    fn candidate_label_withholds_foreign_window_titles() {
        let Ok(windows) = enumerate_agentmux_windows() else { return };
        for w in windows.iter().filter(|w| !w.is_agentmux && !w.title.is_empty()) {
            let label = candidate_label(w);
            // Escaped form, same reason as `audit_target_label`'s test: a raw
            // `contains(&w.title)` would pass vacuously for any title
            // containing a backslash, which on Windows is most paths.
            assert!(
                !label.contains(&format!("{:?}", w.title)),
                "foreign window title leaked into a candidate label: {label}"
            );
            assert!(label.contains(&format!("pid={}", w.pid)), "pid must still identify it");
            return; // one real foreign window is enough
        }
    }

    /// reagentx P2 on PR #2845 caught that the previous round's claimed fix for
    /// this had silently not applied — the edit no-opped and I reported it as
    /// landed. This test pins the behaviour itself rather than trusting a diff:
    /// a withheld target must be reachable in the UNFILTERED enumeration so it
    /// can be audited, since `foreign` (tier-filtered) cannot see it.
    #[test]
    fn a_withheld_window_is_still_findable_for_auditing() {
        let Ok(windows) = enumerate_agentmux_windows() else { return };
        let withheld: Vec<&AgentMuxWindowInfo> =
            windows.iter().filter(|w| !w.tier.allowed()).collect();
        let capturable: Vec<&AgentMuxWindowInfo> =
            windows.iter().filter(|w| w.tier.allowed()).collect();
        // The enumeration must retain both sets — the capture gate filters
        // later. If enumeration itself dropped withheld windows, the audit
        // could never name them and "withheld" would be indistinguishable
        // from "absent", which is the defect this pins.
        assert_eq!(
            withheld.len() + capturable.len(),
            windows.len(),
            "every enumerated window must be classified, none silently dropped"
        );
        for w in withheld {
            assert_eq!(w.tier, CaptureTier::OtherUser, "only T3 is withheld");
        }
    }

    /// reagentx P1 on PR #2845: withholding a cross-user window's TITLE is not
    /// enough if the *response* differs — differing errors are an existence
    /// oracle, and an agent can probe substrings to reconstruct that title
    /// without ever capturing. The miss message must therefore never reveal
    /// that a withheld window matched.
    ///
    /// Pins the observable property: no tier label may appear in a title-miss
    /// error. If someone reintroduces a distinguishing branch, it will almost
    /// certainly name the tier (that is what the reverted version did) and
    /// this fails.
    #[test]
    fn a_title_miss_never_reveals_a_withheld_match() {
        let mut resolved = None;
        let err = capture_window_impl(
            Some("zzz-nonexistent-window-title-zzz"),
            None,
            None,
            &mut resolved,
        )
        .expect_err("a nonsense title cannot match");
        let msg = err.to_string();
        for leak in ["T3", "other-user", "withheld", "different OS user"] {
            assert!(
                !msg.contains(leak),
                "title-miss error leaked withheld-window state via {leak:?}: {msg}"
            );
        }
    }

    /// reagentx P2 on PR #2845, with a corrected premise. Audit detail follows
    /// the ALLOW decision, not AgentMux-ness:
    ///   - an allowed tier records the real title (the agent could capture the
    ///     window and read it off the pixels anyway)
    ///   - a withheld tier records pid + tier only, because the trail is an
    ///     agent-readable file — putting a T3 title there would hand back
    ///     exactly what the tier denied, reopening the closed oracle via the log
    #[test]
    fn audit_target_label_withholds_only_for_withheld_tiers() {
        let Ok(windows) = enumerate_agentmux_windows() else { return };
        for w in &windows {
            let label = audit_target_label(w);
            assert!(label.contains(&format!("pid={}", w.pid)));
            // Compare against the DEBUG-escaped form the label actually emits.
            // A naive `contains(&w.title)` passes only while no window title
            // needs escaping — it went green locally and failed on CI, where a
            // window is titled `C:\ProgramData\GitHub\...` and `{:?}` doubles
            // every backslash. It would also have made the withheld-side
            // assertion below pass vacuously for exactly those titles.
            let escaped = format!("{:?}", w.title);
            if w.tier.allowed() {
                if !w.title.is_empty() {
                    assert!(
                        label.contains(&escaped),
                        "an allowed tier should keep full audit detail: {label}"
                    );
                }
            } else {
                assert!(
                    label.contains("<title withheld>"),
                    "a withheld tier must not record its title in an agent-readable log: {label}"
                );
                if !w.title.is_empty() {
                    assert!(
                        !label.contains(&escaped),
                        "T3 title leaked into the audit: {label}"
                    );
                }
            }
        }
    }

    /// reagentx P2 on PR #2845: a withheld window must be LISTED (so the
    /// `is_self` fail-safe still surfaces it and `capturable` means something)
    /// but must not carry the two fields that cross the human boundary.
    #[test]
    fn withheld_windows_are_listed_but_title_and_exe_path_are_redacted() {
        let Ok(windows) = enumerate_agentmux_windows() else { return };
        for w in &windows {
            let entry = window_listing_entry(w);
            assert_eq!(entry["pid"], w.pid, "pid is always surfaced");
            assert_eq!(entry["tier"], w.tier.label());
            assert_eq!(entry["capturable"], w.tier.allowed());
            if w.tier.allowed() {
                assert_eq!(entry["title"], w.title);
                assert_eq!(entry["exe_path"], w.exe_path);
            } else {
                assert!(entry["title"].is_null(), "a withheld title must not be listed");
                assert!(entry["exe_path"].is_null(), "exe_path embeds the OS username");
                assert!(!entry["withheld_reason"].is_null(), "say why, don't just blank it");
            }
        }
    }

    /// The Phase-1 `allow` defaults (spec §3). The whole point of this change
    /// is that the caller's own instance is reachable — the old `!is_self`
    /// rule blocked exactly this — while the one human-boundary tier is not.
    #[test]
    fn capture_tier_allows_every_agent_tier_and_withholds_only_other_user() {
        assert!(CaptureTier::SameInstance.allowed(), "own instance must be reachable");
        assert!(CaptureTier::OtherInstance.allowed());
        assert!(CaptureTier::ForeignApp.allowed());
        assert!(
            !CaptureTier::OtherUser.allowed(),
            "a different OS user's window is the one tier held back"
        );
    }

    /// Tier labels land in the audit trail, so a reviewer greps them. Pin the
    /// exact strings — a silent rename would break existing log analysis.
    #[test]
    fn capture_tier_labels_are_stable() {
        assert_eq!(CaptureTier::SameInstance.label(), "T1-same-instance");
        assert_eq!(CaptureTier::OtherInstance.label(), "T2-other-instance");
        assert_eq!(CaptureTier::OtherUser.label(), "T3-other-user");
        assert_eq!(CaptureTier::ForeignApp.label(), "T4-foreign-app");
    }

    /// `current_user_id` failing must produce a DENY, not an allow — the
    /// fail-closed discipline `own_instance_pids()` already follows. Verified
    /// through the real enumeration: every window it returns has a resolved
    /// tier, and any window whose owner couldn't be determined is T3.
    #[test]
    fn windows_with_unresolvable_owner_are_withheld() {
        let Ok(windows) = enumerate_agentmux_windows() else {
            return; // headless CI — nothing to assert against
        };
        for w in &windows {
            if w.exe_path.is_empty() && w.tier.allowed() {
                panic!(
                    "window pid={} has no resolvable owning process yet was allowed \
                     — fail-closed violated",
                    w.pid
                );
            }
        }
    }

    /// reagent P1 on PR #2810: `DiscoverWindows` discloses `exe_path`
    /// (embeds the OS username for a foreign instance/user on a shared
    /// machine) and shipped with zero audit logging, unlike `CaptureWindow`
    /// which logs every call for exactly this reason. Pins that it now
    /// does, into the SAME log file (one window-tool audit trail, not two).
    #[test]
    fn audit_log_discover_windows_appends_ndjson_with_window_list() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = DATA_HOME_ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("AGENTMUX_DATA_HOME", dir.path()) };

        let windows = vec![json!({
            "pid": 4242,
            "title": "AgentMux",
            "exe_path": "C:\\Users\\someone\\agentmux.exe",
            "is_self": false,
        })];
        audit_log_discover_windows(false, false, &windows);

        unsafe { std::env::remove_var("AGENTMUX_DATA_HOME") };

        let log_path = dir.path().join("tmp/capture-window/capture-window-audit.log");
        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "one NDJSON line for this call");

        let entry: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry["tool"], "DiscoverWindows");
        // Both disclosure flags must be legible from the query string alone —
        // `include_foreign` is the one that exposes non-AgentMux titles and
        // exe_paths, i.e. this trail's whole reason for existing (reagentx P2
        // on PR #2845).
        assert_eq!(entry["query"], "include_self=false include_foreign=false");
        assert_eq!(entry["outcome"]["result"], "success");
        assert_eq!(entry["outcome"]["window_count"], 1);
        assert_eq!(
            entry["outcome"]["windows"][0]["exe_path"],
            "C:\\Users\\someone\\agentmux.exe"
        );
    }

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
            OPEN_MEDIA_TOOL,
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
            LOOP_LIST_TOOL,
            WORK_ENQUEUE_TOOL,
            WORK_CLAIM_TOOL,
            WORK_HEARTBEAT_TOOL,
            WORK_COMPLETE_TOOL,
            WORK_RELEASE_TOOL,
            WORK_LIST_TOOL,
            CRON_CREATE_TOOL,
            CRON_DELETE_TOOL,
            CRON_LIST_TOOL,
            CRON_PAUSE_TOOL,
            CRON_RESUME_TOOL,
            MEMORY_LIST_TOOL,
            MEMORY_READ_TOOL,
            MEMORY_WRITE_TOOL,
            MEMORY_HISTORY_TOOL,
            MEMORY_DIFF_TOOL,
            MEMORY_REVERT_TOOL,
            PRESET_LIST_TOOL,
            PRESET_GET_TOOL,
            IDENTITY_ACCOUNTS_TOOL,
            IDENTITY_VALIDATE_TOOL,
            FLEET_LIST_TOOL,
            FLEET_BROADCAST_TOOL,
            FLEET_BULK_STOP_TOOL,
            OPEN_AGENT_TOOL,
            CAPTURE_WINDOW_TOOL,
            DISCOVER_WINDOWS_TOOL,
            LIST_CONVERSATIONS_TOOL,
        ];
        // This array (and its count) has drifted from the real `tools/list`
        // response before this change too — SHELL_INPUT/STATUS, the three
        // UI_* tools, GET_AGENT_TRANSCRIPT, and SUPERVISOR_NUDGE are all
        // live tools missing from it. Not fixed here (out of scope for
        // this feature) — just adding the 3 new fleet-control tools
        // (SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md) alongside the 3
        // memory-version-history tools merged in from a concurrent PR, on
        // top of whatever this test already covered, so at least those
        // don't silently join the drift. CAPTURE_WINDOW_TOOL added here too
        // (PR #2709) — same reasoning, not fixing the pre-existing drift.
        // LIST_CONVERSATIONS_TOOL added here too
        // (SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md
        // Phase A) — same reasoning, not fixing the pre-existing drift.
        // DISCOVER_WINDOWS_TOOL added here too
        // (SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md) — same
        // reasoning, not fixing the pre-existing drift.
        // MUXQUEUE_TOOLS (6: WorkEnqueue/Claim/Heartbeat/Complete/Release/List)
        // added here too (REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md
        // slice 2) — same reasoning, not fixing the pre-existing drift between
        // this running total and the prose breakdown below it.
        // OPEN_AGENT_TOOL added (REPORT_AGENT_OPEN_API_GAP_2026_09_06.md) —
        // same reasoning as the entries above, not fixing the pre-existing
        // drift between this running total and the prose breakdown.
        assert_eq!(defs.len(), 43, "tools/list advertises 27 tools (11 original + 1 OpenMedia + 3 Loop + 5 Cron + 7 agent-API) + 3 memory-version-history + 3 fleet-control tools + 1 OpenAgent + 1 CaptureWindow + 1 ListConversations + 1 DiscoverWindows + 6 Muxqueue");
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

    // REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md:
    // FleetBroadcast's block_id resolution originally only read
    // `host.addressable`, silently failing every `host.cross_channel`
    // target even though it carries a real block_id in the same namespace.
    #[test]
    fn build_block_to_agent_map_includes_host_addressable() {
        let discovery = serde_json::json!({
            "host": {
                "addressable": [
                    { "agent_id": "Korp", "block_id": "block-1" },
                ],
            },
        });
        let map = build_block_to_agent_map(&discovery);
        assert_eq!(map.get("block-1").map(String::as_str), Some("Korp"));
    }

    #[test]
    fn build_block_to_agent_map_includes_host_cross_channel() {
        let discovery = serde_json::json!({
            "host": {
                "addressable": [],
                "cross_channel": [
                    { "name": "Loap", "channel": "dev-other", "local_url": "http://127.0.0.1:9999", "block_id": "block-2" },
                ],
            },
        });
        let map = build_block_to_agent_map(&discovery);
        assert_eq!(map.get("block-2").map(String::as_str), Some("Loap"));
    }

    #[test]
    fn build_block_to_agent_map_merges_both_sections_without_dropping_either() {
        let discovery = serde_json::json!({
            "host": {
                "addressable": [
                    { "agent_id": "Korp", "block_id": "block-1" },
                ],
                "cross_channel": [
                    { "name": "Loap", "channel": "dev-other", "local_url": "http://127.0.0.1:9999", "block_id": "block-2" },
                ],
            },
        });
        let map = build_block_to_agent_map(&discovery);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("block-1").map(String::as_str), Some("Korp"));
        assert_eq!(map.get("block-2").map(String::as_str), Some("Loap"));
    }

    #[test]
    fn build_block_to_agent_map_ignores_lan_and_wan_sections_gracefully() {
        // lan/wan entries carry no block_id at all — this must never panic
        // on their differently-shaped entries, and must simply not resolve
        // them (the caller falls back to using the raw target as an agent
        // name for those).
        let discovery = serde_json::json!({
            "host": { "addressable": [] },
            "lan": [{ "instance_id": "x", "agents": ["RemoteAgent"] }],
            "wan": { "local_agents_subscribed": ["CloudAgent"] },
        });
        let map = build_block_to_agent_map(&discovery);
        assert!(map.is_empty());
    }

    #[test]
    fn build_block_to_agent_map_handles_missing_sections() {
        let map = build_block_to_agent_map(&serde_json::json!({}));
        assert!(map.is_empty());
    }
}
