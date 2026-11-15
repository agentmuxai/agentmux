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

/// Wait `grace` for a navigation to be asked for; when one was, wait up to `bound` for a load to end after it.
/// `true` when a requested navigation ended in time, `false` when none was asked for or it outlasted `bound`.
async fn settle_after(
    requested: &mut UnboundedReceiver<Value>,
    stopped: &mut UnboundedReceiver<Value>,
    grace: Duration,
    bound: Duration,
) -> bool {
    if tokio::time::timeout(grace, requested.recv()).await.ok().flatten().is_none() {
        return false;
    }
    // A load that ended before the request (the page was still settling) is not the navigation's end.
    while stopped.try_recv().is_ok() {}
    tokio::time::timeout(bound, stopped.recv()).await.ok().flatten().is_some()
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
    async fn a_navigation_that_never_ends_is_given_up_on_at_the_bound() {
        let (req_tx, mut req) = unbounded_channel::<Value>();
        let (_stop_tx, mut stop) = unbounded_channel::<Value>();
        req_tx.send(json!({})).unwrap();
        let started = std::time::Instant::now();
        assert!(!settle_after(&mut req, &mut stop, SHORT, SHORT).await);
        assert!(started.elapsed() < LONG);
    }
}
