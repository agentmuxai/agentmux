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

// ── PTY Shell ─────────────────────────────────────────────────────────────────
//
// A REAL PTY-backed shell (not the piped subprocess `Shell` above) — for
// driving genuinely interactive programs that check for a real terminal
// (Sharprompt-style wizards, `sudo`, `ssh`, REPLs). Reuses the same backend
// `blockcontroller::shell` already uses for the `term` widget and
// `AgentShellSubblock.tsx`; this is a new agent-facing entry point onto
// existing PTY plumbing, not a new implementation. See
// docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md.
//
// Deliberately NOT UI-driven: every one of these is a plain backend RPC
// (agent -> agentmux-mcp -> agentmux-srv), with no dependency on any window
// being open, focused, or even rendering the shell's `term` sub-block — the
// same posture the existing `Shell`/`ShellInput` family already has.

/// `POST /api/v1/ptyshell/create`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellCreateRequest {
    pub agent_block_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
}

/// Response from `POST /api/v1/ptyshell/create`. `shell_id` is the new
/// sub-block's id — the same value every other `PtyShell*` call takes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellCreateResponse {
    pub shell_id: String,
}

/// `POST /api/v1/ptyshell/input` — write raw text to the PTY, as if typed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellInputRequest {
    pub shell_id: String,
    pub text: String,
}

/// Response from `POST /api/v1/ptyshell/input`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellInputResponse {
    pub written: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `POST /api/v1/ptyshell/resize` — resize the PTY. Some TUIs render
/// differently or wrap badly at the fallback geometry (25x200).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellResizeRequest {
    pub shell_id: String,
    pub rows: u16,
    pub cols: u16,
}

/// Response from `POST /api/v1/ptyshell/resize`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellResizeResponse {
    pub resized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `POST /api/v1/ptyshell/read` — read back the shell's raw output tail.
///
/// This is a raw byte/line log, not a rendered-screen snapshot — a
/// full-screen TUI that redraws in place (cursor movement, `\r`-only
/// updates) will not read back the way it visually renders. Sufficient for
/// line-oriented wizards (the common case); see the spec's §5 for the
/// rendered-snapshot follow-up this deliberately does not attempt yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellReadRequest {
    pub shell_id: String,
    /// Number of trailing lines to return. Defaults to 200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_lines: Option<u32>,
}

/// Response from `POST /api/v1/ptyshell/read`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellReadResponse {
    pub content: String,
    /// True if the shell's output has more lines than `tail_lines` returned.
    pub truncated: bool,
}

/// `POST /api/v1/ptyshell/status` — query whether the shell is still running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellStatusRequest {
    pub shell_id: String,
}

/// Response from `POST /api/v1/ptyshell/status`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellStatusResponse {
    /// False both for "still initializing" and "exited" — callers that need
    /// to distinguish those should check `exit_code` (`None` for the former).
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// `POST /api/v1/ptyshell/stop` — kill the PTY and delete its sub-block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellStopRequest {
    pub shell_id: String,
}

/// Response from `POST /api/v1/ptyshell/stop`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyShellStopResponse {
    pub stopped: bool,
}

// ── Inject (SendMessage + Loop) ───────────────────────────────────────────────

/// `POST /agentmux/reactive/inject` — deliver a message to an agent.
///
/// Used by the `SendMessage` and `Loop` MCP tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InjectRequest {
    pub target_agent: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    /// Message id this jekt was signed under — required for `jekt_sig` to be
    /// verifiable (the signed material binds msgid, sender, target, and
    /// timestamp together). `None` when unsigned (e.g. `AGENTMUX_JEKT_KEY`
    /// wasn't available — legacy/unverified, same as omitting `jekt_sig`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Unix seconds this jekt was signed at — part of the signed material,
    /// not just a display timestamp. See `agentmux_common::jekt_sign`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_secs: Option<i64>,
    /// Base64 HMAC-SHA256 over (msgid, source_agent, target_agent, ts_secs,
    /// message), signed with the sender's own `AGENTMUX_JEKT_KEY`. Absent
    /// when the sending agent has no key yet (first-ever send before one is
    /// provisioned) — srv treats an absent/invalid signature as unverified,
    /// not as an error, and downgrades trust accordingly rather than
    /// rejecting the message outright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jekt_sig: Option<String>,
    /// Base64 Ed25519 signature over the same signed material as `jekt_sig`,
    /// produced with the sender's own `AGENTMUX_LAN_KEY`
    /// (SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.3). Sent unconditionally
    /// alongside `jekt_sig` regardless of the message's actual destination
    /// — this process can't know in advance whether delivery will end up
    /// LAN, host, or WAN (that's a server-side routing decision); srv only
    /// ever consults this field when it has independently determined
    /// `delivery_tier == "lan"`, so it's simply ignored otherwise. Absent
    /// under the same "no key yet" conditions as `jekt_sig`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_sig: Option<String>,
    /// The sending instance's channel id (`AGENTMUX_CHANNEL`, injected into
    /// the MCP env at spawn alongside the keys). Bound into `channel_sig`'s
    /// signed material (SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md §D5) so
    /// a signature minted in one channel can't be replayed as the same agent
    /// speaking from another. Absent whenever `channel_sig` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel: Option<String>,
    /// Base64 Ed25519 signature for the same-machine, different-instance
    /// (cross-channel) tier, produced with the sender's own `AGENTMUX_LAN_KEY`
    /// over a domain-separated payload that also binds `source_channel`
    /// (`agentmux_common::jekt_sign::sign_channel_jekt`). Sent unconditionally
    /// for the same reason `lan_sig` is: srv only consults it once it has
    /// itself labelled the delivery `channel`. Absent under the same "no key
    /// yet" conditions as `lan_sig`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_sig: Option<String>,
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

// ── UI automation ───────────────────────────────────────────────────────────
//
// `block_id` is NOT a field on any of these requests, on purpose (2026-08-19,
// reagent + Codex review, PR #2662 — a client-supplied `block_id` here was a
// real cross-agent content-disclosure vulnerability: /api/v1/ui/* shares the
// same instance-wide X-AuthKey every App-API route trusts, and any agent can
// read that key from its own environment, so a bare `block_id` field could
// never be trusted from the request body no matter which layer stamped it).
//
// Instead, every request here carries `UiAutomationAuth`: the calling
// agent's own slug, a timestamp, and an HMAC-SHA256 signature over them
// using that agent's own `AGENTMUX_JEKT_KEY` (the SAME per-agent signing key
// already used for jekt sender authentication — reused rather than inventing
// a parallel credential system; see `agentmux_common::jekt_sign`). srv
// verifies the signature against the claimed agent_id's key on file
// (`Store::agent_jekt_key_load`) and, ONLY once that succeeds, looks up that
// agent's actual current block_id server-side (`ReactiveHandler::get_agent`)
// — the block_id a UI-automation call actually operates on is never taken
// from the client at all, so there is nothing left to spoof. See
// `agentmux-srv/src/server/ui_handlers.rs::verified_block_id` and
// docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md.

/// Identity proof shared by every `/api/v1/ui/*` request. `sig` is
/// `jekt_sign::sign_jekt(key, "ui-automation-identity", agent_id,
/// "__srv__", ts_secs, "")` — the fixed msgid/target_agent/message values
/// exist purely for domain separation from real jekt message signatures
/// (no genuine jekt uses this literal msgid, and "__srv__" is not a
/// spawnable agent name), not because those fields carry meaning here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiAutomationAuth {
    pub agent_id: String,
    pub ts_secs: i64,
    pub sig: String,
}

/// `POST /api/v1/ui/screenshot`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiScreenshotRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
}

/// Response for `POST /api/v1/ui/screenshot`. `path` is a file already
/// written to disk (openable via the `OpenMedia` tool); `png_base64` is the
/// same image inline, for a caller that can render an MCP `ImageContent`
/// block directly without a second round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiScreenshotResponse {
    pub path: String,
    pub png_base64: String,
}

/// `POST /api/v1/ui/click`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiClickRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    pub selector: String,
}

/// `POST /api/v1/ui/query`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiQueryRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `POST /api/v1/agent/pane/close` — backs the `ClosePane` MCP tool.
/// See docs/specs/SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md.
///
/// `auth` is reused unchanged from `UiAutomationAuth` (the same signed
/// per-agent identity `verified_block_id` already checks for UI automation)
/// — it is what resolves the caller's OWN pane when `block_id` is omitted,
/// and what supplies a verified `source_agent` for the audit log when
/// `block_id` targets another agent's pane (§5.2 of that spec).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosePaneRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    /// Target pane to close. Omitted = the caller's own pane (self-only,
    /// no ownership check needed — it's already the caller's). Present =
    /// fleet tier: close ANY pane by block_id, no ownership check on the
    /// TARGET either, by design (§5.1) — this is the capability that
    /// motivated the whole spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    /// Optional human-readable reason, threaded into the audit log entry
    /// when closing another agent's pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
