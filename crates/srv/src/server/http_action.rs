// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The record of what each HTTP caller did: one line in the audit log for every call of a
//! state-changing route (`POST`, `PUT`, `PATCH`, `DELETE`), saying who made it, which route, what
//! it acted on (the route's path parameters) and how it went (the response's status). The websocket
//! RPC has the same record (`backend::rpc::action_record`); both end in the same audit line.
//!
//! It is made in one place, a layer under the authentication layers, so a handler cannot forget it
//! and a route nobody decided is recorded ([`super::http_action_routes`], with a test that every
//! registered route is decided). The caller is whoever the request's agent token names
//! (`Caller::Agent`); a request without one is `unproven`, never the operator: nothing on an HTTP
//! request proves the window made it. A request's contents are never recorded; the one body read is
//! the service call's `service` and `method` names.
//!
//! A service call with no agent token is recorded as `unproven`, not skipped, so a process that merely
//! holds the instance key leaves a line. The window's own reads and bookkeeping ([`QUIET`]) are the
//! exception; a method not named there is recorded. The host's own plumbing ([`QUIET_SERVICES`]) is quiet
//! only while it succeeds: a call of it that is refused or fails is recorded. A write of an object's meta is left out only when every
//! key it writes is one the window keeps for itself ([`QUIET_META_KEYS`]); any other write is recorded with the
//! object and the key names, never the values.

use axum::body::Body;
use axum::extract::{MatchedPath, State};
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::caller::Caller;
use super::http_action_routes::{policy, Policy};
use super::AppState;
use crate::backend::rpc::action_record::{ActionRecord, Target};
use crate::backend::rpc::actor::Actor;

/// The generic service call: its `service` and `method` say what it does.
const SERVICE_ROUTE: &str = "/agentmux/service";
/// The largest service call read for its two names; a bigger one is refused with 413, as the
/// route's own body limit would, and the refusal is recorded.
const SERVICE_BODY_MAX: usize = 2 * 1024 * 1024;
/// The largest service reply read for its verdict.
const SERVICE_REPLY_MAX: usize = 32 * 1024 * 1024;
const MAX_FIELD: usize = 200;

/// The `service.method` calls that are not recorded when they carry no agent token: reads, and the window's
/// own bookkeeping, which it makes constantly. A method in no list is recorded.
const QUIET: &[&str] = &[
    "object.GetObject", "object.GetObjects",
    "client.GetClientData", "client.GetTab", "client.GetAllConnStatus", "client.FocusWindow", "client.TelemetryUpdate",
    "window.GetWindow", "window.FindWindowByLabel", "window.SaveSessionSnapshot", "window.TakeRestoredWindows",
    "window.ListClosedWindows", "window.WindowAgents", "window.PlaceWindowSlots", "window.SetWindowPosAndSize",
    "window.SetWindowOpacity", "window.SetWindowTopology", "window.MarkWindowOpen", "window.InheritWindowAppearance",
    "workspace.GetWorkspace", "workspace.ListWorkspaces", "workspace.ListUserWorkspaces", "workspace.UserWorkspaceAgents",
    "workspace.ExportUserWorkspace", "workspace.PreviewImport", "workspace.SetActiveTab", "workspace.Flush",
    "block.GetControllerStatus", "block.SaveTerminalState", "block.SendCommand", "userinput.SendUserInputResponse",
    "shell.ListActive", "cron.ListActive", "subagent.ListActive", "subagent.ListDispatches", "subagent.GetHistory",
    "subagent.GetInfo", "history.List", "history.ListForAgent", "history.Get",
];
/// The generic write of an object's meta, whose arguments are the object and the keys to set.
const META_WRITE: &str = "object.UpdateObjectMeta";
/// The meta keys the window writes constantly for itself (titles, activity summaries, prompts, timers, the
/// shell's directory); a write of only these, with no agent token, is not recorded.
const QUIET_META_KEYS: &[&str] = &[
    "term:ambient_summary", "term:ambient_summary_at", "term:ambient_summary_turn", "term:osc_title", "term:restored_summary",
    "term:last_prompt", "term:human_turns", "term:next_prompt_suggestion", "term:next_prompt_suggestion_gen",
    "term:question_timer", "term:awaiting_user", "term:awaiting_confirm", "cmd:cwd",
];
/// The services that are the host's own plumbing (the credential bridge, the host's registration): a call of
/// them with no agent token is not recorded while it succeeds, and is when it is refused or fails.
const QUIET_SERVICES: &[&str] = &["credential", "host_ipc"];

/// How a service call with no agent token is left out of the record.
#[derive(Debug, PartialEq)]
enum Quiet {
    /// Recorded.
    No,
    /// Never recorded.
    Always,
    /// Recorded only when the reply says it was refused or failed.
    UnlessFailed,
}

/// A service call's names, and for a write of an object's meta, the object and the keys it sets.
struct ServiceCall {
    names: String,
    object: Option<String>,
    keys: Option<Vec<String>>,
}

impl ServiceCall {
    /// How the call, with no agent token, is left out of the record.
    fn quiet(&self) -> Quiet {
        if self.names == META_WRITE {
            let quiet = self.keys.as_ref().is_some_and(|keys| !keys.is_empty() && keys.iter().all(|k| QUIET_META_KEYS.contains(&k.as_str())));
            return if quiet { Quiet::Always } else { Quiet::No };
        }
        if QUIET.contains(&self.names.as_str()) {
            Quiet::Always
        } else if self.names.split_once('.').is_some_and(|(service, _)| QUIET_SERVICES.contains(&service)) {
            Quiet::UnlessFailed
        } else {
            Quiet::No
        }
    }

    /// What the record says the call acted on: the names, and a meta write's object and key names (never values).
    fn subject(&self) -> String {
        let mut subject = self.names.clone();
        if let Some(object) = &self.object {
            subject.push_str(&format!(" object={object}"));
        }
        if let Some(keys) = &self.keys {
            subject.push_str(&format!(" keys={}", keys.join(",")));
        }
        subject
    }
}

/// The values a request's path gives for the route's `:name` parameters, as `name=value`.
pub(crate) fn path_params(template: &str, path: &str) -> Vec<String> {
    template
        .split('/')
        .zip(path.split('/'))
        .filter_map(|(t, p)| t.strip_prefix(':').map(|name| (name, p)))
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| format!("{name}={}", value.chars().take(MAX_FIELD).collect::<String>()))
        .collect()
}

/// A service call from its body, when it names a service and a method.
fn service_call(body: &[u8]) -> Option<ServiceCall> {
    let serde_json::Value::Object(fields) = serde_json::from_slice(body).ok()? else { return None };
    let name = |key: &str| fields.get(key)?.as_str().map(|s| s.chars().take(MAX_FIELD).collect::<String>());
    let names = format!("{}.{}", name("service")?, name("method")?);
    let args = fields.get("args").and_then(|a| a.as_array());
    let (mut object, mut keys) = (None, None);
    if names == META_WRITE {
        object = args.and_then(|a| a.first()).and_then(|v| v.as_str()).map(|s| s.chars().take(MAX_FIELD).collect());
        keys = args
            .and_then(|a| a.get(1))
            .and_then(|v| v.as_object())
            .map(|meta| meta.keys().map(|k| k.chars().take(MAX_FIELD).collect()).collect());
    }
    Some(ServiceCall { names, object, keys })
}

/// Whether a service reply says the call failed. The reply's own `error` text is not kept: it can
/// quote the request's values. (`success` is left out of a failed reply, so a reply without it failed.)
fn service_failure(reply: &[u8]) -> Option<String> {
    let serde_json::Value::Object(fields) = serde_json::from_slice(reply).ok()? else { return None };
    let ok = fields.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    (!ok).then(|| "service call failed".to_string())
}

fn is_write(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE | Method::CONNECT)
}

/// Layered inside `caller_middleware`: records the call once the route has answered.
pub(super) async fn record_action(State(state): State<AppState>, mut req: Request<Body>, next: Next) -> Response {
    if !is_write(req.method()) {
        return next.run(req).await;
    }
    let Some(template) = req.extensions().get::<MatchedPath>().map(|m| m.as_str().to_string()) else {
        return next.run(req).await;
    };
    let command = format!("{} {template}", req.method());
    let policy = policy(&command);
    let caller = req.extensions().get::<Caller>().cloned();
    if policy == Some(Policy::Never) {
        return next.run(req).await;
    }
    // The window's own calls and the host's cannot be told from a process that merely holds the
    // key, so these routes are recorded when an agent's token is on the request.
    let untokened = caller.as_ref().and_then(Caller::uid).is_none();
    if policy == Some(Policy::Plumbing) && untokened && template != SERVICE_ROUTE {
        return next.run(req).await;
    }
    let mut subject = path_params(&template, req.uri().path());
    let mut refused = None;
    let mut quiet_unless_failed = false;
    if template == SERVICE_ROUTE {
        let (parts, body) = req.into_parts();
        match axum::body::to_bytes(body, SERVICE_BODY_MAX).await {
            Ok(bytes) => {
                let call = service_call(&bytes);
                req = Request::from_parts(parts, Body::from(bytes));
                match call.as_ref().filter(|_| untokened).map(ServiceCall::quiet) {
                    Some(Quiet::Always) => return next.run(req).await,
                    Some(Quiet::UnlessFailed) => quiet_unless_failed = true,
                    _ => {}
                }
                subject.extend(call.map(|c| c.subject()));
            }
            Err(_) => {
                refused = Some((StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response());
                req = Request::from_parts(parts, Body::empty());
            }
        }
    }
    let request_id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let response = match refused {
        Some(r) => r,
        None => next.run(req).await,
    };
    let status = response.status();
    let mut error = (status.as_u16() >= 400).then(|| format!("HTTP {}", status.as_u16()));
    // A route that records its own successes still has its refusals recorded here, since it may
    // refuse before it gets to its own record.
    if policy == Some(Policy::Handler) && error.is_none() {
        return response;
    }
    // A service call answers 200 with its own verdict in the body.
    let mut response = response;
    if template == SERVICE_ROUTE && error.is_none() {
        let (parts, body) = response.into_parts();
        match axum::body::to_bytes(body, SERVICE_REPLY_MAX).await {
            Ok(bytes) => {
                error = service_failure(&bytes);
                response = Response::from_parts(parts, Body::from(bytes));
            }
            Err(_) => {
                error = Some("unreadable reply".to_string());
                response = (StatusCode::INTERNAL_SERVER_ERROR, "reply too large").into_response();
            }
        }
    }
    if quiet_unless_failed && error.is_none() {
        return response;
    }
    let store = state.mstore.clone();
    // Off the reply path: naming the agent reads the store.
    tokio::task::spawn_blocking(move || {
        let actor = match caller.as_ref().and_then(Caller::uid) {
            Some(uid) => Actor::Agent(store.agent_names_by_id(uid).ok().flatten().map(|row| row.slug).unwrap_or_else(|| uid.to_string())),
            None => Actor::Unproven,
        };
        let subject = (!subject.is_empty()).then(|| subject.join(" ").chars().take(MAX_FIELD * 2).collect());
        let record = ActionRecord { actor, command, target: Target { agent: None, block_id: None, subject }, request_id, error };
        crate::backend::reactive::get_global_handler().audit_action(&record);
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routes_path_parameters_are_read_from_the_path() {
        assert_eq!(path_params("/agentmux/work/:id/heartbeat", "/agentmux/work/t-9/heartbeat"), vec!["id=t-9"]);
        assert_eq!(path_params("/a/:x/b/:y", "/a/1/b/2"), vec!["x=1", "y=2"]);
        assert!(path_params("/api/v1/tab/new", "/api/v1/tab/new").is_empty());
        assert_eq!(path_params("/a/:id", "/a/").len(), 0);
    }

    #[test]
    fn only_the_service_and_method_names_are_read_from_a_service_call() {
        let subject = |body: &[u8]| service_call(body).map(|c| c.subject());
        assert_eq!(subject(br#"{"service":"window","method":"CloseWindow","args":["secret"]}"#).as_deref(), Some("window.CloseWindow"));
        assert_eq!(subject(br#"{"service":"window"}"#), None);
        assert_eq!(subject(b"not json"), None);
        let meta = subject(br#"{"service":"object","method":"UpdateObjectMeta","args":["block:b1",{"controller":"s3cret","cmd:cwd":"/x"}]}"#).unwrap();
        assert!(meta.starts_with("object.UpdateObjectMeta object=block:b1 keys=") && meta.contains("controller") && meta.contains("cmd:cwd"), "{meta}");
        assert!(!meta.contains("s3cret") && !meta.contains("/x"), "{meta}");
    }

    #[test]
    fn a_service_call_with_no_token_is_recorded_unless_it_is_a_read_or_the_windows_bookkeeping() {
        let quiet = |service: &str, method: &str| {
            let body = serde_json::json!({"service": service, "method": method, "args": []}).to_string();
            service_call(body.as_bytes()).unwrap().quiet() == Quiet::Always
        };
        assert!(quiet("object", "GetObject") && quiet("window", "SetWindowPosAndSize") && quiet("history", "List"));
        for (service, method) in [("client", "AgreeTos"), ("object", "QuitAgent"), ("object", "ShutdownAgent"), ("agent", "define"), ("history", "Delete"), ("workspace", "DeleteWorkspace"), ("object", "NewMethod")] {
            assert!(!quiet(service, method), "{service}.{method}");
        }
    }

    #[test]
    fn the_hosts_plumbing_is_quiet_only_while_it_succeeds() {
        let quiet = |service: &str, method: &str| {
            let body = serde_json::json!({"service": service, "method": method, "args": []}).to_string();
            service_call(body.as_bytes()).unwrap().quiet()
        };
        assert_eq!(quiet("credential", "Fill"), Quiet::UnlessFailed);
        assert_eq!(quiet("host_ipc", "Register"), Quiet::UnlessFailed);
        assert_eq!(quiet("object", "GetObject"), Quiet::Always);
        assert_eq!(quiet("object", "QuitAgent"), Quiet::No);
    }

    #[test]
    fn a_meta_write_is_quiet_only_when_every_key_is_one_the_window_keeps_for_itself() {
        let meta = |keys: serde_json::Value| {
            let body = serde_json::json!({"service": "object", "method": "UpdateObjectMeta", "args": ["block:b1", keys]}).to_string();
            service_call(body.as_bytes()).unwrap().quiet() == Quiet::Always
        };
        assert!(meta(serde_json::json!({"term:osc_title": "x", "term:human_turns": 3, "term:last_prompt": null})));
        assert!(!meta(serde_json::json!({"term:osc_title": "x", "controller": "term"})));
        assert!(!meta(serde_json::json!({"cmd:env": {"A": "b"}})));
        assert!(!meta(serde_json::json!({})));
        let no_meta = serde_json::json!({"service": "object", "method": "UpdateObjectMeta", "args": ["block:b1"]}).to_string();
        assert_eq!(service_call(no_meta.as_bytes()).unwrap().quiet(), Quiet::No);
    }

    #[test]
    fn a_service_reply_without_success_is_a_failure_and_its_text_is_not_kept() {
        let failed = service_failure(br#"{"error":"invalid type: string \"s3cret\""}"#).unwrap();
        assert!(!failed.contains("s3cret"), "{failed}");
        assert!(service_failure(br#"{"success":false}"#).is_some());
        assert_eq!(service_failure(br#"{"success":true,"data":1}"#), None);
        assert_eq!(service_failure(b"not json"), None);
    }

    #[test]
    fn reads_are_not_writes() {
        assert!(!is_write(&Method::GET) && !is_write(&Method::OPTIONS) && !is_write(&Method::HEAD));
        assert!(is_write(&Method::POST) && is_write(&Method::PUT) && is_write(&Method::PATCH) && is_write(&Method::DELETE));
    }
}
