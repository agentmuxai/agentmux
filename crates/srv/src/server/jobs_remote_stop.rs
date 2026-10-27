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

/// Where the AgentMux holding a pane listens, and its key: `(url, auth key)`.
type Resolve = Arc<dyn Fn(&str) -> Option<(String, String)> + Send + Sync>;

/// The shared registry's entry for the pane, on another instance of this computer.
fn registry_resolver(state: &AppState) -> Resolve {
    let state = state.clone();
    Arc::new(move |block_id: &str| shared_channel_for(&state, block_id).map(|e| (e.local_url, e.auth_key)))
}

/// Deliver `stop` to the srv that follows its run, and say how it went.
async fn forward(http: &reqwest::Client, resolve: &Resolve, stop: &RemoteStop) -> Result<(), String> {
    let (url, auth_key) = resolve(&stop.block_id)
        .ok_or_else(|| "the AgentMux that follows this run is not listed on this computer".to_string())?;
    post_stop(http, &url, &auth_key, stop).await
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
    install_with(state.http_client.clone(), registry_resolver(state), ledger);
}

/// [`install`] with the lookup of the other srv given: tests point it at one they started.
fn install_with(http: reqwest::Client, resolve: Resolve, ledger: &Arc<RunLedger>) {
    let weak = Arc::downgrade(ledger);
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    ledger.install_remote_stop(Arc::new(move |stop: RemoteStop| {
        let (http, resolve, weak) = (http.clone(), resolve.clone(), weak.clone());
        rt.spawn(async move {
            let outcome = forward(&http, &resolve, &stop).await;
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

    /// Two AgentMux instances sharing one ledger, the receiving one served over
    /// loopback HTTP: a timeout on the firing side closes the run and interrupts the
    /// pane on the other, through the hook, the request, the route and the ledger.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_timeout_on_the_firing_side_interrupts_the_pane_of_the_other_instance_over_http() {
        use crate::backend::cron::test_job;
        use crate::backend::jobs::{Begin, BY_SCHEDULER, TRIGGER_SCHEDULE};
        use crate::backend::storage::job_runs::{infra, reason, state as run_state};
        use crate::backend::storage::store::Store;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap());

        // The AgentMux that follows the run, listening on loopback.
        let state_b = test_state();
        let ledger_b = RunLedger::new_in(store.clone(), Some(state_b.mstore.clone()), state_b.broker.clone(), "chan-b");
        let interrupted: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let sink = interrupted.clone();
        ledger_b.install_interrupt(Arc::new(move |block: &str| sink.lock().unwrap().push(block.to_string())));
        state_b.cron_scheduler.install_runs(ledger_b.clone());
        let app = axum::Router::new().route(STOP_RUN_PATH, axum::routing::post(handle_jobs_stop_run)).with_state(state_b);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async move { child.cancelled().await }).await;
        });
        let _guard = token.drop_guard();

        // The AgentMux that fired it.
        let state_a = test_state();
        let ledger_a = RunLedger::new_in(store.clone(), Some(state_a.mstore.clone()), state_a.broker.clone(), "chan-a");
        let looked_up: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let seen = looked_up.clone();
        let resolve: Resolve = Arc::new(move |block: &str| {
            seen.lock().unwrap().push(block.to_string());
            Some((url.clone(), "k-b".to_string()))
        });
        install_with(reqwest::Client::new(), resolve, &ledger_a);

        // A run fired by A, delivered to a pane B holds, and followed there.
        let job = test_job("j-live", "check the build", "AgentY", "uid-y", "uid-creator");
        let Begin::Run(run_id) = ledger_a.begin_cron(&job, TRIGGER_SCHEDULE, Some(1), BY_SCHEDULER) else { panic!("no run") };
        assert!(ledger_b.follow_forwarded(&run_id, "b-remote", agentmux_common::time::now_ms()));
        let run = store.job_run_get(&run_id).unwrap().unwrap();
        assert_eq!(run.state, run_state::RUNNING);

        // Its timeout comes up on A.
        assert!(!ledger_a.stop_run(&run, infra::TIMEOUT, reason::TIMEOUT, "stopped after 1 minutes (timeout_minutes)"));
        let mut closed = None;
        for _ in 0..200 {
            let r = store.job_run_get(&run_id).unwrap().unwrap();
            if r.state == run_state::FINISHED {
                closed = Some(r);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let r = closed.expect("the run never closed");
        assert_eq!((r.infra.as_str(), r.reason.as_str()), (infra::TIMEOUT, reason::TIMEOUT));
        assert_eq!(r.summary, "stopped after 1 minutes (timeout_minutes)");
        assert_eq!(*interrupted.lock().unwrap(), ["b-remote"], "the pane on B was interrupted once");
        assert_eq!(*looked_up.lock().unwrap(), ["b-remote"]);
        assert!(r.links.get("stop_error").is_none(), "{:?}", r.links);

        // Another AgentMux (a third channel) naming the same run stops nothing more.
        let ledger_c = RunLedger::new_in(store, None, state_a.broker.clone(), "chan-c");
        assert!(ledger_c.stop_followed_here(&run_id, "b-remote", "chan-c").is_err());
        assert_eq!(interrupted.lock().unwrap().len(), 1);
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
