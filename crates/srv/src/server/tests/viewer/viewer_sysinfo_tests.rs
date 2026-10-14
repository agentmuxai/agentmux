// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A paired device's Sysinfo routes (agentmux-mobile's
//! PLAN_CHANNEL_AS_ONE_LIST_2026_10_13 §3a): only the viewer token opens
//! them; the latest sample; the feed's history, resume and live samples.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use futures_util::StreamExt;
use tower::ServiceExt;

use super::*;
use crate::backend::mps_events::SysInfo;
use crate::backend::rpc_types::TimeSeriesData;
use crate::backend::viewer::pairing;
use crate::server::http_viewer::build_viewer_router;
use crate::server::tests::test_state;

fn pair_device(state: &AppState) -> String {
    let token = pairing::new_token();
    let device_id = uuid::Uuid::new_v4().to_string();
    state
        .mstore
        .viewer_device_insert(&device_id, &pairing::token_hash(&token), "Pixel", "", 1)
        .unwrap();
    token
}

fn get(uri: &str, auth: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::GET).uri(uri);
    if let Some(v) = auth {
        b = b.header("Authorization", v);
    }
    b.body(Body::empty()).unwrap()
}

fn publish(state: &AppState, ts: i64, cpu: f64) {
    let sample = TimeSeriesData {
        ts,
        values: HashMap::from([("cpu".to_string(), cpu), ("mem:used".to_string(), 8.0)]),
        uptime_secs: Some(60),
    };
    state.broker.publish_event::<SysInfo>(&sample, vec!["local".to_string()], 1024);
}

/// The `sample` events in `text`, as their ids.
fn ids(text: &str) -> Vec<i64> {
    text.lines().filter_map(|l| l.strip_prefix("id: ")).map(|v| v.parse().unwrap()).collect()
}

#[tokio::test]
async fn only_the_viewer_token_opens_the_sysinfo_routes() {
    let state = test_state();
    let token = pair_device(&state);
    let app = build_viewer_router(state.clone());
    for uri in ["/agentmux/viewer/sysinfo", "/agentmux/viewer/sysinfo/feed"] {
        let none = app.clone().oneshot(get(uri, None)).await.unwrap();
        assert_eq!(none.status(), StatusCode::UNAUTHORIZED, "{uri} with no token");
        for key in ["test-secret-key", "test-lan-key", "amxv_not-a-paired-token"] {
            let resp = app.clone().oneshot(get(uri, Some(&format!("Bearer {key}")))).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri} with {key}");
        }
    }
    let resp = app.clone().oneshot(get("/agentmux/viewer/sysinfo", Some(&format!("Bearer {token}")))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "no sample yet");

    // Not served on the loopback or LAN routers.
    let routers = crate::server::build_routers(state.clone());
    for app in [routers.full, routers.lan] {
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/agentmux/viewer/sysinfo")
                    .header("X-AuthKey", "test-secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn the_latest_sample_is_a_time_series_data() {
    let state = test_state();
    let token = pair_device(&state);
    publish(&state, 1_000, 10.0);
    publish(&state, 2_000, 42.5);
    let resp = build_viewer_router(state.clone())
        .oneshot(get("/agentmux/viewer/sysinfo", Some(&format!("Bearer {token}"))))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let sample: TimeSeriesData = serde_json::from_slice(&body).unwrap();
    assert_eq!(sample.ts, 2_000);
    assert_eq!(sample.values["cpu"], 42.5);
    assert_eq!(sample.uptime_secs, Some(60));
}

/// Reads `stream` until it has `n` sample events, or a second passes.
async fn read_samples<S>(stream: &mut S, n: usize) -> String
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>> + Unpin,
{
    let mut text = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        while ids(&text).len() < n {
            match stream.next().await {
                Some(Ok(chunk)) => text.push_str(&String::from_utf8_lossy(&chunk)),
                _ => break,
            }
        }
    })
    .await;
    text
}

#[tokio::test]
async fn the_feed_sends_history_after_since_then_new_samples() {
    let state = test_state();
    for ts in [1_000, 2_000, 3_000] {
        publish(&state, ts, 1.0);
    }
    let lease = state.viewer.open_feed("dev").unwrap();
    let cancel = lease.token.clone();
    let timing = SysinfoTiming { heartbeat: Duration::from_secs(60), poll: Duration::from_millis(10) };
    let mut stream = Box::pin(sysinfo_stream(state.broker.clone(), Some(1_000), lease, timing));

    let first = read_samples(&mut stream, 2).await;
    assert!(first.starts_with("retry: 3000\n\n"), "{first}");
    assert_eq!(ids(&first), vec![2_000, 3_000], "history after since only");
    assert!(first.contains("event: sample\nid: 2000\ndata: {"), "{first}");

    publish(&state, 4_000, 2.0);
    let next = read_samples(&mut stream, 1).await;
    assert_eq!(ids(&next), vec![4_000], "a new sample, once");

    // Revoked or gone: the stream ends.
    cancel.cancel();
    let rest = tokio::time::timeout(Duration::from_secs(1), stream.next()).await.unwrap();
    assert!(rest.is_none());
}

/// A clock that goes back must not hide new samples.
#[tokio::test]
async fn a_clock_that_goes_back_does_not_stop_the_feed() {
    let state = test_state();
    publish(&state, 5_000, 1.0);
    let lease = state.viewer.open_feed("dev").unwrap();
    let timing = SysinfoTiming { heartbeat: Duration::from_secs(60), poll: Duration::from_millis(10) };
    let mut stream = Box::pin(sysinfo_stream(state.broker.clone(), None, lease, timing));
    assert_eq!(ids(&read_samples(&mut stream, 1).await), vec![5_000]);
    // The clock is set back a minute: the next samples are "older".
    publish(&state, 1_000, 2.0);
    publish(&state, 2_000, 3.0);
    assert_eq!(ids(&read_samples(&mut stream, 2).await), vec![1_000, 2_000]);
}

/// A stream that wasn't read for longer than one poll's
/// read still gets every kept sample it missed.
#[tokio::test]
async fn a_stalled_feed_gets_every_kept_sample_it_missed() {
    let state = test_state();
    publish(&state, 1, 0.0);
    let lease = state.viewer.open_feed("dev").unwrap();
    let timing = SysinfoTiming { heartbeat: Duration::from_secs(60), poll: Duration::from_millis(10) };
    let mut stream = Box::pin(sysinfo_stream(state.broker.clone(), None, lease, timing));
    assert_eq!(ids(&read_samples(&mut stream, 1).await), vec![1]);
    // More than POLL_READ arrive before the stream is read again.
    for ts in 2..=101 {
        publish(&state, ts, 0.0);
    }
    let got = ids(&read_samples(&mut stream, 100).await);
    assert_eq!(got, (2..=101).collect::<Vec<_>>());
}

#[test]
fn a_resume_after_the_clock_went_back_gets_everything_kept() {
    let kept = |tss: &[i64]| tss.iter().map(|ts| (*ts, serde_json::json!({ "ts": ts }))).collect::<Vec<_>>();
    let tss = |v: Vec<(i64, serde_json::Value)>| v.into_iter().map(|(ts, _)| ts).collect::<Vec<_>>();
    assert_eq!(tss(history_after(kept(&[1, 2, 3]), Some(2))), vec![3]);
    assert_eq!(tss(history_after(kept(&[1, 2, 3]), Some(3))), Vec::<i64>::new(), "up to date");
    assert_eq!(tss(history_after(kept(&[1, 2, 3]), None)), vec![1, 2, 3]);
    assert_eq!(tss(history_after(kept(&[1, 2, 3]), Some(9_000))), vec![1, 2, 3], "every kept sample is older");
}

#[tokio::test]
async fn the_feed_without_since_sends_everything_kept_and_resumes_by_last_event_id() {
    let state = test_state();
    let token = pair_device(&state);
    for ts in [1_000, 2_000] {
        publish(&state, ts, 1.0);
    }
    let app = build_viewer_router(state.clone());
    let all = app.clone().oneshot(get("/agentmux/viewer/sysinfo/feed", Some(&format!("Bearer {token}")))).await.unwrap();
    assert_eq!(all.status(), StatusCode::OK);
    assert_eq!(all.headers()["content-type"], "text/event-stream");
    let mut body = all.into_body().into_data_stream();
    let mut text = String::new();
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        while ids(&text).len() < 2 {
            match body.next().await {
                Some(Ok(chunk)) => text.push_str(&String::from_utf8_lossy(&chunk)),
                _ => break,
            }
        }
    })
    .await;
    assert_eq!(ids(&text), vec![1_000, 2_000]);

    let resumed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/agentmux/viewer/sysinfo/feed?since=0")
                .header("Authorization", format!("Bearer {token}"))
                .header("Last-Event-ID", "1000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut body = resumed.into_body().into_data_stream();
    let mut text = String::new();
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        while ids(&text).is_empty() {
            match body.next().await {
                Some(Ok(chunk)) => text.push_str(&String::from_utf8_lossy(&chunk)),
                _ => break,
            }
        }
    })
    .await;
    assert_eq!(ids(&text), vec![2_000], "Last-Event-ID wins over since");
}
