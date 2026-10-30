// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which HTTP routes are recorded (`super::http_action`), decided for every route that can change
//! state. A route is `"METHOD /path/:param"`, exactly as `routes.rs` registers it; the test in
//! `tests/http_action_tests.rs` compares these lists with the registered routes, so a new route
//! cannot be added without deciding it. A route in no list is recorded.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Policy {
    /// Recorded whoever calls: an agent by its token, anything else as `unproven`.
    Always,
    /// The window's and the host's own calls, which cannot be told from a process that holds the
    /// key: recorded when an agent's token is on the request.
    Plumbing,
    /// The route records its successes itself, with more than the layer knows; the layer records
    /// its refusals.
    Handler,
    /// Reads data or carries a stream, and changes nothing that is kept.
    Never,
}

/// Changes state, runs or stops a process, writes files, sends to an agent or changes who may do
/// what.
pub(super) const ALWAYS: &[&str] = &[
    "DELETE /agentmux/cron/:id", "DELETE /agentmux/jobs/:id", "PATCH /agentmux/cron/:id", "PATCH /agentmux/jobs/:id",
    "POST /agentmux/agent/broadcast", "POST /agentmux/agent/release", "POST /agentmux/agent/stop",
    "POST /agentmux/agent/stop-pending", "POST /agentmux/cron", "POST /agentmux/cron/:id/run",
    "POST /agentmux/cron/runs/:run_id/report", "POST /agentmux/jobs", "POST /agentmux/jobs/:id/run",
    "POST /agentmux/jobs/runs/:run_id/report", "POST /agentmux/jobs/stop-run", "POST /agentmux/old-versions",
    "POST /agentmux/reactive/ensure-signing-key", "POST /agentmux/reactive/poller/config",
    "POST /agentmux/reactive/register", "POST /agentmux/reactive/unregister", "POST /agentmux/work",
    "POST /agentmux/work/:id/accept", "POST /agentmux/work/:id/ask", "POST /agentmux/work/:id/complete",
    "POST /agentmux/work/:id/heartbeat", "POST /agentmux/work/:id/hold", "POST /agentmux/work/:id/release",
    "POST /agentmux/work/:id/send_back", "POST /agentmux/work/:id/submit", "POST /agentmux/work/:id/unhold",
    "POST /agentmux/work/:id/update", "POST /agentmux/work/act", "POST /agentmux/work/claim",
    "POST /agentmux/work/media", "POST /api/bus/broadcast", "POST /api/bus/inject", "POST /api/bus/messages/delete",
    "POST /api/bus/register", "POST /api/bus/send", "POST /api/messaging/discord/send",
    "POST /api/messaging/slack/send", "POST /api/messaging/telegram/send", "POST /api/messaging/whatsapp/send",
    "POST /api/v1/agent/container/update", "POST /api/v1/agent/desktop/authorize",
    "POST /api/v1/agent/desktop/policy", "POST /api/v1/agent/desktop/record", "POST /api/v1/agent/desktop/release",
    "POST /api/v1/agent/dev_server/register", "POST /api/v1/agent/globalmemory/remove",
    "POST /api/v1/agent/globalmemory/revert", "POST /api/v1/agent/globalmemory/write",
    "POST /api/v1/agent/history/reclaim",
    "POST /api/v1/agent/remote/op", "POST /agentmux/remote/op",
    "POST /api/v1/agent/identity/switch", "POST /api/v1/agent/identity/validate", "POST /api/v1/agent/memory/revert",
    "POST /api/v1/agent/memory/write", "POST /api/v1/agent/open", "POST /api/v1/agent/open_many", "POST /api/v1/agent/pane/close",
    "POST /api/v1/agent/self/compact", "POST /api/v1/agent/self/fresh", "POST /api/v1/agent/self/mcp-heal",
    "POST /api/v1/agent/self/quit", "POST /api/v1/agent/self/upgrade", "POST /api/v1/agent/skill", "POST /api/v1/agent/spawn", "POST /api/v1/agent/spawn_many",
    "POST /api/v1/agent/takeover", "POST /api/v1/coord/posts", "POST /api/v1/fleet/bulk-stop", "POST /api/v1/github/token",
    "POST /api/v1/google/calendar", "POST /api/v1/google/drive", "POST /api/v1/google/gmail",
    "POST /api/v1/muxspect/dock/clear", "POST /api/v1/pane/open", "POST /api/v1/pane/title",
    "POST /api/v1/ptyshell/create", "POST /api/v1/ptyshell/input", "POST /api/v1/ptyshell/resize",
    "POST /api/v1/ptyshell/stop", "POST /api/v1/sensitive/:action", "POST /api/v1/shell/create",
    "POST /api/v1/shell/input", "POST /api/v1/shell/stop", "POST /api/v1/tab/activate", "POST /api/v1/tab/name",
    "POST /api/v1/tab/new", "POST /api/v1/ui/browser/act", "POST /api/v1/ui/browser/back",
    "POST /api/v1/ui/browser/dispatch_key", "POST /api/v1/ui/browser/end_bypass", "POST /api/v1/ui/browser/eval",
    "POST /api/v1/ui/browser/focus_element", "POST /api/v1/ui/browser/forward", "POST /api/v1/ui/browser/handoff",
    "POST /api/v1/ui/browser/navigate", "POST /api/v1/ui/browser/open", "POST /api/v1/ui/browser/reload",
    "POST /api/v1/ui/browser/request_bypass", "POST /api/v1/ui/browser/set_files",
    "POST /api/v1/ui/browser/vault_fill", "POST /api/v1/ui/click", "POST /api/v1/ui/shortcuts/press",
    "POST /api/v1/ui/shortcuts/run", "POST /api/v1/vault/generate", "POST /api/v1/vault/request", "POST /api/v1/widgets/install",
    "POST /api/v1/window/focus", "POST /api/v1/window/name", "POST /api/v1/work-claims",
    "POST /api/v1/work-claims/release", "POST /api/v1/workspace/name",
    "POST /api/v1/agent/detach", "POST /api/v1/agent/start", "POST /api/v1/agent/stop",
];

/// Window and host plumbing, and the service call the window makes.
pub(super) const PLUMBING: &[&str] = &[
    "POST /agentmux/service", "POST /api/v1/attachments/upload", "POST /api/v1/host/app_update",
    "POST /api/v1/host/browser_attention", "POST /api/v1/host/browser_bypass", "POST /api/v1/host/browser_dialog",
    "POST /api/v1/host/browser_navigation", "POST /api/v1/host/browser_pause", "POST /api/v1/host/browser_popup",
    "POST /api/v1/host/browser_popup_window", "POST /api/v1/host/desktop_request", "POST /api/v1/host/desktop_stop",
    "POST /api/v1/host/google", "POST /api/v1/host/job_templates", "POST /api/v1/host/jobs", "POST /api/v1/host/operator_attest",
    "POST /api/v1/host/upgrade_request", "POST /api/v1/host/vault", "POST /api/v1/host/widget_approval",
    "POST /api/v1/voice/transcribe",
];

/// Recorded by the route's own handler when it goes through; a refusal is recorded by the layer.
pub(super) const HANDLER: &[&str] = &[
    "POST /agentmux/reactive/inject", "POST /agentmux/reactive/inject-files", "POST /api/v1/agent/env",
    "POST /api/v1/agent/seats",
];

/// Answers a question or carries a stream; nothing it changes is kept.
pub(super) const NEVER: &[&str] = &[
    "POST /agentmux/agents/resolve", "POST /agentmux/wps/publish", "POST /api/post-chat-message",
    "POST /api/v1/agent/memory/session-start/ack", "POST /api/v1/agent/memory/session-start/part",
    "POST /api/v1/agent/self/context", "POST /api/v1/agent/self/mcp-beat", "POST /api/v1/agent/self/mcp-health",
    "POST /api/v1/agent/tool-chunk", "POST /api/v1/agent/watch", "POST /api/v1/agent/workers",
    "POST /api/v1/askpass",
    "POST /api/v1/ptyshell/read", "POST /api/v1/ptyshell/status", "POST /api/v1/shell/output",
    "POST /api/v1/shell/status", "POST /api/v1/ui/browser/actions", "POST /api/v1/ui/browser/dialogs",
    "POST /api/v1/ui/browser/find", "POST /api/v1/ui/browser/focus_info", "POST /api/v1/ui/browser/snapshot",
    "POST /api/v1/ui/browser/wait_for", "POST /api/v1/ui/query", "POST /api/v1/ui/record", "POST /api/v1/ui/screenshot",
    "POST /api/v1/ui/shortcuts/list", "POST /webhook/whatsapp",
];

/// How `route` (`"METHOD /template"`) is recorded; `None` for a route in no list.
pub(super) fn policy(route: &str) -> Option<Policy> {
    if ALWAYS.contains(&route) {
        Some(Policy::Always)
    } else if PLUMBING.contains(&route) {
        Some(Policy::Plumbing)
    } else if HANDLER.contains(&route) {
        Some(Policy::Handler)
    } else if NEVER.contains(&route) {
        Some(Policy::Never)
    } else {
        None
    }
}
