// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A page in a browser pane that closes itself (`window.close()`) closes that
//! pane's page, never the window (SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md
//! §6).
//!
//! CEF answers a page's `window.close()` with `do_close` on its browser, and
//! for a windowed browser an allowed close sends `WM_CLOSE` to the browser's
//! top-level window: for a pane, the AgentMux window around it. An OAuth
//! callback page does exactly this (it expects to be a popup), and on
//! 2026-10-16 one closed a whole AgentMux instance. AgentMux's own pane closes
//! never reach `do_close` (`BrowserPaneManager::close` destroys the pane's
//! wrapper window instead), so a `do_close` on a `Live` pane's browser is the
//! page's. `intercept` takes it: `do_close` returns true (no `WM_CLOSE`), and
//! the pane is replaced in place by a short page saying the site closed
//! itself. The browser itself can't be kept: CEF has already committed it to
//! close.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use cef::*;

use crate::state::{AppState, BrowserPaneLifecycle, PendingBrowserPaneCreate};

/// The last rect each pane was created or laid out at, in the units
/// `BrowserPaneManager::create` takes: where the replacement goes.
static LAST_RECTS: LazyLock<Mutex<HashMap<String, (i32, i32, i32, i32)>>> = LazyLock::new(Default::default);

pub(crate) fn note_rect(block_id: &str, rect: &Rect) {
    let r = (rect.x, rect.y, rect.width, rect.height);
    LAST_RECTS.lock().unwrap_or_else(|e| e.into_inner()).insert(block_id.to_string(), r);
}

pub(crate) fn forget(block_id: &str) {
    LAST_RECTS.lock().unwrap_or_else(|e| e.into_inner()).remove(block_id);
}

fn last_rect(block_id: &str) -> Option<(i32, i32, i32, i32)> {
    LAST_RECTS.lock().unwrap_or_else(|e| e.into_inner()).get(block_id).copied()
}

/// For `do_close`: true when `browser` is a `Live` pane's (its page closed
/// itself), and the pane's replacement is under way; the caller then returns
/// true, so CEF doesn't close the window. False for anything else, which
/// keeps its close.
pub(crate) fn intercept(state: &Arc<AppState>, browser: Option<&Browser>) -> bool {
    let Some(browser) = browser else {
        return false;
    };
    let Some((block_id, window_label)) = live_pane_of_browser(state, browser.identifier()) else {
        return false;
    };
    let closed_url = browser
        .main_frame()
        .map(|f| CefString::from(&f.url()).to_string())
        .unwrap_or_default();
    tracing::warn!(
        target: "browser-pane",
        block_id = %block_id,
        url = %closed_url,
        "a page closed its own pane (window.close): replacing the pane, keeping the window"
    );
    let mut task = ReplaceSelfClosedPaneTask::new(state.clone(), block_id, window_label, closed_url);
    post_task(ThreadId::UI, Some(&mut task));
    true
}

/// The block id and window of the `Live` pane whose browser is `browser_id`.
/// What the browser is, from the host's registries: not the handler's
/// `is_browser_pane`, which a cloned client can carry into a top-level window
/// (#3028).
fn live_pane_of_browser(state: &Arc<AppState>, browser_id: i32) -> Option<(String, String)> {
    let host = state.host_state.lock();
    let label = host
        .browsers
        .iter()
        .find(|(_, h)| h.browser.identifier() == browser_id)
        .map(|(label, _)| label.clone())?;
    host.browser_panes
        .values()
        .find(|e| e.label == label && e.lifecycle == BrowserPaneLifecycle::Live)
        .map(|e| (e.block_id.clone(), e.window_label.clone()))
}

wrap_task! {
    pub struct ReplaceSelfClosedPaneTask {
        state: Arc<AppState>,
        block_id: String,
        window_label: String,
        closed_url: String,
    }

    impl Task {
        fn execute(&self) {
            // Stash the replacement first: the close below replays it once
            // the pane's entry is gone, in the same place.
            let (x, y, width, height) = last_rect(&self.block_id).unwrap_or_default();
            self.state.host_dispatch(crate::reducer::HostCommand::StashBrowserPaneCreate {
                block_id: self.block_id.clone(),
                pending: PendingBrowserPaneCreate {
                    url: closed_page_url(&self.closed_url),
                    x,
                    y,
                    width,
                    height,
                    window_label: self.window_label.clone(),
                },
            });
            self.state.browser_panes.close(&self.block_id, &self.state);
        }
    }
}

/// The page that replaces a pane whose page closed itself, as a `data:` URI
/// marked (as a load-error page is) with the site's address, which the pane
/// shows, saves and reopens: the site, not the page that closed (an OAuth
/// callback's one-time code shouldn't be submitted again).
pub(crate) fn closed_page_url(closed_url: &str) -> String {
    let site = site_of(closed_url);
    let html = closed_page_html(closed_url, &site);
    let b64 = CefString::from(&cef::base64_encode(Some(html.as_bytes()))).to_string();
    let marker = if site.is_empty() { String::new() } else { crate::client::navigation::failed_url_fragment(&site) };
    format!("data:text/html;base64,{b64}{marker}")
}

/// `scheme://host/` of an http(s) URL, else empty.
fn site_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::new();
    };
    if !matches!(scheme, "http" | "https") {
        return String::new();
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return String::new();
    }
    format!("{scheme}://{host}/")
}

fn closed_page_html(closed_url: &str, site: &str) -> String {
    use crate::client::helpers::html_escape;
    let url = html_escape(closed_url);
    let open = if site.is_empty() {
        String::new()
    } else {
        format!(r#"<p><a href="{0}">Open {0}</a></p>"#, html_escape(site))
    };
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>The page closed itself</title>
<style>
body {{ font: 14px system-ui, sans-serif; color: #c9c9c9; background: #1e1e1e; margin: 0; display: flex; align-items: center; justify-content: center; height: 100vh; }}
div {{ max-width: 34em; padding: 2em; }}
h1 {{ font-size: 17px; font-weight: 600; color: #e6e6e6; }}
code {{ word-break: break-all; color: #9aa4b2; }}
a {{ color: #6ea8fe; }}
</style></head>
<body><div>
<h1>The page closed itself</h1>
<p>This page asked to close its window: <code>{url}</code></p>
<p>Sign-in pages do this when they finish, because they expect to be a popup. AgentMux closed only this page; the window and the rest of your work are as they were.</p>
{open}
</div></body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_replacement_names_the_page_and_reopens_the_site_not_the_page() {
        let closed = "https://claude.ai/connect/github/callback?code=d3a9&state=x";
        assert_eq!(site_of(closed), "https://claude.ai/");
        let url = closed_page_url(closed);
        assert!(url.starts_with("data:text/html;base64,"), "{url}");
        assert_eq!(
            crate::client::navigation::error_page_failed_url(&url).as_deref(),
            Some("https://claude.ai/"),
            "the pane shows and saves the site"
        );
        let html = closed_page_html(closed, &site_of(closed));
        assert!(html.contains("The page closed itself"));
        assert!(html.contains("code=d3a9&amp;state=x"), "the address, escaped");
        assert!(html.contains(r#"<a href="https://claude.ai/">"#));
    }

    #[test]
    fn a_page_with_no_site_gets_no_link() {
        assert_eq!(site_of("about:blank"), "");
        assert_eq!(site_of("data:text/html,x"), "");
        assert_eq!(site_of("file:///c:/x.html"), "");
        assert!(!closed_page_html("about:blank", "").contains("<a href"));
        assert!(crate::client::navigation::error_page_failed_url(&closed_page_url("about:blank")).is_none());
    }

    #[test]
    fn the_last_rect_is_where_the_replacement_goes() {
        note_rect("b-self-close", &Rect { x: 1, y: 2, width: 30, height: 40 });
        assert_eq!(last_rect("b-self-close"), Some((1, 2, 30, 40)));
        forget("b-self-close");
        assert!(last_rect("b-self-close").is_none());
    }
}
