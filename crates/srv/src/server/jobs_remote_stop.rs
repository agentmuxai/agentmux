// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Stopping a Job's run in another AgentMux on this computer: the sender's
//! hook and the receiving route (`backend/jobs/remote.rs` says why and what
//! is written to the run).
//!
//! A run delivered to an agent in another channel is followed by that channel
//! (`RunLedger::follow_forwarded`). A timeout or a `cancel_other` overlap on
//! the firing side asks that channel's srv, over loopback with the key it
//! lists in the shared registry (the same trust as the inject forward), to
//! close the run and interrupt the agent's turn, the interrupt a local stop
//! sends. It is not the Stop tool: that closes the agent, and asks its user
//! first.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde_json::json;

use super::app_api::fleet::shared_channel_for;
use super::AppState;
use crate::backend::jobs::remote::RemoteStop;
use crate::backend::jobs::RunLedger;
use agentmux_common::AUTH_KEY_HEADER;

/// The route on the receiving srv.
pub(crate) const STOP_RUN_PATH: &str = "/agentmux/jobs/stop-run";

/// How long the other srv has to answer: it only closes a row and signals a pane.
const ASK_TIMEOUT: Duration = Duration::from_secs(10);

/// Deliver `stop` to the srv that follows its run, and say how it went.
async fn forward(state: &AppState, stop: &RemoteStop) -> Result<(), String> {
    let entry = shared_channel_for(state, &stop.block_id)
        .ok_or_else(|| "the AgentMux that follows this run is not listed on this computer".to_string())?;
    post_stop(&state.http_client, &entry.local_url, &entry.auth_key, stop).await
}

/// `POST` `stop` to the srv at `url`, which answers whether it took it.
async fn post_stop(http: &reqwest::Client, url: &str, auth_key: &str, stop: &RemoteStop) -> Result<(), String> {
    let mut req = http
        .post(format!("{url}{STOP_RUN_PATH}"))
        .timeout(ASK_TIMEOUT)
        .json(&json!({
            "run_id": stop.run_id,
            "block_id": stop.block_id,
            "from_channel": stop.from_channel,
        }));
    if !auth_key.is_empty() {
        req = req.header(AUTH_KEY_HEADER, auth_key);
    }
    let resp = req.send().await.map_err(|e| format!("could not reach the AgentMux that follows this run: {e}"))?;
    match resp.status() {
        s if s.is_success() => {}
        StatusCode::NOT_FOUND => return Err("the AgentMux that follows this run is too old to take a stop".to_string()),
        s => return Err(format!("the AgentMux that follows this run answered HTTP {s}")),
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("unreadable answer from the AgentMux that follows this run: {e}"))?;
    if body["success"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(body["error"].as_str().unwrap_or("the AgentMux that follows this run refused the stop").to_string())
    }
}

/// Let `ledger` ask another channel to stop the runs it follows.
pub(crate) fn install(state: &AppState, ledger: &Arc<RunLedger>) {
    let (state, weak) = (state.clone(), Arc::downgrade(ledger));
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    ledger.install_remote_stop(Arc::new(move |stop: RemoteStop| {
        let (state, weak) = (state.clone(), weak.clone());
        rt.spawn(async move {
            let outcome = forward(&state, &stop).await;
            let Some(ledger) = weak.upgrade() else { return };
            let _ = tokio::task::spawn_blocking(move || match outcome {
                Ok(()) => ledger.remote_stop_done(&stop.run_id),
                Err(e) => ledger.remote_stop_failed(&stop.run_id, &e),
            })
            .await;
        });
    }));
}

#[derive(serde::Deserialize)]
pub(crate) struct StopRunRequest {
    run_id: String,
    block_id: String,
    /// The AgentMux asking: it must be the one that fired the run, and must
    /// have asked in the run's row.
    from_channel: String,
}

/// `POST /agentmux/jobs/stop-run`: close the run this instance follows and
/// interrupt its pane's turn. Only a run this process follows in the named
/// pane, that the asking AgentMux fired and asked in its row to stop. The
/// route is on the full-key router only: not LAN-reachable, and not on the
/// container allowlist.
pub(crate) async fn handle_jobs_stop_run(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<StopRunRequest>,
) -> impl IntoResponse {
    let refuse = |status: StatusCode, error: &str| (status, Json(json!({ "success": false, "error": error }))).into_response();
    // The firing instance is a server; an agent's own request doesn't qualify.
    if caller.as_deref().and_then(super::caller::Caller::uid).is_some() {
        return refuse(StatusCode::FORBIDDEN, "FORBIDDEN: only another AgentMux asks this");
    }
    let Some(ledger) = state.cron_scheduler.runs().cloned() else {
        return refuse(StatusCode::OK, "the run ledger isn't ready");
    };
    let outcome = tokio::task::spawn_blocking(move || {
        ledger.stop_followed_here(&req.run_id, &req.block_id, &req.from_channel)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match outcome {
        Ok(closed) => (StatusCode::OK, Json(json!({ "success": true, "closed": closed }))).into_response(),
        Err(e) => refuse(StatusCode::OK, &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    fn stop() -> RemoteStop {
        RemoteStop {
            run_id: "r1".into(),
            block_id: "b1".into(),
            from_channel: "chan-a".into(),
            infra: "timeout".into(),
            reason: "timeout".into(),
            summary: "stopped after 5 minutes (timeout_minutes)".into(),
        }
    }

    /// A srv that answers the stop route with `status` and `body`, and keeps what it was sent.
    async fn peer(status: StatusCode, body: &'static str) -> (String, Arc<std::sync::Mutex<Vec<(Option<String>, serde_json::Value)>>>, tokio_util::sync::DropGuard) {
        let seen: Arc<std::sync::Mutex<Vec<(Option<String>, serde_json::Value)>>> = Arc::default();
        let sink = seen.clone();
        let app = axum::Router::new().route(
            STOP_RUN_PATH,
            axum::routing::post(move |headers: axum::http::HeaderMap, Json(req): Json<serde_json::Value>| {
                let sink = sink.clone();
                async move {
                    let key = headers.get(AUTH_KEY_HEADER).and_then(|v| v.to_str().ok()).map(str::to_string);
                    sink.lock().unwrap().push((key, req));
                    (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async move { child.cancelled().await }).await;
        });
        (format!("http://{addr}"), seen, token.drop_guard())
    }

    #[tokio::test]
    async fn the_ask_carries_the_run_the_pane_and_the_stop_with_the_peers_key() {
        let (url, seen, _guard) = peer(StatusCode::OK, r#"{"success":true,"closed":true}"#).await;
        assert_eq!(post_stop(&reqwest::Client::new(), &url, "k-b", &stop()).await, Ok(()));
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0.as_deref(), Some("k-b"));
        // Only who is asking and about what: the stop itself is in the shared ledger.
        assert_eq!(seen[0].1, json!({"run_id": "r1", "block_id": "b1", "from_channel": "chan-a"}));
    }

    /// The route is on the full-key router: a container's credential is not
    /// accepted for it, and it is not among the LAN-reachable routes.
    #[test]
    fn the_route_is_not_open_to_containers() {
        assert!(!crate::backend::container_credential::container_route_allowed(STOP_RUN_PATH));
    }

    #[tokio::test]
    async fn what_the_peer_says_is_the_outcome() {
        let http = reqwest::Client::new();
        let (url, _, _g1) = peer(StatusCode::OK, r#"{"success":false,"error":"this AgentMux does not follow that run"}"#).await;
        assert_eq!(post_stop(&http, &url, "", &stop()).await, Err("this AgentMux does not follow that run".to_string()));
        let (url, _, _g2) = peer(StatusCode::NOT_FOUND, "").await;
        assert!(post_stop(&http, &url, "", &stop()).await.unwrap_err().contains("too old"));
        let (url, _, _g3) = peer(StatusCode::INTERNAL_SERVER_ERROR, "").await;
        assert!(post_stop(&http, &url, "", &stop()).await.unwrap_err().contains("500"));
        let (url, _, _g4) = peer(StatusCode::OK, "not json").await;
        assert!(post_stop(&http, &url, "", &stop()).await.unwrap_err().contains("unreadable"));
        // Nothing listens there.
        assert!(post_stop(&http, "http://127.0.0.1:9", "", &stop()).await.unwrap_err().contains("could not reach"));
    }

    fn request(run_id: &str) -> Json<StopRunRequest> {
        Json(StopRunRequest {
            run_id: run_id.into(),
            block_id: "b1".into(),
            from_channel: "chan-a".into(),
        })
    }

    async fn answer(response: impl IntoResponse) -> (StatusCode, serde_json::Value) {
        let response = response.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn the_route_answers_for_a_run_it_does_not_follow() {
        let state = test_state();
        // No ledger yet.
        let (status, body) = answer(handle_jobs_stop_run(State(state.clone()), None, request("r1")).await).await;
        assert_eq!((status, body["success"].as_bool()), (StatusCode::OK, Some(false)));
        assert!(body["error"].as_str().unwrap().contains("isn't ready"));
        // A ledger that has no such run.
        let ledger = RunLedger::new(state.work_store.clone(), None, state.broker.clone());
        state.cron_scheduler.install_runs(ledger);
        let (status, body) = answer(handle_jobs_stop_run(State(state), None, request("r1")).await).await;
        assert_eq!((status, body["success"].as_bool(), body["error"].as_str()), (StatusCode::OK, Some(false), Some("no such run")));
    }
}
