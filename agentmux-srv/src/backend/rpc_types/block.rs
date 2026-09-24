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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
    /// Opt OUT of `resync_controller`'s default "revive a `STATUS_DONE`
    /// controller" behaviour, surfacing `RESYNC_ERR_ALREADY_EXITED` instead.
    ///
    /// That default is right for this command's original caller — `term.tsx`'s
    /// `TermResyncHandler`, whose whole job is transparently reviving a shell
    /// that died in a crash or backend restart. It is wrong for a caller that
    /// is only ASKING whether the shell is still alive: the agent pane's
    /// drawer reattaching on open (`AgentShellSubblock.tsx`) would otherwise
    /// silently resurrect a shell the human had deliberately `exit`ed,
    /// appending a fresh banner to the block's append-only `term` file — the
    /// respawn-loop symptom SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md fixed on
    /// the `PtyShellCreate` side (§7-8) and this closes on the drawer side.
    ///
    /// Defaults to `false`, so every existing caller keeps the old behaviour
    /// without sending the field. Codex P2 on PR #3253.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub norespawn: bool,
}

/// Matches Go's `CommandBlockInputData`
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandBlockInputData {
    pub blockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inputdata64: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signame: String,
    /// `Option<Value>` in Rust because this handler forwards it to the PTY
    /// layer without inspecting it, but the shape IS known and the hand-written
    /// declaration said so. Spelled inline rather than as `TermSize` because
    /// that name is still an ambient global -- `#[ts(type)]` bypasses ts-rs
    /// dependency tracking, so a named reference would generate an import that
    /// resolves to nothing (the `DroneRun.block_states` trap from #3329).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "{ rows: number, cols: number }")]
    pub termsize: Option<serde_json::Value>,
    /// Per-TermViewModel monotonic counter for seq-based input ordering (optional, shell only).
    ///
    /// The hand-written `CommandBlockInputData` OMITTED this field entirely,
    /// so no TypeScript caller could pass it without casting -- the generated
    /// type surfaces it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub seq: Option<u64>,
}

/// Matches TS `CommandCreateSubBlockData` (frontend/types/srv-types.d.ts:238-241).
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
/// permission gate. The backend validates the payload and delivers it
/// to the agent CLI over the Agent SDK control protocol
/// (`PersistentSubprocessController::decide_tool_permission`) — real
/// delivery, not just logging, as of the Phase 2 plumbing landing.
/// **Still inert in practice**: nothing populates a pending decision
/// yet, because `should_route_to_decision_panel` (persistent.rs) is
/// hardcoded false pending a product decision on which tools/modes
/// should prompt at all (see that function's own doc comment — the
/// spec's own §7 flags "permission chatter" as a real regression risk,
/// not a hypothetical one). Spec:
/// docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §9.1.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandToolDecisionData {
    pub blockid: String,
    /// The `tool_use_id` of the pending `can_use_tool` control_request —
    /// despite the field name, this does NOT correlate against the
    /// speculative `PermissionRequestEvent` §4.1 describes (that event
    /// type was never built). It is what
    /// `PersistentInner::pending_permissions` and
    /// `decide_tool_permission` key on, the same identifier
    /// AskUserQuestion's `awaiting_answer` flow already correlates on.
    /// Kept named `request_id` rather than renamed to avoid an
    /// unrelated frontend/TS-binding churn for a field whose producer
    /// (the still-unbuilt frontend event wiring) doesn't exist yet.
    pub request_id: String,
    /// "allow" or "deny". Anything else returns an error.
    #[ts(type = "\"allow\" | \"deny\"")]
    pub outcome: String,
    /// "once" / "session" / "project" / "global". Validated, but not yet
    /// acted on — no rules-persistence layer (§6, `permissions.json`)
    /// exists anywhere in this tree yet. A future PR that builds one can
    /// trust this value without re-validating it.
    #[ts(type = "\"once\" | \"session\" | \"project\" | \"global\"")]
    pub scope: String,
    /// User-typed denial reason. Optional. Delivered verbatim to the
    /// agent CLI as the `deny` `control_response`'s `message`
    /// (`SPEC_DECISION_PROMPT_2026_04_24.md` G6) when present; a generic
    /// default is used otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub feedback: Option<String>,
}

/// Data for `ambientnarrate` — ask the backend to generate a short
/// user-facing line describing something AgentMux just did on its own.
///
/// Best-effort: the caller does not await a reply and must not gate any UI
/// state change on one. If narration is capped, cancelled, or fails, the thing
/// being narrated still happened and the UI must already show it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAmbientNarrateData {
    pub blockid: String,
    /// Selects the prompt. Unknown kinds are a deliberate no-op rather than an
    /// unconstrained prompt — see `narration_prompt`.
    pub kind: String,
    /// What to narrate, in the caller's own words (e.g. the command line of the
    /// call that just went background).
    pub context: String,
    /// De-duplication key, unique per narrated event (the `node_id` for a
    /// background launch). A node can be re-observed; narrating twice for one
    /// event would be both noisy and a wasted model call.
    pub dedupe_key: String,
}

/// Data for `docknodestatus` — a fire-and-forget push whenever a
/// `ToolNode`'s status changes. Spec:
/// docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.1.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandDockNodeStatusData {
    pub blockid: String,
    pub node_id: String,
    pub tool_name: String,
    pub status: String,
    /// `ToolNode.timestamp` (ms), if the pushing client had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub timestamp: Option<i64>,
    /// `params.run_in_background === true` on the pushing client's own
    /// `ToolNode`, if it's a Bash call. See
    /// `DockNodeSnapshot::run_in_background`'s doc comment (issue #2518).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandBackgroundTaskCompletionData {
    pub blockid: String,
    pub node_id: String,
    /// One of "done" | "error" | "stopped" — the same `ActivityStatus`
    /// vocabulary `tool-adapter.ts`'s `parseTaskNotification` already maps
    /// `<status>completed|failed|*</status>` onto client-side.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub timestamp: Option<i64>,
}

/// Data for `COMMAND_BACKGROUND_TASK_PID` — a declared-background task's
/// real OS pid, relayed from `agentmux-bashwrap`'s own MPS `"pid"` chunk.
/// `node_id` is the originating tool call's node_id/tool_use_id, same join
/// key as `CommandBackgroundTaskCompletionData` above. See
/// docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandBackgroundTaskPidData {
    pub blockid: String,
    pub node_id: String,
    pub pid: u32,
}

/// Data for `COMMAND_LIST_BACKGROUND_TASKS` — request/response, returns
/// this block's current `db_background_tasks` rows (as
/// `muxspect_handlers::BackgroundTaskView`s). See
/// docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.1.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandListBackgroundTasksData {
    pub blockid: String,
}

/// Data for AgentAnswerCommand — an AskUserQuestion answer delivered back to
/// the running agent CLI via the Agent SDK control protocol (a `control_response`
/// carrying `updatedInput.answers`). Spec:
/// docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentAnswerData {
    pub blockid: String,
    /// The `AskUserQuestion` tool_use id the answer responds to (correlates with
    /// the parked `can_use_tool` control_request).
    pub tool_use_id: String,
    /// The user's selections as a JSON object mapping each question's text to the
    /// chosen option label (a `[labels]` array for multiSelect, or free-text for
    /// "Other"). Becomes `updatedInput.answers` in the control_response.
    /// `Value` in Rust because it is forwarded verbatim into the control
    /// response, but the hand-written declaration knew the shape and keeping it
    /// is not a downgrade: question text -> chosen label, an array for
    /// multiSelect, or free text for "Other".
    #[serde(default)]
    #[ts(type = "Record<string, string | string[]>")]
    pub answers: serde_json::Value,
}

/// Data for AgentCancelCommand — a real protocol-level decline of a pending
/// AskUserQuestion (Cancel button / Escape), delivered as a control_response
/// carrying `behavior: "deny"` rather than the allow+answers shape above. No
/// `answers` field: there is nothing to carry, the deny message is a fixed
/// server-owned string (see `ASK_USER_QUESTION_DENY_MESSAGE` in
/// blockcontroller/persistent.rs). Spec:
/// docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentCancelData {
    pub blockid: String,
    /// The `AskUserQuestion` tool_use id being declined (correlates with the
    /// parked `can_use_tool` control_request).
    pub tool_use_id: String,
}

// ---- Subprocess agent command data types ----

/// Data for SubprocessSpawnCommand — spawn agent CLI for a single turn.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
    #[ts(optional)]
    pub message_id: Option<String>,
    /// Set only by the hidden memory-reinjection turn (see
    /// `memory-reinjection-controller.ts` / `session::set_hidden_reinjection_active`).
    /// Marks the block as suppressed for ambient digest reads
    /// (`extract_digest_text`'s next_prompt_suggestion / activity_summary /
    /// activity_watcher callers) until the NEXT `AgentInputCommand` for the
    /// same block, hidden or not. Authoritative and set at RPC-dispatch
    /// time — unlike the marker-text detection inside `extract_digest_text`,
    /// it doesn't depend on the reinjection marker line still being inside
    /// that function's 32 KB / ~30-line tail window, which large Personal
    /// memory content routinely scrolls past.
    /// SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md finding #8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hidden: Option<bool>,
}

/// Data for AskSideQuestionCommand — the `/btw` slash command's one-shot,
/// tool-less side question. `context_snapshot` is a frontend-supplied
/// compact rendering of the asking pane's currently-visible transcript,
/// prepended to `question` as the prompt sent to the CLI (there is no
/// `--resume`/live session context on this path — see
/// `server/agent_handlers/side_question.rs`'s module doc comment for why).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAskSideQuestionData {
    pub block_id: String,
    pub question: String,
    #[serde(default)]
    pub context_snapshot: String,
}

/// Result of AskSideQuestionCommand. `request_id` is the correlator the
/// caller uses to scope its `EVENT_BTW_ANSWER_CHUNK` subscription:
/// `block:<block_id>:btw:<request_id>`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AskSideQuestionResult {
    pub request_id: String,
}

/// Data for AgentStopCommand — stop the running subprocess.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentStopData {
    pub blockid: String,
    #[serde(default)]
    pub force: bool,
}

/// Data for ShellExecCommand — run a shell command in the agent's working directory.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandShellExecData {
    pub blockid: String,
    pub command: String,
    #[serde(default)]
    pub working_dir: String,
}

/// Result of ShellExecCommand.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ShellExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Data for ShellStopCommand — stop a running persistent shell node by id.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandShellStopData {
    pub shell_id: String,
}

/// Result of `shellstop`. Was an inline `json!({ "stopped": .. })`.
///
/// `false` means "no live registry entry for that id", which covers both
/// "already exited" and "never existed" -- the caller cannot tell them apart,
/// and does not need to: either way there is nothing left to stop.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ShellStopResult {
    pub stopped: bool,
}

/// Data for ShellStatusCommand — query a shell's current running state by id.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandShellStatusData {
    pub shell_id: String,
}

/// Result of `shellstatus`. Was two inline `json!({..})` literals.
///
/// `known: false` is NOT the same as `running: false`, and collapsing them is
/// the bug this shape exists to prevent: a shell that has not finished
/// registering yet is unknown, not exited, and treating it as exited
/// misreported live shells as failed (reagent P1 on #2770). Callers must
/// branch on `known` first.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ShellStatusResult {
    pub known: bool,
    pub running: bool,
    /// Always present, null while running or when unknown -- NOT an omitted
    /// key. The hand-written stub said `exit_code?: number`, but both arms of
    /// the handler write the key unconditionally.
    #[ts(type = "number | null")]
    pub exit_code: Option<i32>,
    #[ts(type = "number")]
    pub line_count: u64,
}

/// A file to write as part of agent config.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AgentConfigFile {
    pub path: String,
    pub content: String,
}

/// Data for WriteAgentConfigCommand — write config files atomically.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandWriteAgentConfigResult {
    pub working_dir: String,
}

/// Data for ResolveCliCommand — detect or install a CLI tool.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ResolveCliResult {
    /// Absolute path to the CLI binary
    pub cli_path: String,
    /// CLI version string
    pub version: String,
    /// How it was resolved: "path", "local_install", "installed"
    pub source: String,
}

/// Data for CheckCliAuthCommand — check if CLI is authenticated.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CheckCliAuthResult {
    pub authenticated: bool,
    pub email: Option<String>,
    pub auth_method: Option<String>,
    /// Raw stdout from auth check command
    pub raw_output: String,
}

/// Input for RunCliLoginCommand — spawns the CLI login flow and extracts the OAuth URL
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandRunCliLoginData {
    pub cli_path: String,
    pub login_args: Vec<String>,
    #[serde(default)]
    pub auth_env: HashMap<String, String>,
}

/// Result from RunCliLoginCommand
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
// `Serialize` is additive here — nothing currently serializes an incoming
// pane.open request — added specifically so a contract test can construct a
// full instance and compare its real field-name set against
// docs/specs/app-api-manifest.json (SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md
// §2.8), catching drift between this struct and the CLI/manifest at CI time
// on this side, the same way a corresponding muxsh.contract.test.mjs catches
// it on the Node side.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// `tear_off_block` saga — the block exists (and the frontend's MOS cache
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
    /// Create the block directly as a new tab of the pane that holds this
    /// block (visible or background), in ONE reducer step
    /// (`Command::CreateBlockInStack`) — the block never exists without a
    /// pane, unlike `skip_placement` + a frontend push, which could be
    /// interrupted in between. The frontend is told via a queued `stackpush`
    /// layout action. Takes precedence over `skip_placement` and split
    /// placement; `floating` still wins (checked first).
    /// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_onto_block_id: Option<String>,
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

/// Request for pane.moveTab — reorder `block_id` within its own pane, or
/// move it into a different pane. `position` is `"before"` / `"after"` /
/// `"end"`, relative to `target_block_id` (matches
/// `agentmux_common::StackMovePosition`'s serde encoding).
/// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1, Phase 3.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandPaneMoveTabData {
    pub block_id: String,
    pub target_block_id: String,
    pub position: String,
    pub activate: bool,
}

/// Request for blockfile:line_count — count total lines in a blockfile.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileLineCountData {
    pub block_id: String,
    pub filename: String,
}

/// Response from blockfile:line_count.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct BlockfileLineCountResult {
    #[ts(type = "number")]
    pub count: u64,
    /// The transcript stream `count` is of — `b:<blockId>` (the block's own
    /// `output`) or `g:<zone>` (the agent's global zone) — and its
    /// generation, when the file is counted (Phase 5a-3,
    /// SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7).
    /// Absent: not a counted transcript; addressing by line is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gen: Option<String>,
}

/// Request for blockfile:read_range — read a range of lines from a blockfile.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileReadRangeData {
    pub block_id: String,
    pub filename: String,
    #[ts(type = "number")]
    pub offset: u64,
    #[ts(type = "number")]
    pub limit: u64,
    /// Read only from this generation of the stream: if the file has been
    /// replaced since (another generation), answer `gen_mismatch` with no
    /// lines rather than lines of another file (Phase 5a-3).
    #[serde(default)]
    #[ts(optional)]
    pub expect_gen: Option<String>,
}

/// Response from blockfile:read_range.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct BlockfileReadRangeResult {
    pub lines: Vec<String>,
    #[ts(type = "number")]
    pub total: u64,
    /// Receive-time stamps (unix ms) parallel to `lines`, joined from the
    /// `output.tsidx` sidecar; `0` = unknown for that line. Absent entirely
    /// when no sidecar exists or the request didn't take the `output.idx`
    /// fast path — old frontends ignore it, new frontends tolerate absence.
    /// Spec: SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §4.4.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number[]")]
    pub stamps: Option<Vec<i64>>,
    /// The stream and generation `lines` were read from, when the file is a
    /// counted transcript and the read provably saw one generation (it was
    /// the same before and after the read). Absent otherwise (Phase 5a-3).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gen: Option<String>,
    /// `expect_gen` was given and the file is now another generation (or
    /// changed during the read): `lines` is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gen_mismatch: Option<bool>,
}

/// Request for blockfile:read_state — read a sidecar JSON file
/// (e.g. `output.state.json`) associated with a block.
/// Spec: docs/specs/SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileReadStateData {
    pub block_id: String,
    /// Sidecar filename — e.g. "output.state.json". Resolved within the
    /// block's filestore directory; must not contain path separators.
    pub filename: String,
}

/// Response from blockfile:read_state. `content` is the raw file bytes
/// as a UTF-8 string, or null if the sidecar does not exist.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct BlockfileReadStateResult {
    pub content: Option<String>,
}

/// Request for blockfile:write_state — atomically write a sidecar JSON
/// file for a block. Uses tmp + fsync + rename to guarantee partial
/// writes never surface to readers.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandBlockfileWriteStateData {
    pub block_id: String,
    pub filename: String,
    pub content: String,
}

/// Response from blockfile:write_state.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct BlockfileWriteStateResult {
    #[ts(type = "number")]
    pub bytes_written: u64,
}

#[cfg(test)]
mod app_api_manifest_contract_tests {
    //! Rust half of the DRY contract check described in
    //! docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md §2.8: asserts the
    //! `pane.open` entry in docs/specs/app-api-manifest.json names exactly
    //! this struct's real serde field set — no more, no less. A Node-side
    //! test (muxsh.contract.test.mjs) makes the matching assertion against
    //! what `muxsh` actually sends, against the same manifest file. Neither
    //! test can catch both sides being wrong in the same way, but together
    //! a field rename here now fails CI in both suites instead of shipping
    //! as a silent runtime mismatch (which is exactly how Phase 1's real bug,
    //! ReAgent on PR #3255, was actually found — after merge, not before).
    use super::*;
    use std::collections::HashSet;
    use std::path::Path;

    /// Walks up from this crate's manifest dir to the repo root — the same
    /// assumption `include_str!`-based fixtures elsewhere in this crate make
    /// about the workspace layout, just done at runtime instead of compile
    /// time since this needs to open the file, not embed it.
    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("agentmux-srv's parent dir is the repo root")
            .to_path_buf()
    }

    fn load_manifest() -> serde_json::Value {
        let path = repo_root().join("docs/specs/app-api-manifest.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        serde_json::from_str(&raw).expect("app-api-manifest.json must be valid JSON")
    }

    #[test]
    fn pane_open_manifest_request_fields_match_the_real_struct() {
        let manifest = load_manifest();
        let manifest_fields: HashSet<String> = manifest["routes"]["pane.open"]["requestFields"]
            .as_array()
            .expect("routes.pane.open.requestFields must be an array")
            .iter()
            .map(|v| v.as_str().expect("field name must be a string").to_string())
            .collect();

        // A fully-populated instance, so serializing it surfaces every field
        // this struct actually has — Option::None fields still appear as
        // `null`, not omitted, because CommandPaneOpenData has no
        // `#[serde(skip_serializing_if = ...)]` attributes.
        let instance = CommandPaneOpenData {
            view: "editor".to_string(),
            file: Some("/tmp/x".to_string()),
            url: None,
            cwd: None,
            title: None,
            tab_id: None,
            split_direction: None,
            split_reference_block_id: None,
            focus: None,
            tree_expanded: None,
            floating: None,
            meta: None,
            skip_placement: None,
            stack_onto_block_id: None,
            reuse_editor_pane: None,
        };
        let value = serde_json::to_value(&instance).expect("CommandPaneOpenData must serialize");
        let struct_fields: HashSet<String> = value
            .as_object()
            .expect("serialized CommandPaneOpenData must be a JSON object")
            .keys()
            .cloned()
            .collect();

        let manifest_only: Vec<_> = manifest_fields.difference(&struct_fields).collect();
        let struct_only: Vec<_> = struct_fields.difference(&manifest_fields).collect();
        assert!(
            manifest_only.is_empty() && struct_only.is_empty(),
            "docs/specs/app-api-manifest.json's pane.open.requestFields has drifted from \
             CommandPaneOpenData's real fields — in manifest but not the struct: {manifest_only:?}; \
             in the struct but not the manifest: {struct_only:?}"
        );
    }

    #[test]
    fn pane_open_manifest_response_fields_match_the_real_struct() {
        let manifest = load_manifest();
        let manifest_fields: HashSet<String> = manifest["routes"]["pane.open"]["responseFields"]
            .as_array()
            .expect("routes.pane.open.responseFields must be an array")
            .iter()
            .map(|v| v.as_str().expect("field name must be a string").to_string())
            .collect();

        let instance = PaneOpenResult {
            block_id: "b".to_string(),
            tab_id: "t".to_string(),
            view: "editor".to_string(),
            created: true,
        };
        let value = serde_json::to_value(&instance).expect("PaneOpenResult must serialize");
        let struct_fields: HashSet<String> = value
            .as_object()
            .expect("serialized PaneOpenResult must be a JSON object")
            .keys()
            .cloned()
            .collect();

        assert_eq!(
            manifest_fields, struct_fields,
            "docs/specs/app-api-manifest.json's pane.open.responseFields has drifted from PaneOpenResult's real fields"
        );
    }
}

// Request-shape tests for the four `blockfile:*` commands.
//
// Nothing else catches a Req/payload mismatch: `tsc` only checks the frontend
// against the GENERATED types, and `scripts/check-rpc-bindings.sh` only checks
// that a generated type exists per command and is current. Neither ever
// deserializes a real payload into the Rust struct.
#[cfg(test)]
mod blockfile_req_shape_tests {
    use super::*;
    use serde_json::json;

    // AgentHistoryView.tsx:165, useHistoryPagination.ts:331
    #[test]
    fn line_count_accepts_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandBlockfileLineCountData>(
            json!({"block_id": "b1", "filename": "output"}),
        )
        .expect("blockfile:line_count must accept block_id and filename");
    }

    // AgentHistoryView.tsx:178
    #[test]
    fn read_range_accepts_the_payload_the_stub_sends() {
        let r: CommandBlockfileReadRangeData = serde_json::from_value(
            json!({"block_id": "b1", "filename": "output", "offset": 0, "limit": 500}),
        )
        .expect("blockfile:read_range must accept the full range payload");
        assert_eq!((r.offset, r.limit), (0, 500));
    }

    // `offset`/`limit` are u64 carrying JS numbers, which is why the generated
    // binding overrides them to `number` rather than `bigint`. A negative
    // offset must be rejected rather than wrapping to a huge positive.
    #[test]
    fn read_range_rejects_a_negative_offset() {
        assert!(
            serde_json::from_value::<CommandBlockfileReadRangeData>(
                json!({"block_id": "b1", "filename": "output", "offset": -1, "limit": 10}),
            )
            .is_err(),
            "a negative offset must fail loudly, not wrap into a huge u64"
        );
    }

    #[test]
    fn the_state_commands_accept_their_payloads() {
        serde_json::from_value::<CommandBlockfileReadStateData>(
            json!({"block_id": "b1", "filename": "output.state.json"}),
        )
        .expect("blockfile:read_state");
        serde_json::from_value::<CommandBlockfileWriteStateData>(
            json!({"block_id": "b1", "filename": "output.state.json", "content": "{}"}),
        )
        .expect("blockfile:write_state");
    }

    // `stamps` is `skip_serializing_if = "Option::is_none"`, so the key is
    // OMITTED rather than nulled when absent -- which is why the generated
    // binding says `stamps?: number[]`. The doc comment on the field promises
    // old frontends can ignore it and new ones tolerate absence; that promise
    // only holds while it is genuinely omitted.
    #[test]
    fn read_range_result_omits_stamps_rather_than_nulling_them() {
        let without = serde_json::to_value(BlockfileReadRangeResult {
            lines: vec!["a".to_string()],
            total: 1,
            ..Default::default()
        })
        .expect("serializable");
        assert_eq!(without, json!({"lines": ["a"], "total": 1}));

        let with = serde_json::to_value(BlockfileReadRangeResult {
            lines: vec!["a".to_string()],
            total: 1,
            stamps: Some(vec![7]),
            ..Default::default()
        })
        .expect("serializable");
        assert_eq!(with, json!({"lines": ["a"], "total": 1, "stamps": [7]}));

        // The Phase 5a-3 fields follow the same rule: omitted when unset.
        let positioned = serde_json::to_value(BlockfileReadRangeResult {
            lines: vec![],
            total: 3,
            stream: Some("g:agent:x:current".to_string()),
            gen: Some("0123456789abcdef".to_string()),
            gen_mismatch: Some(true),
            ..Default::default()
        })
        .expect("serializable");
        assert_eq!(
            positioned,
            json!({"lines": [], "total": 3, "stream": "g:agent:x:current", "gen": "0123456789abcdef", "gen_mismatch": true})
        );
    }

    // `content` is a plain `Option<String>` with NO skip_serializing_if, so the
    // key is always present and carries null -- a different shape from
    // `stamps` above, and the generated binding says `string | null` for
    // exactly that reason. The two live side by side in one domain, so pin
    // both rather than assuming they behave alike.
    #[test]
    fn read_state_result_nulls_content_rather_than_omitting_it() {
        let v = serde_json::to_value(BlockfileReadStateResult { content: None })
            .expect("serializable");
        assert_eq!(v, json!({"content": null}));
    }
}

// Request-shape tests for the eight websocket-hosted block commands migrated
// in this slice.
#[cfg(test)]
mod block_ws_req_shape_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_single_field_commands_accept_their_payloads() {
        serde_json::from_value::<CommandDeleteBlockData>(json!({"blockid": "b1"}))
            .expect("deletesubblock");
        serde_json::from_value::<CommandListBackgroundTasksData>(json!({"blockid": "b1"}))
            .expect("listbackgroundtasks");
        serde_json::from_value::<CommandAgentCancelData>(
            json!({"blockid": "b1", "tool_use_id": "t1"}),
        )
        .expect("agentcancel");
    }

    // `outcome` and `scope` are Rust `String`s carrying closed sets. The
    // generated binding keeps the unions via #[ts(type = ...)], but the SERVER
    // is the thing that actually enforces them -- the handler rejects an
    // unknown outcome explicitly. Pin that the wire type stays permissive
    // (a bad value deserializes, then fails with a domain error) so the
    // union in TypeScript is understood as a caller-side aid, not the guard.
    #[test]
    fn tool_decision_parses_any_outcome_string_and_leaves_validation_to_the_handler() {
        let good: CommandToolDecisionData = serde_json::from_value(
            json!({"blockid": "b1", "request_id": "r1", "outcome": "allow", "scope": "once"}),
        )
        .expect("tooldecision must accept the payload the stub sends");
        assert!(good.feedback.is_none(), "feedback is optional");

        serde_json::from_value::<CommandToolDecisionData>(
            json!({"blockid": "b1", "request_id": "r1", "outcome": "bogus", "scope": "once"}),
        )
        .expect("an unknown outcome still DESERIALIZES; the handler rejects it, not serde");
    }

    #[test]
    fn ambient_narrate_accepts_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandAmbientNarrateData>(json!({
            "blockid": "b1", "kind": "k", "context": "c", "dedupe_key": "d",
        }))
        .expect("ambientnarrate");
    }

    // `timestamp` / `run_in_background` are Options with no
    // skip_serializing_if. serde treats a missing Option field as None, so the
    // generated binding marks them optional -- which is what the hand-written
    // declarations already said. Both halves matter, so pin both.
    #[test]
    fn dock_node_status_accepts_the_payload_with_and_without_its_optionals() {
        let bare: CommandDockNodeStatusData = serde_json::from_value(
            json!({"blockid": "b1", "node_id": "n1", "tool_name": "t", "status": "s"}),
        )
        .expect("docknodestatus must accept the bare payload");
        assert!(bare.timestamp.is_none() && bare.run_in_background.is_none());

        let full: CommandDockNodeStatusData = serde_json::from_value(json!({
            "blockid": "b1", "node_id": "n1", "tool_name": "t", "status": "s",
            "timestamp": 1_789_000_000_000_i64, "run_in_background": true,
        }))
        .expect("docknodestatus must accept the full payload");
        assert_eq!(full.timestamp, Some(1_789_000_000_000));
        assert_eq!(full.run_in_background, Some(true));
    }

    #[test]
    fn background_task_commands_accept_their_payloads() {
        let done: CommandBackgroundTaskCompletionData =
            serde_json::from_value(json!({"blockid": "b1", "node_id": "n1", "status": "done"}))
                .expect("backgroundtaskcompletion, timestamp omitted");
        assert!(done.timestamp.is_none());

        let pid: CommandBackgroundTaskPidData =
            serde_json::from_value(json!({"blockid": "b1", "node_id": "n1", "pid": 4321}))
                .expect("backgroundtaskpid");
        assert_eq!(pid.pid, 4321);

        // `pid` is a u32, so a negative must be rejected rather than wrapping
        // into a huge process id.
        assert!(
            serde_json::from_value::<CommandBackgroundTaskPidData>(
                json!({"blockid": "b1", "node_id": "n1", "pid": -1}),
            )
            .is_err(),
            "a negative pid must fail loudly, not wrap"
        );
    }
}
