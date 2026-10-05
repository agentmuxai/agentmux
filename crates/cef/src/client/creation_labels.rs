// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which pending creation a new browser belongs to.
//!
//! Every window or pane creation queues a `PendingWindowCreation` carrying its
//! label, and `on_after_created` registers the new browser under a label from
//! that queue. It used to pop the head, which is only right if browsers finish
//! creating in the order they were requested. They don't: on 2026-10-05 a
//! pane-pool window (windowed browser in our own popup) and a window-pool
//! window (CEF Views), requested in that order in the same millisecond,
//! finished in the other order and got each other's labels. The pane tear-off
//! that later promoted the pane-pool label showed the right frame but sent
//! `pool:pane-promote` to a window-pool page, leaving a blank white window
//! (docs/incident/INCIDENT_2026_10_05_PANE_TEAROFF_WHITE_WINDOW_LABEL_SWAP.md).
//!
//! So each creation now names its browser, and `on_after_created` takes that
//! label's entry wherever it is in the queue:
//! - a browser with a handler of its own (floating panes, the Windows pane
//!   pool, browser panes) carries the label in that handler
//!   (`AgentMuxHandler::new_for_creation`);
//! - a CEF Views window shares the top-level client, so its `BrowserView` is
//!   tagged with a view ID here right after `browser_view_create`, before
//!   `window_create_top_level` creates the browser, and `on_after_created`
//!   reads the tag back from the browser's view.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;

use cef::{browser_view_get_for_browser, Browser, BrowserView, ImplView};
use parking_lot::Mutex;

/// View IDs we hand out. Nothing else in the app sets view IDs; starting far
/// from 0 keeps them clear of any CEF default.
static NEXT_TAG: AtomicI32 = AtomicI32::new(0x4d58_0000);

fn tags() -> &'static Mutex<HashMap<i32, String>> {
    static TAGS: OnceLock<Mutex<HashMap<i32, String>>> = OnceLock::new();
    TAGS.get_or_init(Default::default)
}

/// Remember that the browser this view is about to create belongs to `label`.
/// Call after `browser_view_create`, before the view is put in a window.
pub(crate) fn tag_browser_view(view: &BrowserView, label: &str) {
    let tag = NEXT_TAG.fetch_add(1, Ordering::Relaxed);
    view.set_id(tag);
    tags().lock().insert(tag, label.to_string());
}

/// The label a Views browser was created for, if its view was tagged. Each
/// tag is consumed once.
pub(crate) fn take_view_label(browser: &Browser) -> Option<String> {
    let mut b = browser.clone();
    let tag = browser_view_get_for_browser(Some(&mut b))?.id();
    take_tag(tag)
}

fn take_tag(tag: i32) -> Option<String> {
    tags().lock().remove(&tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_resolves_once() {
        tags().lock().insert(7, "window-pool-a".into());
        assert_eq!(take_tag(7).as_deref(), Some("window-pool-a"));
        assert_eq!(take_tag(7), None);
    }

    #[test]
    fn an_untagged_view_resolves_to_nothing() {
        assert_eq!(take_tag(0), None);
    }
}
