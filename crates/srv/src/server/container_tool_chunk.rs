// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/tool-chunk`: a container agent's `agentmux-bashwrap` streaming a Bash
//! call's output into its own pane.
//!
//! `/agentmux/wps/publish` is a generic relay: it republishes an undeclared event to any scope,
//! so a container token is refused there (`container::credential`). This route takes one event,
//! `tool_chunk`, for one scope, the container's own `block:<id>`, and only as the typed
//! [`ToolChunk`]. A request that is not a container's, names another event or scope, or is not a
//! `ToolChunk` never reaches the broker.
//!
//! A `pid` or `call` op carries a process id from the container's own pid namespace. The pane
//! files a `pid` against the background task, and `tool_calls` labels a host process with a
//! `call`'s description, so on this srv's host that pid would name some other process. Neither
//! goes where a host pid goes: not to the broker, not to the registry's host `pid` column, not to
//! `tool_calls`. They are filed apart, with the container's name
//! (`storage::background_container_calls`): the liveness sweep checks the pid inside that
//! container (`background_task_container_sweep`), and the description is the registry row's
//! label, so a backgrounded call's dock row knows whether it is alive and is not just "Bash".

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::json;

use agentmux_common::api_types::WpsPublishRequest;
use agentmux_common::client_events::ToolChunk;

use crate::backend::container::credential::ContainerGrant;
use crate::server::AppState;

/// The largest request body: a line of output, with its envelope.
pub(crate) const MAX_BODY_BYTES: usize = 64 * 1024;

/// How many chunks the broker keeps per block for a late subscriber, as `agentmux-bashwrap` asks
/// on the host. srv decides it here; the container's value is not read.
const PERSIST: usize = 1024;

fn refuse(status: StatusCode, why: impl Into<String>) -> Response {
    (status, Json(json!({ "error": why.into() }))).into_response()
}

/// The chunk to publish for `block_id`'s `req`, or why the request is refused.
fn check(block_id: &str, req: &WpsPublishRequest) -> Result<ToolChunk, Response> {
    if req.event != "tool_chunk" {
        return Err(refuse(StatusCode::FORBIDDEN, "this route takes tool_chunk only"));
    }
    if req.scopes != [format!("block:{block_id}")] {
        return Err(refuse(StatusCode::FORBIDDEN, "a container agent's output goes to its own pane only"));
    }
    serde_json::from_value::<ToolChunk>(req.data.clone()).map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("tool_chunk: {e}")))
}

pub(crate) async fn handle_container_tool_chunk(
    State(state): State<AppState>,
    grant: Option<Extension<ContainerGrant>>,
    body: Result<Json<WpsPublishRequest>, JsonRejection>,
) -> Response {
    let Some(Extension(grant)) = grant else {
        return refuse(StatusCode::FORBIDDEN, "this route is for container agents");
    };
    let req = match body {
        Ok(Json(req)) => req,
        Err(rejection) => return refuse(rejection.status(), rejection.body_text()),
    };
    let chunk = match check(&grant.block_id, &req) {
        Ok(chunk) => chunk,
        Err(refusal) => return refusal,
    };
    if matches!(chunk, ToolChunk::Chunk { .. } | ToolChunk::Terminal { .. }) {
        state.broker.publish_event::<crate::backend::mps_events::ToolChunk>(&chunk, req.scopes, PERSIST);
    } else {
        file_report(&state, &grant.block_id, &chunk).await;
    }
    Json(json!({ "ok": true })).into_response()
}

/// A call's id as the container may name it (`toolu_…`): short, and plain characters only.
fn is_call_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// File a `pid` or `call` report of the container agent on `block_id` (see the module doc). Anything else, a
/// pane that is not a container's, an id that is not a call's, or a failed write, is dropped: the report is
/// only ever help for the dock, and the CLI's own task feed still describes the row.
async fn file_report(state: &AppState, block_id: &str, chunk: &ToolChunk) {
    let (tool_id, pid, description) = match chunk {
        ToolChunk::Pid { tool_id, pid, .. } => (tool_id.clone(), *pid, None),
        ToolChunk::Call { tool_id, pid, description, .. } => (tool_id.clone(), *pid, crate::backend::storage::background_container_calls::clean_description(description)),
        _ => return,
    };
    if !is_call_id(&tool_id) {
        return;
    }
    let (store, broker, block_id) = (state.mstore.clone(), state.broker.clone(), block_id.to_string());
    let _ = tokio::task::spawn_blocking(move || {
        let Some(container) = store.get::<crate::backend::obj::Block>(&block_id).ok().flatten().and_then(|b| crate::backend::container::container_of_meta(&b.meta)) else {
            return;
        };
        let now_ms = agentmux_common::time::now_ms();
        // Zero is no process.
        let pid = (pid > 0).then_some(i64::from(pid));
        if let Err(e) = store.background_container_call_file(&block_id, &tool_id, &container, pid, description.as_deref(), now_ms) {
            tracing::warn!(target: "background_tasks", block_id = %block_id, node_id = %tool_id, error = %e, "could not file a container agent's call report");
            return;
        }
        // The row may exist already, with the renderer's generic label.
        if let Some(label) = description {
            if store.background_task_describe_if_generic(&block_id, &tool_id, &label).unwrap_or(false) {
                crate::backend::background_task_feed::publish_background_task_updated(&broker, &block_id);
            }
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn chunk(content: &str) -> serde_json::Value {
        json!({ "op": "chunk", "tool_id": "toolu_1", "kind": "stdout", "content": content, "timestamp": 1 })
    }

    fn body(event: &str, scopes: &[&str], data: serde_json::Value) -> serde_json::Value {
        json!({ "event": event, "scopes": scopes, "persist": 999_999, "data": data })
    }

    async fn post(token: &str, body: String) -> (StatusCode, serde_json::Value) {
        let state = crate::server::tests::test_state();
        let app = crate::server::build_router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/agent/tool-chunk")
            .header("X-AuthKey", token)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn a_container_streams_into_its_own_pane_only() {
        let mine = crate::backend::container::credential::token_for_block("tc-block-a", None);
        let own = ["block:tc-block-a"];
        let (status, _) = post(&mine, body("tool_chunk", &own, chunk("hello")).to_string()).await;
        assert_eq!(status, StatusCode::OK);

        // Another block's pane, no scope, or more than its own.
        for scopes in [&["block:tc-block-b"][..], &[], &["block:tc-block-a", "block:tc-block-b"], &["tab:tc-block-a"]] {
            let (status, _) = post(&mine, body("tool_chunk", scopes, chunk("x")).to_string()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{scopes:?}");
        }
        // Any other event, whatever is in it: the generic relay's other uses, and the compaction start, which has its own route.
        for event in ["compaction_started", "agent_event:tc-block-a", "controllerstatus", "custom:thing"] {
            let (status, _) = post(&mine, body(event, &own, json!({ "anything": true })).to_string()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{event}");
        }
        // Not a ToolChunk.
        let (status, _) = post(&mine, body("tool_chunk", &own, json!({ "op": "chunk", "tool_id": "t" })).to_string()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_request_over_the_cap_is_413() {
        let mine = crate::backend::container::credential::token_for_block("tc-block-big", None);
        let huge = "x".repeat(MAX_BODY_BYTES);
        let (status, _) = post(&mine, body("tool_chunk", &["block:tc-block-big"], chunk(&huge)).to_string()).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        let fits = "x".repeat(MAX_BODY_BYTES / 2);
        let (status, _) = post(&mine, body("tool_chunk", &["block:tc-block-big"], chunk(&fits)).to_string()).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_route_is_a_container_agents_and_the_generic_relay_stays_refused() {
        // The instance key is not a container: this route is theirs alone.
        let (status, _) = post("test-secret-key", body("tool_chunk", &["block:b"], chunk("x")).to_string()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!crate::backend::container::credential::container_route_allowed("/agentmux/wps/publish"));
        assert!(crate::backend::container::credential::container_route_allowed("/api/v1/agent/tool-chunk"));
    }

    #[test]
    fn pid_and_call_reports_are_valid_but_never_published() {
        let pid = json!({ "op": "pid", "tool_id": "t", "pid": 7, "timestamp": 1 });
        let call = json!({ "op": "call", "tool_id": "t", "pid": 7, "description": "Run it", "timestamp": 1 });
        for data in [pid, call] {
            let req: WpsPublishRequest = serde_json::from_value(body("tool_chunk", &["block:b"], data)).unwrap();
            let chunk = check("b", &req).unwrap_or_else(|_| panic!("valid"));
            assert!(!matches!(chunk, ToolChunk::Chunk { .. } | ToolChunk::Terminal { .. }));
        }
    }

    async fn send(app: &axum::Router, token: &str, block: &str, data: serde_json::Value) {
        let scope = format!("block:{block}");
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/agent/tool-chunk")
            .header("X-AuthKey", token)
            .header("Content-Type", "application/json")
            .body(Body::from(body("tool_chunk", &[scope.as_str()], data).to_string()))
            .unwrap();
        assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
    }

    fn pane(state: &crate::server::AppState, block_id: &str, meta: serde_json::Value) {
        let mut block = crate::backend::obj::Block { oid: block_id.to_string(), ..Default::default() };
        for (k, v) in meta.as_object().unwrap() {
            block.meta.insert(k.clone(), v.clone());
        }
        state.mstore.insert(&mut block).unwrap();
    }

    /// A container's pid and description are filed with the container's name, apart from the host's `pid`
    /// column and the host's process labels, and the description labels the row.
    #[tokio::test]
    async fn a_container_pid_and_description_are_filed_apart_and_the_description_labels_the_row() {
        let state = crate::server::tests::test_state();
        pane(&state, "tc-file-a", json!({ "agentMode": "container", "agentId": "ag-file" }));
        let token = crate::backend::container::credential::token_for_block("tc-file-a", None);
        let app = crate::server::build_router(state.clone());
        let host_pid = 4_242_417u32;
        // The row exists first (the renderer made it), then the report; and a row the CLI made after the report.
        state.mstore.background_task_observe("toolu_a", "tc-file-a", "Bash", 100, 100).unwrap();
        send(&app, &token, "tc-file-a", json!({ "op": "pid", "tool_id": "toolu_a", "pid": host_pid, "timestamp": 1 })).await;
        send(&app, &token, "tc-file-a", json!({ "op": "call", "tool_id": "toolu_a", "pid": host_pid, "description": "Run the build\n", "timestamp": 2 })).await;
        send(&app, &token, "tc-file-a", json!({ "op": "call", "tool_id": "toolu_b", "pid": 9, "description": "Watch the tests", "timestamp": 3 })).await;
        state.mstore.background_task_observe("toolu_b", "tc-file-a", "Bash", 100, 100).unwrap();

        let row = state.mstore.background_task_get("toolu_a").unwrap().unwrap();
        assert_eq!((row.pid, row.label.as_str()), (None, "Run the build"), "no host pid; the call's description is the label");
        assert_eq!(state.mstore.background_task_get("toolu_b").unwrap().unwrap().label, "Watch the tests", "filed before the row, found by it");
        let filed = state.mstore.background_container_running().unwrap();
        let mine = filed.iter().find(|t| t.id == "toolu_a").unwrap();
        assert_eq!((mine.container.as_str(), mine.pid), ("agentmux-ag-file", i64::from(host_pid)));
        assert!(crate::backend::tool_calls::lookup(host_pid, None).is_none(), "a container pid never labels a host process");
    }

    /// Only a container pane files a report, only under a call's id, and only for its own pane.
    #[tokio::test]
    async fn a_report_from_a_pane_that_is_not_a_container_or_with_an_odd_id_is_dropped() {
        let state = crate::server::tests::test_state();
        pane(&state, "tc-file-host", json!({ "agentId": "plain" }));
        pane(&state, "tc-file-b", json!({ "agentMode": "container", "agentId": "ag-b" }));
        pane(&state, "tc-file-c", json!({ "agentMode": "container", "agentId": "ag-c" }));
        let app = crate::server::build_router(state.clone());
        state.mstore.background_task_observe("toolu_x", "tc-file-c", "Bash", 100, 100).unwrap();
        let host = crate::backend::container::credential::token_for_block("tc-file-host", None);
        let b = crate::backend::container::credential::token_for_block("tc-file-b", None);
        send(&app, &host, "tc-file-host", json!({ "op": "pid", "tool_id": "toolu_h", "pid": 5, "timestamp": 1 })).await;
        send(&app, &b, "tc-file-b", json!({ "op": "pid", "tool_id": "../odd id", "pid": 5, "timestamp": 1 })).await;
        // Pane b reports a call that pane c's row has: it files under b, not c.
        send(&app, &b, "tc-file-b", json!({ "op": "call", "tool_id": "toolu_x", "pid": 6, "description": "Not yours", "timestamp": 1 })).await;
        assert!(state.mstore.background_container_running().unwrap().is_empty());
        assert_eq!(state.mstore.background_task_get("toolu_x").unwrap().unwrap().label, "Bash", "another pane's row is not relabelled");
        assert_eq!(state.mstore.background_container_call_description("tc-file-host", "toolu_h").unwrap(), None);
    }

    #[tokio::test]
    async fn a_chunk_reaches_the_panes_subscribers_and_a_pid_does_not() {
        let state = crate::server::tests::test_state();
        let token = crate::backend::container::credential::token_for_block("tc-block-live", None);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        state.broker.add_observer(std::sync::Arc::new(move |e: &crate::backend::mps::MuxEvent| {
            if e.scopes == ["block:tc-block-live"] {
                sink.lock().unwrap().push(e.clone());
            }
        }));
        let app = crate::server::build_router(state);
        for data in [json!({ "op": "pid", "tool_id": "t", "pid": 7, "timestamp": 1 }), chunk("streamed\n")] {
            let req = Request::builder()
                .method("POST")
                .uri("/api/v1/agent/tool-chunk")
                .header("X-AuthKey", &token)
                .header("Content-Type", "application/json")
                .body(Body::from(body("tool_chunk", &["block:tc-block-live"], data).to_string()))
                .unwrap();
            assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "only the chunk was published, not the pid");
        assert_eq!(seen[0].event, "tool_chunk");
        assert_eq!(seen[0].data.as_ref().unwrap()["content"], "streamed\n");
        assert_eq!(seen[0].persist, PERSIST, "srv sets how much is kept, not the container");
    }
}
