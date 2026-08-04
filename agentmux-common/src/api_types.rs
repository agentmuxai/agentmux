// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Canonical HTTP request/response DTOs shared between agentmux-srv,
//! agentmux-mcp, and agentmux-bashwrap.  Every struct here is the single
//! source of truth for the corresponding wire shape; the three crates all
//! import rather than redeclare these types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn is_zero(n: &usize) -> bool {
    *n == 0
}

// ── WPS publish ───────────────────────────────────────────────────────────────

/// `POST /agentmux/wps/publish` — shared client/server envelope.
///
/// Sent by `agentmux-bashwrap` and received by `agentmux-srv`.
/// Mirrors `WaveEvent` but omits the server-populated `sender` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WpsPublishRequest {
    pub event: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub persist: usize,
    pub data: serde_json::Value,
}

// ── Shell ─────────────────────────────────────────────────────────────────────

/// `POST /api/v1/shell/create`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCreateRequest {
    pub agent_block_id: String,
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    /// If true, pipe the child's stdin so ShellInput() can write to it.
    /// Default false (stdin is /dev/null) — avoids blocking programs that
    /// read stdin to EOF (e.g. `cat` with no args).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_stdin: Option<bool>,
}

/// Response from `POST /api/v1/shell/create`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCreateResponse {
    pub shell_id: String,
}

/// `POST /api/v1/shell/stop`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellStopRequest {
    pub shell_id: String,
}

/// Response from `POST /api/v1/shell/stop`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellStopResponse {
    pub stopped: bool,
}

/// `POST /api/v1/shell/input` — write text to a running shell's stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellInputRequest {
    pub shell_id: String,
    /// Text to write. A newline is appended automatically so single answers
    /// like "y" work without the caller knowing the line discipline.
    pub text: String,
}

/// Response from `POST /api/v1/shell/input`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellInputResponse {
    /// false if the shell is not running, has no captured stdin, or the write failed.
    pub written: bool,
    /// Why the write did not happen (None when `written` is true). Lets callers
    /// distinguish "shell exited" from "shell is running but was created without
    /// capture_stdin=true", which are otherwise indistinguishable from `written`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ShellInputFailure>,
}

/// Reason a `ShellInput` write did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellInputFailure {
    /// The shell id is unknown or the process has already exited.
    NotRunning,
    /// The shell is running but was created without `capture_stdin=true`, so its
    /// stdin is `/dev/null` — recreate it with capture_stdin to send input.
    StdinNotCaptured,
    /// The stdin relay is gone / the write channel is closed (process closed stdin).
    WriteFailed,
}

/// `POST /api/v1/shell/status` — query whether a shell is still running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellStatusRequest {
    pub shell_id: String,
}

/// Response from `POST /api/v1/shell/status`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellStatusResponse {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub line_count: u64,
}

// ── Inject (SendMessage + Loop) ───────────────────────────────────────────────

/// `POST /agentmux/reactive/inject` — deliver a message to an agent.
///
/// Used by the `SendMessage` and `Loop` MCP tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectRequest {
    pub target_agent: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
}

// ── Pane ──────────────────────────────────────────────────────────────────────

/// `POST /api/v1/pane/open` — open a new pane.
///
/// Structurally equivalent to `rpc_types::CommandPaneOpenData` (same wire
/// fields) so the two are interchangeable on the wire; srv deserializes into
/// `CommandPaneOpenData`, mcp serializes from this type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneOpenRequest {
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_reference_block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_expanded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floating: Option<bool>,
    /// `Some(true)`, `view: "editor"` only: explicit opt-in to reuse an
    /// already-open Editor pane in the caller's own tab (identified via
    /// `split_reference_block_id`) instead of always creating a new one.
    /// Set only by the `OpenEditor` MCP tool — NOT inferred from other
    /// fields, since other legitimate `pane.open` callers also set
    /// `split_reference_block_id` for split placement without wanting
    /// reuse (`EditorViewModel.openToTheSide`/`openInTerminal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse_editor_pane: Option<bool>,
}

/// Response from `POST /api/v1/pane/open`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneOpenResponse {
    pub block_id: String,
}

/// `POST /api/v1/pane/title` — set a pane's display title.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneTitleRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    pub title: String,
}

// ── Tab ───────────────────────────────────────────────────────────────────────

/// `POST /api/v1/tab/activate` — switch the active tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabActivateRequest {
    pub tab_id: String,
}

/// `POST /api/v1/tab/new` — create a new tab in a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabNewRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `POST /api/v1/tab/name` — rename a tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabNameRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    pub name: String,
}

// ── Window ────────────────────────────────────────────────────────────────────

/// `POST /api/v1/window/focus` — bring a window to the foreground.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowFocusRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

/// `POST /api/v1/window/name` — set a window's display name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowNameRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

// ── Workspace ─────────────────────────────────────────────────────────────────

/// `POST /api/v1/workspace/name` — rename a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceNameRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub name: String,
}
