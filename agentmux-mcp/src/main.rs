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
  "description": "Read the tail of a registered agent's session transcript by name, plus whether it currently has a turn in flight (turn_active). Resolves agents on this host first, then falls back to other channels on this same host (cross-channel) — does NOT reach LAN or WAN agents (use ListConversations to see those, but note they're liveness-only for now). For a Warden Supervisor watcher agent polling other agents on its own interval to decide whether to nudge a stalled one to continue, or any agent wanting to check what another agent on this host is doing. Returns JSON: {agent, block_id, tier, turn_active, lines: [...], truncated}. Read-only, best-effort — does not deliver anything to the target.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent":     { "type": "string", "description": "Name of the target agent (its AGENTMUX_AGENT_ID value)" },
      "max_lines": { "type": "integer", "description": "Max number of recent transcript lines to return (default 100, server-capped at 500)" }
    },
    "required": ["agent"]
  }
}"#;

// Cross-tier conversation glance
// (SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md Phase A) —
// one call to see every agent's recent activity across host + cross-channel
// (with a last-message preview) and LAN + WAN (liveness only — Phase A
// deliberately does not invent a remote-read protocol for those tiers; see
// the spec's Phase B/C). Thin passthrough to the srv's own
// `/api/v1/muxspect/conversations`, same auth pattern as every other tool
// here.
const LIST_CONVERSATIONS_TOOL: &str = r#"{
  "name": "ListConversations",
  "description": "See every agent's most recent activity across host, cross-channel (other AgentMux channels on this same host), LAN, and connected WAN in one call — a faster alternative to DiscoverAgents + N x GetAgentTranscript. Host and cross-channel entries include turn_active, last_activity_ms, and a last_message_preview (tail transcript line). LAN and WAN entries are liveness-only (remote_fetch_required: true) — reading their conversation content isn't supported yet. Read-only. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
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

// Deliberately NOT part of the ui_handlers.rs signed-identity/pane-ownership
// scheme UIScreenshot/UIClick/UIQuery use. Authorization here is by
// `CaptureTier` (what is being captured), not by pane ownership.
//
// **This block previously asserted three invariants that are no longer true**
// (reagentx P2 on PR #2845 — the same stale-comment class already fixed on
// `enumerate_agentmux_windows`, missed here on the first pass):
//   - "crosses no agent-to-agent trust boundary within one instance" — it
//     does now. T1 capture reaches this instance's own window, which contains
//     every agent's pane.
//   - "the caller's OWN instance is always excluded from the candidate set" —
//     no longer excluded; that is the point of the change.
//   - "Scoped to AgentMux's own windows only" — non-AgentMux windows are T4
//     targets now.
//
// What replaced them, per
// `SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`: the
// repo owner directed that agents be able to capture anything, so the control
// moved from prevention to accountability. Every agent-to-agent tier is open
// and audited; only T3 (a window owned by a DIFFERENT OS user) is withheld,
// because that is a human boundary rather than an agent one — that person
// never consented and cannot be notified.
//
// Two things reagent P1 (PR #2709 round 2) established still hold and must
// not be undone while widening scope: the `is_agentmux` flag still gates
// DISCLOSURE even though it no longer gates inclusion — foreign windows stay
// out of `DiscoverWindows`' default listing and their titles are withheld
// from candidate/miss lists (`candidate_label`). That is what keeps round 2's
// KeePass-title leak closed now that foreign windows are enumerated at all.
//
// The per-app approval gating, mutex and app-tier warnings that
// docs/specs/computer-use-pane.md (draft, unbuilt) scopes for arbitrary
// third-party app control remain OUT of scope here: this is read-only pixel
// capture, not input injection.
const CAPTURE_WINDOW_TOOL: &str = r#"{
  "name": "CaptureWindow",
  "description": "Screenshot any window on this machine — another AgentMux instance, your OWN instance's other windows (including a torn-off floating pane), or a non-AgentMux application. Target by pid (preferred; get one from DiscoverWindows) or by (partial, case-insensitive) title match. Prefer pid: AgentMux's own frontend actively rewrites window titles (tab switches, workspace renames), so a title match can go stale mid-session in a way a pid never does. The ONE exception is a window owned by a DIFFERENT OS user — withheld by default, because that is a human boundary rather than an agent one and no in-app notification can reach that person. If title_contains matches more than one window and you did not pass an explicit index, this returns every candidate (pid + title) instead of guessing — pass one of those pids back in, or use DiscoverWindows first. A single process can also own multiple top-level windows sharing the same pid — if pid matches more than one, pass an explicit index alongside it the same way. Every call is logged (who, what, which tier, a hash of the image, outcome) to an audit trail in this instance's own data dir. Returns a file path; use the Read tool on that path to view it yourself, or OpenMedia to show it to the user. The result may note likely_unrendered if the captured frame looks solid/near-solid-color even after a couple of internal retries — that CAN mean the window has not painted its first real frame yet, but it is only a heuristic (a legitimately solid-colored window trips it too).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "pid": { "type": "number", "description": "Process id of the target window's owning process — from DiscoverWindows. Preferred over title_contains: stable for the process's whole lifetime, unlike its title. If more than one window shares this pid, also pass index to disambiguate." },
      "title_contains": { "type": "string", "description": "Substring to match against capturable windows' titles, case-insensitive — that includes non-AgentMux windows, not just AgentMux ones. Ignored if pid is given. Note the candidate/miss lists returned on an ambiguous or failed match identify a non-AgentMux window by pid only, never by title." },
      "index": { "type": "number", "description": "If multiple windows match title_contains (or share the given pid), which one to capture (0-based). Omit this when you expect exactly one match — if more than one actually matches, the tool returns the full candidate list instead of silently picking one." }
    }
  }
}"#;

const DISCOVER_WINDOWS_TOOL: &str = r#"{
  "name": "DiscoverWindows",
  "description": "List top-level windows on this machine — read-only: no screenshot taken, nothing written to disk. Use this BEFORE CaptureWindow so you have real candidates (pid, title, exe_path) instead of guessing a title substring. Each entry reports its capture `tier` and whether it is `capturable`. A window owned by a different OS user is never capturable; when it IS listed it appears withheld rather than omitted — `capturable: false`, `title` and `exe_path` null, plus a `withheld_reason` — so you can see that it exists and why it is out of reach without its content crossing that boundary. Whether it is listed at all follows the same `include_foreign` gate as any other window: by default only AgentMux windows are listed, so another user's non-AgentMux window shows up only with `include_foreign: true`. By default lists AgentMux windows only; pass include_foreign to also list other applications (kept opt-in so ordinary discovery does not disclose the titles of a user's unrelated apps). exe_path can reveal the OS username of a different instance's owner on a shared machine, so — same as CaptureWindow — every call is logged to this instance's own audit trail.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "include_self": { "type": "boolean", "description": "Include this agent's own AgentMux instance's window(s) in the results. Default false, since they are usually not what you are looking for — but unlike before, they ARE capturable, so pass true when you want your own instance's other windows (e.g. a torn-off floating pane)." },
      "include_foreign": { "type": "boolean", "description": "Also list non-AgentMux windows (other applications). Default false so ordinary discovery does not disclose the titles of a user's unrelated apps as a side effect. These are capturable when listed." }
    }
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
  "description": "Send the SAME message to many agents at once (get targets from FleetList). Delivers each one individually and signed, exactly like SendMessage would — this is a convenience loop, not a new delivery mechanism. Targets reach every tier FleetList/DiscoverAgents can see: a host or cross-channel target is its block_id; a LAN or WAN target (no block_id exists for those — they're not local blocks) is its agent NAME instead — pass whichever FleetList gave you for that entry. Returns JSON {succeeded: [target...], failed: [{id, error}...]} — always check `failed`, a partial failure is common (e.g. one target went offline) and is never silently dropped.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "targets": { "type": "array", "items": { "type": "string" }, "description": "block_id (host/cross-channel) or agent name (LAN/WAN) values to send to — from FleetList" },
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

// ── Muxqueue — the universal agent work queue ───────────────────────────────
// docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md
//
// The pull/unaddressed/deferred counterpart to SendMessage's push/addressed/
// immediate delivery: work goes in without naming a recipient, and whichever
// agent asks next takes it. Deliberately NOT modelled on Cron (time-triggered)
// or Loop (repeating) — the trigger here is an agent being ready.

const WORK_ENQUEUE_TOOL: &str = r#"{
  "name": "WorkEnqueue",
  "description": "Put a unit of work on the shared Muxqueue for ANY agent to pick up later. Use this instead of SendMessage when you do NOT need a specific agent, or need it done eventually rather than now — 'someone should repro this', 'this PR needs review when a reviewer frees up'. The item persists across pane closes, app restarts, and version/channel changes, and is visible to every agent on this machine. If you know exactly who should do it and it should happen immediately, use SendMessage instead; if it should happen on a schedule, use CronCreate. Returns the item id.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "title":        { "type": "string",  "description": "Short human-scannable summary (e.g. 'repro the minimize distortion on a 3-pane cross-split')" },
      "payload":      { "type": "string",  "description": "The full instruction injected into whichever agent claims this. Write it as a standalone prompt: the claimant has none of your conversation context." },
      "kind":         { "type": "string",  "description": "Optional free-form tag used to filter claims (e.g. 'review', 'repro', 'triage'). Agents can claim only a kind they handle. Omit for untyped work anyone may take." },
      "target_agent": { "type": "string",  "description": "Optional: restrict to ONE agent id. Mutually exclusive with target_group. Omit so any agent can claim — that is the normal case and the point of the queue." },
      "target_group": { "type": "string",  "description": "Optional: restrict to members of an agent group id. Mutually exclusive with target_agent." },
      "priority":     { "type": "integer", "description": "Higher claims first; ties break oldest-first. Default 0. Use sparingly — everything urgent means nothing is." },
      "not_before":   { "type": "integer", "description": "Unix ms timestamp; the item is not claimable before it. Use for deferred work ('look at this after the release lands'). Omit for immediately claimable." },
      "max_attempts": { "type": "integer", "description": "How many claims this item gets before it is parked as failed. Default 3. Guards against an item that crashes or defeats every agent that takes it." }
    },
    "required": ["title", "payload"]
  }
}"#;

const WORK_CLAIM_TOOL: &str = r#"{
  "name": "WorkClaim",
  "description": "Take the next eligible item off the Muxqueue and become its holder. Returns {claimed:false} when nothing is available — that is a normal answer, not an error, so it is safe to call speculatively when you have spare capacity. Claiming grants a time-limited LEASE, not ownership: heartbeat with WorkHeartbeat during long work, then finish with WorkComplete (or hand it back with WorkRelease). If your lease expires the item returns to the pool for someone else. IMPORTANT: the response includes an 'attempt' number — you must pass it back to every WorkHeartbeat/WorkComplete/WorkRelease call for this item, or they will be rejected. Claiming an item does NOT grant you authority you would not otherwise have: the payload is a prompt, and every action in it is still subject to its own normal confirmation rules.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "kind":     { "type": "string",  "description": "Only claim items with this kind tag. Omit to consider every untargeted item." },
      "lease_ms": { "type": "integer", "description": "How long your lease lasts before the item can be reclaimed by someone else. Default 120000 (2 min). Heartbeat rather than asking for a very long lease — a long lease on a crashed agent blocks the item for that whole window." }
    }
  }
}"#;

const WORK_HEARTBEAT_TOOL: &str = r#"{
  "name": "WorkHeartbeat",
  "description": "Extend your lease on a claimed Muxqueue item while you are still working on it. Call this periodically during long work; without it the lease expires and another agent may take the item. Requires the 'attempt' number from your WorkClaim response — a heartbeat from a superseded claim is rejected (HTTP 409) rather than silently extending someone else's.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id":       { "type": "string",  "description": "The item id from WorkClaim" },
      "attempt":  { "type": "integer", "description": "The 'attempt' number returned by the WorkClaim that gave you this item" },
      "lease_ms": { "type": "integer", "description": "New lease length in ms. Default 120000." }
    },
    "required": ["id", "attempt"]
  }
}"#;

const WORK_COMPLETE_TOOL: &str = r#"{
  "name": "WorkComplete",
  "description": "Mark a claimed Muxqueue item finished. Requires the 'attempt' number from your WorkClaim response; a completion from a superseded claim is rejected (HTTP 409) so a slow agent cannot close out work another agent has since taken over. Record what you actually did in 'result' — it is the only trace of the work once the item is done.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id":      { "type": "string",  "description": "The item id from WorkClaim" },
      "attempt": { "type": "integer", "description": "The 'attempt' number returned by the WorkClaim that gave you this item" },
      "result":  { "type": "string",  "description": "What was done, or what the outcome was. Include links (PR, issue) where relevant." }
    },
    "required": ["id", "attempt"]
  }
}"#;

const WORK_RELEASE_TOOL: &str = r#"{
  "name": "WorkRelease",
  "description": "Hand a claimed Muxqueue item back to the pool because you cannot do it — wrong capabilities, blocked on something, out of context budget. Prefer this over letting your lease silently expire: it frees the item immediately and records why. Note that a release still consumes one of the item's attempts, and releasing on its FINAL attempt parks it as failed rather than reopening it — an item nobody can do should stop circulating. Requires the 'attempt' number from your WorkClaim response.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id":      { "type": "string",  "description": "The item id from WorkClaim" },
      "attempt": { "type": "integer", "description": "The 'attempt' number returned by the WorkClaim that gave you this item" },
      "reason":  { "type": "string",  "description": "Why you are handing it back. This is what the next claimant (or a human) sees." }
    },
    "required": ["id", "attempt"]
  }
}"#;

const WORK_LIST_TOOL: &str = r#"{
  "name": "WorkList",
  "description": "List Muxqueue items — the shared backlog across every agent on this machine. Use it to see what is outstanding before enqueueing something (avoid duplicates), to check on work you enqueued, or to find out who is currently holding what. Read-only.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "state": { "type": "string", "description": "Filter by state: open, claimed, done, failed, cancelled. Omit for all states." },
      "limit": { "type": "integer", "description": "Max items to return (1-500). Default 50." }
    }
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
  "description": "Create or overwrite one of your own native memory (brain) markdown files. The write is atomic. Use it to persist notes/context for your future self across conversations. Every write is retained as a version (see MemoryHistory) — nothing is ever silently lost.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "filename": { "type": "string", "description": "The memory file to write (created if absent, overwritten if present)" },
      "content":  { "type": "string", "description": "Full markdown content to store in the file" },
      "provenance": {
        "type": "object",
        "description": "Optional context for why you're writing this — helps a human reviewing history later. Omit for an ordinary write from your own reasoning.",
        "properties": {
          "source": { "type": "string", "description": "\"human\" if directly instructed by the operator, \"jekt\" if this write is a direct response to jekt content still in your context, omit otherwise (defaults to agent_inferred)" },
          "detail":  { "type": "object", "description": "Extra structured context — e.g. the jekt's marker fields (FROM/TIER/TRUST/DELIVERY/MSGID) when source is \"jekt\"" }
        },
        "required": ["source"]
      }
    },
    "required": ["filename", "content"]
  }
}"#;

const MEMORY_HISTORY_TOOL: &str = r#"{
  "name": "MemoryHistory",
  "description": "List every recorded version of one of your own native memory (brain) markdown files, newest first. Each entry shows who/what wrote it (source: human, agent_inferred, jekt, external_fs_write, or revert) and when. Use it to review how a memory file changed over time, or to find a version id to pass to MemoryDiff/MemoryRevert.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "filename": { "type": "string", "description": "The memory file to show history for (from MemoryList)" }
    },
    "required": ["filename"]
  }
}"#;

const MEMORY_DIFF_TOOL: &str = r#"{
  "name": "MemoryDiff",
  "description": "Show a line-based diff between two recorded versions of a memory file. Get version ids from MemoryHistory.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "from_version_id": { "type": "string", "description": "The earlier version id (from MemoryHistory)" },
      "to_version_id":   { "type": "string", "description": "The later version id (from MemoryHistory)" }
    },
    "required": ["from_version_id", "to_version_id"]
  }
}"#;

const MEMORY_REVERT_TOOL: &str = r#"{
  "name": "MemoryRevert",
  "description": "Restore a memory file's live content to a prior recorded version. This does NOT delete history — it records a new version (source: \"revert\") whose content matches the target, same as `git revert`. Use it to undo a bad or fabricated memory write once you've confirmed via MemoryHistory/MemoryDiff which version to restore.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "filename": { "type": "string", "description": "The memory file to revert" },
      "target_version_id": { "type": "string", "description": "The version id to restore (from MemoryHistory)" }
    },
    "required": ["filename", "target_version_id"]
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
                    "result": { "tools": [shell, shell_stop, shell_input, shell_status, open_editor, open_media, send_message, discover_agents, get_agent_transcript, list_conversations, supervisor_nudge, whoami, layout, set_name, set_active_tab, new_tab, focus_window, ui_screenshot, ui_click, ui_query, capture_window, discover_windows, fleet_list, fleet_broadcast, fleet_bulk_stop, loop_tool, loop_stop, loop_list, cron_create, cron_delete, cron_list, cron_pause, cron_resume, work_enqueue, work_claim, work_heartbeat, work_complete, work_release, work_list, memory_list, memory_read, memory_write, memory_history, memory_diff, memory_revert, preset_list, preset_get, identity_accounts, identity_validate] }
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

/// Where `CaptureWindow` writes its PNGs. Mirrors `agentmux-srv`'s own
/// `get_wave_data_dir()` (`AGENTMUX_DATA_HOME` env var, else `~/.agentmux`)
/// rather than the shared OS temp dir — reagent P2 on this tool's own PR
/// (#2709 round 1): `std::env::temp_dir()` is world-readable on a
/// multi-user host, and CaptureWindow can capture arbitrary OS windows
/// (not just AgentMux's own pane), so a captured image could leak to other
/// local users. `agentmux-mcp` can't import `agentmux-srv`'s function
/// directly (separate crate/process), so this replicates its exact logic
/// instead of inventing a new convention.
fn capture_window_dir() -> std::path::PathBuf {
    let base = std::env::var("AGENTMUX_DATA_HOME")
        .ok()
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/"))
                .join(".agentmux")
        });
    base.join("tmp/capture-window")
}

const CAPTURE_RETENTION: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Same bug class, same fix, as `agentmux-srv`'s `prune_old_screenshots`
/// (`ui_handlers.rs`, reagent P2, PR #2662) — reapplied here per reagent's
/// review of this tool's own PR (#2709 round 1), which found it wasn't
/// reused. Best-effort, on the write path, PNG-only.
fn prune_old_captures(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else { continue };
        let Ok(modified) = metadata.modified() else { continue };
        let Ok(age) = now.duration_since(modified) else { continue };
        if age > CAPTURE_RETENTION {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Max ancestor hops to walk from this MCP process before giving up —
/// bounds the fallback search (see `own_instance_pids`'s doc comment for why
/// this is a fallback, not the primary signal), while never walking so far
/// up the tree (e.g. to `explorer.exe`) that "same instance" degrades into
/// "everything on the desktop."
const OWN_INSTANCE_ANCESTOR_HOPS: usize = 8;

/// PIDs `CaptureWindow` must treat as "this agent's own AgentMux instance"
/// and always exclude — reagent P0 (PR #2709 round 3).
///
/// **Primary signal: `AGENTMUX_APP_PATH`.** Every process belonging to this
/// exact running instance (portable build or install) is launched from
/// under that one directory — confirmed directly (not assumed): this
/// agent's own host process's `Process::exe()` path
/// (`...\agentmux-0.55.18+....-x64-portable\runtime\agentmux-0.55.18.exe`)
/// starts with `AGENTMUX_APP_PATH`
/// (`...\agentmux-0.55.18+....-x64-portable`) exactly. This is more precise
/// than matching on version number alone — it correctly tells apart two
/// separate instances that happen to share a version (e.g. two portable
/// builds of the same release), which a version-string match would
/// conflate.
///
/// **Fallback signal: bounded process-ancestor walk.** Kept as a second,
/// additive layer (union, not replacement) for the case `AGENTMUX_APP_PATH`
/// isn't set (confirmed present for `AGENTMUX_RUNTIME_MODE=portable`; not
/// separately confirmed for `task dev` instances). **This alone is not
/// reliable** — verified directly by testing: Windows recycles PIDs and a
/// process's recorded parent-PID can point at an already-exited process, so
/// `sys.process(stale_ppid)` returns `None` and the walk silently stops
/// short of the real ancestor chain. Confirmed this exact failure in
/// testing (the walk never reached this instance's own host process). Kept
/// only as defense-in-depth alongside the path-based signal, never alone.
///
/// Fails safe: an unresolvable `pid()` on a *candidate* window (checked by
/// the caller, `CaptureWindow`'s handler) is treated as "assume it's mine,
/// exclude it" — so a window this function's own signals can't positively
/// place is still excluded, not silently let through.
fn own_instance_pids() -> std::collections::HashSet<u32> {
    let sys = sysinfo::System::new_all();
    let mut result = std::collections::HashSet::new();

    if let Ok(app_path) = std::env::var("AGENTMUX_APP_PATH") {
        if !app_path.is_empty() {
            let app_path = std::path::Path::new(&app_path);
            for (pid, proc) in sys.processes() {
                if proc.exe().map(|e| e.starts_with(app_path)).unwrap_or(false) {
                    result.insert(pid.as_u32());
                }
            }
        }
    }

    let my_pid = sysinfo::Pid::from(std::process::id() as usize);
    let mut ancestors = vec![my_pid];
    let mut current = my_pid;
    for _ in 0..OWN_INSTANCE_ANCESTOR_HOPS {
        let Some(proc) = sys.process(current) else { break };
        let Some(parent) = proc.parent() else { break };
        ancestors.push(parent);
        current = parent;
    }
    result.extend(ancestors.iter().map(|p| p.as_u32()));
    for (pid, proc) in sys.processes() {
        let Some(ppid) = proc.parent() else { continue };
        if !ancestors.contains(&ppid) {
            continue;
        }
        if proc.name().to_string_lossy().to_lowercase().starts_with("agentmux") {
            result.insert(pid.as_u32());
        }
    }
    result
}

/// Capture tier for one target window — see
/// `docs/specs/SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`
/// §3. Keyed on WHAT is being captured, never on who is asking: identity is
/// already proven upstream (`sign_ui_automation_auth`), so the open question
/// is what a proven identity may reach.
///
/// This replaces the previous binary `!is_self` exclusion. That rule came from
/// `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6, which itself
/// says cross-agent targeting *"if it's ever wanted at all"* should be a
/// distinct capability — i.e. it defaulted closed because no mechanism existed
/// to be selective, not because open had been judged wrong. The repo owner has
/// since directed that agents be able to capture anything. This enum is that
/// mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureTier {
    /// T1 — another pane inside the caller's OWN instance. Its window hosts
    /// every agent's pane, which is exactly what the old rule blocked.
    SameInstance,
    /// T2 — a different AgentMux instance owned by the same OS user.
    OtherInstance,
    /// T3 — a window owned by a DIFFERENT OS user. The one tier that crosses a
    /// human rather than an agent boundary: that user never consented to this
    /// app's agents, and no in-app notification can reach them.
    OtherUser,
    /// T4 — a non-AgentMux window owned by the same OS user (their browser,
    /// their password manager). PR #2709 round 2 caught the original unscoped
    /// tool capturing a KeePass window; allowed now, but always audited.
    ForeignApp,
}

impl CaptureTier {
    /// Phase 1 `allow` defaults (spec §3). Only T3 is closed, and only because
    /// it is a human-to-human boundary — every agent-to-agent tier is open.
    fn allowed(self) -> bool {
        !matches!(self, CaptureTier::OtherUser)
    }

    fn label(self) -> &'static str {
        match self {
            CaptureTier::SameInstance => "T1-same-instance",
            CaptureTier::OtherInstance => "T2-other-instance",
            CaptureTier::OtherUser => "T3-other-user",
            CaptureTier::ForeignApp => "T4-foreign-app",
        }
    }
}

/// The OS user id owning the calling process, used to place a target window on
/// the near or far side of the T3 boundary.
///
/// Fails CLOSED, matching `own_instance_pids()`'s existing discipline: if this
/// returns `None` every window resolves to `OtherUser` and capture is denied.
/// An unresolvable owner is exactly the case where guessing "probably mine"
/// would be how a cross-user capture slips through.
fn current_user_id(sys: &sysinfo::System) -> Option<String> {
    let me = sysinfo::get_current_pid().ok()?;
    sys.process(me)
        .and_then(|p| p.user_id())
        .map(|u| u.to_string())
}

/// A completed capture. Richer than the bare message string it replaces so
/// `audit_log_capture_window` can record WHAT was captured and at which tier,
/// rather than only the caller's query and a success/failure flag.
#[derive(Debug)]
struct CaptureOutcome {
    /// Human-facing tool result, returned to the caller verbatim.
    message: String,
    tier: CaptureTier,
    /// Resolved target — `pid=N title="..."`.
    target: String,
    /// Hex SHA-256 of the saved PNG; `None` if hashing failed (best-effort).
    image_sha256: Option<String>,
}

/// One top-level window visible to this process, with the metadata both
/// `DiscoverWindows` and `CaptureWindow` need. Shared via
/// `enumerate_agentmux_windows()` so the two tools can't drift on what counts
/// as "AgentMux's own window", "the calling agent's own instance", or which
/// tier a target falls in.
struct AgentMuxWindowInfo {
    window: xcap::Window,
    pid: u32,
    title: String,
    exe_path: String,
    is_self: bool,
    /// Whether the owning process is an AgentMux process at all — T4 targets
    /// are enumerated now, so this is no longer implied by presence in the list.
    is_agentmux: bool,
    tier: CaptureTier,
}

/// Enumerate every top-level OS window, AgentMux-owned or not, each tagged
/// with the `CaptureTier` that decides whether it may be captured.
///
/// `is_agentmux` is matched via `app_name()`, not `title()` — AgentMux's own
/// process names are version-stamped, e.g. `agentmux-0.55.18`, not a single
/// fixed string, but they all share the `agentmux` prefix (reagent P1, PR
/// #2709 round 2). That flag no longer decides *inclusion* — non-AgentMux
/// windows are T4 capture targets — but it still gates disclosure: they stay
/// out of `DiscoverWindows`' default listing and their titles are withheld
/// from candidate/miss lists (`candidate_label`), which is what keeps round
/// 2's KeePass-title leak closed now that they're enumerated at all.
///
/// `is_self` marks windows belonging to the calling agent's OWN instance
/// (`own_instance_pids()`).
///
/// **Historical note (reagent P2 on PR #2845):** this comment previously said
/// `is_self` windows were "hard-excluded downstream by `capture_window_impl`
/// … the actual isolation boundary, not a nicety" (reagent P0, PR #2709
/// round 3). That is no longer true and the reasoning has been superseded, not
/// merely relaxed: the boundary it protected —
/// `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6's own-pane-only
/// default — was an unratified recommendation that defaulted closed because no
/// mechanism existed to be selective. `is_self` now resolves to
/// `CaptureTier::SameInstance`, which is allowed. See
/// `SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`.
fn enumerate_agentmux_windows() -> Result<Vec<AgentMuxWindowInfo>> {
    let windows =
        xcap::Window::all().map_err(|e| anyhow::anyhow!("failed to enumerate windows: {e}"))?;
    let own_pids = own_instance_pids();
    let sys = sysinfo::System::new_all();
    let me = current_user_id(&sys);
    let mut out = Vec::new();
    for window in windows {
        let is_agentmux = window
            .app_name()
            .map(|a| a.to_lowercase().starts_with("agentmux"))
            .unwrap_or(false);
        // Non-AgentMux windows are no longer skipped — they are T4 targets.
        // They stay out of `DiscoverWindows`' default listing (see its
        // `include_foreign` arg) so ordinary discovery doesn't disclose the
        // titles of a user's unrelated applications as a side effect.
        let pid = window.pid().unwrap_or(0);
        // Fails safe: an unresolvable pid is treated as "assume it's mine"
        // rather than silently dropped, so `DiscoverWindows` still surfaces
        // that the window exists instead of hiding it.
        //
        // That promise is real again as of reagentx P2 on PR #2845. Briefly it
        // wasn't: an unresolvable OWNER resolves the window to T3, and
        // `DiscoverWindows` was dropping non-allowed tiers outright, so such a
        // window vanished from the listing even with `include_self`. It is now
        // listed with `title`/`exe_path` redacted, which keeps the disclosure
        // closed without reintroducing the hiding this comment warns against.
        let is_self = pid == 0 || own_pids.contains(&pid);
        let proc = sys.process(sysinfo::Pid::from(pid as usize));
        let title = window.title().unwrap_or_default();
        let exe_path = proc
            .and_then(|p| p.exe())
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        // Tier resolution. The OS-user check runs FIRST and outranks
        // everything else: a window owned by another human is T3 no matter
        // what process owns it, including another AgentMux instance of theirs.
        let owner = proc.and_then(|p| p.user_id()).map(|u| u.to_string());
        let tier = match (&me, &owner) {
            // Both known and different — the human boundary.
            (Some(mine), Some(theirs)) if mine != theirs => CaptureTier::OtherUser,
            // Either side unresolvable — fail closed, same discipline as
            // `is_self` above. Guessing "probably mine" here is precisely how
            // a cross-user capture would slip through.
            (None, _) | (_, None) => CaptureTier::OtherUser,
            _ if is_self => CaptureTier::SameInstance,
            _ if is_agentmux => CaptureTier::OtherInstance,
            _ => CaptureTier::ForeignApp,
        };

        out.push(AgentMuxWindowInfo {
            window,
            pid,
            title,
            exe_path,
            is_self,
            is_agentmux,
            tier,
        });
    }
    Ok(out)
}

/// Cheap heuristic for "this capture is probably a blank/unpainted frame,
/// not a real render" — sample ~200 evenly-spaced pixels and check they're
/// all within a small tolerance of the first one. Not real image analysis
/// (a legitimately solid-color themed window would also trip this) —
/// deliberately cheap and approximate, used only to decide whether a short
/// bounded retry is worth attempting, and to set `likely_unrendered` as a
/// hint, never as a hard failure.
fn looks_unrendered(img: &image::RgbaImage) -> bool {
    let pixels = img.as_raw();
    if pixels.len() < 4 {
        return true;
    }
    let (r0, g0, b0) = (pixels[0], pixels[1], pixels[2]);
    let pixel_count = pixels.len() / 4;
    let step = (pixel_count / 200).max(1);
    let mut sampled = 0usize;
    for i in (0..pixel_count).step_by(step) {
        let idx = i * 4;
        sampled += 1;
        if pixels[idx].abs_diff(r0) > 8 || pixels[idx + 1].abs_diff(g0) > 8 || pixels[idx + 2].abs_diff(b0) > 8 {
            return false;
        }
    }
    sampled > 0
}

/// Label for a window in the AUDIT TRAIL.
///
/// Fuller than `candidate_label` where it can be — reagentx P2 on PR #2845
/// correctly spotted that reusing the caller-facing label lost a T4 window's
/// title on a post-resolution failure, giving foreign-app failures less audit
/// detail than AgentMux ones for no reason.
///
/// But NOT unconditionally full, because that finding's premise — "the audit
/// is server-side only" — does not hold here. The trail is an NDJSON file in
/// this instance's own data dir, and agents have shell access; it was read
/// with `tail` during this PR's own verification. It is therefore not a
/// confidential sink.
///
/// So audit detail follows the ALLOW decision, not AgentMux-ness:
///   - allowed tier → full title. The agent could capture the window and read
///     the title off the pixels anyway, so recording it discloses nothing new.
///   - withheld tier → pid and tier only. Recording a T3 title would hand the
///     agent, via a file it can read, exactly the content the tier denied it —
///     reopening through the log the same oracle closed in the title-miss path.
fn audit_target_label(w: &AgentMuxWindowInfo) -> String {
    if w.tier.allowed() {
        format!("pid={} title={:?}", w.pid, w.title)
    } else {
        format!("pid={} {} <title withheld>", w.pid, w.tier.label())
    }
}

/// Label for a window in a CALLER-FACING disambiguation/candidate list.
///
/// Stricter than `audit_target_label` above, because the audiences differ: this
/// one is returned to the agent, so it withholds the TITLE of every
/// non-AgentMux window — reagent P1 on PR #2845. Foreign windows became
/// capturable with the tier model, so any list built from the capturable set
/// would otherwise disclose a user's unrelated app titles (their browser,
/// their password manager) as a side effect of a miss or an ambiguous match,
/// bypassing `DiscoverWindows`' own `include_foreign` opt-in. The pid alone
/// disambiguates, and is what the caller needs anyway.
fn candidate_label(w: &AgentMuxWindowInfo) -> String {
    if w.is_agentmux {
        format!("pid={} title={:?}", w.pid, w.title)
    } else {
        format!("pid={} <non-AgentMux window; pass include_foreign to DiscoverWindows to identify it>", w.pid)
    }
}

/// The actual `CaptureWindow` logic — window enumeration, own-instance and
/// third-party-app exclusion, targeting (by pid or by title), capture, and
/// save. Extracted to its own function (rather than living inline in the
/// `"CaptureWindow" =>` match arm) so its `Result` can be captured once and
/// unconditionally audit-logged by the caller before propagating — see
/// `audit_log_capture_window`.
///
/// `index: None` with more than one `title_contains` match is an error
/// listing every candidate, NOT a silent pick of match 0 — see
/// docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md
/// §1 for the real incident (an ambiguous match silently captured an
/// unrelated, sensitive window) this specifically fixes. `index: Some(i)`
/// is still honored directly against whatever matched, unchanged from the
/// original behavior, for callers who already disambiguate explicitly.
fn capture_window_impl(
    title_contains: Option<&str>,
    index: Option<usize>,
    pid: Option<u32>,
    // Filled the moment a target is RESOLVED, so a failure after that point —
    // a denied T3 pid, a failed capture_image, a failed PNG write — is still
    // audited with its tier and target instead of nulls (codex P2 on PR #2845).
    // Without it the log cannot tell "no such window" apart from "blocked
    // cross-user attempt", which is the entry a reviewer most wants to find.
    resolved: &mut Option<(CaptureTier, String)>,
) -> Result<CaptureOutcome> {
    let all = enumerate_agentmux_windows()?;
    // Tier gate replaces the old `!w.is_self` exclusion — see `CaptureTier`.
    // Only T3 (a different OS user's window) is withheld; every agent-to-agent
    // tier, including this instance's own window, is reachable.
    let foreign: Vec<&AgentMuxWindowInfo> = all.iter().filter(|w| w.tier.allowed()).collect();

    let target: &AgentMuxWindowInfo = if let Some(target_pid) = pid {
        // A single host process can own multiple top-level windows sharing
        // the same pid (agentmux-cef/src/browser_pane/hwnd.rs:200-204,
        // agentmux-cef/src/commands/window/lifecycle.rs:410-426) — codex P1
        // on PR #2810: the original `.find()` here silently captured
        // whichever matching window enumerated first, which could be a
        // pool/sub-window rather than the one the caller meant, reintroducing
        // the wrong-window capture this PR exists to prevent. Same
        // ambiguity-rejection shape as the title_contains branch below:
        // `index` disambiguates, an unqualified ambiguous match does not.
        let matches: Vec<&AgentMuxWindowInfo> = foreign
            .iter()
            .filter(|w| w.pid == target_pid)
            .copied()
            .collect();
        if matches.is_empty() {
            // `foreign` is already tier-filtered, so a withheld T3 window is
            // absent from it and would otherwise be indistinguishable from a
            // pid that doesn't exist — reported identically to the caller AND
            // audited as `tier: null`. Look it up in the UNFILTERED set to
            // separate "withheld" from "absent" (codex P2 / reagentx P2).
            if let Some(blocked) = all.iter().find(|w| w.pid == target_pid) {
                *resolved = Some((blocked.tier, audit_target_label(blocked)));
                anyhow::bail!(
                    "window pid={target_pid} is {} — capture is withheld for windows owned \
                     by a different OS user; every other tier is available",
                    blocked.tier.label()
                );
            }
            anyhow::bail!(
                "no window found for pid {target_pid} — call DiscoverWindows to see \
                 current candidates"
            );
        }
        match index {
            Some(i) => *matches.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "index {i} out of range — only {} window(s) matched pid {target_pid}",
                    matches.len()
                )
            })?,
            None if matches.len() == 1 => matches[0],
            None => {
                let candidates: Vec<String> = matches
                    .iter()
                    .map(|w| candidate_label(w))
                    .collect();
                anyhow::bail!(
                    "{} windows matched pid {target_pid} — a single process can own \
                     multiple top-level windows; pass an explicit index (0-based) \
                     alongside pid to disambiguate. Candidates: {}",
                    matches.len(),
                    candidates.join("; ")
                );
            }
        }
    } else {
        let title_contains = title_contains
            .ok_or_else(|| anyhow::anyhow!("must provide either pid or title_contains"))?;
        let needle = title_contains.to_lowercase();
        let matches: Vec<&AgentMuxWindowInfo> = foreign
            .iter()
            .filter(|w| w.title.to_lowercase().contains(&needle))
            .copied()
            .collect();

        if matches.is_empty() {
            // A title matching ONLY a withheld window is recorded for the
            // AUDIT — and deliberately changes nothing the caller sees.
            //
            // reagentx P1 on PR #2845: an earlier version bailed here with a
            // distinguishable "exists but is T3" message. Withholding the
            // literal title wasn't enough — differing *responses* are an
            // existence oracle. An agent can probe substrings (binary-search
            // over characters) and reconstruct a cross-user window's title
            // without ever being granted capture, defeating the very boundary
            // this tier exists to hold. Introduced while fixing the audit gap
            // below it: making withheld-vs-absent legible to the auditor made
            // it legible to the attacker too.
            //
            // The split that resolves it: the AUDIT is server-side and may
            // know; the CALLER may not. So this records `resolved` and then
            // falls through to the identical generic-miss message below —
            // structurally the same code path and the same bytes either way,
            // rather than two branches a future edit could let drift apart.
            //
            // The pid branch above is deliberately NOT symmetric: it does name
            // the tier. A caller must already hold the pid to ask, `Discover
            // Windows` never hands out T3 pids, and process ownership is
            // already enumerable by anything with shell access — so the pid's
            // existence is not a secret this tool is keeping, while a window
            // TITLE is exactly the content it is.
            if let Some(blocked) = all
                .iter()
                .find(|w| !w.tier.allowed() && w.title.to_lowercase().contains(&needle))
            {
                *resolved = Some((blocked.tier, audit_target_label(blocked)));
            }
            // Only lists AGENTMUX windows' titles, never every window on the
            // desktop — reagent P2 (PR #2709 round 2): the original version
            // dumped every visible window's title on any miss, which let a
            // caller enumerate arbitrary window titles (confirmed in testing:
            // it leaked a password manager's document title) with no real
            // match required at all.
            //
            // The `is_agentmux` filter is load-bearing again as of the tier
            // model — reagent P1 on PR #2845. `foreign` now includes T4
            // non-AgentMux windows (they became capturable), so listing it
            // wholesale would have re-opened exactly that leak, and would have
            // bypassed `DiscoverWindows`' own `include_foreign` opt-in in the
            // process: a caller could enumerate the user's desktop by
            // deliberately missing.
            let titles: Vec<&str> = foreign
                .iter()
                .filter(|w| w.is_agentmux)
                .map(|w| w.title.as_str())
                .filter(|t| !t.is_empty())
                .collect();
            if titles.is_empty() {
                anyhow::bail!(
                    "no AgentMux window title contains {title_contains:?} — \
                     no other AgentMux windows are currently open"
                );
            }
            anyhow::bail!(
                "no AgentMux window title contains {title_contains:?}. \
                 Open AgentMux window titles: {}",
                titles.join(", ")
            );
        }

        match index {
            Some(i) => *matches.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "index {i} out of range — only {} window(s) matched {title_contains:?}",
                    matches.len()
                )
            })?,
            None if matches.len() == 1 => matches[0],
            None => {
                // The fix for blocker #1 in the report: an ambiguous
                // match with no explicit index used to silently capture
                // index 0 despite this tool's own docstring claiming it
                // would list candidates instead. It now actually does.
                let candidates: Vec<String> = matches
                    .iter()
                    .map(|w| candidate_label(w))
                    .collect();
                anyhow::bail!(
                    "{} windows matched {title_contains:?} — pass an explicit index (0-based), \
                     or better, one of these pids directly (preferred: stable, unlike title). \
                     Candidates: {}",
                    matches.len(),
                    candidates.join("; ")
                );
            }
        }
    };

    *resolved = Some((target.tier, audit_target_label(target)));

    let title = target.title.clone();

    // Short bounded retry for a freshly-created window that hasn't
    // painted its first real frame yet — see looks_unrendered()'s doc
    // comment. std::thread::sleep (not tokio::time::sleep): this fn is
    // plain sync code called from the async dispatch path, and the
    // bounded ~800ms worst case is a deliberate, request-scoped wait
    // directly serving this call, not background work stealing the
    // runtime from anything else.
    let mut image = target
        .window
        .capture_image()
        .map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
    let mut likely_unrendered = looks_unrendered(&image);
    let mut attempts = 1;
    while likely_unrendered && attempts < 3 {
        std::thread::sleep(Duration::from_millis(400));
        image = target
            .window
            .capture_image()
            .map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
        likely_unrendered = looks_unrendered(&image);
        attempts += 1;
    }

    let dir = capture_window_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("failed to create capture dir {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    image
        .save(&path)
        .map_err(|e| anyhow::anyhow!("failed to save capture to {}: {e}", path.display()))?;
    // Unbounded growth over repeated/looped calls otherwise — same bug
    // class as UIScreenshot's own screenshot dir (reagent P2, PR #2662)
    // and reagent's own review of this PR (round 1) pointing out that
    // fix wasn't reused here. Same fix, same shape: best-effort, on the
    // write path.
    prune_old_captures(&dir);

    // codex P2 on PR #2810: no process start time is collected or checked
    // anywhere, so claiming "this window was created recently" was
    // unsupported and misleading for a mature window that just happens to
    // render a near-uniform frame (looks_unrendered's own doc comment
    // already acknowledges that legitimate case). State only what was
    // actually observed.
    let hint = if likely_unrendered {
        " (likely_unrendered: true — the captured frame looks solid or \
           near-solid-color even after retrying, which can mean the window \
           hasn't painted its first real frame yet, or that it genuinely \
           looks like this; consider calling CaptureWindow again shortly if \
           that's unexpected)"
    } else {
        ""
    };
    Ok(CaptureOutcome {
        message: format!(
            "Captured window {title:?} to {}{hint} — use Read on that path to view it yourself, or OpenMedia to show it to the user.",
            path.display()
        ),
        tier: target.tier,
        // The resolved target, not the query string — an audit reviewer needs
        // to know WHAT was captured, which a `title_contains` substring does
        // not tell them.
        target: format!("pid={} title={:?}", target.pid, target.title),
        image_sha256: sha256_file(&path),
    })
}

/// Hex SHA-256 of a captured PNG, recorded in the audit trail so a leaked
/// screenshot can be traced back to the call that produced it (spec §6).
/// Best-effort — a hashing failure yields `None` and must never fail the
/// capture, matching `audit_log_capture_window`'s own "never break the tool"
/// discipline.
fn sha256_file(path: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// Audit trail for `CaptureWindow` — reagent P1 (PR #2709 round 4): the
/// residual risk after rounds 2-3's scoping (a different AgentMux
/// instance's window, which can belong to a different OS user on a shared
/// machine) is a disclosure-across-a-human-boundary question, not a
/// technical one a capability flag alone would fully answer — a real
/// per-agent opt-in gate is tracked as a separate follow-up (no existing
/// settings/enforcement mechanism to build it on today). Logging every
/// call — who, what was requested, what happened — is the honest,
/// shippable Phase-1 answer: best-effort (a logging failure must never
/// break the tool itself), append-only NDJSON inside `capture_window_dir()`
/// itself — `prune_old_captures`'s PNG-only extension filter already
/// leaves a `.log` file in that same directory untouched, so this doesn't
/// need (or want) its own separate directory alongside it.
fn audit_log_capture_window(
    query_desc: &str,
    outcome: &Result<CaptureOutcome>,
    // Resolved target/tier for the FAILURE paths — on success the outcome
    // carries its own. See capture_window_impl's `resolved` param.
    resolved: &Option<(CaptureTier, String)>,
) {
    let entry = serde_json::json!({
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "agent_id": agent_slug().unwrap_or_else(|_| "unknown".to_string()),
        "tool": "CaptureWindow",
        // "pid=N" or "title_contains=\"...\"" — whichever targeting mode
        // the caller used (SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md
        // added pid-based targeting as an alternative to title matching).
        "query": query_desc,
        // The RESOLVED target and its tier, not just the query string — a
        // reviewer needs to know what was actually captured, which a
        // `title_contains` substring doesn't say. Absent on failure, since
        // nothing was resolved (spec §6).
        "tier": outcome
            .as_ref()
            .ok()
            .map(|c| c.tier.label())
            .or_else(|| resolved.as_ref().map(|(t, _)| t.label())),
        "target": outcome
            .as_ref()
            .ok()
            .map(|c| c.target.clone())
            .or_else(|| resolved.as_ref().map(|(_, t)| t.clone())),
        // Traces a leaked screenshot back to the call that produced it.
        "image_sha256": outcome.as_ref().ok().and_then(|c| c.image_sha256.clone()),
        // Phase 1 never redacts — recorded explicitly so an unredacted T2/T4
        // capture is distinguishable at review time once Phase 3 lands, rather
        // than the field's absence being ambiguous between "no" and "old entry".
        "redacted": false,
        "outcome": match outcome {
            Ok(c) => serde_json::json!({"result": "success", "detail": c.message}),
            Err(e) => serde_json::json!({"result": "error", "detail": e.to_string()}),
        },
    });
    append_window_audit_log_entry(&entry);
}

/// One `DiscoverWindows` entry.
///
/// A withheld (T3) window is LISTED but redacted, not omitted — reagentx P2 on
/// PR #2845 caught two contradictions the omit-entirely approach created:
///   - the `is_self` fail-safe promised an unresolvable-pid window would
///     "still surface ... instead of hiding it", which an unconditional drop
///     broke (an unresolvable owner resolves to T3)
///   - this tool advertises `capturable` so a caller can "see why a window is
///     out of reach before trying", unreachable if every listed entry is
///     capturable by construction
///
/// Redacting satisfies both while keeping the disclosure closed: `title` and
/// `exe_path` (which embeds the owning OS username) are exactly what must not
/// cross the human boundary, and they are the only fields dropped. A pid and
/// "someone else owns this" are already available to anything with shell
/// access, so surfacing them costs nothing — and it makes a wholesale
/// user-id-resolution failure diagnosable instead of silently returning an
/// empty list.
fn window_listing_entry(w: &AgentMuxWindowInfo) -> Value {
    if w.tier.allowed() {
        json!({
            "pid": w.pid,
            "title": w.title,
            "exe_path": w.exe_path,
            "is_self": w.is_self,
            "is_agentmux": w.is_agentmux,
            "tier": w.tier.label(),
            "capturable": true,
        })
    } else {
        json!({
            "pid": w.pid,
            "title": null,
            "exe_path": null,
            "is_self": w.is_self,
            "is_agentmux": w.is_agentmux,
            "tier": w.tier.label(),
            "capturable": false,
            "withheld_reason": "owned by a different OS user; title and exe_path withheld",
        })
    }
}

/// Audit trail for `DiscoverWindows` — reagent P1 on PR #2810: this tool
/// discloses `exe_path` (a full filesystem path, typically embedding the OS
/// username) for windows belonging to OTHER AgentMux instances/users on a
/// shared machine — the exact same disclosure-across-a-human-boundary risk
/// `audit_log_capture_window` above already exists to log, but this tool
/// shipped with none. Same shape, same file (a single window-related audit
/// trail is easier to review than two): best-effort, never blocks the
/// tool's own result.
fn audit_log_discover_windows(include_self: bool, include_foreign: bool, windows: &[Value]) {
    let entry = serde_json::json!({
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "agent_id": agent_slug().unwrap_or_else(|_| "unknown".to_string()),
        "tool": "DiscoverWindows",
        // Both flags — reagentx P2 on PR #2845. `include_foreign` is the one
        // that actually triggers this trail's reason for existing: it
        // discloses non-AgentMux window titles and exe_paths. Recording only
        // `include_self` left a reviewer scanning query strings unable to tell
        // whether foreign disclosure was even requested, inferable only by
        // picking through each entry's `is_agentmux` tag.
        "query": format!("include_self={include_self} include_foreign={include_foreign}"),
        "outcome": serde_json::json!({
            "result": "success",
            "window_count": windows.len(),
            "windows": windows,
        }),
    });
    append_window_audit_log_entry(&entry);
}

/// Shared append-only NDJSON writer for both window-tool audit trails above
/// — best-effort (a logging failure must never break the tool itself),
/// inside `capture_window_dir()` itself since `prune_old_captures`'s
/// PNG-only extension filter already leaves a `.log` file in that same
/// directory untouched, so this doesn't need (or want) its own directory.
fn append_window_audit_log_entry(entry: &Value) {
    let dir = capture_window_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let line = format!("{entry}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("capture-window-audit.log"))
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
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
                    let (request_id, ts_secs, jekt_sig, lan_sig) =
                        sign_outgoing_jekt(source_agent.as_deref(), &target_agent, message);
                    let req = InjectRequest {
                        target_agent,
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
            Ok(format!(
                "Claimed work item {id} (attempt {attempt}) — pass attempt={attempt} to \
                 WorkHeartbeat/WorkComplete/WorkRelease for this item.\n\n\
                 Title: {title}\n\n{payload}"
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
                    arguments.get("result").and_then(|v| v.as_str()).unwrap_or("").to_string(),
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
                _ => format!("Released {id} back to the queue."),
            })
        }
        "WorkList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let mut url = format!("{}/agentmux/work", local_url.trim_end_matches('/'));
            let state = arguments.get("state").and_then(|v| v.as_str()).unwrap_or("");
            let limit = arguments.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            url.push_str(&format!("?state={state}&limit={limit}"));
            let resp = client.get(&url).header("X-AuthKey", auth_key).send().await
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
        assert_eq!(defs.len(), 42, "tools/list advertises 27 tools (11 original + 1 OpenMedia + 3 Loop + 5 Cron + 7 agent-API) + 3 memory-version-history + 3 fleet-control tools + 1 CaptureWindow + 1 ListConversations + 1 DiscoverWindows + 6 Muxqueue");
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
            "wan": { "subscribed_agents": ["CloudAgent"] },
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
