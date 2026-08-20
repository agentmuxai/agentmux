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
    InjectRequest, PaneOpenRequest, PaneOpenResponse, ShellCreateRequest, ShellCreateResponse,
    ShellInputFailure, ShellInputRequest, ShellInputResponse, ShellStatusRequest, ShellStatusResponse,
    ShellStopRequest, ShellStopResponse, TabActivateRequest, TabNameRequest, TabNewRequest,
    UiClickRequest, UiQueryRequest, UiScreenshotRequest, UiScreenshotResponse,
    WindowFocusRequest, WindowNameRequest, WorkspaceNameRequest, PaneTitleRequest,
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

const SHELL_TOOL: &str = r#"{
  "name": "Shell",
  "description": "Start a long-running shell process. Returns immediately with a shell_id. Output streams live in the conversation document. Use for build systems, watchers, dev servers — anything that should run in the background without blocking the conversation. Stop it later with ShellStop(shell_id) — never use kill/taskkill on a shell you started.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "cmd":            { "type": "string",  "description": "Command to run (passed to sh -c / cmd /C)" },
      "cwd":            { "type": "string",  "description": "Working directory (defaults to agent workdir)" },
      "title":          { "type": "string",  "description": "Display label shown in the conversation row (defaults to cmd)" },
      "env":            { "type": "object",  "description": "Extra environment variables", "additionalProperties": { "type": "string" } },
      "capture_stdin":  { "type": "boolean", "description": "Pipe stdin so ShellInput() can write to it. Default false — avoids blocking programs that read stdin to EOF (e.g. `cat` with no args). Set true only when you intend to use ShellInput()." }
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

const SHELL_INPUT_TOOL: &str = r#"{
  "name": "ShellInput",
  "description": "Write text to the stdin of a running shell started by Shell(). A newline is appended automatically. Use for interactive prompts like 'Terminate batch job (Y/N)?' or REPL commands. REQUIRES the shell to have been created with capture_stdin=true; otherwise its stdin is /dev/null and this returns an error telling you to recreate the shell. Also returns an error if the shell has exited. Note: processes that block waiting for stdin-EOF (e.g. `cat` with no args) will not exit until ShellStop is called — ShellStop always unblocks them via kill.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "shell_id": { "type": "string", "description": "The shell_id returned by a prior Shell() call" },
      "text":     { "type": "string", "description": "Text to write to stdin (newline appended automatically)" }
    },
    "required": ["shell_id", "text"]
  }
}"#;

const SHELL_STATUS_TOOL: &str = r#"{
  "name": "ShellStatus",
  "description": "Query whether a shell started by Shell() is still running. Returns running status, exit code (when exited), and total line count so far. Use to poll for completion or check if a dev server is still up before starting a second one.",
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

const GET_AGENT_TRANSCRIPT_TOOL: &str = r#"{
  "name": "GetAgentTranscript",
  "description": "Read the tail of a registered agent's session transcript by name, plus whether it currently has a turn in flight (turn_active). For a Warden Supervisor watcher agent polling other agents on its own interval to decide whether to nudge a stalled one to continue. Returns JSON: {agent, block_id, turn_active, lines: [...], truncated}. Read-only, best-effort — does not deliver anything to the target.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent":     { "type": "string", "description": "Name of the target agent (its AGENTMUX_AGENT_ID value)" },
      "max_lines": { "type": "integer", "description": "Max number of recent transcript lines to return (default 100, server-capped at 500)" }
    },
    "required": ["agent"]
  }
}"#;

const SUPERVISOR_NUDGE_TOOL: &str = r#"{
  "name": "SupervisorNudge",
  "description": "For a Warden Supervisor watcher agent: record a decision about a target agent that just ended its turn, after inspecting it with GetAgentTranscript. action=\"nudge\" delivers a fixed, server-owned continuation message to the target as an ordinary jekt and logs the decision — this tool does NOT accept custom message text; the nudge is deliberately a narrow, non-composable template, not an instruction you write per-situation. action=\"decline\" sends nothing and just logs that you chose not to nudge (e.g. the target isn't opted in, or looks genuinely done/blocked, not merely pausing to ask). A nudge is refused (tool error) if the target hasn't opted in via auto_continue_enabled, or if it would exceed the server-side consecutive-nudge ceiling — treat either as a signal to stop nudging this target and escalate to a human via SendMessage instead of retrying.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "target_agent": { "type": "string", "description": "Name of the agent this decision is about (its AGENTMUX_AGENT_ID value)" },
      "action":       { "type": "string", "enum": ["nudge", "decline"], "description": "Whether to nudge the target to continue or decline to" },
      "reason":       { "type": "string", "description": "Your stated reasoning for this decision, recorded in the audit log" }
    },
    "required": ["target_agent", "action"]
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

const UI_SCREENSHOT_TOOL: &str = r#"{
  "name": "UIScreenshot",
  "description": "Capture a screenshot of your OWN AgentMux pane's UI — clipped to just that pane, not the whole shared window (can't see other panes/agents). Returns a file path; use the Read tool on that path to view the image yourself, or OpenMedia to show it to the user. Use this to visually verify a UI change (e.g. after UIClick-ing a button).",
  "inputSchema": { "type": "object", "properties": {} }
}"#;

const UI_CLICK_TOOL: &str = r#"{
  "name": "UIClick",
  "description": "Click an element in AgentMux's UI — a real synthesized mouse click (not a scripted .click()), so focus/hover/pointer behavior matches a human click. Reaches your OWN pane and shared app chrome (status bar, hamburger menu, window controls); cannot reach a DIFFERENT pane or agent's UI. Use UIQuery first if you're not sure of the right CSS selector.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "selector": { "type": "string", "description": "CSS selector for the element to click, scoped to your own pane" }
    },
    "required": ["selector"]
  }
}"#;

const UI_QUERY_TOOL: &str = r#"{
  "name": "UIQuery",
  "description": "Find elements in AgentMux's UI matching a CSS selector — returns tag, text, attributes, bounding rect, and focus state for each match. Reaches your OWN pane and shared app chrome (status bar, hamburger menu, window controls); cannot reach a DIFFERENT pane or agent's UI. Use this to locate an element before UIClick-ing it, or to read rendered text/state (e.g. did a button's label change) without taking a screenshot.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "selector": { "type": "string", "description": "CSS selector to match, scoped to your own pane" },
      "limit": { "type": "number", "description": "Max number of matches to return (default: all)" }
    },
    "required": ["selector"]
  }
}"#;

// Fleet control (SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md) — select,
// broadcast, and bulk-act on many agents at once, from an agent's own
// perspective. FleetList is a thin, fleet-framed alias of DiscoverAgents
// (same `/agentmux/discovery` call — its response already carries each
// reachable agent's `block_id`, exactly what FleetBroadcast/FleetBulkStop
// need as `targets`). FleetBroadcast loops the SAME signed single-target
// delivery path SendMessage uses, once per target, entirely client-side —
// only this process holds AGENTMUX_JEKT_KEY, so per-message signing can
// only happen here, never in a server-side batch RPC. FleetBulkStop calls
// the one fleet action that genuinely IS a server-side batch RPC
// (`POST /api/v1/fleet/bulk-stop`) since stopping a controller involves no
// jekt signing at all.
const FLEET_LIST_TOOL: &str = r#"{
  "name": "FleetList",
  "description": "List every agent reachable from here (same data as DiscoverAgents), framed for fleet targeting: each entry's block_id is what FleetBroadcast/FleetBulkStop expect in their `targets` array. Use this before either of those to build your target list. Read-only. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const FLEET_BROADCAST_TOOL: &str = r#"{
  "name": "FleetBroadcast",
  "description": "Send the SAME message to many agents at once by block_id (get these from FleetList). Delivers each one individually and signed, exactly like SendMessage would — this is a convenience loop, not a new delivery mechanism. Returns JSON {succeeded: [block_id...], failed: [{id, error}...]} — always check `failed`, a partial failure is common (e.g. one target went offline) and is never silently dropped.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "targets": { "type": "array", "items": { "type": "string" }, "description": "block_id values to send to (from FleetList)" },
      "message": { "type": "string", "description": "Message text to inject into each target's conversation" }
    },
    "required": ["targets", "message"]
  }
}"#;

const FLEET_BULK_STOP_TOOL: &str = r#"{
  "name": "FleetBulkStop",
  "description": "Stop many agent panes at once by block_id (get these from FleetList). Destructive — double-check your target list first. Returns JSON {succeeded, failed: [{id, error}...], aborted_early}. Optionally pass `staged` to cap blast radius on a bad selection: stops `batch_size` targets at a time, and if a batch's failure rate exceeds `max_fail_percentage`, the remaining targets are recorded as failed (untried) instead of being attempted — `aborted_early` will be true. Without `staged`, every target is attempted as one batch.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "targets": { "type": "array", "items": { "type": "string" }, "description": "block_id values to stop (from FleetList)" },
      "signal": { "type": "string", "description": "Optional: SIGKILL or SIGTERM for a forceful stop; default is a graceful stop" },
      "staged": {
        "type": "object",
        "description": "Optional staged rollout to cap blast radius",
        "properties": {
          "batch_size": { "type": "integer", "description": "How many targets to stop per batch" },
          "max_fail_percentage": { "type": "integer", "description": "Abort remaining batches if a completed batch's failure rate exceeds this (0-100)" }
        },
        "required": ["batch_size", "max_fail_percentage"]
      }
    },
    "required": ["targets"]
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
  "description": "Rename an AgentMux UI element. `target` selects which: \"window\" (the OS taskbar / window-title name; clamped to 64 chars), \"tab\" (the tab-bar label), \"pane\" (a conversation pane's header title), or \"workspace\" (the workspace name). Defaults to your own element; pass `target_id` (from Layout/WhoAmI) to rename any specific element by id. Names are trimmed; non-window names clamp to 128 chars.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "target":    { "type": "string", "enum": ["window", "tab", "pane", "workspace"], "description": "Which UI element to rename" },
      "name":      { "type": "string", "description": "The new name/title to display" },
      "target_id": { "type": "string", "description": "Explicit id of the element to rename (window_id / tab_id / workspace_id / block_id depending on target). Omit to default to your own." }
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

const OPEN_MEDIA_TOOL: &str = r#"{
  "name": "OpenMedia",
  "description": "Open an image, video, or audio file in an AgentMux media pane next to this conversation. Use when you want the user to see/watch generated media you're discussing. Pass an absolute host path. Fire-and-forget: returns once the pane is opened.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file":     { "type": "string", "description": "Absolute path to the media file to open" },
      "title":    { "type": "string", "description": "Optional tab/pane title (defaults to the file name)" },
      "split":    { "type": "string", "enum": ["right", "left", "down", "up"], "description": "Where to place the new pane relative to this agent pane (default: right). Ignored when floating is true." },
      "floating": { "type": "boolean", "description": "Open the file in a floating window (a chromeless pane over the app) instead of a docked split. Default: false." }
    },
    "required": ["file"]
  }
}"#;

const LOOP_TOOL: &str = r#"{
  "name": "Loop",
  "description": "Cross-agent recurring inject: run a prompt or slash command on a recurring interval by injecting it into ANOTHER agent's conversation (or your own, if you explicitly need muxbus-delivered self-messaging). Returns immediately with a loop_id; the prompt is injected on a fixed schedule until you call LoopStop(loop_id) or it exhausts max_iterations. If you're scheduling your OWN future turn (a same-session self-check, no other agent involved) — prefer the native ScheduleWakeup (one-off or adaptive-delay recurring, via re-arming) or native CronCreate (durable, cron-expression) tools instead: they have zero delivery overhead (no cross-agent messaging envelope) and built-in cache-window-aware backoff guidance this tool doesn't have. Use THIS tool only when the target is a different agent, or you specifically need AgentMux-persisted delivery across a restart. Loops stop automatically when the agent pane closes.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "prompt":         { "type": "string",  "description": "The prompt or slash command to inject each interval (e.g. 'check the PR status' or '/babysit-prs')" },
      "interval":       { "type": "string",  "description": "How often to run: a number with optional unit s/m/h (e.g. '30s', '5m', '1h'). A bare number is minutes. Default '10m'. Minimum 10s." },
      "to":             { "type": "string",  "description": "Target agent name (its AGENTMUX_AGENT_ID) to inject into. Defaults to this agent itself (a self-loop)." },
      "immediate":      { "type": "boolean", "description": "Run once immediately on start in addition to every interval. Default false (first run after one interval)." },
      "max_iterations": { "type": "integer", "description": "Stop automatically after this many fires. Omit or set to 0 for unlimited." }
    },
    "required": ["prompt"]
  }
}"#;

const LOOP_STOP_TOOL: &str = r#"{
  "name": "LoopStop",
  "description": "Stop a recurring loop started by Loop(). Pass the loop_id it returned. Loops also stop automatically when the agent pane closes or max_iterations is reached.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "loop_id": { "type": "string", "description": "The loop_id returned by a prior Loop() call" }
    },
    "required": ["loop_id"]
  }
}"#;

const LOOP_LIST_TOOL: &str = r#"{
  "name": "LoopList",
  "description": "List all currently running loops in this agent session. Returns each loop's id, prompt, target, interval, fire count, and remaining iterations (if capped). Like 'ps' for loops.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const CRON_CREATE_TOOL: &str = r#"{
  "name": "CronCreate",
  "description": "AgentMux's own cross-agent cron — NOT the same tool as the native (non-mcp__agentmux__-prefixed) CronCreate your harness may also expose, which schedules only your own session's future turn. Use this one specifically to target a DIFFERENT agent (or when you need AgentMux-persisted delivery independent of any single session). Creates a persistent scheduled cron job that survives agent pane restarts. Fires the prompt on a UTC cron schedule by injecting it into the target agent. Unlike Loop, cron jobs persist as long as agentmux-srv is running. Returns a job id and the next scheduled fire time. If you're scheduling your OWN future turn instead, prefer the native CronCreate/ScheduleWakeup tools — no cross-agent envelope overhead.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name":       { "type": "string",  "description": "Human-readable label for the job (e.g. 'daily-standup-check')" },
      "expression": { "type": "string",  "description": "5-field UTC cron expression: 'min hour dom month dow' (e.g. '0 9 * * 1-5' = 9am weekdays). Standard cron syntax; ranges, lists, and step values are supported." },
      "prompt":     { "type": "string",  "description": "The prompt or slash command to inject at each scheduled fire" },
      "to":         { "type": "string",  "description": "Target agent id to inject into. Required." },
      "max_fires":  { "type": "integer", "description": "Auto-disable after this many fires (the job row stays in DB for audit; use CronDelete to remove it). Omit for unlimited." },
      "max_age_secs": { "type": "integer", "description": "Auto-disable this many seconds after creation, regardless of fire count (a hard staleness/stuck-loop bound, matching the spirit of native CronCreate's 7-day auto-expiry). Omit for no expiry — appropriate for genuinely long-running cross-agent automations; set this when babysitting something that should have a natural end (e.g. 'stop checking this PR after 6 hours even if it's still open')." }
    },
    "required": ["name", "expression", "prompt", "to"]
  }
}"#;

const CRON_DELETE_TOOL: &str = r#"{
  "name": "CronDelete",
  "description": "Delete a persistent AgentMux cross-agent cron job (created via this same mcp__agentmux__ tool family's CronCreate, not the native per-session one) by id. Stops all future fires immediately.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "The job id returned by CronCreate or CronList" }
    },
    "required": ["id"]
  }
}"#;

const CRON_LIST_TOOL: &str = r#"{
  "name": "CronList",
  "description": "List all persistent AgentMux cross-agent cron jobs (created via this same mcp__agentmux__ tool family, not the native per-session ones). Returns each job's id, name, expression, next fire time, fire count, and enabled state.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const CRON_PAUSE_TOOL: &str = r#"{
  "name": "CronPause",
  "description": "Pause a persistent AgentMux cross-agent cron job (this mcp__agentmux__ tool family, not the native per-session one — native cron has no pause/resume, only delete). The job definition is kept in the DB but no fires occur until CronResume is called.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "The job id to pause" }
    },
    "required": ["id"]
  }
}"#;

const CRON_RESUME_TOOL: &str = r#"{
  "name": "CronResume",
  "description": "Resume a paused AgentMux cross-agent cron job (this mcp__agentmux__ tool family). The job will fire at its next scheduled UTC time.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "The job id to resume" }
    },
    "required": ["id"]
  }
}"#;

const MEMORY_LIST_TOOL: &str = r#"{
  "name": "MemoryList",
  "description": "List your own native memory (brain) markdown files. Returns each file's filename, whether it is the index, its metadata_type, size in bytes, and last-modified time. Use it to see what you've remembered before reading or writing a specific file. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const MEMORY_READ_TOOL: &str = r#"{
  "name": "MemoryRead",
  "description": "Read one of your own native memory (brain) markdown files by filename. Returns its content. Get valid filenames from MemoryList.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "filename": { "type": "string", "description": "The memory file to read (from MemoryList)" }
    },
    "required": ["filename"]
  }
}"#;

const MEMORY_WRITE_TOOL: &str = r#"{
  "name": "MemoryWrite",
  "description": "Create or overwrite one of your own native memory (brain) markdown files. The write is atomic. Use it to persist notes/context for your future self across conversations.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "filename": { "type": "string", "description": "The memory file to write (created if absent, overwritten if present)" },
      "content":  { "type": "string", "description": "Full markdown content to store in the file" }
    },
    "required": ["filename", "content"]
  }
}"#;

const PRESET_LIST_TOOL: &str = r#"{
  "name": "PresetList",
  "description": "List the presets available to you (summary fields only). A preset is a provider-agnostic config bundle — instructions, context files, MCP servers, and skills. Use it to discover presets before fetching one in full with PresetGet. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const PRESET_GET_TOOL: &str = r#"{
  "name": "PresetGet",
  "description": "Fetch a full preset object by id or name. With BOTH id and name omitted, returns your OWN bound preset (the one you are currently configured with). Use PresetList to discover ids/names.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id":   { "type": "string", "description": "Preset id to fetch (optional)" },
      "name": { "type": "string", "description": "Preset name to fetch (optional)" }
    }
  }
}"#;

const IDENTITY_ACCOUNTS_TOOL: &str = r#"{
  "name": "IdentityAccounts",
  "description": "List your own linked identity accounts. Returns each account's account_id, provider, name, kind, status, masked_tail, and updated_at. Secrets are never returned — only masked tails. Use it to see which provider accounts you can use and to get account_ids for IdentityValidate. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

const IDENTITY_VALIDATE_TOOL: &str = r#"{
  "name": "IdentityValidate",
  "description": "Live-probe one of your own linked accounts against its provider using the stored key, to confirm the credential still works. You never supply a secret — pass an account_id from IdentityAccounts. Returns valid, status, masked_tail, and error.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "account_id": { "type": "string", "description": "One of your linked accounts (from IdentityAccounts)" }
    },
    "required": ["account_id"]
  }
}"#;

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
                let open_editor: Value = serde_json::from_str(OPEN_EDITOR_TOOL).expect("static json");
                let open_media: Value = serde_json::from_str(OPEN_MEDIA_TOOL).expect("static json");
                let send_message: Value = serde_json::from_str(SEND_MESSAGE_TOOL).expect("static json");
                let discover_agents: Value =
                    serde_json::from_str(DISCOVER_AGENTS_TOOL).expect("static json");
                let get_agent_transcript: Value =
                    serde_json::from_str(GET_AGENT_TRANSCRIPT_TOOL).expect("static json");
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
                let fleet_list: Value = serde_json::from_str(FLEET_LIST_TOOL).expect("static json");
                let fleet_broadcast: Value =
                    serde_json::from_str(FLEET_BROADCAST_TOOL).expect("static json");
                let fleet_bulk_stop: Value =
                    serde_json::from_str(FLEET_BULK_STOP_TOOL).expect("static json");
                let loop_tool: Value = serde_json::from_str(LOOP_TOOL).expect("static json");
                let loop_stop: Value = serde_json::from_str(LOOP_STOP_TOOL).expect("static json");
                let loop_list: Value = serde_json::from_str(LOOP_LIST_TOOL).expect("static json");
                let cron_create: Value = serde_json::from_str(CRON_CREATE_TOOL).expect("static json");
                let cron_delete: Value = serde_json::from_str(CRON_DELETE_TOOL).expect("static json");
                let cron_list: Value = serde_json::from_str(CRON_LIST_TOOL).expect("static json");
                let cron_pause: Value = serde_json::from_str(CRON_PAUSE_TOOL).expect("static json");
                let cron_resume: Value = serde_json::from_str(CRON_RESUME_TOOL).expect("static json");
                let memory_list: Value = serde_json::from_str(MEMORY_LIST_TOOL).expect("static json");
                let memory_read: Value = serde_json::from_str(MEMORY_READ_TOOL).expect("static json");
                let memory_write: Value = serde_json::from_str(MEMORY_WRITE_TOOL).expect("static json");
                let preset_list: Value = serde_json::from_str(PRESET_LIST_TOOL).expect("static json");
                let preset_get: Value = serde_json::from_str(PRESET_GET_TOOL).expect("static json");
                let identity_accounts: Value =
                    serde_json::from_str(IDENTITY_ACCOUNTS_TOOL).expect("static json");
                let identity_validate: Value =
                    serde_json::from_str(IDENTITY_VALIDATE_TOOL).expect("static json");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": [shell, shell_stop, shell_input, shell_status, open_editor, open_media, send_message, discover_agents, get_agent_transcript, supervisor_nudge, whoami, layout, set_name, set_active_tab, new_tab, focus_window, ui_screenshot, ui_click, ui_query, fleet_list, fleet_broadcast, fleet_bulk_stop, loop_tool, loop_stop, loop_list, cron_create, cron_delete, cron_list, cron_pause, cron_resume, memory_list, memory_read, memory_write, preset_list, preset_get, identity_accounts, identity_validate] }
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
fn agent_slug() -> Result<String> {
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
fn sign_outgoing_jekt(
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
) -> (String, i64, Option<String>, Option<String>) {
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
    let lan_sig = (|| {
        let key_b64 = std::env::var("AGENTMUX_LAN_KEY").ok().filter(|s| !s.is_empty())?;
        let key = agentmux_common::jekt_sign::decode_key(&key_b64)?;
        let src = source_agent?;
        agentmux_common::jekt_sign::sign_lan_jekt(&key, &msgid, src, target_agent, ts_secs, message)
    })();
    (msgid, ts_secs, jekt_sig, lan_sig)
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

            let (request_id, ts_secs, jekt_sig, lan_sig) =
                sign_outgoing_jekt(source_agent.as_deref(), to, message);

            let url = format!(
                "{}/agentmux/reactive/inject",
                local_url.trim_end_matches('/')
            );
            let req = InjectRequest {
                target_agent: to.to_string(),
                message: message.to_string(),
                source_agent,
                request_id: Some(request_id),
                ts_secs: Some(ts_secs),
                jekt_sig,
                lan_sig,
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
                anyhow::bail!("targets must be a non-empty array of block_id strings");
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

            // Same signed single-target delivery SendMessage uses, looped
            // once per target — only this process holds AGENTMUX_JEKT_KEY,
            // so per-message signing can only happen here (see this
            // tool's own doc comment). Never a single aggregate result:
            // per-target success/failure is collected below regardless of
            // how many targets fail.
            let url = format!("{}/agentmux/reactive/inject", local_url.trim_end_matches('/'));
            let mut succeeded: Vec<String> = Vec::new();
            let mut failed: Vec<serde_json::Value> = Vec::new();
            for target in targets {
                let (request_id, ts_secs, jekt_sig, lan_sig) =
                    sign_outgoing_jekt(source_agent.as_deref(), &target, message);
                let req = InjectRequest {
                    target_agent: target.clone(),
                    message: message.to_string(),
                    source_agent: source_agent.clone(),
                    request_id: Some(request_id),
                    ts_secs: Some(ts_secs),
                    jekt_sig,
                    lan_sig,
                };
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

            let result = json!({ "succeeded": succeeded, "failed": failed });
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
                    let (request_id, ts_secs, jekt_sig, lan_sig) =
                        sign_outgoing_jekt(task_source.as_deref(), &task_target, &task_prompt);
                    let req = InjectRequest {
                        target_agent: task_target.clone(),
                        message: task_prompt.clone(),
                        source_agent: task_source.clone(),
                        request_id: Some(request_id),
                        ts_secs: Some(ts_secs),
                        jekt_sig,
                        lan_sig,
                    };
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
            let body = json!({
                "agent_id": agent_id,
                "filename": filename,
                "content": content,
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
                anyhow::bail!("memory/write failed: HTTP {status} — {text}");
            }
            Ok(format!("Wrote memory file \"{filename}\""))
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
            CRON_CREATE_TOOL,
            CRON_DELETE_TOOL,
            CRON_LIST_TOOL,
            CRON_PAUSE_TOOL,
            CRON_RESUME_TOOL,
            MEMORY_LIST_TOOL,
            MEMORY_READ_TOOL,
            MEMORY_WRITE_TOOL,
            PRESET_LIST_TOOL,
            PRESET_GET_TOOL,
            IDENTITY_ACCOUNTS_TOOL,
            IDENTITY_VALIDATE_TOOL,
            FLEET_LIST_TOOL,
            FLEET_BROADCAST_TOOL,
            FLEET_BULK_STOP_TOOL,
        ];
        // This array (and its count) has drifted from the real `tools/list`
        // response before this change too — SHELL_INPUT/STATUS, the three
        // UI_* tools, GET_AGENT_TRANSCRIPT, and SUPERVISOR_NUDGE are all
        // live tools missing from it. Not fixed here (out of scope for
        // this feature) — just adding the 3 new fleet-control tools to
        // whatever this test already covered, so at least those don't
        // silently join the drift.
        assert_eq!(defs.len(), 30, "tools/list advertises 27 tools (11 original + 1 OpenMedia + 3 Loop + 5 Cron + 7 agent-API) + 3 fleet-control tools added by SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md");
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
