// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a container agent is told when it calls a route it may not. The routes and who each is for
//! are in `route_audience` (every route has an audience; one that has none is refused), so there is
//! one list to read. This module is the container listener's view of it.

use crate::backend::route_audience::{self, UNCLASSIFIED};

/// Why a container agent may not call `path`, or `None` when it may.
pub fn refusal(path: &str) -> Option<&'static str> {
    route_audience::refusal(path)
}

/// What a refused container agent is told: the reason, in a sentence of its own. A path no entry
/// claims is logged, so it gets classified.
pub fn refusal_message(path: &str) -> Option<String> {
    let why = refusal(path)?;
    if why == UNCLASSIFIED {
        tracing::warn!(path, "a container agent called a route with no audience; add it to backend/route_audience.rs");
    }
    Some(format!("this route is not available to container agents: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            ("/agentmux/stream-local-file", "reads files on the host"),
            ("/api/v1/agent/self/upgrade", "replaces the app"),
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
            "/api/v1/vault/generate",
            "/api/v1/vault/history",
            "/api/v1/vault/import",
            "/api/v1/vault/request",
            "/api/v1/ui/browser/open",
            "/api/v1/ui/browser/snapshot",
            "/api/v1/ui/browser/find",
            "/api/v1/ui/browser/act",
            "/api/v1/ui/browser/navigate",
            "/api/v1/ui/browser/eval",
            "/api/v1/ui/browser/vault_fill",
            "/api/v1/ui/browser/signin",
            "/api/v1/ui/screenshot",
            "/api/v1/ui/record",
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
            "/api/v1/agent/create",
            "/api/v1/agent/open",
            "/api/v1/agent/tool-chunk",
            "/api/v1/agent/compaction-started",
            "/health",
            "/",
        ] {
            assert_eq!(refusal(open), None, "{open} should be open to containers");
        }
    }

    #[test]
    fn near_misses_are_not_taken_for_a_refused_prefix() {
        // Not routes, so nothing claims them; they are told so, not that a host route is closed.
        for path in ["/api/v1/shells", "/api/v1/hostile", "/api/v1/agent/desktops"] {
            assert_eq!(refusal(path), Some(UNCLASSIFIED), "{path}");
        }
    }
}
