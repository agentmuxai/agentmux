// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-place discard of a browser pane's page, through the AgentMux CEF patch
//! `CefBrowserHost::Discard` / `IsDiscarded` / `LoadIfNecessary` (Chrome's
//! `WebContents::Discard`, the way Chrome's own tab discarding works): the
//! page's document goes, and its renderer process too if no other page uses
//! it, while the browser and its back/forward history stay. Loading it again
//! is a history navigation to the same entry, so the page gets back the state
//! it last reported: scroll position, form contents.
//!
//! A libcef without the patch allocates the stock `_cef_browser_host_t` (592
//! bytes, without the three appended function pointers), so every call
//! answers `None` (not supported) and the caller keeps closing and recreating
//! the pane instead. Same runtime-size guard as `autocomplete`.

use std::sync::Arc;
use std::time::Duration;

use cef::sys::_cef_browser_host_t;
use cef::*;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::state::AppState;

use super::BrowserPaneManager;

/// What to do with a pane's page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardOp {
    /// Discard it. `Some(false)`: not now (already discarded, a navigation
    /// is about to commit, or the WebContentsDiscard feature is off).
    Discard,
    /// Whether it is discarded.
    IsDiscarded,
    /// Load it again if it was discarded (answers `Some(true)`).
    Load,
}

impl DiscardOp {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "discard" => Some(Self::Discard),
            "is_discarded" => Some(Self::IsDiscarded),
            "load" => Some(Self::Load),
            _ => None,
        }
    }
}

const UI_WAIT: Duration = Duration::from_secs(5);

type Reply = Arc<Mutex<Option<oneshot::Sender<Option<bool>>>>>;

impl BrowserPaneManager {
    /// Run `op` on the pane's browser on the UI thread. `None`: this libcef
    /// has no Discard API, or the pane has no live browser.
    pub async fn discard_op(&self, state: &Arc<AppState>, block_id: &str, op: DiscardOp) -> Option<bool> {
        let (tx, rx) = oneshot::channel();
        let mut task = DiscardOpTask::new(state.clone(), block_id.to_string(), op, Arc::new(Mutex::new(Some(tx))));
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            return None;
        }
        tokio::time::timeout(UI_WAIT, rx).await.ok().and_then(Result::ok).flatten()
    }
}

wrap_task! {
    pub struct DiscardOpTask {
        state: Arc<AppState>,
        block_id: String,
        op: DiscardOp,
        reply: Reply,
    }

    impl Task {
        fn execute(&self) {
            let answer = self
                .state
                .browser_panes
                .live_browser(&self.state, &self.block_id)
                .and_then(|browser| browser.host())
                .and_then(|host| run(&host, self.op));
            if let Some(tx) = self.reply.lock().take() {
                let _ = tx.send(answer);
            }
        }
    }
}

/// UI thread. `None` when this libcef has no Discard API.
fn run(host: &BrowserHost, op: DiscardOp) -> Option<bool> {
    let raw: *mut _cef_browser_host_t = ImplBrowserHost::get_raw(host);
    unsafe {
        // The struct libcef allocated: a stock build's ends before our fields.
        if (*raw).base.size < std::mem::size_of::<_cef_browser_host_t>() {
            return None;
        }
        match op {
            DiscardOp::Discard => {
                let discard = (*raw).discard?;
                Some(discard(raw, std::ptr::null_mut()) != 0)
            }
            DiscardOp::IsDiscarded => {
                let is_discarded = (*raw).is_discarded?;
                Some(is_discarded(raw) != 0)
            }
            DiscardOp::Load => {
                let load_if_necessary = (*raw).load_if_necessary?;
                load_if_necessary(raw);
                Some(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_ops_the_page_sends() {
        assert_eq!(DiscardOp::parse("discard"), Some(DiscardOp::Discard));
        assert_eq!(DiscardOp::parse("is_discarded"), Some(DiscardOp::IsDiscarded));
        assert_eq!(DiscardOp::parse("load"), Some(DiscardOp::Load));
        assert_eq!(DiscardOp::parse("close"), None);
    }
}
