// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a new window a browser pane's page asks for goes
//! (docs/specs/SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §2–§3).
//!
//! Pure, so the whole table is tested here; `on_before_popup` gathers the
//! facts and acts on the answer. The page chooses the kind by what it asked
//! for; `agentmux_common::popup_rules` decides whether it is honored.

use agentmux_common::popup_rules;

/// What the page asked for, from CEF's `WindowOpenDisposition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    /// `window.open` with popup features (a size, `popup`): a bare window.
    Popup,
    /// A new window (shift-click, some `window.open` calls): a full browsing
    /// context, so a pane.
    Window,
    /// A new tab (`target=_blank`, `window.open(url)`, middle-click): a pane.
    Tab,
    /// Anything else (the current tab, save-to-disk, …): left as before.
    Other,
}

/// Where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    /// A real CEF child popup window (`on_before_popup` returns false).
    NativeWindow,
    /// A browser pane beside the opener, opened by srv.
    Pane,
    /// The person's system browser (what popups always did).
    SystemBrowser,
    /// The opener's own frame (what links always did).
    LoadInFrame,
    /// Nothing.
    Cancel,
}

pub(crate) struct Request<'a> {
    pub asked: Asked,
    pub url: &'a str,
    /// The opener page's address.
    pub opener_url: &'a str,
    pub user_gesture: bool,
    /// The browser-pane block that opened it (the root pane, for a popup's
    /// own popup), if known.
    pub opener: Option<&'a str>,
    /// Does an agent own that pane (srv's set, as the host last heard it)?
    pub opener_owned: bool,
    /// Popup windows that pane has open.
    pub windows_open: usize,
    /// A known sign-in provider's authorization URL (`is_known_idp_host` and
    /// `is_oauth_authorization_url`): the native popup it always got.
    pub sign_in: bool,
    /// An http(s) URL off this machine (`is_external_http_url`).
    pub external: bool,
    /// AgentMux's own origins (its frontend, srv), never opened in-app.
    pub own_origins: &'a [String],
}

pub(crate) fn route(r: &Request) -> Route {
    let fallback = |asked: Asked| match asked {
        Asked::Tab => Route::LoadInFrame,
        _ => Route::SystemBrowser,
    };
    match r.asked {
        Asked::Other => return Route::LoadInFrame,
        // A sign-in provider's popup keeps the native window it has always
        // had, whatever else holds.
        Asked::Popup | Asked::Window if r.sign_in => return Route::NativeWindow,
        _ => {}
    }
    let loopback_ok = popup_rules::loopback_allowed(r.url, r.opener_url, r.own_origins);
    if !r.external && !loopback_ok {
        // An internal address: a link loads in place as before; a popup is
        // dropped, so a page can't put AgentMux's own pages in a window.
        return match r.asked {
            Asked::Tab => Route::LoadInFrame,
            _ => Route::Cancel,
        };
    }
    if r.opener.is_none() {
        return fallback(r.asked);
    }
    // srv counts popup panes itself when it opens one; the host counts its
    // popup windows.
    let open = if r.asked == Asked::Popup { r.windows_open } else { 0 };
    match popup_rules::decide(r.url, r.opener_url, r.user_gesture, r.opener_owned, open) {
        Ok(()) if r.asked == Asked::Popup => Route::NativeWindow,
        Ok(()) => Route::Pane,
        Err(_) => fallback(r.asked),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENER_URL: &str = "https://console.example.com/home";

    fn req(asked: Asked, url: &str) -> Request<'static> {
        Request {
            asked,
            url: Box::leak(url.to_string().into_boxed_str()),
            opener_url: OPENER_URL,
            user_gesture: true,
            opener: Some("pane-1"),
            opener_owned: false,
            windows_open: 0,
            sign_in: false,
            external: !popup_rules::is_loopback(url),
            own_origins: &[],
        }
    }

    #[test]
    fn the_page_chooses_window_or_pane() {
        let same_site = "https://signin.example.com/x";
        assert_eq!(route(&req(Asked::Popup, same_site)), Route::NativeWindow);
        assert_eq!(route(&req(Asked::Window, same_site)), Route::Pane);
        assert_eq!(route(&req(Asked::Tab, same_site)), Route::Pane);
        assert_eq!(route(&req(Asked::Other, same_site)), Route::LoadInFrame);
    }

    #[test]
    fn not_honored_falls_back_to_what_each_did_before() {
        // Another site, from a person's pane.
        let other = "https://shop.other.org/";
        assert_eq!(route(&req(Asked::Popup, other)), Route::SystemBrowser);
        assert_eq!(route(&req(Asked::Window, other)), Route::SystemBrowser);
        assert_eq!(route(&req(Asked::Tab, other)), Route::LoadInFrame);
        // No click.
        let mut r = req(Asked::Popup, "https://signin.example.com/x");
        r.user_gesture = false;
        assert_eq!(route(&r), Route::SystemBrowser);
        // The pane that opened it isn't known.
        let mut r = req(Asked::Popup, "https://signin.example.com/x");
        r.opener = None;
        assert_eq!(route(&r), Route::SystemBrowser);
    }

    #[test]
    fn an_agent_owned_pane_keeps_other_sites_in_app() {
        let mut r = req(Asked::Popup, "https://pay.other.org/");
        r.opener_owned = true;
        assert_eq!(route(&r), Route::NativeWindow);
        r.asked = Asked::Tab;
        assert_eq!(route(&r), Route::Pane);
    }

    #[test]
    fn popup_windows_are_capped_per_pane() {
        let mut r = req(Asked::Popup, "https://signin.example.com/x");
        r.windows_open = popup_rules::MAX_POPUPS_PER_PANE;
        assert_eq!(route(&r), Route::SystemBrowser);
        // Panes are counted by srv, not here.
        r.asked = Asked::Tab;
        assert_eq!(route(&r), Route::Pane);
    }

    #[test]
    fn a_sign_in_provider_always_gets_its_native_popup() {
        let mut r = req(Asked::Popup, "https://accounts.google.com/o/oauth2/v2/auth?client_id=x");
        r.sign_in = true;
        r.user_gesture = false;
        r.opener = None;
        assert_eq!(route(&r), Route::NativeWindow);
    }

    #[test]
    fn loopback_only_on_the_openers_own_origin_and_never_agentmuxs() {
        let own = vec!["http://localhost:5300".to_string()];
        let mut r = req(Asked::Popup, "http://localhost:3000/popup");
        r.opener_url = "http://localhost:3000/";
        r.own_origins = Box::leak(own.into_boxed_slice());
        assert_eq!(route(&r), Route::NativeWindow);
        // Another port.
        r.url = "http://localhost:3001/popup";
        assert_eq!(route(&r), Route::Cancel);
        // AgentMux's own frontend.
        r.url = "http://localhost:5300/x";
        r.opener_url = "http://localhost:5300/";
        assert_eq!(route(&r), Route::Cancel);
        // A link to an internal address still loads in place.
        r.asked = Asked::Tab;
        assert_eq!(route(&r), Route::LoadInFrame);
    }
}
