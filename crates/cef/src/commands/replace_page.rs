// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Navigate a recycled window's page by REPLACING its history entry.
//!
//! A recycled window (demoted into the pool, or parked and blanked) is
//! navigated away from the page it hosted. A plain `load_url` adds a history
//! entry, which makes the old page eligible for Chromium's back/forward cache,
//! and the cache keeps it (up to six per window, the whole workspace app with
//! its DOM and heap): in the lab, 24 demotes left every recycled pool window
//! at 16 documents and ~10,800 DOM nodes, against 1 document and 143 nodes
//! with the cache off (docs/specs/SPEC_WINDOW_CHURN_RENDERER_LEAK_2026_10_13.md
//! §3.2, §4). AgentMux never navigates back in these windows. A replacing
//! navigation leaves nothing to restore, so nothing is cached, while browser
//! panes keep the cache for their own Back/Forward.

/// The script that navigates a page to `url` by replacing its history entry.
pub(crate) fn replace_page_script(url: &str) -> String {
    // A JSON string is a valid JS string literal.
    format!(
        "location.replace({})",
        serde_json::to_string(url).unwrap_or_else(|_| "\"about:blank\"".to_string())
    )
}

/// Whether the page itself can navigate to `url`. Chromium blocks a
/// page-initiated top-frame navigation to a `data:` URL, and the
/// install-broken page `pool_frontend_url` falls back to is one; the
/// frontend itself is always served over http.
pub(crate) fn page_can_navigate_to(url: &str) -> bool {
    let url = url.trim_start().to_ascii_lowercase();
    url.starts_with("http://") || url.starts_with("https://") || url == "about:blank"
}

/// Navigate `frame` to `url`, replacing its history entry (see
/// `replace_page_script`). Falls back to `load_url` when the page can't do
/// it: a target it may not navigate to, or a frame whose renderer is gone,
/// which runs no script while `load_url` starts a new renderer.
pub(crate) fn replace_page(frame: &cef::Frame, url: &str) {
    use cef::ImplFrame;
    if !page_can_navigate_to(url) || frame.is_valid() == 0 {
        frame.load_url(Some(&cef::CefString::from(url)));
        return;
    }
    let code = cef::CefString::from(replace_page_script(url).as_str());
    let empty = cef::CefString::from("");
    frame.execute_java_script(Some(&code), Some(&empty), 0);
}

/// How long a demoted window gets to start booting its pool page before
/// `watch_demote_navigation` checks on it. Matches the promote liveness
/// timeout, which covers the same boot.
const DEMOTE_NAV_CHECK_INTERVAL: std::time::Duration = crate::state::PROMOTE_LIVENESS_TIMEOUT;
/// Checks before the watch gives up. Three covers a hung renderer: the hang
/// watchdog kills it into a recovery page, and the next check loads the pool
/// page in a new renderer.
const DEMOTE_NAV_CHECKS: u32 = 3;

/// Whether a demoted window that hasn't re-entered the pool queue needs the
/// pool page loaded from outside. A frame already on the pool page (its
/// `pool=1` flag, which only promote removes; the boot strips `ipc_port` and
/// `ipc_token`, so the URL isn't the one we navigated to) is booting it, just
/// slowly, and reloading would restart that boot. Anything else (the old
/// workspace page, a recovery page) means the page never navigated.
pub(crate) fn demote_needs_load(still_unpromoted: bool, current_url: &str) -> bool {
    let on_pool_page = current_url
        .split_once('?')
        .is_some_and(|(_, query)| query.split(['&', '#']).any(|param| param == "pool=1"));
    still_unpromoted && !on_pool_page
}

/// Navigate a demoted window to its pool page `target`, then confirm it got
/// there. `replace_page` runs a script in the page, which a hung renderer
/// never runs: `is_valid()` still holds, so the window would sit in
/// `pool.unpromoted` forever, never sending `pool_window_ready`, and hold a
/// pool slot. Each check falls back to `load_url` (see `demote_needs_load`).
pub(crate) fn navigate_demoted_window(
    state: &std::sync::Arc<crate::state::AppState>,
    label: &str,
    target: String,
) {
    use cef::ImplBrowser;
    if let Some(frame) = state.get_browser(label).and_then(|b| b.main_frame()) {
        replace_page(&frame, &target);
    }
    let state = state.clone();
    let label = label.to_string();
    std::thread::spawn(move || {
        for _ in 0..DEMOTE_NAV_CHECKS {
            std::thread::sleep(DEMOTE_NAV_CHECK_INTERVAL);
            if !state.host_state.lock().pool.unpromoted.contains(&label) {
                return; // re-queued as ready, or closed
            }
            let Some(browser) = state.get_browser(&label) else { return };
            let mut task = nav_check::DemoteNavCheckTask::new(browser, label.clone(), target.clone());
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
        }
    });
}

mod nav_check {
    use super::demote_needs_load;
    use cef::*;

    wrap_task! {
        pub(super) struct DemoteNavCheckTask {
            browser: Browser,
            label: String,
            target: String,
        }

        impl Task {
            fn execute(&self) {
                let Some(frame) = self.browser.main_frame() else { return };
                let current = CefString::from(&frame.url()).to_string();
                // Still unpromoted: the watch checked just before posting this.
                if !demote_needs_load(true, &current) {
                    return;
                }
                tracing::warn!(
                    target: "dnd:tearoff:pool",
                    label = %self.label,
                    current = %current,
                    "[pool] demoted window never navigated to its pool page; loading it with load_url"
                );
                frame.load_url(Some(&CefString::from(self.target.as_str())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{demote_needs_load, page_can_navigate_to, replace_page_script};

    #[test]
    fn a_demote_loads_only_when_the_page_never_navigated() {
        // The old workspace page (promote removed `pool`), or a recovery page
        // after the hang watchdog.
        assert!(demote_needs_load(true, "http://127.0.0.1:1234/?windowLabel=window-pool-1&workspaceId=w"));
        assert!(demote_needs_load(true, "data:text/html;base64,PGh0bWw+"));
        assert!(demote_needs_load(true, "http://127.0.0.1:1234/?pool=10"));
        // Booting the pool page, before and after the boot strips the IPC
        // params: reloading would restart the boot.
        assert!(!demote_needs_load(true, "http://127.0.0.1:1234/?ipc_port=1&ipc_token=t&windowLabel=window-pool-1&pool=1"));
        assert!(!demote_needs_load(true, "http://127.0.0.1:1234/?windowLabel=window-pool-1&pool=1"));
        assert!(!demote_needs_load(true, "http://localhost:5173/?pool=1#x"));
        // Re-queued as ready, or gone.
        assert!(!demote_needs_load(false, "http://127.0.0.1:1234/"));
    }

    #[test]
    fn the_page_navigates_only_to_http_and_blank() {
        assert!(page_can_navigate_to("http://127.0.0.1:1234/?pool=1"));
        assert!(page_can_navigate_to("http://localhost:5173"));
        assert!(page_can_navigate_to("https://example.com"));
        assert!(page_can_navigate_to("about:blank"));
        // The install-broken page: Chromium refuses it from the page.
        assert!(!page_can_navigate_to("data:text/html;base64,PGh0bWw+"));
        assert!(!page_can_navigate_to("DATA:text/html,x"));
        assert!(!page_can_navigate_to("file:///C:/x.html"));
    }

    #[test]
    fn replaces_rather_than_pushes() {
        assert_eq!(replace_page_script("about:blank"), r#"location.replace("about:blank")"#);
    }

    #[test]
    fn the_url_is_a_js_string_literal() {
        let js = replace_page_script(r#"http://localhost:5173/?pool=1&label=window-pool-1"#);
        assert_eq!(js, r#"location.replace("http://localhost:5173/?pool=1&label=window-pool-1")"#);
        // Quotes and backslashes can't break out of the literal.
        let js = replace_page_script(r#"x"); alert(1); ("#);
        assert!(js.starts_with("location.replace(\"") && js.ends_with("\")"), "{js}");
        assert!(!js.contains(r#"x"); alert"#), "{js}");
    }
}
