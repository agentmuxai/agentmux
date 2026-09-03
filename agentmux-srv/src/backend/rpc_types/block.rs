// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! block.* / pane.* request/response types, plus core meta / controller /
//! subprocess / CLI-resolution / blockfile command data types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::backend::oref::ORef;
use crate::backend::obj::{BlockDef, MetaMapType};

/// Matches Go's `CommandGetMetaData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetMetaData {
    pub oref: ORef,
}

/// Matches Go's `CommandSetMetaData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSetMetaData {
    pub oref: ORef,
    pub meta: MetaMapType,
}

/// Matches Go's `CommandMessageData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMessageData {
    #[serde(default)]
    pub oref: ORef,
    pub message: String,
}

/// Matches Go's `CommandCreateBlockData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandCreateBlockData {
    #[serde(default)]
    pub tabid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blockdef: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtopts: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub magnified: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub targetblockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub targetaction: String,
}

/// Matches Go's `CommandDeleteBlockData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteBlockData {
    pub blockid: String,
}

/// Matches Go's `CommandBlockSetViewData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBlockSetViewData {
    pub blockid: String,
    pub view: String,
}

/// Matches Go's `CommandControllerResyncData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandControllerResyncData {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forcerestart: bool,
    #[serde(default)]
    pub tabid: String,
    #[serde(default)]
    pub blockid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtopts: Option<serde_json::Value>,
}

/// Matches Go's `CommandBlockInputData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandBlockInputData {
    pub blockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inputdata64: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signame: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termsize: Option<serde_json::Value>,
    /// Per-TermViewModel monotonic counter for seq-based input ordering (optional, shell only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

/// Matches TS `CommandCreateSubBlockData` (frontend/types/gotypes.d.ts:238-241).
/// Creates a headless sub-block (no tab/layout entry) parented to
/// `parentblockid` — e.g. a `term`-view PTY embedded in an agent
/// pane's details drawer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCreateSubBlockData {
    pub parentblockid: String,
    pub blockdef: BlockDef,
}

/// Matches TS `CommandDeleteBlockData` as used by `DeleteSubBlockCommand`
/// (frontend/app/store/rpc-api/block.ts:64-66) — same shape as a plain
/// block delete, but routed to the sub-block teardown path (kills the
/// controller, deletes the row, unlinks from the parent's `subblockids`;
/// does not touch tab bookkeeping since sub-blocks are never tab-referenced).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteSubBlockData {
    pub blockid: String,
}

/// Data for `tooldecision` — frontend's reply to a per-tool-call
/// permission gate. Today the backend validates the outcome and
/// logs the decision; actual delivery to the agent CLI is deferred
/// to PR-3b/PR-4 (rules persistence vs. interactive subprocess
/// path). Spec:
/// docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §9.1.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandToolDecisionData {
    pub blockid: String,
    /// Opaque id matched against a `PermissionRequestEvent`. Echoed
    /// in the audit log so the audit trail can be cross-referenced.
    pub request_id: String,
    /// "allow" or "deny". Anything else returns an error.
    pub outcome: String,
    /// "once" / "session" / "project" / "global". Captured so the
    /// rules-persistence layer (PR-3b) can write a matching rule
    /// without re-asking the user.
    pub scope: String,
    /// User-typed denial reason. Optional. Future PR will relay
    /// this verbatim into the agent's next prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
}

/// Data for `docknodestatus` — a fire-and-forget push whenever a
/// `ToolNode`'s status changes. Spec:
/// docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.1.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandDockNodeStatusData {
    pub blockid: String,
    pub node_id: String,
    pub tool_name: String,
    pub status: String,
    /// `ToolNode.timestamp` (ms), if the pushing client had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// `params.run_in_background === true` on the pushing client's own
    /// `ToolNode`, if it's a Bash call. See
    /// `DockNodeSnapshot::run_in_background`'s doc comment (issue #2518).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_in_background: Option<bool>,
}

/// Data for `COMMAND_BACKGROUND_TASK_COMPLETION` — a declared-background
/// task's real terminal outcome, parsed client-side from its
/// `<task-notification>` message (not any change to the originating
/// `ToolNode.status`, which stays `"success"` forever once the launch is
/// accepted — that's the raw tool_result's own outcome, not the background
/// task's). Deliberately a separate command from `CommandDockNodeStatusData`
/// above: this fires for a `user_message` node, which has no `tool_name`/
/// raw `ToolNode.status` of its own, and `DockSnapshotCache::push_delta` is
/// a full per-node overwrite — routing a partial payload through it would
/// blank the original tool node's `run_in_background`/`tool_name` (the
/// exact bug class #2520 already fixed once for a different call site).
/// `node_id` is the ORIGINATING tool call's node_id/tool_use_id (the join
/// key back to the `db_background_tasks` row `docknodestatus` created), not
/// this notification message's own id.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandBackgroundTaskCompletionData {
    pub blockid: String,
    pub node_id: String,
    /// One of "done" | "error" | "stopped" — the same `ActivityStatus`
    /// vocabulary `tool-adapter.ts`'s `parseTaskNotification` already maps
    /// `<status>completed|failed|*</status>` onto client-side.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

/// Data for `COMMAND_BACKGROUND_TASK_PID` — a declared-background task's
/// real OS pid, relayed from `agentmux-bashwrap`'s own WPS `"pid"` chunk.
/// `node_id` is the originating tool call's node_id/tool_use_id, same join
/// key as `CommandBackgroundTaskCompletionData` above. See
/// docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandBackgroundTaskPidData {
    pub blockid: String,
    pub node_id: String,
    pub pid: u32,
}

/// Data for `COMMAND_LIST_BACKGROUND_TASKS` — request/response, returns
/// this block's current `db_background_tasks` rows (as
/// `muxspect_handlers::BackgroundTaskView`s). See
/// docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.1.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandListBackgroundTasksData {
    pub blockid: String,
}

/// Data for AgentAnswerCommand — an AskUserQuestion answer delivered back to
/// the running agent CLI via the Agent SDK control protocol (a `control_response`
/// carrying `updatedInput.answers`). Spec:
/// docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandAgentAnswerData {
    pub blockid: String,
    /// The `AskUserQuestion` tool_use id the answer responds to (correlates with
    /// the parked `can_use_tool` control_request).
    pub tool_use_id: String,
    /// The user's selections as a JSON object mapping each question's text to the
    /// chosen option label (a `[labels]` array for multiSelect, or free-text for
    /// "Other"). Becomes `updatedInput.answers` in the control_response.
    #[serde(default)]
    pub answers: serde_json::Value,
}

/// Data for AgentCancelCommand — a real protocol-level decline of a pending
/// AskUserQuestion (Cancel button / Escape), delivered as a control_response
/// carrying `behavior: "deny"` rather than the allow+answers shape above. No
/// `answers` field: there is nothing to carry, the deny message is a fixed
/// server-owned string (see `ASK_USER_QUESTION_DENY_MESSAGE` in
/// blockcontroller/persistent.rs). Spec:
/// docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandAgentCancelData {
    pub blockid: String,
    /// The `AskUserQuestion` tool_use id being declined (correlates with the
    /// parked `can_use_tool` control_request).
    pub tool_use_id: String,
}

// ---- Subprocess agent command data types ----

/// Data for SubprocessSpawnCommand — spawn agent CLI for a single turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSubprocessSpawnData {
    pub blockid: String,
    pub tabid: String,
    pub cli_command: String,
    #[serde(default)]
    pub cli_args: Vec<String>,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub env_vars: std::collections::HashMap<String, String>,
    /// The user's JSON message to write to subprocess stdin.
    pub message: String,
}

/// Data for AgentInputCommand — send a follow-up message (re-spawns with --resume).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentInputData {
    pub blockid: String,
    /// The user's JSON message string.
    pub message: String,
    /// Optional client-supplied id. Echoed back via the
    /// `agent-message-accepted` event when this message transitions
    /// from queued to running so the frontend can match its pending
    /// `PendingMessage` entry and promote it into the conversation
    /// document. Absent for pre-existing callers; treated as no-id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Data for AgentStopCommand — stop the running subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentStopData {
    pub blockid: String,
    #[serde(default)]
    pub force: bool,
}

/// Data for ShellExecCommand — run a shell command in the agent's working directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandShellExecData {
    pub blockid: String,
    pub command: String,
    #[serde(default)]
    pub working_dir: String,
}

/// Result of ShellExecCommand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Data for ShellStopCommand — stop a running persistent shell node by id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandShellStopData {
    pub shell_id: String,
}

/// Data for ShellStatusCommand — query a shell's current running state by id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandShellStatusData {
    pub shell_id: String,
}

/// A file to write as part of agent config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigFile {
    pub path: String,
    pub content: String,
}

/// Data for WriteAgentConfigCommand — write config files atomically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandWriteAgentConfigData {
    /// Agent working directory where files are written.
    pub working_dir: String,
    /// Files to write (path relative to working_dir, content).
    pub files: Vec<AgentConfigFile>,
    /// When true, treat `working_dir` as an auto-generated instance
    /// path eligible for `<base>-N` collision resolution. When false
    /// (user-specified `agent.working_directory` like `~/projects/X`),
    /// write into the path as-is — no rewrite, no suffixing. The
    /// frontend sets this based on whether it constructed the path
    /// itself or pulled it from the agent definition.
    #[serde(default)]
    pub auto_allocate: bool,
}

/// Result of WriteAgentConfigCommand. Returns the final working
/// directory used; callers should compare against the requested
/// `working_dir` and patch `cmd:cwd` (via SetMeta) when they differ
/// so the controller spawns the CLI in the actually-created dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandWriteAgentConfigResult {
    pub working_dir: String,
}

/// Data for ResolveCliCommand — detect or install a CLI tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResolveCliData {
    /// Provider ID (e.g. "claude", "codex", "gemini")
    pub provider_id: String,
    /// CLI command name (e.g. "claude")
    pub cli_command: String,
    /// npm package name for fallback install (e.g. "@anthropic-ai/claude-code")
    pub npm_package: String,
    /// Version to install ("latest" or specific version)
    pub pinned_version: String,
    /// Windows install command (e.g. "irm https://claude.ai/install.ps1 | iex")
    #[serde(default)]
    pub windows_install_command: String,
    /// Unix install command (e.g. "curl -fsSL https://claude.ai/install.sh | bash")
    #[serde(default)]
    pub unix_install_command: String,
    /// Block ID to stream install output into (optional — if empty, no streaming)
    #[serde(default)]
    pub block_id: String,
}

/// Result from ResolveCliCommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveCliResult {
    /// Absolute path to the CLI binary
    pub cli_path: String,
    /// CLI version string
    pub version: String,
    /// How it was resolved: "path", "local_install", "installed"
    pub source: String,
}

/// Data for CheckCliAuthCommand — check if CLI is authenticated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCheckCliAuthData {
    /// Absolute path to CLI binary
    pub cli_path: String,
    /// Auth check args (e.g. ["auth", "status", "--json"])
    pub auth_check_args: Vec<String>,
    /// Environment variables to set when running the auth check (e.g. CLAUDE_CONFIG_DIR).
    /// Must match the env vars used when spawning the actual subprocess so the check
    /// reads credentials from the same isolated directory.
    #[serde(default)]
    pub auth_env: std::collections::HashMap<String, String>,
}

/// Result from CheckCliAuthCommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckCliAuthResult {
    pub authenticated: bool,
    pub email: Option<String>,
    pub auth_method: Option<String>,
    /// Raw stdout from auth check command
    pub raw_output: String,
}

/// Input for RunCliLoginCommand — spawns the CLI login flow and extracts the OAuth URL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRunCliLoginData {
    pub cli_path: String,
    pub login_args: Vec<String>,
    #[serde(default)]
    pub auth_env: HashMap<String, String>,
}

/// Result from RunCliLoginCommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCliLoginResult {
    /// OAuth URL extracted from the CLI's output (open in browser)
    pub auth_url: Option<String>,
    pub raw_output: String,
}

/// Request for pane.open — create a new pane showing the given view.
///
/// Supported views: `editor`, `term`, `browser`, `sysinfo`, `help`.
/// `file` is required for `editor`; `url` is required for `browser`.
/// Placement: if `split_direction` ("right" / "left" / "down" / "up")
/// and `split_reference_block_id` are provided, the new pane splits
/// relative to that block. Otherwise it is inserted at the tab root.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandPaneOpenData {
    pub view: String,
    pub file: Option<String>,
    pub url: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub tab_id: Option<String>,
    pub split_direction: Option<String>,
    pub split_reference_block_id: Option<String>,
    pub focus: Option<bool>,
    /// `editor` only: initial file-tree sidebar state. `Some(false)` opens the
    /// editor with its tree collapsed (just the file, no explorer). Written to
    /// `block.meta["editor:tree_expanded"]`, which the frontend `EditorViewModel`
    /// restores on init. Absent / `Some(true)` → the frontend default (expanded).
    pub tree_expanded: Option<bool>,
    /// `Some(true)` opens the pane as a floating window instead of a docked
    /// split. The block is created then moved into a fresh floating workspace
    /// via the `tear_off_block` saga; the launcher broadcasts an
    /// `openfloatingpane` directive scoped to the source window, whose frontend
    /// calls the host `open_floating_pane_window` command to materialize the OS
    /// window. `split_direction` / `split_reference_block_id` are ignored when
    /// floating. See docs/specs/SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md.
    pub floating: Option<bool>,
    /// When present, used as the block meta directly instead of going through
    /// `build_pane_meta`. Allows callers with a complete blockdef (e.g. widget
    /// bar actions) to bypass the view-specific argument validation that
    /// `build_pane_meta` enforces. `view` must still be set to the canonical
    /// view string so the block is routed to the correct renderer.
    pub meta: Option<MetaMapType>,
    /// `Some(true)` creates the block through the reducer (same as the docked
    /// path) but skips BOTH the layout-placement step AND the floating path's
    /// `tear_off_block` saga — the block exists (and the frontend's WOS cache
    /// knows about it) but isn't rendered anywhere yet. `split_direction` /
    /// `split_reference_block_id` are ignored when set (there's no placement
    /// to direct). Review finding: `floating` is checked BEFORE this field
    /// in `open_pane` (the floating branch returns first), so `floating`
    /// takes precedence if a caller ever sets both — these two are meant to
    /// be mutually exclusive (skip_placement = no window of its own at all;
    /// floating = its own OS window), no legitimate caller sets both, but
    /// documenting actual precedence rather than claiming `floating` is
    /// "ignored" here, which it isn't. For a
    /// caller that's about to attach the new block to an existing pane's
    /// block-stack instead of giving it its own tile (in-pane tabs —
    /// see `frontend/layout/lib/layoutStack.ts`'s `pushBlockOntoStack`,
    /// docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.2).
    pub skip_placement: Option<bool>,
    /// `Some(true)`, `view: "editor"` only: if the caller (identified by
    /// `split_reference_block_id`) already has an Editor pane open in its
    /// own tab, push `file` into that pane as a new tab instead of creating
    /// a second Editor pane. Explicit opt-in, set only by the `OpenEditor`
    /// MCP tool — NOT inferred from `meta`/`split_reference_block_id` being
    /// present, since other legitimate callers of this same `pane.open` RPC
    /// (`EditorViewModel.openToTheSide`/`openInTerminal`,
    /// `frontend/app/view/editor/editor-model.ts:958-984`) also set
    /// `split_reference_block_id` to their OWN block id purely for split
    /// placement and must NOT trigger reuse (reagent P1 on PR #2404 — an
    /// earlier version of this field inferred intent from `meta.is_none()`,
    /// which incorrectly reused the calling pane itself for
    /// `openToTheSide`). Ignored when `floating` is `Some(true)` — a
    /// floating request always gets its own new window, never reuses a
    /// docked pane. See
    /// docs/specs/SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md
    /// Part 2.
    pub reuse_editor_pane: Option<bool>,
}

/// Response from pane.open.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneOpenResult {
    pub block_id: String,
    pub tab_id: String,
    pub view: String,
    pub created: bool,
}

/// Request for blockfile:line_count — count total lines in a blockfile.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileLineCountData {
    pub block_id: String,
    pub filename: String,
}

/// Response from blockfile:line_count.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BlockfileLineCountResult {
    pub count: u64,
}

/// Request for blockfile:read_range — read a range of lines from a blockfile.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileReadRangeData {
    pub block_id: String,
    pub filename: String,
    pub offset: u64,
    pub limit: u64,
}

/// Response from blockfile:read_range.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BlockfileReadRangeResult {
    pub lines: Vec<String>,
    pub total: u64,
    /// Receive-time stamps (unix ms) parallel to `lines`, joined from the
    /// `output.tsidx` sidecar; `0` = unknown for that line. Absent entirely
    /// when no sidecar exists or the request didn't take the `output.idx`
    /// fast path — old frontends ignore it, new frontends tolerate absence.
    /// Spec: SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §4.4.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stamps: Option<Vec<i64>>,
}

/// Request for blockfile:read_state — read a sidecar JSON file
/// (e.g. `output.state.json`) associated with a block.
/// Spec: docs/specs/SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileReadStateData {
    pub block_id: String,
    /// Sidecar filename — e.g. "output.state.json". Resolved within the
    /// block's filestore directory; must not contain path separators.
    pub filename: String,
}

/// Response from blockfile:read_state. `content` is the raw file bytes
/// as a UTF-8 string, or null if the sidecar does not exist.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BlockfileReadStateResult {
    pub content: Option<String>,
}

/// Request for blockfile:write_state — atomically write a sidecar JSON
/// file for a block. Uses tmp + fsync + rename to guarantee partial
/// writes never surface to readers.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileWriteStateData {
    pub block_id: String,
    pub filename: String,
    pub content: String,
}

/// Response from blockfile:write_state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BlockfileWriteStateResult {
    pub bytes_written: u64,
}
