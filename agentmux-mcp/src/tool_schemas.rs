// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! MCP tool schemas — the `inputSchema` JSON advertised to the client, one
//! `const` per tool.
//!
//! Split out of `main.rs` (audit step 17,
//! `docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md`). This is
//! pure data: every const is byte-identical to the one it replaced, and the
//! declaration order is unchanged, so the tool list this crate advertises is
//! the same. Keeping it out of `main.rs` means a schema edit no longer
//! shows up as a diff against the file that also holds the dispatcher.
//!
//! Adding a tool: add the const here, then register it in `main.rs`'s
//! `tools/list` response and handle it in `call_tool`.

pub(crate) const SHELL_TOOL: &str = r#"{
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

pub(crate) const SHELL_STOP_TOOL: &str = r#"{
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

pub(crate) const SHELL_INPUT_TOOL: &str = r#"{
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

pub(crate) const SHELL_STATUS_TOOL: &str = r#"{
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

pub(crate) const PTY_SHELL_TOOL: &str = r#"{
  "name": "PtyShell",
  "description": "Open a REAL PTY-backed interactive shell — unlike Shell(), which runs on a plain pipe, this behaves like a real terminal, so programs that check for one (password/wizard prompts, sudo, ssh, REPLs) work instead of refusing or misbehaving. Returns a shell_id immediately; use PtyShellInput to type into it, PtyShellRead to see its output, PtyShellStatus to poll, PtyShellStop to end it. Works with no UI involved at all — the window need not be open, focused, or rendering anything for this to work; it's a plain backend call, not simulated keystrokes into a visible terminal.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "cwd":  { "type": "string", "description": "Working directory (defaults to agent workdir)" },
      "rows": { "type": "integer", "description": "Initial terminal rows (default 25)" },
      "cols": { "type": "integer", "description": "Initial terminal columns (default 200)" }
    }
  }
}"#;

pub(crate) const PTY_SHELL_INPUT_TOOL: &str = r#"{
  "name": "PtyShellInput",
  "description": "Write raw text to a PtyShell()'s PTY, as if typed at a keyboard. Unlike ShellInput, no newline is appended — send exactly what a keypress would produce (e.g. \"y\\n\" to answer a prompt and press Enter, or \"\\u0003\" for Ctrl+C — the OS PTY layer delivers that as a real interrupt to the foreground process, same as a human pressing Ctrl+C, without ending the shell).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "shell_id": { "type": "string", "description": "The shell_id returned by a prior PtyShell() call" },
      "text":     { "type": "string", "description": "Raw text to write — no newline is added automatically" }
    },
    "required": ["shell_id", "text"]
  }
}"#;

pub(crate) const PTY_SHELL_RESIZE_TOOL: &str = r#"{
  "name": "PtyShellResize",
  "description": "Resize a PtyShell()'s terminal. Some interactive programs render differently or wrap badly at the default geometry (25 rows x 200 cols).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "shell_id": { "type": "string", "description": "The shell_id returned by a prior PtyShell() call" },
      "rows":     { "type": "integer", "description": "New row count" },
      "cols":     { "type": "integer", "description": "New column count" }
    },
    "required": ["shell_id", "rows", "cols"]
  }
}"#;

pub(crate) const PTY_SHELL_READ_TOOL: &str = r#"{
  "name": "PtyShellRead",
  "description": "Read back a PtyShell()'s output tail (default last 200 lines). This is a raw text log, not a rendered screen — a full-screen program that redraws in place (progress bars, cursor-addressed UIs) won't read back exactly as it visually renders, but line-oriented prompts (the common case) read back cleanly.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "shell_id":   { "type": "string", "description": "The shell_id returned by a prior PtyShell() call" },
      "tail_lines": { "type": "integer", "description": "Number of trailing lines to return (default 200)" }
    },
    "required": ["shell_id"]
  }
}"#;

pub(crate) const PTY_SHELL_STATUS_TOOL: &str = r#"{
  "name": "PtyShellStatus",
  "description": "Query whether a PtyShell() is still running, and its exit code once it has ended.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "shell_id": { "type": "string", "description": "The shell_id returned by a prior PtyShell() call" }
    },
    "required": ["shell_id"]
  }
}"#;

pub(crate) const PTY_SHELL_STOP_TOOL: &str = r#"{
  "name": "PtyShellStop",
  "description": "Kill a PtyShell() and clean up its terminal. Prefer this over letting it linger once you're done with an interactive session.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "shell_id": { "type": "string", "description": "The shell_id returned by a prior PtyShell() call" }
    },
    "required": ["shell_id"]
  }
}"#;

pub(crate) const SEND_MESSAGE_TOOL: &str = r#"{
  "name": "SendMessage",
  "description": "Send a message to another agent by name. The message is injected as input into the target agent's active conversation. Use for agent-to-agent coordination — handoff, task delegation, status notifications. Delivery is best-effort and tries local → same-host → LAN → cloud in order. The first three tiers return only once the message has actually been injected; the cloud tier is store-and-forward, so success there means the relay accepted it and the recipient's AgentMux will pick it up when it next syncs (which never happens if that instance stays offline). The return value names which happened: \"Delivered to X\" means it was injected into a live conversation; \"QUEUED for X\" means only the relay has it. An agent name that exists nowhere also returns QUEUED, so treat that as 'unconfirmed', not 'sent' — check DiscoverAgents if you expected local delivery.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "to":      { "type": "string", "description": "Name of the target agent (its AGENTMUX_AGENT_ID value)" },
      "message": { "type": "string", "description": "Message text to inject into the target agent's conversation" }
    },
    "required": ["to", "message"]
  }
}"#;

pub(crate) const DISCOVER_AGENTS_TOOL: &str = r#"{
  "name": "DiscoverAgents",
  "description": "List the agents and instances reachable from here across the muxbus delivery tiers (host, LAN, cloud), so you can pick a valid target before SendMessage. Returns JSON: host.addressable (agents reachable right now via local delivery), host.agents (this host's agent directory, each with an `addressable` flag), lan (LAN peers and the agents on them), and wan.local_agents_subscribed (THIS host's own agents that are subscribed to the cloud relay — not a list of remote agents you can reach over WAN; the relay exposes no such directory, so a cloud-only target simply won't appear anywhere in this output). Addressing is by agent name, case-insensitive. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

pub(crate) const GET_AGENT_TRANSCRIPT_TOOL: &str = r#"{
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
pub(crate) const LIST_CONVERSATIONS_TOOL: &str = r#"{
  "name": "ListConversations",
  "description": "See every agent's most recent activity across host, cross-channel (other AgentMux channels on this same host), LAN, and connected WAN in one call — a faster alternative to DiscoverAgents + N x GetAgentTranscript. Host and cross-channel entries include turn_active, last_activity_ms, and a last_message_preview (tail transcript line). LAN and WAN entries are liveness-only (remote_fetch_required: true) — reading their conversation content isn't supported yet. Read-only. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

pub(crate) const SUPERVISOR_NUDGE_TOOL: &str = r#"{
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

pub(crate) const SET_ACTIVE_TAB_TOOL: &str = r#"{
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

pub(crate) const NEW_TAB_TOOL: &str = r#"{
  "name": "NewTab",
  "description": "Open a new tab in your own workspace and switch to it. Optionally name it; otherwise AgentMux auto-names it.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name": { "type": "string", "description": "Optional name for the new tab" }
    }
  }
}"#;

pub(crate) const FOCUS_WINDOW_TOOL: &str = r#"{
  "name": "FocusWindow",
  "description": "Bring an AgentMux window to the foreground. Defaults to your own window. Pass window_id (from Layout(query:\"windows\") or Layout(query:\"layout\")) to focus a specific one.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "window_id": { "type": "string", "description": "Optional window to focus; defaults to your own" }
    }
  }
}"#;

pub(crate) const UI_SCREENSHOT_TOOL: &str = r#"{
  "name": "UIScreenshot",
  "description": "Capture a screenshot of your OWN AgentMux pane's UI — clipped to just that pane, not the whole shared window (can't see other panes/agents). Returns a file path; use the Read tool on that path to view the image yourself, or OpenMedia to show it to the user. Use this to visually verify a UI change (e.g. after UIClick-ing a button).",
  "inputSchema": { "type": "object", "properties": {} }
}"#;

pub(crate) const UI_CLICK_TOOL: &str = r#"{
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

pub(crate) const UI_QUERY_TOOL: &str = r#"{
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
pub(crate) const CAPTURE_WINDOW_TOOL: &str = r#"{
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

pub(crate) const DISCOVER_WINDOWS_TOOL: &str = r#"{
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
pub(crate) const FLEET_LIST_TOOL: &str = r#"{
  "name": "FleetList",
  "description": "List every agent reachable from here (same data as DiscoverAgents), framed for fleet targeting: each entry's block_id is what FleetBroadcast/FleetBulkStop expect in their `targets` array. Use this before either of those to build your target list. Read-only. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

pub(crate) const FLEET_BROADCAST_TOOL: &str = r#"{
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

pub(crate) const OPEN_AGENT_TOOL: &str = r#"{
  "name": "OpenAgent",
  "description": "Launch an agent into a pane by name or definition id (get these from FleetList/DiscoverAgents). Idempotent: if the agent is already open in the target tab, returns its existing pane with created:false instead of opening a second one. This spawns a real provider process (and, for container agents, execs into their container) — it consumes provider tokens like any human-opened pane. Returns JSON {block_id, tab_id, agent_id, provider, controller_type, status, created}. After a successful open the agent becomes addressable for SendMessage.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string", "description": "Agent definition id or name (case-insensitive), e.g. from FleetList's host.agents[]" },
      "tab_id": { "type": "string", "description": "Optional target tab id; defaults to the active tab" },
      "focus": { "type": "boolean", "description": "Optional: focus the new pane (default true)" }
    },
    "required": ["agent_id"]
  }
}"#;

pub(crate) const FLEET_BULK_STOP_TOOL: &str = r#"{
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
pub(crate) const LAYOUT_TOOL: &str = r#"{
  "name": "Layout",
  "description": "Read the AgentMux UI structure around you. `query` selects what to return: \"layout\" (the full tree — every window with its workspace, tabs, and panes [block_id, view, title], and which tab is active), \"windows\" (window_id, display name, assigned workspace), \"workspaces\" (workspace_id, name, tab count, active tab), or \"tabs\" (tabs in your own workspace: tab_id, name, pane count). Read-only; use before naming or focusing things. For your OWN ids (block/tab/window/workspace), use WhoAmI instead.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "enum": ["layout", "windows", "workspaces", "tabs"], "description": "What to return (default: layout)" }
    }
  }
}"#;

pub(crate) const WHOAMI_TOOL: &str = r#"{
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
pub(crate) const SET_NAME_TOOL: &str = r#"{
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

pub(crate) const OPEN_EDITOR_TOOL: &str = r#"{
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

pub(crate) const OPEN_MEDIA_TOOL: &str = r#"{
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

pub(crate) const LOOP_TOOL: &str = r#"{
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

pub(crate) const LOOP_STOP_TOOL: &str = r#"{
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

pub(crate) const LOOP_LIST_TOOL: &str = r#"{
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

pub(crate) const WORK_ENQUEUE_TOOL: &str = r#"{
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

pub(crate) const WORK_CLAIM_TOOL: &str = r#"{
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

pub(crate) const WORK_HEARTBEAT_TOOL: &str = r#"{
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

pub(crate) const WORK_COMPLETE_TOOL: &str = r#"{
  "name": "WorkComplete",
  "description": "Mark a claimed Muxqueue item finished. Requires the 'attempt' number from your WorkClaim response; a completion from a superseded claim is rejected (HTTP 409) so a slow agent cannot close out work another agent has since taken over. Record what you actually did in 'result' — it is the only trace of the work once the item is done.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id":      { "type": "string",  "description": "The item id from WorkClaim" },
      "attempt": { "type": "integer", "description": "The 'attempt' number returned by the WorkClaim that gave you this item" },
      "result":  { "type": "string",  "description": "REQUIRED. What was done, or what the outcome was. Include links (PR, issue) where relevant. Once an item is done this is the only record that it happened — a completion with no result silently destroys the trace." }
    },
    "required": ["id", "attempt", "result"]
  }
}"#;

pub(crate) const WORK_RELEASE_TOOL: &str = r#"{
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

pub(crate) const WORK_LIST_TOOL: &str = r#"{
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

pub(crate) const CRON_CREATE_TOOL: &str = r#"{
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

pub(crate) const CRON_DELETE_TOOL: &str = r#"{
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

pub(crate) const CRON_LIST_TOOL: &str = r#"{
  "name": "CronList",
  "description": "List all persistent AgentMux cross-agent cron jobs (created via this same mcp__agentmux__ tool family, not the native per-session ones). Returns each job's id, name, expression, next fire time, fire count, and enabled state.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

pub(crate) const CRON_PAUSE_TOOL: &str = r#"{
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

pub(crate) const CRON_RESUME_TOOL: &str = r#"{
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

pub(crate) const MEMORY_LIST_TOOL: &str = r#"{
  "name": "MemoryList",
  "description": "List your own native memory (brain) markdown files. Returns each file's filename, whether it is the index, its metadata_type, size in bytes, and last-modified time. Use it to see what you've remembered before reading or writing a specific file. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

pub(crate) const MEMORY_READ_TOOL: &str = r#"{
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

pub(crate) const MEMORY_WRITE_TOOL: &str = r#"{
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

pub(crate) const MEMORY_HISTORY_TOOL: &str = r#"{
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

pub(crate) const MEMORY_DIFF_TOOL: &str = r#"{
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

pub(crate) const MEMORY_REVERT_TOOL: &str = r#"{
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

pub(crate) const PRESET_LIST_TOOL: &str = r#"{
  "name": "PresetList",
  "description": "List the presets available to you (summary fields only). A preset is a provider-agnostic config bundle — instructions, context files, MCP servers, and skills. Use it to discover presets before fetching one in full with PresetGet. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

pub(crate) const PRESET_GET_TOOL: &str = r#"{
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

pub(crate) const IDENTITY_ACCOUNTS_TOOL: &str = r#"{
  "name": "IdentityAccounts",
  "description": "List your own linked identity accounts. Returns each account's account_id, provider, name, kind, status, masked_tail, and updated_at. Secrets are never returned — only masked tails. Use it to see which provider accounts you can use and to get account_ids for IdentityValidate. Takes no arguments.",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}"#;

pub(crate) const IDENTITY_VALIDATE_TOOL: &str = r#"{
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
