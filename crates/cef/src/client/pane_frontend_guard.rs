// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A browser pane shows other sites, never AgentMux's own frontend: that page
//! boots the whole app, and the host's credentials belong to the app's windows
//! only. The rule is `app_navigation::pane_may_load`; this is its handler.
//!
//! Which browsers are panes is decided from what each is registered as, not from
//! the client's `is_browser_pane` flag: floating pane windows carry the flag and
//! host the frontend (`takes_pane_path`).

use cef::*;

use super::AgentMuxHandler;
use crate::state::BrowserKind;

/// Whether a browser registered as `kind` is a browser pane, or a popup one
/// opened: the browsers that show web pages. A window, a floating pane window and
/// a browser not registered yet are not.
pub(super) fn shows_web_pages(kind: Option<&BrowserKind>) -> bool {
    matches!(kind, Some(BrowserKind::Pane { .. } | BrowserKind::Popup))
}

impl AgentMuxHandler {
    /// What `browser` is registered as, if it is.
    pub(super) fn registered_kind(&self, browser: &Browser) -> Option<BrowserKind> {
        let mut browser = browser.clone();
        let label = self.window_label_for(&mut browser)?;
        self.state.browser_kind_for_label(&label)
    }

    /// `on_before_browse`, main frame: true (cancel) when `browser` is a browser
    /// pane (or a popup of one) and `url` is the app's own frontend. The pane says
    /// why instead of staying blank.
    pub(super) fn stop_frontend_navigation(&self, browser: &Browser, url: &str) -> bool {
        if !shows_web_pages(self.registered_kind(browser).as_ref())
            || super::app_navigation::pane_may_load(url, &self.frontend_origins())
        {
            return false;
        }
        tracing::warn!("on_before_browse: refused a browser-pane navigation to the app's own address");
        if let Some(block_id) = crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, browser) {
            let payload = serde_json::json!({
                "block_id": block_id,
                "url": super::app_navigation::redact_ipc_token(url),
                "sites": [],
                "why": "A browser pane can't open AgentMux's own pages.",
            });
            crate::events::emit_browser_pane_event(&self.state, &block_id, "browser-pane-notice", &payload);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pane_or_its_popup_shows_web_pages() {
        assert!(shows_web_pages(Some(&BrowserKind::Pane { block_id: "b".into() })));
        assert!(shows_web_pages(Some(&BrowserKind::Popup)));
    }

    #[test]
    fn the_apps_own_windows_do_not() {
        // Floating pane windows host the frontend and must keep loading it, in a pool or not.
        assert!(!shows_web_pages(Some(&BrowserKind::Floater { is_pool: false })));
        assert!(!shows_web_pages(Some(&BrowserKind::Floater { is_pool: true })));
        assert!(!shows_web_pages(Some(&BrowserKind::TopLevel { is_pool: false })));
        assert!(!shows_web_pages(Some(&BrowserKind::TopLevel { is_pool: true })));
        assert!(!shows_web_pages(None));
    }
}
