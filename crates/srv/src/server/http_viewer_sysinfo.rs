// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! This computer's CPU and memory for a paired device's Sysinfo, read-only
//! (agentmux-mobile's PLAN_CHANNEL_AS_ONE_LIST_2026_10_13 §3a):
//!
//! - `GET /agentmux/viewer/sysinfo`: the latest sample, a `TimeSeriesData`
//!   (`{ts, values, uptime_secs}`, the metric names the desktop's Sysinfo
//!   reads: `cpu`, `cpu:N`, `mem:*`, `net:*`, `disk:*`). 204 before the
//!   first sample.
//! - `GET /agentmux/viewer/sysinfo/feed?since=<ms>`: the same as SSE, one
//!   `sample` event per sample, its `id` its `ts`: first the kept history
//!   after `since` (or after `Last-Event-ID`; all of it with neither), then
//!   each new sample, with a heartbeat on a quiet stream.
//!
//! Both read the samples the sysinfo loop already keeps
//! (`Broker::read_event_history`), so a phone watching costs no extra
//! sampling. Built on REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13's
//! data layer as agreed with its author: when that layer's shared scheduler
//! and `since seq N` read exist, the feed holds a lease while open and
//! resumes by sequence, and `since=<ms>` keeps working.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::Query;
use futures_util::Stream;
use serde::Deserialize;

use super::http_viewer::{ViewerCaller, FEED_HEARTBEAT, HEARTBEAT, RETRY_PREAMBLE};
use super::*;
use crate::backend::mps::{Broker, EVENT_SYS_INFO};
use crate::backend::viewer::{FeedLease, FeedRefused};

/// How often the feed looks for new samples: the sysinfo loop's own cadence.
const SAMPLE_POLL: Duration = Duration::from_secs(1);
/// The most samples one poll reads back. More than a poll can miss even
/// when the loop runs at its fastest (0.2 s) through a long stall.
const POLL_READ: usize = 64;
/// The most samples kept, and so the most a feed sends first.
const HISTORY_READ: usize = 1024;

/// The samples kept for the whole computer (the sysinfo loop also files them
/// under its connection's scope; the unscoped ring holds them all).
const GLOBAL_SCOPE: &str = "";

#[derive(Deserialize, Default)]
pub(crate) struct SysinfoFeedQuery {
    since: Option<i64>,
}

/// One kept sample, as `(ts, the TimeSeriesData as sent)`.
fn samples(broker: &Broker, max: usize) -> Vec<(i64, serde_json::Value)> {
    broker
        .read_event_history(EVENT_SYS_INFO, GLOBAL_SCOPE, max)
        .into_iter()
        .filter_map(|e| {
            let data = e.data?;
            let ts = data.get("ts")?.as_i64()?;
            Some((ts, data))
        })
        .collect()
}

pub(super) async fn handle_viewer_sysinfo(State(state): State<AppState>) -> Response {
    match samples(&state.broker, 1).pop() {
        Some((_, data)) => Json(data).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

pub(super) async fn handle_viewer_sysinfo_feed(
    State(state): State<AppState>,
    Extension(caller): Extension<ViewerCaller>,
    Query(q): Query<SysinfoFeedQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let lease = match state.viewer.open_feed(&caller.device_id) {
        Ok(lease) => lease,
        Err(FeedRefused::PerDevice) => {
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "too many open feeds for this device" })))
                .into_response()
        }
        Err(FeedRefused::Total) => {
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "too many open feeds" }))).into_response()
        }
    };
    // A reconnect resumes after the last sample it got.
    let since = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
        .or(q.since);
    tracing::info!(device = %caller.device_id, resume = since.is_some(), "viewer sysinfo feed: opened");
    let stream = sysinfo_stream(state.broker.clone(), since, lease, SysinfoTiming::default());
    (
        [(header::CONTENT_TYPE, "text/event-stream"), (header::CACHE_CONTROL, "no-cache")],
        Body::from_stream(stream),
    )
        .into_response()
}

/// A Sysinfo feed's clocks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SysinfoTiming {
    pub heartbeat: Duration,
    pub poll: Duration,
}

impl Default for SysinfoTiming {
    fn default() -> Self {
        Self { heartbeat: FEED_HEARTBEAT, poll: SAMPLE_POLL }
    }
}

fn sample_event(ts: i64, data: &serde_json::Value) -> Bytes {
    Bytes::from(format!("event: sample\nid: {ts}\ndata: {data}\n\n"))
}

/// The kept samples a resume asks for: those after `since`, by their time.
/// All of them with no `since`, or when every kept sample is older than it
/// (the computer's clock went back since the device's last one).
fn history_after(kept: Vec<(i64, serde_json::Value)>, since: Option<i64>) -> Vec<(i64, serde_json::Value)> {
    let Some(since) = since else { return kept };
    if kept.iter().all(|(ts, _)| *ts < since) {
        return kept;
    }
    kept.into_iter().filter(|(ts, _)| *ts > since).collect()
}

/// The samples kept after `cursor` (the last one sent), by their place in
/// the ring, not their time: a clock that goes back doesn't hide new ones.
/// The newest `POLL_READ` usually hold it; when they don't (a stalled
/// stream), the whole ring is read, so nothing kept is skipped. A cursor
/// no longer kept means everything kept is newer.
fn samples_after(broker: &Broker, cursor: Option<&serde_json::Value>) -> Vec<(i64, serde_json::Value)> {
    let recent = samples(broker, POLL_READ);
    let Some(cursor) = cursor else { return recent };
    if let Some(at) = recent.iter().rposition(|(_, d)| d == cursor) {
        return recent[at + 1..].to_vec();
    }
    let kept = samples(broker, HISTORY_READ);
    match kept.iter().rposition(|(_, d)| d == cursor) {
        Some(at) => kept[at + 1..].to_vec(),
        None => kept,
    }
}

/// The kept samples after `since` (all of them with none), then each new
/// one; a heartbeat when nothing new came for `timing.heartbeat`. Ends when
/// the device is revoked or goes away.
pub(crate) fn sysinfo_stream(
    broker: Arc<Broker>,
    since: Option<i64>,
    lease: FeedLease,
    timing: SysinfoTiming,
) -> impl Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    async_stream::stream! {
        let lease = lease;
        yield Ok(Bytes::from_static(RETRY_PREAMBLE));
        let kept = samples(&broker, HISTORY_READ);
        // The newest kept sample is where the live part starts, whether or
        // not the resume sent it.
        let mut cursor: Option<serde_json::Value> = kept.last().map(|(_, d)| d.clone());
        for (ts, data) in history_after(kept, since) {
            yield Ok(sample_event(ts, &data));
        }
        let mut poll = tokio::time::interval_at(tokio::time::Instant::now() + timing.poll, timing.poll);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut quiet_since = tokio::time::Instant::now();
        loop {
            tokio::select! {
                biased;
                _ = lease.token.cancelled() => break,
                _ = poll.tick() => {
                    let mut sent = false;
                    for (ts, data) in samples_after(&broker, cursor.as_ref()) {
                        sent = true;
                        yield Ok(sample_event(ts, &data));
                        cursor = Some(data);
                    }
                    if sent {
                        quiet_since = tokio::time::Instant::now();
                    } else if quiet_since.elapsed() >= timing.heartbeat {
                        quiet_since = tokio::time::Instant::now();
                        yield Ok(Bytes::from_static(HEARTBEAT));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/viewer/viewer_sysinfo_tests.rs"]
mod tests;
