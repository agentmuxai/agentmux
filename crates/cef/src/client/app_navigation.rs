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

/// The path srv serves a sandboxed widget's files under
/// (`/agentmux/widget-files/<id>/<hash>/<key>/<file>`).
const WIDGET_FILES_PATH: &str = "/agentmux/widget-files/";

/// `<origin>/agentmux/widget-files/<id>/<hash>/<key>/` of a widget file URL:
/// the folder one approved package is served from. None for any other URL,
/// and for one whose rest could climb out of it.
pub(crate) fn widget_package_root(url: &str) -> Option<String> {
    let url = url.trim();
    let origin = url_origin(url)?;
    let rest = url[origin.len()..].strip_prefix(WIDGET_FILES_PATH)?;
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    let lower = path.to_ascii_lowercase();
    if path.contains('\\') || lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
        return None;
    }
    let mut parts = path.splitn(4, '/');
    let (id, hash, key, file) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if [id, hash, key].iter().any(|seg| seg.is_empty() || *seg == "." || *seg == "..") {
        return None;
    }
    if file.split('/').any(|seg| seg == "." || seg == "..") {
        return None;
    }
    Some(format!("{origin}{WIDGET_FILES_PATH}{id}/{hash}/{key}/"))
}

/// Whether a frame of an app window may navigate to `target`. A sandboxed
/// widget's frame (one showing a widget file) may only move within its own
/// package: a link, `location`, a form, a redirect or a refresh tag that
/// points anywhere else would send a request to a server the widget never
/// declared (`net:`), and its CSP can't stop a navigation. Any other frame is
/// left alone.
pub(crate) fn subframe_may_navigate(frame_url: &str, target: &str) -> bool {
    let Some(root) = widget_package_root(frame_url) else {
        return true;
    };
    widget_package_root(target).as_deref() == Some(root.as_str())
}

/// The `on_before_browse` side of [`subframe_may_navigate`]: true to cancel
/// the navigation of a subframe of an app window.
pub(crate) fn refuse_subframe_navigation(frame: Option<&mut cef::Frame>, request: Option<&mut cef::Request>) -> bool {
    use cef::{CefString, ImplFrame, ImplRequest};
    let Some(frame) = frame else { return false };
    let from = CefString::from(&frame.url()).to_string();
    let to = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
    if subframe_may_navigate(&from, &to) {
        return false;
    }
    tracing::warn!(from = %from, to = %to, "app window: refused a sandboxed widget's frame navigating outside its package");
    true
}

/// Whether a link an app window asked to open in a new tab (a middle-click)
/// becomes an AgentMux browser pane: any web page other than the frontend
/// itself. Anything else is just cancelled.
pub(crate) fn opens_in_browser_pane(url: &str, frontend_origins: &[String]) -> bool {
    is_web_url(url) && !on_frontend(url, frontend_origins)
}

/// Whether a loaded main-frame document gets the host IPC credentials: only
/// when it is the frontend itself in an app window (a top-level window or a
/// floating pane window). A browser pane never gets them, whatever page it
/// shows: `browser_pane` is what the browser is registered as, so a pane that
/// was pointed at the frontend's own address stays without them. The host's own
/// error pages don't use them.
pub(crate) fn injects_ipc_credentials(frame_url: &str, frontend_origins: &[String], browser_pane: bool) -> bool {
    !browser_pane && on_frontend(frame_url, frontend_origins)
}

/// The host and port of an http(s) URL, lowercased, without any `user@`.
fn host_and_port(url: &str) -> Option<(String, Option<u16>)> {
    let origin = url_origin(url)?;
    let rest = &origin[origin.find("://")? + 3..];
    let authority = rest.rsplit('@').next()?.to_ascii_lowercase();
    if let Some(stripped) = authority.strip_prefix('[') {
        let (host, after) = stripped.split_once(']')?;
        let port = after.strip_prefix(':').and_then(|p| p.parse().ok());
        return Some((format!("[{host}]"), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok())),
        None => Some((authority, None)),
    }
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host == "[::1]" || host == "0.0.0.0" || host.starts_with("127.")
}

/// Whether `url` is served by AgentMux's own frontend: an address on a frontend
/// origin, or the same port on another name for this machine (`127.0.0.1` for
/// `localhost`), which is the same server.
pub(crate) fn is_frontend_address(url: &str, frontend_origins: &[String]) -> bool {
    if on_frontend(url, frontend_origins) {
        return true;
    }
    let Some((host, port)) = host_and_port(url) else { return false };
    if !is_loopback_host(&host) {
        return false;
    }
    frontend_origins.iter().any(|origin| {
        let Some((frontend_host, frontend_port)) = host_and_port(origin) else { return false };
        is_loopback_host(&frontend_host) && port.is_some() && port == frontend_port
    })
}

/// Whether a browser pane may load `url` in its main frame. Not AgentMux's own
/// frontend: a pane shows other sites, and the frontend is for the app's windows.
pub(crate) fn pane_may_load(url: &str, frontend_origins: &[String]) -> bool {
    !is_frontend_address(url, frontend_origins)
}

/// `url` with the value of an `ipc_token` parameter replaced, for logs.
pub(crate) fn redact_ipc_token(url: &str) -> String {
    const KEY: &str = "ipc_token=";
    let mut out = String::with_capacity(url.len());
    let mut rest = url;
    while let Some(at) = rest.find(KEY) {
        let value_start = at + KEY.len();
        let is_param = at > 0 && matches!(rest.as_bytes()[at - 1], b'?' | b'&' | b'#');
        out.push_str(&rest[..value_start]);
        rest = &rest[value_start..];
        if is_param {
            let end = rest.find(['&', '#']).unwrap_or(rest.len());
            out.push_str("<redacted>");
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
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
        assert!(injects_ipc_credentials("http://localhost:5173/", &both, false));
        assert!(injects_ipc_credentials("http://127.0.0.1:62876/", &both, false));
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
        assert!(injects_ipc_credentials("http://127.0.0.1:62876/?windowLabel=main", &prod, false));
        assert!(!injects_ipc_credentials("https://github.com/login", &prod, false));
        assert!(!injects_ipc_credentials("data:text/html;base64,PGgxPg==", &prod, false));
        assert!(!injects_ipc_credentials("https://github.com/login", &[], false));
    }

    #[test]
    fn a_browser_pane_never_gets_the_ipc_credentials() {
        let prod = origins(&["http://127.0.0.1:62876"]);
        let dev = origins(&["http://localhost:5173"]);
        assert!(!injects_ipc_credentials("http://127.0.0.1:62876/", &prod, true));
        assert!(!injects_ipc_credentials("http://127.0.0.1:62876/?windowLabel=main", &prod, true));
        assert!(!injects_ipc_credentials("http://localhost:5173/", &dev, true));
    }

    #[test]
    fn a_browser_pane_may_not_load_the_frontend() {
        let both = origins(&["http://127.0.0.1:62876", "http://localhost:5173"]);
        for url in [
            "http://127.0.0.1:62876/",
            "http://127.0.0.1:62876/?windowLabel=main#x",
            "http://localhost:5173",
            "HTTP://LOCALHOST:5173/",
            // Another name for the same machine and port is the same server.
            "http://localhost:62876/",
            "http://127.0.0.1:5173/",
            "http://[::1]:5173/",
            "http://127.0.0.2:5173/",
            "http://someone@localhost:5173/",
            "http://app.localhost:5173/",
        ] {
            assert!(!pane_may_load(url, &both), "should refuse: {url}");
        }
    }

    #[test]
    fn a_browser_pane_may_load_other_sites() {
        let both = origins(&["http://127.0.0.1:62876", "http://localhost:5173"]);
        for url in [
            "https://example.com/",
            "https://example.com:5173/",
            "http://127.0.0.1:8080/",
            "http://localhost:3000/",
            "http://127.0.0.1:628760/",
            "https://localhost.evil.test:5173/",
            "http://localhost@evil.test:5173/",
            "about:blank",
            "data:text/html,x",
        ] {
            assert!(pane_may_load(url, &both), "should allow: {url}");
        }
        // No known frontend origin: nothing to keep a pane off.
        assert!(pane_may_load("http://localhost:5173/", &[]));
    }

    #[test]
    fn the_ipc_token_is_redacted_for_logs() {
        assert_eq!(
            redact_ipc_token("http://localhost:5173/?ipc_port=1&ipc_token=abc-123&windowLabel=main"),
            "http://localhost:5173/?ipc_port=1&ipc_token=<redacted>&windowLabel=main"
        );
        assert_eq!(redact_ipc_token("http://h/?ipc_token=abc"), "http://h/?ipc_token=<redacted>");
        assert_eq!(redact_ipc_token("http://h/#a=1&ipc_token=abc#x"), "http://h/#a=1&ipc_token=<redacted>#x");
        assert_eq!(
            redact_ipc_token("http://h/?x=1&ipc_token=a&ipc_token=b"),
            "http://h/?x=1&ipc_token=<redacted>&ipc_token=<redacted>"
        );
        // Not the parameter: left alone.
        assert_eq!(redact_ipc_token("http://h/?my_ipc_token=abc"), "http://h/?my_ipc_token=abc");
        assert_eq!(redact_ipc_token("https://example.com/"), "https://example.com/");
    }

    #[test]
    fn url_origin_is_scheme_and_authority() {
        assert_eq!(url_origin("http://localhost:5173").as_deref(), Some("http://localhost:5173"));
        assert_eq!(url_origin("http://localhost:5173/?a=1").as_deref(), Some("http://localhost:5173"));
        assert_eq!(url_origin(" https://127.0.0.1:9/x#y ").as_deref(), Some("https://127.0.0.1:9"));
        assert_eq!(url_origin("http://"), None);
        assert_eq!(url_origin("data:text/html,x"), None);
    }

    const FILES: &str = "http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123/k3y";

    #[test]
    fn a_widget_package_root_is_its_id_hash_and_key() {
        assert_eq!(
            widget_package_root(&format!("{FILES}/index.html?pane=1")).as_deref(),
            Some("http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123/k3y/")
        );
        assert_eq!(widget_package_root(&format!("{FILES}/sub/page.html#x")), widget_package_root(&format!("{FILES}/index.html")));
        assert_eq!(widget_package_root("http://127.0.0.1:62876/agentmux/widget-sdk/v1.js"), None);
        assert_eq!(widget_package_root("http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123"), None);
        assert_eq!(widget_package_root("http://127.0.0.1:62876/agentmux/widget-files//abc123/k3y/index.html"), None);
        assert_eq!(widget_package_root("about:blank"), None);
        assert_eq!(widget_package_root(""), None);
    }

    #[test]
    fn a_widget_frame_stays_in_its_own_package() {
        let frame = format!("{FILES}/index.html?pane=1");
        assert!(subframe_may_navigate(&frame, &format!("{FILES}/index.html?pane=2")));
        assert!(subframe_may_navigate(&frame, &format!("{FILES}/other/page.html#top")));
    }

    #[test]
    fn a_widget_frame_can_go_nowhere_else() {
        let frame = format!("{FILES}/index.html");
        for target in [
            "https://evil.example/?data=secret",
            "http://127.0.0.1:62876/",
            "http://127.0.0.1:62876/agentmux/wps/publish",
            "http://127.0.0.1:62877/agentmux/widget-files/acme.notes/abc123/k3y/index.html",
            "http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123/other/index.html",
            "http://127.0.0.1:62876/agentmux/widget-files/acme.other/abc123/k3y/index.html",
            "http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123/k3y/../../x/y/z/a.html",
            "http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123/k3y/%2e%2e/%2e%2e/x",
            "http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123/k3y/..%5c..%5cx",
            "about:blank",
            "data:text/html,<h1>x</h1>",
            "blob:http://127.0.0.1:62876/0e1d",
            "file:///etc/passwd",
            "javascript:void(0)",
            "",
        ] {
            assert!(!subframe_may_navigate(&frame, target), "{target}");
        }
    }

    #[test]
    fn a_frame_that_is_not_a_widgets_is_left_alone() {
        assert!(subframe_may_navigate("", "http://127.0.0.1:62876/agentmux/widget-files/acme.notes/abc123/k3y/index.html"));
        assert!(subframe_may_navigate("about:blank", "https://example.com/"));
        assert!(subframe_may_navigate("blob:http://127.0.0.1:62876/0e1d", "blob:http://127.0.0.1:62876/9f"));
    }
}
