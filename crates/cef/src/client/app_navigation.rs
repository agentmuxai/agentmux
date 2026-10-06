// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an app window's own page may go.
//!
//! An app window (the main window and other windows of the app itself, not a
//! browser pane) shows AgentMux's frontend and nothing else. A link that would
//! load in it instead opens elsewhere: the system browser, or, for a
//! middle-click, an AgentMux browser pane. And only the frontend's own pages
//! receive the host IPC credentials.
//!
//! Pure decisions, so they're tested without CEF; the handlers in
//! `lifecycle.rs` (`on_before_browse`, `on_open_url_from_tab`) and
//! `navigation.rs` (`on_load_end`) act on them.

use super::recovery_pages::url_on_origin;

/// What to do with a top-level navigation of an app window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppWindowNavigation {
    /// The frontend itself, or one of the host's own internal pages.
    Allow,
    /// Another website: cancel, and open it in the system browser.
    OpenInSystemBrowser,
    /// Something an app window must never load (a local file).
    Cancel,
}

fn is_web_url(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// The decision for a navigation of an app window's main frame to `url`.
/// `frontend_base` is the frontend's origin (`resolve_frontend_base_url`).
/// Without one (its assets are missing) everything is allowed as before: the
/// host's own error page has to load.
///
/// Internal schemes the host uses itself (`data:` error and recovery pages,
/// `about:`, `devtools:`, Chromium's error pages) are allowed.
pub(crate) fn app_window_navigation(url: &str, frontend_base: Option<&str>) -> AppWindowNavigation {
    let Some(base) = frontend_base else {
        return AppWindowNavigation::Allow;
    };
    if url_on_origin(url, base) {
        return AppWindowNavigation::Allow;
    }
    if is_web_url(url) {
        return AppWindowNavigation::OpenInSystemBrowser;
    }
    if url.trim_start().to_ascii_lowercase().starts_with("file:") {
        return AppWindowNavigation::Cancel;
    }
    AppWindowNavigation::Allow
}

/// Whether a link an app window asked to open in a new tab (a middle-click)
/// becomes an AgentMux browser pane: any web page other than the frontend
/// itself. Anything else is just cancelled.
pub(crate) fn opens_in_browser_pane(url: &str, frontend_base: Option<&str>) -> bool {
    is_web_url(url) && !frontend_base.is_some_and(|base| url_on_origin(url, base))
}

/// Whether a loaded main-frame document gets the host IPC credentials: only
/// when it is the frontend itself, in any window. Without a frontend origin
/// nothing is injected; the host's error page doesn't use them.
pub(crate) fn injects_ipc_credentials(frame_url: &str, frontend_base: Option<&str>) -> bool {
    frontend_base.is_some_and(|base| url_on_origin(frame_url, base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppWindowNavigation::*;

    const PROD: Option<&str> = Some("http://127.0.0.1:62876");
    const DEV: Option<&str> = Some("http://localhost:5300");

    #[test]
    fn the_frontend_itself_loads() {
        assert_eq!(app_window_navigation("http://127.0.0.1:62876/?windowLabel=main", PROD), Allow);
        assert_eq!(app_window_navigation("http://127.0.0.1:62876", PROD), Allow);
        assert_eq!(app_window_navigation("http://localhost:5300/#x", DEV), Allow);
    }

    #[test]
    fn another_website_goes_to_the_system_browser() {
        let url = "https://github.com/login?return_to=https%3A%2F%2Fgithub.com%2Fsettings%2Finstallations";
        assert_eq!(app_window_navigation(url, PROD), OpenInSystemBrowser);
        assert_eq!(app_window_navigation("HTTP://example.com", PROD), OpenInSystemBrowser);
        // Same host, another port: another origin.
        assert_eq!(app_window_navigation("http://127.0.0.1:628760/", PROD), OpenInSystemBrowser);
        assert_eq!(app_window_navigation("http://127.0.0.1:5300/", PROD), OpenInSystemBrowser);
    }

    #[test]
    fn a_local_file_is_refused() {
        assert_eq!(app_window_navigation("file:///C:/Users/x/notes.md", PROD), Cancel);
    }

    #[test]
    fn the_hosts_own_pages_still_load() {
        assert_eq!(app_window_navigation("data:text/html;base64,PGgxPg==", PROD), Allow);
        assert_eq!(app_window_navigation("about:blank", PROD), Allow);
        assert_eq!(app_window_navigation("devtools://devtools/bundled/inspector.html", PROD), Allow);
    }

    #[test]
    fn without_a_frontend_origin_nothing_changes() {
        assert_eq!(app_window_navigation("https://github.com/", None), Allow);
    }

    #[test]
    fn a_middle_clicked_web_link_becomes_a_browser_pane() {
        assert!(opens_in_browser_pane("https://github.com/settings/installations", PROD));
        assert!(opens_in_browser_pane("http://127.0.0.1:8080/", PROD));
        assert!(!opens_in_browser_pane("http://127.0.0.1:62876/?x=1", PROD));
        assert!(!opens_in_browser_pane("mailto:a@b.c", PROD));
        assert!(!opens_in_browser_pane("file:///C:/x", PROD));
        assert!(opens_in_browser_pane("https://github.com/", None));
    }

    #[test]
    fn only_the_frontend_gets_the_ipc_credentials() {
        assert!(injects_ipc_credentials("http://127.0.0.1:62876/?windowLabel=main", PROD));
        assert!(injects_ipc_credentials("http://localhost:5300/", DEV));
        assert!(!injects_ipc_credentials("https://github.com/login", PROD));
        assert!(!injects_ipc_credentials("data:text/html;base64,PGgxPg==", PROD));
        assert!(!injects_ipc_credentials("https://github.com/login", None));
    }
}
