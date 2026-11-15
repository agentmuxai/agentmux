// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! After a click that started a navigation (a form submit, a link): wait for the page to load before reading where
//! it is. A click's answer names the page, and read straight after the click it was still the old one, so an agent
//! (and the note that wakes a waiting one) was told the page had not moved (#47532).
//!
//! The page says it asked for a navigation (`Page.frameRequestedNavigation`) in the same moment as the click, so a
//! click that asked for none is not held up beyond a short grace, and one that did is waited for until the load
//! ends (`Page.frameStoppedLoading`), up to a bound.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;

use super::cdp::{subscribe, CdpSession, Subscription};

/// How long after a click to wait for the page to say it asked for a navigation.
const GRACE: Duration = Duration::from_millis(150);
/// The longest wait for a requested navigation to finish loading.
const BOUND: Duration = Duration::from_secs(5);

/// Listens for the navigation events of one browser; made before the click, used after it.
pub(super) struct NavWatch {
    requested: Subscription,
    stopped: Subscription,
}

impl NavWatch {
    /// `None` when the pane is gone or its page can't be asked: the answer is then read as before.
    pub(super) async fn start(cdp: &mut CdpSession) -> Option<Self> {
        let browser_id = cdp.browser_id()?;
        let requested = subscribe(browser_id, "Page.frameRequestedNavigation");
        let stopped = subscribe(browser_id, "Page.frameStoppedLoading");
        cdp.call("Page.enable", json!({})).await.ok()?;
        Some(Self { requested, stopped })
    }

    /// After the click: wait for the navigation it started, if it started one.
    pub(super) async fn settle(mut self) {
        settle_after(&mut self.requested.rx, &mut self.stopped.rx, GRACE, BOUND).await;
    }
}

/// The frame an event is about (`params.frameId`), when it names one.
fn frame_of(event: &Value) -> Option<&str> {
    event.pointer("/params/frameId").and_then(Value::as_str)
}

/// Whether a load that ended is the end of the navigation asked for: the same session (a frame from another site loads
/// on its own, and a load there says nothing of the page) and the same frame. An event that names no frame, or a
/// request that named none, is taken as the navigation's.
fn ends(stop: &Value, request: &Value) -> bool {
    if stop.get("sessionId") != request.get("sessionId") {
        return false;
    }
    match (frame_of(stop), frame_of(request)) {
        (Some(s), Some(r)) => s == r,
        _ => true,
    }
}

/// Wait `grace` for a navigation to be asked for; when one was, wait up to `bound` for a load of that frame to end
/// after it. A load of another frame (an embedded one finishing) used to end the wait, and the page was then read
/// while it was still the old one (#47641).
/// `true` when a requested navigation ended in time, `false` when none was asked for or it outlasted `bound`.
async fn settle_after(
    requested: &mut UnboundedReceiver<Value>,
    stopped: &mut UnboundedReceiver<Value>,
    grace: Duration,
    bound: Duration,
) -> bool {
    let Some(request) = tokio::time::timeout(grace, requested.recv()).await.ok().flatten() else {
        return false;
    };
    // A load that ended before the request (the page was still settling) is not the navigation's end.
    while stopped.try_recv().is_ok() {}
    let wait = async {
        while let Some(stop) = stopped.recv().await {
            if ends(&stop, &request) {
                return true;
            }
        }
        false
    };
    tokio::time::timeout(bound, wait).await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    const SHORT: Duration = Duration::from_millis(40);
    const LONG: Duration = Duration::from_millis(400);

    #[tokio::test]
    async fn a_click_that_asked_for_no_navigation_is_held_up_for_the_grace_only() {
        let (_req_tx, mut req) = unbounded_channel::<Value>();
        let (_stop_tx, mut stop) = unbounded_channel::<Value>();
        let started = std::time::Instant::now();
        assert!(!settle_after(&mut req, &mut stop, SHORT, LONG).await);
        assert!(started.elapsed() < LONG, "it must not wait for the bound when nothing was asked for");
    }

    #[tokio::test]
    async fn a_requested_navigation_is_waited_for_until_the_load_ends() {
        let (req_tx, mut req) = unbounded_channel::<Value>();
        let (stop_tx, mut stop) = unbounded_channel::<Value>();
        req_tx.send(json!({})).unwrap();
        let ends = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            stop_tx.send(json!({})).unwrap();
            stop_tx
        });
        let started = std::time::Instant::now();
        assert!(settle_after(&mut req, &mut stop, SHORT, Duration::from_secs(5)).await);
        assert!(started.elapsed() >= Duration::from_millis(90), "the load had not ended yet");
        drop(ends.await);
    }

    #[tokio::test]
    async fn a_load_that_ended_before_the_request_does_not_end_the_wait() {
        let (req_tx, mut req) = unbounded_channel::<Value>();
        let (stop_tx, mut stop) = unbounded_channel::<Value>();
        stop_tx.send(json!({ "frameId": "earlier" })).unwrap();
        req_tx.send(json!({})).unwrap();
        // Nothing ends after the request: the wait runs to its bound and says so.
        assert!(!settle_after(&mut req, &mut stop, SHORT, SHORT).await);
        drop(stop_tx);
    }

    #[tokio::test]
    async fn another_frame_finishing_does_not_end_the_wait_but_the_navigating_one_does() {
        let (req_tx, mut req) = unbounded_channel::<Value>();
        let (stop_tx, mut stop) = unbounded_channel::<Value>();
        req_tx.send(json!({ "params": { "frameId": "main" } })).unwrap();
        let ends = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop_tx.send(json!({ "params": { "frameId": "embedded" } })).unwrap();
            stop_tx.send(json!({ "sessionId": "other-site", "params": { "frameId": "main" } })).unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            stop_tx.send(json!({ "params": { "frameId": "main" } })).unwrap();
            stop_tx
        });
        let started = std::time::Instant::now();
        assert!(settle_after(&mut req, &mut stop, SHORT, Duration::from_secs(5)).await);
        assert!(started.elapsed() >= Duration::from_millis(140), "the embedded frame's load must not have ended it");
        drop(ends.await);
    }

    #[tokio::test]
    async fn a_navigation_in_an_attached_frame_ends_on_that_frames_own_session() {
        let (req_tx, mut req) = unbounded_channel::<Value>();
        let (stop_tx, mut stop) = unbounded_channel::<Value>();
        req_tx.send(json!({ "sessionId": "s1", "params": { "frameId": "child" } })).unwrap();
        let sender = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stop_tx.send(json!({ "sessionId": "s2", "params": { "frameId": "child" } })).unwrap();
            tokio::time::sleep(Duration::from_millis(60)).await;
            stop_tx.send(json!({ "sessionId": "s1", "params": { "frameId": "child" } })).unwrap();
            stop_tx
        });
        let started = std::time::Instant::now();
        assert!(settle_after(&mut req, &mut stop, SHORT, Duration::from_secs(5)).await);
        assert!(started.elapsed() >= Duration::from_millis(70), "the other session's load must not have ended it");
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(sender.await);
    }

    #[tokio::test]
    async fn only_another_frame_loading_runs_the_wait_to_its_bound() {
        let (req_tx, mut req) = unbounded_channel::<Value>();
        let (stop_tx, mut stop) = unbounded_channel::<Value>();
        req_tx.send(json!({ "params": { "frameId": "main" } })).unwrap();
        let sender = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            stop_tx.send(json!({ "params": { "frameId": "embedded" } })).unwrap();
            stop_tx
        });
        assert!(!settle_after(&mut req, &mut stop, SHORT, SHORT).await);
        drop(sender.await);
    }

    #[tokio::test]
    async fn a_navigation_that_never_ends_is_given_up_on_at_the_bound() {
        let (req_tx, mut req) = unbounded_channel::<Value>();
        let (_stop_tx, mut stop) = unbounded_channel::<Value>();
        req_tx.send(json!({})).unwrap();
        let started = std::time::Instant::now();
        assert!(!settle_after(&mut req, &mut stop, SHORT, SHORT).await);
        assert!(started.elapsed() < LONG);
    }
}
