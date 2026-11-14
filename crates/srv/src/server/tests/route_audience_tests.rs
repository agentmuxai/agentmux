// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The route audience table against the routes `routes.rs` registers. These tests read the server's own route
//! list, so they live in the server layer; the table and its other tests stay in `backend/route_audience.rs`.

use crate::backend::audience::Audience;
use crate::backend::route_audience::*;
use super::route_scan::registered_routes;

/// A route as a request would carry it: `:name` and `*name` filled in.
fn sample(route: &str) -> String {
    route.split('/').map(|s| if s.starts_with(':') || s.starts_with('*') { "x" } else { s }).collect::<Vec<_>>().join("/")
}

/// Routes srv serves that `routes.rs` does not register itself: the viewer, the peers' stop, the
/// discovery file, the WebSocket, srv's own signing.
const ELSEWHERE: [&str; 5] = ["/agentmux/jobs/stop-run", "/.well-known/", "/agentmux/viewer/", "/ws", "/agentmux/agents/self/keys"];

#[test]
fn every_registered_route_has_an_audience() {
    let routes = registered_routes();
    assert!(routes.len() > 200, "the parser found {} routes", routes.len());
    let unclaimed: Vec<_> = routes.iter().map(|r| &r.path).filter(|p| audience_of(&sample(p)).is_none()).collect();
    assert!(unclaimed.is_empty(), "routes with no audience in backend/route_audience.rs (add an entry; Everyone is the normal answer): {unclaimed:?}");
}

/// An entry that names a route srv does not have is a typo, or stale.
#[test]
fn every_entry_names_a_route_srv_serves() {
    let registered: Vec<String> = registered_routes().into_iter().map(|r| r.path).collect();
    for (m, _) in ROUTES {
        let named = match m {
            Match::Exact(p) | Match::Prefix(p) => {
                registered.iter().any(|r| r.starts_with(p)) || ELSEWHERE.contains(p)
            }
            Match::Route(p) => registered.iter().any(|r| r == p),
        };
        assert!(named, "{m:?} is not a route srv serves");
    }
}

/// The refusals before this change, entry for entry. The audiences must produce exactly this set
/// and these messages: moving a route in either direction is a separate, reviewed change.
fn todays_refused() -> Vec<(Match, &'static str)> {
    vec![
        (Match::Prefix("/api/v1/shell/"), HOST_COMMANDS),
        (Match::Prefix("/api/v1/ptyshell/"), HOST_COMMANDS),
        (Match::Prefix("/api/v1/conn/"), HOST_COMMANDS),
        (Match::Exact("/api/v1/askpass"), HOST_COMMANDS),
        (Match::Prefix("/api/v1/agent/desktop/"), DESKTOP),
        (Match::Prefix("/api/v1/host/"), APP_ONLY),
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
        (Match::Exact("/agentmux/service"), GATEWAY),
        (Match::Exact("/ws"), GATEWAY),
        (Match::Exact("/agentmux/wps/publish"), GATEWAY),
        (Match::Exact("/agentmux/stream-local-file"), HOST_FILES),
        (Match::Exact("/agentmux/file"), HOST_FILES),
        (Match::Exact("/agentmux/file-icon"), HOST_FILES),
        (Match::Exact("/api/v1/agent/globalmemory/write"), GLOBAL_MEMORY),
        (Match::Exact("/api/v1/agent/globalmemory/remove"), GLOBAL_MEMORY),
        (Match::Exact("/api/v1/agent/globalmemory/revert"), GLOBAL_MEMORY),
        (Match::Exact("/api/v1/agent/globalmemory/reach"), GLOBAL_MEMORY),
        (Match::Exact("/api/v1/agent/self/upgrade"), UPGRADE),
        (Match::Exact("/agentmux/agents/self/keys"), "srv signs for a container agent, which holds no key to fetch."),
        (Match::Exact("/api/v1/agent/self/mcp-beat"), "health beats belong to persistent controllers; a container's turns are per-exec."),
        (Match::Exact("/agentmux/reactive/register"), SRV_DOES_IT),
        (Match::Exact("/agentmux/jobs/stop-run"), INSTANCE_ONLY),
        (Match::Prefix("/agentmux/viewer/"), INSTANCE_ONLY),
        (Match::Prefix("/.well-known/"), INSTANCE_ONLY),
    ]
}

fn todays_refusal(path: &str) -> Option<&'static str> {
    todays_refused().into_iter().find(|(m, _)| m.matches(path)).map(|(_, why)| why)
}

#[test]
fn the_refused_set_is_the_one_before_audiences() {
    // Every route srv registers, and the paths the refusals name, answer as they did.
    let mut paths: Vec<String> = registered_routes().iter().map(|r| sample(&r.path)).collect();
    for (m, _) in todays_refused() {
        match m {
            Match::Exact(p) => paths.push(p.to_string()),
            Match::Prefix(p) => {
                paths.push(p.to_string());
                paths.push(format!("{p}more"));
            }
            Match::Route(_) => {}
        }
    }
    for path in paths {
        assert_eq!(refusal(&path), todays_refusal(&path), "{path}");
    }
    // Entry for entry: the same number of refusals, none added, none dropped.
    let refused_now = ROUTES.iter().filter(|(_, a)| !matches!(a, Audience::Everyone)).count();
    assert_eq!(refused_now, todays_refused().len());
}
