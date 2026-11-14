// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which of srv's routes a container agent may call: every route has an [`Audience`], and a route
//! that has none is refused.
//!
//! The rule (operator, 2026-10-28): a sandbox agent can do everything a native agent can do except
//! directly update and run files on the machine. [`Audience::Everyone`] is the normal answer. A
//! `NativeOnly` entry names the host capability (`host_exec`, `host_write` or `host_read`) that is
//! the reason and tells the caller what to do instead; an `AppOnly` entry is the app's own
//! plumbing. `docs/specs/container-agent-runtime.md` has the table of what is refused and why.
//!
//! Declaring is enforced two ways. A test reads `server/routes.rs` and fails when a registered
//! route matches no entry (and when an entry names a route srv no longer serves), so a route cannot
//! be added without choosing its audience. And a path that matches no entry at run time, such as a
//! route registered some way the test does not read, is refused with a message that says so; the
//! container listener logs the path. Matched on the raw path, exactly as the router matches it.

use super::audience::{Audience, HostCap, RuntimeCaps};

/// Run a command on the host, in any form.
pub const HOST_COMMANDS: &str = "container agents can't run commands on the host. Run it in your own shell in the container, \
                             or ask a native agent or your leader.";
/// The app's own plumbing: approvals and state the user's windows report.
pub const APP_ONLY: &str = "this route is for the AgentMux app itself, not for agents.";
/// Driving the operating system.
pub const DESKTOP: &str = "desktop control can launch programs on the host, so container agents can't use it. \
                       Ask a native agent or your leader.";
/// Generic gateways that reach everything else.
pub const GATEWAY: &str = "this is a generic gateway to the host that container agents can't use. Use the specific tool for what you need.";
/// Reading host files for the windows.
pub const HOST_FILES: &str = "this reads files on the host for the app's windows. A container agent reads its own files in /workspace directly; \
                          to show one in a pane, open it with the editor.";
/// Replacing the app.
pub const UPGRADE: &str = "upgrading AgentMux replaces the app on the host, so container agents can't. Ask a native agent or your leader.";
/// Global Memory is composed into every native agent's context at launch.
pub const GLOBAL_MEMORY: &str = "Global Memory is read by every native agent at launch, so a container agent can read it but not change it. \
                             Ask a native agent or your leader to write it.";
/// Srv does this for a container agent.
pub const SRV_DOES_IT: &str = "srv does this for container agents itself.";
/// Peer-to-peer routes between AgentMux instances.
pub const INSTANCE_ONLY: &str = "this route is for other AgentMux instances, not for agents.";
/// What a path with no entry is told.
pub const UNCLASSIFIED: &str = "this route has not been classified for container agents yet. Ask your leader to have its audience set.";

/// How an entry matches a path.
#[derive(Debug, Clone, Copy)]
pub enum Match {
    Exact(&'static str),
    Prefix(&'static str),
    /// A route as `routes.rs` registers it: `:name` stands for one segment, `*name` for the rest.
    Route(&'static str),
}

impl Match {
    pub fn matches(&self, path: &str) -> bool {
        match *self {
            Match::Exact(p) => path == p,
            Match::Prefix(p) => path.starts_with(p),
            Match::Route(pattern) => route_matches(pattern, path),
        }
    }
}

fn route_matches(pattern: &str, path: &str) -> bool {
    let mut pat = pattern.split('/');
    let mut got = path.split('/');
    loop {
        match (pat.next(), got.next()) {
            (None, None) => return true,
            (Some(p), Some(g)) if p.starts_with('*') => return !g.is_empty(),
            (Some(p), Some(g)) if p.starts_with(':') => {
                if g.is_empty() {
                    return false;
                }
            }
            (Some(p), Some(g)) if p == g => {}
            _ => return false,
        }
    }
}

/// Every route's audience, first match wins. The routes that are not for everyone come first, then
/// the ones that are, one entry per route as `routes.rs` registers it.
pub const ROUTES: &[(Match, Audience)] = &[
    // Commands on the host
    (Match::Prefix("/api/v1/shell/"), Audience::NativeOnly(HostCap::Exec, HOST_COMMANDS)),
    (Match::Prefix("/api/v1/ptyshell/"), Audience::NativeOnly(HostCap::Exec, HOST_COMMANDS)),
    (Match::Prefix("/api/v1/conn/"), Audience::NativeOnly(HostCap::Exec, HOST_COMMANDS)),
    (Match::Exact("/api/v1/askpass"), Audience::NativeOnly(HostCap::Exec, HOST_COMMANDS)),
    // Driving the OS
    (Match::Prefix("/api/v1/agent/desktop/"), Audience::NativeOnly(HostCap::Exec, DESKTOP)),
    // The app's own plumbing
    (Match::Prefix("/api/v1/host/"), Audience::AppOnly(APP_ONLY)),
    // The app's and the relay's own controls: no agent tool calls them and the handlers don't check who asks
    (Match::Prefix("/agentmux/reactive/poller/"), Audience::AppOnly(APP_ONLY)),
    (Match::Exact("/agentmux/reactive/unregister"), Audience::AppOnly(APP_ONLY)),
    (Match::Exact("/agentmux/reactive/ensure-signing-key"), Audience::AppOnly(APP_ONLY)),
    (Match::Exact("/agentmux/reactive/agents"), Audience::AppOnly(APP_ONLY)),
    (Match::Exact("/agentmux/reactive/audit"), Audience::AppOnly(APP_ONLY)),
    (Match::Exact("/agentmux/agent/holding"), Audience::AppOnly(APP_ONLY)),
    (Match::Exact("/agentmux/diag/sagas"), Audience::AppOnly(APP_ONLY)),
    (Match::Prefix("/api/messaging/"), Audience::AppOnly(APP_ONLY)),
    (Match::Exact("/api/post-chat-message"), Audience::AppOnly(APP_ONLY)),
    (Match::Prefix("/webhook/"), Audience::AppOnly(APP_ONLY)),
    // Generic gateways
    (Match::Exact("/agentmux/service"), Audience::NativeOnly(HostCap::Exec, GATEWAY)),
    (Match::Exact("/ws"), Audience::NativeOnly(HostCap::Exec, GATEWAY)),
    (Match::Exact("/agentmux/wps/publish"), Audience::NativeOnly(HostCap::Exec, GATEWAY)),
    // Host files: the windows read them to draw panes; a container agent reads its own in /workspace
    (Match::Exact("/agentmux/stream-local-file"), Audience::NativeOnly(HostCap::Read, HOST_FILES)),
    (Match::Exact("/agentmux/file"), Audience::NativeOnly(HostCap::Read, HOST_FILES)),
    (Match::Exact("/agentmux/file-icon"), Audience::NativeOnly(HostCap::Read, HOST_FILES)),
    // What every native agent reads at launch
    (Match::Exact("/api/v1/agent/globalmemory/write"), Audience::NativeOnly(HostCap::Write, GLOBAL_MEMORY)),
    (Match::Exact("/api/v1/agent/globalmemory/remove"), Audience::NativeOnly(HostCap::Write, GLOBAL_MEMORY)),
    (Match::Exact("/api/v1/agent/globalmemory/revert"), Audience::NativeOnly(HostCap::Write, GLOBAL_MEMORY)),
    (Match::Exact("/api/v1/agent/globalmemory/reach"), Audience::NativeOnly(HostCap::Write, GLOBAL_MEMORY)),
    // Replacing the app
    (Match::Exact("/api/v1/agent/self/upgrade"), Audience::NativeOnly(HostCap::Write, UPGRADE)),
    // Srv's own work for a container agent
    (Match::Exact("/agentmux/agents/self/keys"), Audience::AppOnly("srv signs for a container agent, which holds no key to fetch.")),
    (Match::Exact("/api/v1/agent/self/mcp-beat"), Audience::AppOnly("health beats belong to persistent controllers; a container's turns are per-exec.")),
    (Match::Exact("/agentmux/reactive/register"), Audience::AppOnly(SRV_DOES_IT)),
    // Between instances
    (Match::Exact("/agentmux/jobs/stop-run"), Audience::AppOnly(INSTANCE_ONLY)),
    (Match::Prefix("/agentmux/viewer/"), Audience::AppOnly(INSTANCE_ONLY)),
    (Match::Prefix("/.well-known/"), Audience::AppOnly(INSTANCE_ONLY)),
    // For everyone: whatever a native agent can use, a container agent can too.
    (Match::Route("/"), Audience::Everyone),
    (Match::Route("/agentmux/agent/broadcast"), Audience::Everyone),
    (Match::Route("/agentmux/agent/release"), Audience::Everyone),
    (Match::Route("/agentmux/agent/stop"), Audience::Everyone),
    (Match::Route("/agentmux/agent/stop-pending"), Audience::Everyone),
    (Match::Route("/agentmux/agents/resolve"), Audience::Everyone),
    (Match::Route("/agentmux/cron"), Audience::Everyone),
    (Match::Route("/agentmux/cron/:id"), Audience::Everyone),
    (Match::Route("/agentmux/cron/:id/run"), Audience::Everyone),
    (Match::Route("/agentmux/cron/:id/runs"), Audience::Everyone),
    (Match::Route("/agentmux/cron/runs/:run_id/report"), Audience::Everyone),
    (Match::Route("/agentmux/discovery"), Audience::Everyone),
    (Match::Route("/agentmux/fleet"), Audience::Everyone),
    (Match::Route("/agentmux/fleet/events"), Audience::Everyone),
    (Match::Route("/agentmux/identity/fallbacks"), Audience::Everyone),
    (Match::Route("/agentmux/jobs"), Audience::Everyone),
    (Match::Route("/agentmux/jobs/:id"), Audience::Everyone),
    (Match::Route("/agentmux/jobs/:id/run"), Audience::Everyone),
    (Match::Route("/agentmux/jobs/:id/runs"), Audience::Everyone),
    (Match::Route("/agentmux/jobs/runs/:run_id/report"), Audience::Everyone),
    (Match::Route("/agentmux/old-versions"), Audience::Everyone),
    (Match::Route("/agentmux/quit-question"), Audience::Everyone),
    (Match::Route("/agentmux/reactive/agent"), Audience::Everyone),
    (Match::Route("/agentmux/reactive/agent-names"), Audience::Everyone),
    (Match::Route("/agentmux/reactive/availability"), Audience::Everyone),
    (Match::Route("/agentmux/reactive/history/search"), Audience::Everyone),
    (Match::Route("/agentmux/reactive/inject"), Audience::Everyone),
    (Match::Route("/agentmux/reactive/inject-files"), Audience::Everyone),
    (Match::Route("/agentmux/reactive/transcript"), Audience::Everyone),
    (Match::Route("/agentmux/remote/op"), Audience::Everyone),
    (Match::Route("/agentmux/stream-file"), Audience::Everyone),
    (Match::Route("/agentmux/stream-file/*path"), Audience::Everyone),
    (Match::Route("/agentmux/widget-files/:id/:hash/:key/*path"), Audience::Everyone),
    (Match::Route("/agentmux/widget-sdk/:file"), Audience::Everyone),
    (Match::Route("/api/v1/model/:route/*tail"), Audience::Everyone),
    (Match::Route("/agentmux/work"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/accept"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/ask"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/complete"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/heartbeat"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/hold"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/release"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/send_back"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/submit"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/unhold"), Audience::Everyone),
    (Match::Route("/agentmux/work/:id/update"), Audience::Everyone),
    (Match::Route("/agentmux/work/act"), Audience::Everyone),
    (Match::Route("/agentmux/work/boards"), Audience::Everyone),
    (Match::Route("/agentmux/work/claim"), Audience::Everyone),
    (Match::Route("/agentmux/work/media"), Audience::Everyone),
    (Match::Route("/api/bus/agents"), Audience::Everyone),
    (Match::Route("/api/bus/broadcast"), Audience::Everyone),
    (Match::Route("/api/bus/inject"), Audience::Everyone),
    (Match::Route("/api/bus/messages"), Audience::Everyone),
    (Match::Route("/api/bus/messages/delete"), Audience::Everyone),
    (Match::Route("/api/bus/register"), Audience::Everyone),
    (Match::Route("/api/bus/send"), Audience::Everyone),
    (Match::Route("/api/lan-instances"), Audience::Everyone),
    (Match::Route("/api/v1/agent/compaction-started"), Audience::Everyone),
    (Match::Route("/api/v1/agent/container/caches"), Audience::Everyone),
    (Match::Route("/api/v1/agent/container/limits"), Audience::Everyone),
    (Match::Route("/api/v1/agent/container/update"), Audience::Everyone),
    (Match::Route("/api/v1/agent/create"), Audience::Everyone),
    (Match::Route("/api/v1/agent/delete"), Audience::Everyone),
    (Match::Route("/api/v1/agent/detach"), Audience::Everyone),
    (Match::Route("/api/v1/agent/dev_server/register"), Audience::Everyone),
    (Match::Route("/api/v1/agent/env"), Audience::Everyone),
    (Match::Route("/api/v1/agent/globalmemory/diff"), Audience::Everyone),
    (Match::Route("/api/v1/agent/globalmemory/history"), Audience::Everyone),
    (Match::Route("/api/v1/agent/globalmemory/list"), Audience::Everyone),
    (Match::Route("/api/v1/agent/globalmemory/read"), Audience::Everyone),
    (Match::Route("/api/v1/agent/history/reclaim"), Audience::Everyone),
    (Match::Route("/api/v1/agent/identity/accounts"), Audience::Everyone),
    (Match::Route("/api/v1/agent/identity/signin/notice"), Audience::Everyone),
    (Match::Route("/api/v1/agent/identity/switch"), Audience::Everyone),
    (Match::Route("/api/v1/agent/identity/validate"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/diff"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/history"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/list"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/read"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/revert"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/session-start/ack"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/session-start/part"), Audience::Everyone),
    (Match::Route("/api/v1/agent/memory/write"), Audience::Everyone),
    (Match::Route("/api/v1/agent/migrate-ids"), Audience::Everyone),
    (Match::Route("/api/v1/agent/open"), Audience::Everyone),
    (Match::Route("/api/v1/agent/open_many"), Audience::Everyone),
    (Match::Route("/api/v1/agent/pane/close"), Audience::Everyone),
    (Match::Route("/api/v1/agent/preset/get"), Audience::Everyone),
    (Match::Route("/api/v1/agent/preset/list"), Audience::Everyone),
    (Match::Route("/api/v1/agent/remote/op"), Audience::Everyone),
    (Match::Route("/api/v1/agent/rename"), Audience::Everyone),
    (Match::Route("/api/v1/agent/update"), Audience::Everyone),
    (Match::Route("/api/v1/agent/bundles"), Audience::Everyone),
    (Match::Route("/api/v1/agent/hide"), Audience::Everyone),
    (Match::Route("/api/v1/agent/unhide"), Audience::Everyone),
    (Match::Route("/api/v1/agent/seats"), Audience::Everyone),
    (Match::Route("/api/v1/agent/self/compact"), Audience::Everyone),
    (Match::Route("/api/v1/agent/self/context"), Audience::Everyone),
    (Match::Route("/api/v1/agent/self/fresh"), Audience::Everyone),
    (Match::Route("/api/v1/agent/self/mcp-heal"), Audience::Everyone),
    (Match::Route("/api/v1/agent/self/mcp-health"), Audience::Everyone),
    (Match::Route("/api/v1/agent/self/quit"), Audience::Everyone),
    (Match::Route("/api/v1/agent/shutdown/:request_id"), Audience::Everyone),
    (Match::Route("/api/v1/agent/skill"), Audience::Everyone),
    (Match::Route("/api/v1/agent/spawn"), Audience::Everyone),
    (Match::Route("/api/v1/agent/spawn_many"), Audience::Everyone),
    (Match::Route("/api/v1/agent/start"), Audience::Everyone),
    (Match::Route("/api/v1/agent/status"), Audience::Everyone),
    (Match::Route("/api/v1/agent/stop"), Audience::Everyone),
    (Match::Route("/api/v1/agent/takeover"), Audience::Everyone),
    (Match::Route("/api/v1/agent/tool-chunk"), Audience::Everyone),
    (Match::Route("/api/v1/agent/watch"), Audience::Everyone),
    (Match::Route("/api/v1/agent/workers"), Audience::Everyone),
    (Match::Route("/api/v1/agent/routes"), Audience::Everyone),
    (Match::Route("/api/v1/app/connectivity"), Audience::Everyone),
    (Match::Route("/api/v1/app/health"), Audience::Everyone),
    (Match::Route("/api/v1/app/logs"), Audience::Everyone),
    (Match::Route("/api/v1/app/update-status"), Audience::Everyone),
    (Match::Route("/api/v1/attachments/:id/:kind"), Audience::Everyone),
    (Match::Route("/api/v1/attachments/upload"), Audience::Everyone),
    (Match::Route("/api/v1/aws/credentials"), Audience::Everyone),
    (Match::Route("/api/v1/bulletin"), Audience::Everyone),
    (Match::Route("/api/v1/coord/posts"), Audience::Everyone),
    (Match::Route("/api/v1/fleet/bulk-stop"), Audience::Everyone),
    (Match::Route("/api/v1/github/token"), Audience::Everyone),
    (Match::Route("/api/v1/github/push-scan"), Audience::Everyone),
    (Match::Route("/api/v1/github/pr-merge"), Audience::Everyone),
    (Match::Route("/api/v1/google/calendar"), Audience::Everyone),
    (Match::Route("/api/v1/google/connect"), Audience::Everyone),
    (Match::Route("/api/v1/google/connect/cancel"), Audience::Everyone),
    (Match::Route("/api/v1/google/connect/status"), Audience::Everyone),
    (Match::Route("/api/v1/google/disconnect"), Audience::Everyone),
    (Match::Route("/api/v1/google/drive"), Audience::Everyone),
    (Match::Route("/api/v1/google/gmail"), Audience::Everyone),
    (Match::Route("/api/v1/google/grant"), Audience::Everyone),
    (Match::Route("/api/v1/google/self/services"), Audience::Everyone),
    (Match::Route("/api/v1/google/status"), Audience::Everyone),
    (Match::Route("/api/v1/layout"), Audience::Everyone),
    (Match::Route("/api/v1/message-flow"), Audience::Everyone),
    (Match::Route("/api/v1/message-stats"), Audience::Everyone),
    (Match::Route("/api/v1/pr-flow"), Audience::Everyone),
    (Match::Route("/api/v1/task-flow"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/background-tasks"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/conversations"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/describe"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/dock"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/dock/clear"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/find"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/instances"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/layout"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/list"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/migrations"), Audience::Everyone),
    (Match::Route("/api/v1/muxspect/verify-sender"), Audience::Everyone),
    (Match::Route("/api/v1/pane/open"), Audience::Everyone),
    (Match::Route("/api/v1/pane/title"), Audience::Everyone),
    (Match::Route("/api/v1/quota-mode"), Audience::Everyone),
    (Match::Route("/api/v1/self"), Audience::Everyone),
    (Match::Route("/api/v1/sensitive/:action"), Audience::Everyone),
    (Match::Route("/api/v1/tab/activate"), Audience::Everyone),
    (Match::Route("/api/v1/tab/name"), Audience::Everyone),
    (Match::Route("/api/v1/tab/new"), Audience::Everyone),
    (Match::Route("/api/v1/tabs"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/act"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/actions"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/back"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/capture"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/close"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/dialogs"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/dispatch_key"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/end_bypass"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/eval"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/find"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/focus_element"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/focus_info"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/forward"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/generate_password"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/handoff"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/handoff_status"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/request"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/navigate"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/open"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/release"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/reload"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/request_bypass"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/retake"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/set_files"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/signin"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/snapshot"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/take_control"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/terminal"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/vault_fill"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/wait_for"), Audience::Everyone),
    (Match::Route("/api/v1/ui/browser/zoom"), Audience::Everyone),
    (Match::Route("/api/v1/ui/click"), Audience::Everyone),
    (Match::Route("/api/v1/ui/query"), Audience::Everyone),
    (Match::Route("/api/v1/ui/record"), Audience::Everyone),
    (Match::Route("/api/v1/ui/screenshot"), Audience::Everyone),
    (Match::Route("/api/v1/ui/shortcuts/list"), Audience::Everyone),
    (Match::Route("/api/v1/ui/shortcuts/press"), Audience::Everyone),
    (Match::Route("/api/v1/ui/shortcuts/run"), Audience::Everyone),
    (Match::Route("/api/v1/ui/shortcuts/set"), Audience::Everyone),
    (Match::Route("/api/v1/ui/shortcuts/reset"), Audience::Everyone),
    (Match::Route("/api/v1/vault"), Audience::Everyone),
    (Match::Route("/api/v1/vault/generate"), Audience::Everyone),
    (Match::Route("/api/v1/vault/history"), Audience::Everyone),
    (Match::Route("/api/v1/vault/import"), Audience::Everyone),
    (Match::Route("/api/v1/vault/show"), Audience::Everyone),
    (Match::Route("/api/v1/vault/update"), Audience::Everyone),
    (Match::Route("/api/v1/vault/verify"), Audience::Everyone),
    (Match::Route("/api/v1/vault/health"), Audience::Everyone),
    (Match::Route("/api/v1/vault/list"), Audience::Everyone),
    (Match::Route("/api/v1/vault/on_screen"), Audience::Everyone),
    (Match::Route("/api/v1/vault/request"), Audience::Everyone),
    (Match::Route("/api/v1/vault/rotate"), Audience::Everyone),
    (Match::Route("/api/v1/voice/transcribe"), Audience::Everyone),
    (Match::Route("/api/v1/voice/status"), Audience::Everyone),
    (Match::Route("/api/v1/widgets"), Audience::Everyone),
    (Match::Route("/api/v1/widgets/install"), Audience::Everyone),
    (Match::Route("/api/v1/window/focus"), Audience::Everyone),
    (Match::Route("/api/v1/window/name"), Audience::Everyone),
    (Match::Route("/api/v1/windows"), Audience::Everyone),
    (Match::Route("/api/v1/work-claims"), Audience::Everyone),
    (Match::Route("/api/v1/work-claims/release"), Audience::Everyone),
    (Match::Route("/api/v1/work-facts"), Audience::Everyone),
    (Match::Route("/api/v1/work-facts/who"), Audience::Everyone),
    (Match::Route("/api/v1/workspace/name"), Audience::Everyone),
    (Match::Route("/api/v1/workspaces"), Audience::Everyone),
    (Match::Route("/docsite/*path"), Audience::Everyone),
    (Match::Route("/health"), Audience::Everyone),
    (Match::Route("/schema/*path"), Audience::Everyone),
];

/// The audience of `path`, or `None` when no entry claims it.
pub fn audience_of(path: &str) -> Option<&'static Audience> {
    ROUTES.iter().find(|(m, _)| m.matches(path)).map(|(_, a)| a)
}

/// Where srv's own routes live. A path under one that no entry claims might be a route nobody classified,
/// so it is refused; the build fails on such a route, this is for one that slipped past.
const ROUTE_NAMESPACES: [&str; 2] = ["/api/", "/agentmux/"];

/// Why a container agent may not call `path`, or `None` when it may. A path no entry claims is refused
/// as [`UNCLASSIFIED`] when it is under srv's route namespaces. Any other path is what the router's
/// fallback serves (the frontend's files, or a 404), as it always was.
pub fn refusal(path: &str) -> Option<&'static str> {
    match audience_of(path) {
        Some(audience) => audience.refusal(RuntimeCaps::CONTAINER),
        None if ROUTE_NAMESPACES.iter().any(|ns| path.starts_with(ns)) => Some(UNCLASSIFIED),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A route as a request would carry it: `:name` and `*name` filled in.
    fn sample(route: &str) -> String {
        route.split('/').map(|s| if s.starts_with(':') || s.starts_with('*') { "x" } else { s }).collect::<Vec<_>>().join("/")
    }

    /// No two `Route` entries for the same route, and none that an earlier entry already claims.
    #[test]
    fn no_entry_is_dead() {
        for (i, (m, _)) in ROUTES.iter().enumerate() {
            if let Match::Route(p) = m {
                let s = sample(p);
                let first = ROUTES.iter().position(|(e, _)| e.matches(&s)).unwrap();
                assert_eq!(first, i, "{p} is claimed by an earlier entry ({:?})", ROUTES[first].0);
            }
        }
    }

    #[test]
    fn a_route_nobody_classified_is_refused_with_a_message_that_says_so() {
        assert_eq!(refusal("/api/v1/not-a-route-yet"), Some(UNCLASSIFIED));
        assert_eq!(refusal("/agentmux/not-a-route-yet"), Some(UNCLASSIFIED));
        assert!(UNCLASSIFIED.contains("Ask your leader"));
    }

    /// The frontend's files and unknown paths are not routes: the router serves or 404s them, as before.
    #[test]
    fn a_path_the_frontend_fallback_serves_is_not_refused() {
        for path in ["/assets/index-abc.js", "/index.html", "/favicon.ico", "/nope"] {
            assert_eq!(refusal(path), None, "{path}");
        }
    }

    #[test]
    fn a_param_matches_one_segment_and_a_rest_matches_one_or_more() {
        assert!(route_matches("/agentmux/work/:id", "/agentmux/work/12"));
        assert!(!route_matches("/agentmux/work/:id", "/agentmux/work/"));
        assert!(!route_matches("/agentmux/work/:id", "/agentmux/work/12/accept"));
        assert!(route_matches("/docsite/*path", "/docsite/a/b/c.html"));
        assert!(!route_matches("/docsite/*path", "/docsite/"));
        assert!(route_matches("/", "/"));
        assert!(!route_matches("/", "/x"));
    }

    /// Every refused entry names the capability it needs, or says it is the app's, so the boundary
    /// reads in one pass.
    #[test]
    fn the_boundary_in_one_table() {
        let mut table = String::new();
        for (m, a) in ROUTES.iter().filter(|(_, a)| !matches!(a, Audience::Everyone)) {
            table.push_str(&format!("{:<12} {:?}\n", a.cap_name(), m));
        }
        assert!(table.contains("host_exec") && table.contains("host_write") && table.contains("host_read"), "{table}");
    }
}
