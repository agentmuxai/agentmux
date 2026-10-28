// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which of srv's routes a container agent may call.
//!
//! The rule (operator, 2026-10-28): a sandbox agent can do everything a native agent can do except
//! directly update and run files on the machine. So a container token is accepted on every route
//! except the ones listed here, which write to, execute on, or read from the host outside the
//! container's own mounts, or that belong to the AgentMux app itself. A route added later is open
//! to containers unless it is added here. `docs/specs/container-agent-runtime.md` has the table of
//! what is refused and why; every refusal answers with its reason and what to do instead.
//!
//! Matched on the raw path, exactly as the router matches it.

/// Run a command on the host, in any form.
const HOST_COMMANDS: &str = "container agents can't run commands on the host. Run it in your own shell in the container, \
                             or ask a native agent or your leader.";
/// The app's own plumbing: approvals and state the user's windows report.
const APP_ONLY: &str = "this route is for the AgentMux app itself, not for agents.";
/// Driving the operating system.
const DESKTOP: &str = "desktop control can launch programs on the host, so container agents can't use it. \
                       Ask a native agent or your leader.";
/// Generic gateways that reach everything else.
const GATEWAY: &str = "this is a generic gateway to the host that container agents can't use. Use the specific tool for what you need.";
/// A host path that would have to be checked against the container's mounts first.
const HOST_PATHS: &str = "container agents can't read or open host paths outside their own workspace yet. \
                          Put the file in /workspace, or ask a native agent.";
/// Replacing the app.
const UPGRADE: &str = "upgrading AgentMux replaces the app on the host, so container agents can't. Ask a native agent or your leader.";
/// The host's own credentials.
const HOST_CREDENTIALS: &str = "the host's GitHub tokens are for native agents; the container has its own git and gh setup.";
/// Global Memory is composed into every native agent's context at launch.
const GLOBAL_MEMORY: &str = "Global Memory is read by every native agent at launch, so a container agent can read it but not change it. \
                             Ask a native agent or your leader to write it.";
/// Srv does this for a container agent.
const SRV_DOES_IT: &str = "srv does this for container agents itself.";
/// Peer-to-peer routes between AgentMux instances.
const INSTANCE_ONLY: &str = "this route is for other AgentMux instances, not for agents.";

enum Match {
    Exact(&'static str),
    Prefix(&'static str),
}

/// Every refusal: the route, and what the caller is told. The table in the runtime doc is this list.
const REFUSED: &[(Match, &str)] = &[
    // Commands on the host
    (Match::Prefix("/api/v1/shell/"), HOST_COMMANDS),
    (Match::Prefix("/api/v1/ptyshell/"), HOST_COMMANDS),
    (Match::Prefix("/api/v1/conn/"), HOST_COMMANDS),
    (Match::Exact("/api/v1/askpass"), HOST_COMMANDS),
    // Driving the OS
    (Match::Prefix("/api/v1/agent/desktop/"), DESKTOP),
    // The app's own plumbing
    (Match::Prefix("/api/v1/host/"), APP_ONLY),
    // The app's and the relay's own controls: no agent tool calls them and the handlers don't check who asks
    (Match::Prefix("/agentmux/reactive/poller/"), APP_ONLY),
    (Match::Exact("/agentmux/reactive/unregister"), APP_ONLY),
    (Match::Exact("/agentmux/reactive/ensure-signing-key"), APP_ONLY),
    (Match::Exact("/agentmux/reactive/agents"), APP_ONLY),
    (Match::Exact("/agentmux/reactive/audit"), APP_ONLY),
    (Match::Exact("/agentmux/agent/holding"), APP_ONLY),
    (Match::Exact("/agentmux/diag/sagas"), APP_ONLY),
    (Match::Prefix("/api/messaging/"), APP_ONLY),
    (Match::Exact("/api/post-chat-message"), APP_ONLY),
    (Match::Prefix("/webhook/"), APP_ONLY),
    // Generic gateways
    (Match::Exact("/agentmux/service"), GATEWAY),
    (Match::Exact("/ws"), GATEWAY),
    (Match::Exact("/agentmux/wps/publish"), GATEWAY),
    // Host paths, until they are checked against the container's mounts
    (Match::Exact("/agentmux/stream-local-file"), HOST_PATHS),
    (Match::Exact("/agentmux/file"), HOST_PATHS),
    (Match::Exact("/agentmux/file-icon"), HOST_PATHS),
    (Match::Exact("/api/v1/attachments/upload"), HOST_PATHS),
    (Match::Exact("/agentmux/work/media"), HOST_PATHS),
    (Match::Exact("/api/v1/ui/browser/set_files"), HOST_PATHS),
    (Match::Exact("/api/v1/pane/open"), HOST_PATHS),
    // What every native agent reads at launch
    (Match::Exact("/api/v1/agent/globalmemory/write"), GLOBAL_MEMORY),
    (Match::Exact("/api/v1/agent/globalmemory/remove"), GLOBAL_MEMORY),
    (Match::Exact("/api/v1/agent/globalmemory/revert"), GLOBAL_MEMORY),
    // Replacing the app
    (Match::Exact("/api/v1/agent/self/upgrade"), UPGRADE),
    // The host's credentials
    (Match::Exact("/api/v1/github/token"), HOST_CREDENTIALS),
    // Srv's own work for a container agent
    (Match::Exact("/agentmux/agents/self/keys"), "srv signs for a container agent, which holds no key to fetch."),
    (Match::Exact("/api/v1/agent/self/mcp-beat"), "health beats belong to persistent controllers; a container's turns are per-exec."),
    (Match::Exact("/agentmux/reactive/register"), SRV_DOES_IT),
    // Between instances
    (Match::Exact("/agentmux/jobs/stop-run"), INSTANCE_ONLY),
    (Match::Prefix("/agentmux/viewer/"), INSTANCE_ONLY),
    (Match::Prefix("/.well-known/"), INSTANCE_ONLY),
];

/// Why a container agent may not call `path`, or `None` when it may.
pub fn refusal(path: &str) -> Option<&'static str> {
    REFUSED
        .iter()
        .find(|(m, _)| match m {
            Match::Exact(p) => path == *p,
            Match::Prefix(p) => path.starts_with(p),
        })
        .map(|(_, why)| *why)
}

/// What a refused container agent is told: the reason, in a sentence of its own.
pub fn refusal_message(path: &str) -> Option<String> {
    refusal(path).map(|why| format!("this route is not available to container agents: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTES: &str = include_str!("../server/routes.rs");

    /// A refusal that names a route srv does not have is a typo, or stale.
    #[test]
    fn every_refused_route_is_one_srv_serves() {
        // A few are registered elsewhere: the viewer, the peers' stop, the discovery file, the WebSocket, srv's own signing.
        let elsewhere = ["/agentmux/jobs/stop-run", "/.well-known/", "/agentmux/viewer/", "/ws", "/agentmux/agents/self/keys"];
        for (m, _) in REFUSED {
            let path = match m {
                Match::Exact(p) | Match::Prefix(p) => *p,
            };
            // A route literal in the router starts with the path (exactly, or as the prefix's start).
            assert!(ROUTES.contains(&format!("\"{path}")) || elsewhere.contains(&path), "{path} is not a route srv serves");
        }
    }

    #[test]
    fn what_a_container_agent_cannot_do_says_why_and_what_to_do() {
        let cases = [
            ("/api/v1/shell/create", "can't run commands on the host"),
            ("/api/v1/ptyshell/input", "can't run commands on the host"),
            ("/api/v1/conn/list", "can't run commands on the host"),
            ("/api/v1/agent/desktop/authorize", "launch programs"),
            ("/api/v1/host/vault", "for the AgentMux app itself"),
            ("/agentmux/reactive/poller/config", "for the AgentMux app itself"),
            ("/api/messaging/discord/send", "for the AgentMux app itself"),
            ("/agentmux/service", "generic gateway"),
            ("/agentmux/stream-local-file", "outside their own workspace"),
            ("/api/v1/agent/self/upgrade", "replaces the app"),
            ("/api/v1/github/token", "native agents"),
            ("/api/v1/agent/globalmemory/write", "read it but not change it"),
        ];
        for (path, says) in cases {
            let message = refusal_message(path).unwrap_or_else(|| panic!("{path} should be refused"));
            assert!(message.starts_with("this route is not available to container agents: "), "{message}");
            assert!(message.contains(says), "{path}: {message}");
        }
    }

    /// The operator's rule: the tools a native agent has are open to a container agent.
    #[test]
    fn what_a_native_agent_can_do_beyond_the_host_is_open() {
        for open in [
            "/api/v1/work-facts/who",
            "/api/v1/coord/posts",
            "/api/v1/vault",
            "/api/v1/vault/request",
            "/api/v1/ui/browser/open",
            "/api/v1/ui/browser/snapshot",
            "/api/v1/ui/browser/find",
            "/api/v1/ui/browser/act",
            "/api/v1/ui/browser/navigate",
            "/api/v1/ui/browser/eval",
            "/api/v1/ui/browser/vault_fill",
            "/api/v1/ui/screenshot",
            "/api/v1/ui/query",
            "/api/v1/ui/click",
            "/api/v1/ui/shortcuts/list",
            "/api/v1/agent/dev_server/register",
            "/api/v1/widgets/install",
            "/agentmux/work",
            "/agentmux/cron",
            "/agentmux/reactive/inject",
            "/api/v1/agent/memory/write",
            "/api/v1/agent/spawn",
            "/api/v1/agent/open",
            "/api/v1/agent/tool-chunk",
            "/health",
            "/",
        ] {
            assert_eq!(refusal(open), None, "{open} should be open to containers");
        }
    }

    #[test]
    fn near_misses_are_not_refused_by_accident() {
        assert_eq!(refusal("/api/v1/shells"), None);
        assert_eq!(refusal("/api/v1/hostile"), None);
        assert_eq!(refusal("/api/v1/agent/desktops"), None);
    }
}
