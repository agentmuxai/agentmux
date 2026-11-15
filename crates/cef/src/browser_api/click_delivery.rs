// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Did a click reach the page? A click is mouse events sent through the browser's input route, and a page that is
//! not being shown (its window minimized or covered, or its tab in the background) takes none of them: the old
//! answer was "done" for a click that did nothing (#47534). Before the click the page is asked whether it is shown;
//! after it, whether a mouse event arrived (`scripts/click_watch.js`, `scripts/click_seen.js`).

use std::time::{Duration, Instant};

use serde_json::json;

use super::act::call_on;
use super::cdp::CdpSession;

/// Before a click: whether the page is being shown, and start counting the mouse events that reach its window.
const CLICK_WATCH: &str = include_str!("scripts/click_watch.js");
/// After it: how many arrived.
const CLICK_SEEN: &str = include_str!("scripts/click_seen.js");

/// How long a click may take to reach the page before it counts as not delivered, and how often to look.
const DELIVERY_WAIT: Duration = Duration::from_millis(1200);
const DELIVERY_POLL: Duration = Duration::from_millis(40);

/// The answer for a click on a page that is not being shown. Nothing is sent, so no click can be held by the
/// browser and land when the pane is shown again.
pub(super) const PAGE_HIDDEN: &str = "this page is not being shown (its window is minimized or covered, or its tab \
    is in the background), so it takes no clicks: nothing was clicked. Ask the person to bring the pane into view, \
    then click again";

/// The answer for a click the page never received.
pub(super) const NOT_DELIVERED: &str = "the click was sent but the page never received it, so nothing was clicked: \
    take a new snapshot and try again (check the page before acting as if it was done)";

/// Before a click on `object_id`: `Err` for a page that is not being shown, else whether the delivery check can run
/// after (a page that can't be asked is clicked as before, without it).
pub(super) async fn begin(cdp: &mut CdpSession, object_id: &str) -> Result<bool, String> {
    match call_on(cdp, object_id, CLICK_WATCH, vec![]).await {
        Ok(v) if v.get("hidden").and_then(|h| h.as_bool()) == Some(true) => Err(PAGE_HIDDEN.to_string()),
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// After a click: `Err` when the page never received it. A page the click replaced (it navigated) can't be asked,
/// and that counts as received: only a page that answered with no event, for as long as `DELIVERY_WAIT`, did not
/// get the click.
pub(super) async fn check(cdp: &mut CdpSession, object_id: &str, watching: bool) -> Result<(), String> {
    if !watching {
        return Ok(());
    }
    let started = Instant::now();
    loop {
        let last = started.elapsed() >= DELIVERY_WAIT;
        match call_on(cdp, object_id, CLICK_SEEN, vec![json!(last)]).await {
            Ok(v) => match v.get("seen").and_then(|s| s.as_u64()) {
                Some(0) if !last => {}
                Some(0) => return Err(NOT_DELIVERED.to_string()),
                _ => return Ok(()),
            },
            Err(_) => return Ok(()),
        }
        tokio::time::sleep(DELIVERY_POLL).await;
    }
}
