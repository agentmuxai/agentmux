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
//! The frontend can be on more than one origin: the one secondary windows are
//! built from (`resolve_frontend_base_url`) and the one the main window was
//! actually loaded from (its `--url`, `AppState::main_frontend_origin`). Each
//! decision takes all of them (`AgentMuxHandler::frontend_origins`).
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

fn on_frontend(url: &str, frontend_origins: &[String]) -> bool {
    frontend_origins.iter().any(|origin| url_on_origin(url, origin))
}

/// `scheme://authority` of a web URL, the form `url_on_origin` compares
/// against. None for anything that isn't http(s).
pub(crate) fn url_origin(url: &str) -> Option<String> {
    let url = url.trim();
    if !is_web_url(url) {
        return None;
    }
    let scheme_end = url.find("://")? + 3;
    let rest = &url[scheme_end..];
    let authority_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if authority_len == 0 {
        return None;
    }
    Some(url[..scheme_end + authority_len].to_string())
}

/// The decision for a navigation of an app window's main frame to `url`.
/// With no known frontend origin everything is allowed as before.
///
/// Internal schemes the host uses itself (`data:` error and recovery pages,
/// `about:`, `devtools:`, Chromium's error pages) are allowed.
pub(crate) fn app_window_navigation(url: &str, frontend_origins: &[String]) -> AppWindowNavigation {
    if frontend_origins.is_empty() || on_frontend(url, frontend_origins) {
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
pub(crate) fn opens_in_browser_pane(url: &str, frontend_origins: &[String]) -> bool {
    is_web_url(url) && !on_frontend(url, frontend_origins)
}

/// Whether a loaded main-frame document gets the host IPC credentials: only
/// when it is the frontend itself, in any window. The host's own error pages
/// don't use them.
pub(crate) fn injects_ipc_credentials(frame_url: &str, frontend_origins: &[String]) -> bool {
    on_frontend(frame_url, frontend_origins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppWindowNavigation::*;

    fn origins(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_frontend_itself_loads() {
        let prod = origins(&["http://127.0.0.1:62876"]);
        assert_eq!(app_window_navigation("http://127.0.0.1:62876/?windowLabel=main", &prod), Allow);
        assert_eq!(app_window_navigation("http://127.0.0.1:62876", &prod), Allow);
        let dev = origins(&["http://localhost:5300"]);
        assert_eq!(app_window_navigation("http://localhost:5300/#x", &dev), Allow);
    }

    // `task run` points a packaged build's main window at Vite with `--url`,
    // and a dev build with a bundled frontend loads its main window from the
    // IPC server, while secondary windows use the other (#4410).
    #[test]
    fn a_main_window_loaded_from_another_frontend_origin_still_works() {
        let both = origins(&["http://127.0.0.1:62876", "http://localhost:5173"]);
        assert_eq!(app_window_navigation("http://localhost:5173/?ipc_port=62876", &both), Allow);
        assert_eq!(app_window_navigation("http://127.0.0.1:62876/?windowLabel=w2", &both), Allow);
        assert!(injects_ipc_credentials("http://localhost:5173/", &both));
        assert!(injects_ipc_credentials("http://127.0.0.1:62876/", &both));
        assert_eq!(app_window_navigation("https://github.com/", &both), OpenInSystemBrowser);
    }

    #[test]
    fn another_website_goes_to_the_system_browser() {
        let prod = origins(&["http://127.0.0.1:62876"]);
        let url = "https://github.com/login?return_to=https%3A%2F%2Fgithub.com%2Fsettings%2Finstallations";
        assert_eq!(app_window_navigation(url, &prod), OpenInSystemBrowser);
        assert_eq!(app_window_navigation("HTTP://example.com", &prod), OpenInSystemBrowser);
        // Same host, another port: another origin.
        assert_eq!(app_window_navigation("http://127.0.0.1:628760/", &prod), OpenInSystemBrowser);
        assert_eq!(app_window_navigation("http://127.0.0.1:5300/", &prod), OpenInSystemBrowser);
    }

    #[test]
    fn a_local_file_is_refused() {
        let prod = origins(&["http://127.0.0.1:62876"]);
        assert_eq!(app_window_navigation("file:///C:/Users/x/notes.md", &prod), Cancel);
    }

    #[test]
    fn the_hosts_own_pages_still_load() {
        let prod = origins(&["http://127.0.0.1:62876"]);
        assert_eq!(app_window_navigation("data:text/html;base64,PGgxPg==", &prod), Allow);
        assert_eq!(app_window_navigation("about:blank", &prod), Allow);
        assert_eq!(app_window_navigation("devtools://devtools/bundled/inspector.html", &prod), Allow);
    }

    #[test]
    fn without_a_frontend_origin_nothing_changes() {
        assert_eq!(app_window_navigation("https://github.com/", &[]), Allow);
    }

    #[test]
    fn a_middle_clicked_web_link_becomes_a_browser_pane() {
        let prod = origins(&["http://127.0.0.1:62876"]);
        assert!(opens_in_browser_pane("https://github.com/settings/installations", &prod));
        assert!(opens_in_browser_pane("http://127.0.0.1:8080/", &prod));
        assert!(!opens_in_browser_pane("http://127.0.0.1:62876/?x=1", &prod));
        assert!(!opens_in_browser_pane("mailto:a@b.c", &prod));
        assert!(!opens_in_browser_pane("file:///C:/x", &prod));
        assert!(opens_in_browser_pane("https://github.com/", &[]));
    }

    #[test]
    fn only_the_frontend_gets_the_ipc_credentials() {
        let prod = origins(&["http://127.0.0.1:62876"]);
        assert!(injects_ipc_credentials("http://127.0.0.1:62876/?windowLabel=main", &prod));
        assert!(!injects_ipc_credentials("https://github.com/login", &prod));
        assert!(!injects_ipc_credentials("data:text/html;base64,PGgxPg==", &prod));
        assert!(!injects_ipc_credentials("https://github.com/login", &[]));
    }

    #[test]
    fn url_origin_is_scheme_and_authority() {
        assert_eq!(url_origin("http://localhost:5173").as_deref(), Some("http://localhost:5173"));
        assert_eq!(url_origin("http://localhost:5173/?a=1").as_deref(), Some("http://localhost:5173"));
        assert_eq!(url_origin(" https://127.0.0.1:9/x#y ").as_deref(), Some("https://127.0.0.1:9"));
        assert_eq!(url_origin("http://"), None);
        assert_eq!(url_origin("data:text/html,x"), None);
    }
}
