// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which new windows a browser pane's page may open inside AgentMux
//! (docs/specs/SPEC_BROWSER_PANE_POPUPS_ADOPTED_2026_10_08.md §3.1,
//! docs/specs/SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §3).
//!
//! Shared by the CEF host, which must decide inside `on_before_popup`
//! (creating a native popup is that callback's return value, so it can't
//! wait on srv), and srv, which re-checks before it opens a pane.

/// New windows one pane may have open at once, per kind (popup windows,
/// popup panes). Beyond it, they go to the system browser.
pub const MAX_POPUPS_PER_PANE: usize = 8;

/// Why a request is not honored. The caller does what it did before this
/// existed (the system browser, or the pane's own frame for a link).
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    NotWeb,
    NoUserAction,
    TooMany,
    AnotherSite,
}

impl Refusal {
    pub fn reason(&self) -> &'static str {
        match self {
            Refusal::NotWeb => "not an http(s) page",
            Refusal::NoUserAction => "the page opened it without a click or key press",
            Refusal::TooMany => "the pane already has as many popups open as it may",
            Refusal::AnotherSite => "it is on another site, and no agent owns the pane that opened it",
        }
    }
}

/// Is this request honored?
///
/// - only one the user (or the agent driving the pane, whose clicks are real
///   input events) caused: a script with no gesture opens nothing new;
/// - an http(s) page;
/// - fewer than [`MAX_POPUPS_PER_PANE`] of its kind already open;
/// - and either an agent owns the opener, so the window is part of what that
///   agent is doing and must stay where it can drive it, or the target is on
///   the opener's own site.
pub fn decide(
    url: &str,
    opener_url: &str,
    user_gesture: bool,
    opener_owned: bool,
    open_popups: usize,
) -> Result<(), Refusal> {
    if !is_web_url(url) {
        return Err(Refusal::NotWeb);
    }
    if !user_gesture {
        return Err(Refusal::NoUserAction);
    }
    if open_popups >= MAX_POPUPS_PER_PANE {
        return Err(Refusal::TooMany);
    }
    if opener_owned || same_site(url, opener_url) {
        Ok(())
    } else {
        Err(Refusal::AnotherSite)
    }
}

fn is_web_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    u.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase())
}

/// Do both URLs belong to the same site: the same registrable domain under
/// the public suffix list (`console.example.com` and `signin.example.com`
/// do; `a.github.io` and `b.github.io` don't)? A host with no registrable
/// domain (an IP address, `localhost`) matches only itself.
pub fn same_site(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (host_of(a), host_of(b)) else {
        return false;
    };
    if a == b {
        return true;
    }
    match (psl::domain_str(&a), psl::domain_str(&b)) {
        (Some(da), Some(db)) => da == db,
        _ => false,
    }
}

/// `url`'s origin (`scheme://host[:port]`), or empty if it isn't http(s).
pub fn origin_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}

/// Is `url` on this machine's loopback (`localhost`, `127.0.0.1`, `[::1]`)?
pub fn is_loopback(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// May a page on loopback open a window to `url`, also on loopback? Only
/// on its own origin (a local dev site opening its own popup), and never one
/// of AgentMux's own (`own_origins`: its frontend and srv), which a page
/// must not be able to put in front of the person
/// (native-popups spec §3).
pub fn loopback_allowed(url: &str, opener_url: &str, own_origins: &[String]) -> bool {
    if !is_loopback(url) {
        return false;
    }
    let origin = origin_of(url);
    !origin.is_empty()
        && origin == origin_of(opener_url)
        && !own_origins.iter().any(|o| o.trim_end_matches('/').eq_ignore_ascii_case(&origin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_site_follows_the_public_suffix_list() {
        assert!(same_site("https://console.example.com/x", "https://signin.example.com/y"));
        assert!(same_site("https://example.com", "https://www.example.com"));
        // Hosting platforms where every customer has their own subdomain.
        assert!(!same_site("https://alice.github.io", "https://bob.github.io"));
        assert!(!same_site("https://a.example.co.uk", "https://b.other.co.uk"));
        assert!(same_site("https://a.example.co.uk", "https://b.example.co.uk"));
        assert!(!same_site("https://example.com", "https://example.org"));
        // Case and a trailing dot don't matter.
        assert!(same_site("https://Console.Example.COM.", "https://signin.example.com"));
    }

    #[test]
    fn hosts_without_a_registrable_domain_match_only_themselves() {
        assert!(same_site("http://localhost:3000/a", "http://localhost:4000/b"));
        assert!(!same_site("http://localhost", "http://127.0.0.1"));
        assert!(same_site("http://192.0.2.5/a", "http://192.0.2.5/b"));
        assert!(!same_site("http://192.0.2.5", "http://192.0.2.6"));
    }

    #[test]
    fn non_web_urls_are_never_the_same_site() {
        assert!(!same_site("file:///c:/x", "file:///c:/y"));
        assert!(!same_site("https://example.com", "not a url"));
    }

    const OPENER: &str = "https://console.example.com/home";

    #[test]
    fn a_clicked_popup_on_the_openers_site_is_admitted() {
        assert_eq!(decide("https://signin.example.com/x", OPENER, true, false, 0), Ok(()));
    }

    #[test]
    fn another_site_needs_an_owned_opener() {
        let other = "https://idp.other.net/login";
        assert_eq!(decide(other, OPENER, true, false, 0), Err(Refusal::AnotherSite));
        assert_eq!(decide(other, OPENER, true, true, 0), Ok(()));
    }

    #[test]
    fn no_gesture_no_pane_even_for_an_owned_opener() {
        assert_eq!(
            decide("https://signin.example.com/x", OPENER, false, true, 0),
            Err(Refusal::NoUserAction)
        );
    }

    #[test]
    fn the_cap_holds_for_owned_openers_too() {
        let url = "https://signin.example.com/x";
        assert_eq!(decide(url, OPENER, true, true, MAX_POPUPS_PER_PANE - 1), Ok(()));
        assert_eq!(decide(url, OPENER, true, true, MAX_POPUPS_PER_PANE), Err(Refusal::TooMany));
    }

    #[test]
    fn only_web_pages() {
        assert_eq!(decide("file:///c:/secrets", OPENER, true, true, 0), Err(Refusal::NotWeb));
        assert_eq!(decide("javascript:alert(1)", OPENER, true, true, 0), Err(Refusal::NotWeb));
    }

    #[test]
    fn origin_of_keeps_scheme_host_and_port_only() {
        assert_eq!(origin_of("https://console.example.com/home?x=1"), "https://console.example.com");
        assert_eq!(origin_of("http://localhost:3000/a"), "http://localhost:3000");
        assert_eq!(origin_of("file:///c:/x"), "");
    }

    #[test]
    fn loopback_is_localhost_and_the_loopback_addresses() {
        assert!(is_loopback("http://localhost:3000/"));
        assert!(is_loopback("http://127.0.0.1/"));
        assert!(is_loopback("http://127.0.0.2:8766/"));
        assert!(is_loopback("http://[::1]:8080/"));
        assert!(!is_loopback("https://example.com/"));
        assert!(!is_loopback("http://192.0.2.5/"));
    }

    #[test]
    fn loopback_popups_stay_on_their_own_origin_and_off_agentmuxs() {
        let own = vec!["http://localhost:5300".to_string(), "http://127.0.0.1:29705/".to_string()];
        // A local dev site opening its own popup.
        assert!(loopback_allowed("http://localhost:3000/popup", "http://localhost:3000/", &own));
        // Another port is another origin.
        assert!(!loopback_allowed("http://localhost:3001/popup", "http://localhost:3000/", &own));
        // A remote page can't open one on this machine.
        assert!(!loopback_allowed("http://localhost:3000/", "https://example.com/", &own));
        // Never AgentMux's own frontend or srv, even from a page on them.
        assert!(!loopback_allowed("http://localhost:5300/x", "http://localhost:5300/", &own));
        assert!(!loopback_allowed("http://127.0.0.1:29705/x", "http://127.0.0.1:29705/", &own));
        // Not a loopback target at all.
        assert!(!loopback_allowed("https://example.com/", "https://example.com/", &own));
    }
}
