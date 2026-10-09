// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A popup a browser pane opens becomes a browser pane beside it
//! (docs/specs/SPEC_BROWSER_PANE_POPUPS_ADOPTED_2026_10_08.md).
//!
//! The CEF host catches the popup (`on_before_popup`) and asks srv, as the
//! host, through `/api/v1/host/browser_popup`. srv decides here because the
//! one fact that widens the rule, whether an agent owns the opener, lives
//! only in srv (`browser_owner`). An admitted popup opens next to its opener;
//! anything else the host sends to the system browser, as before.
//!
//! The pairs are remembered in memory, like ownership: after srv restarts a
//! popup is an ordinary pane, and the opener's snapshot no longer lists it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Block meta: the pane that opened this one. Written only by srv.
pub(crate) const POPUP_OF_META_KEY: &str = "browser:popup_of";
/// Block meta: the opener's origin, shown in the popup's header so the person
/// can tell which site opened it. Written only by srv.
pub(crate) const POPUP_FROM_META_KEY: &str = "browser:popup_from";

/// Popups one pane may have open at once. Beyond it, popups go to the system
/// browser: a page can't flood the window with panes.
pub(crate) const MAX_POPUPS_PER_PANE: usize = 8;

/// Why a popup is not opened as a pane. The host opens it in the system
/// browser instead, which is what every popup did before this.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotWeb,
    NoUserAction,
    TooMany,
    AnotherSite,
}

impl Refusal {
    pub(crate) fn reason(&self) -> &'static str {
        match self {
            Refusal::NotWeb => "not an http(s) page",
            Refusal::NoUserAction => "the page opened it without a click or key press",
            Refusal::TooMany => "the pane already has as many popups open as it may",
            Refusal::AnotherSite => "it is on another site, and no agent owns the pane that opened it",
        }
    }
}

/// Is this popup opened as a pane beside its opener (spec §3.1)?
///
/// - only a popup the user (or the agent driving the pane, whose clicks are
///   real input events) caused: a script with no gesture opens nothing new;
/// - an http(s) page;
/// - fewer than [`MAX_POPUPS_PER_PANE`] already open;
/// - and either an agent owns the opener, so the popup is part of what that
///   agent is doing and must stay where it can drive it, or the popup is on
///   the opener's own site.
pub(crate) fn decide(
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

fn is_ip_host(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| matches!(u.host(), Some(url::Host::Ipv4(_)) | Some(url::Host::Ipv6(_))))
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
pub(crate) fn same_site(a: &str, b: &str) -> bool {
    // An IP address is a site of its own: the public suffix list would read
    // its last octet as an unknown top-level domain and call `192.0.2.5` and
    // `198.51.2.5` one site.
    if is_ip_host(a) || is_ip_host(b) {
        return host_of(a).is_some() && host_of(a) == host_of(b);
    }
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

/// The opener's origin, for the popup's header. Empty if it doesn't parse.
pub(crate) fn origin_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}

#[derive(Default)]
struct Popups {
    /// opener → its popup panes, oldest first.
    open: HashMap<String, Vec<String>>,
    /// opener → popups admitted but still being opened (`Reservation`).
    pending: HashMap<String, usize>,
}

fn popups() -> &'static Mutex<Popups> {
    static POPUPS: OnceLock<Mutex<Popups>> = OnceLock::new();
    POPUPS.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, Popups> {
    popups().lock().unwrap_or_else(|p| p.into_inner())
}

/// Drop `opener`'s closed popups (`exists` says whether a block is still
/// there) and return the open ones.
fn prune(p: &mut Popups, opener: &str, exists: impl Fn(&str) -> bool) -> Vec<String> {
    let Some(list) = p.open.get_mut(opener) else {
        return Vec::new();
    };
    list.retain(|id| exists(id));
    let out = list.clone();
    if list.is_empty() {
        p.open.remove(opener);
    }
    out
}

/// A slot for a popup pane being opened, held from the moment it is admitted
/// until it is open (`commit`) or not (dropped). Counting the slot during the
/// wait is what keeps two popups admitted at once from both fitting under the
/// cap.
pub(crate) struct Reservation {
    opener: String,
    held: bool,
}

impl Reservation {
    /// The popup pane opened: it counts as open from now on.
    pub(crate) fn commit(mut self, popup: &str) {
        let mut p = lock();
        release(&mut p, &self.opener);
        p.open.entry(self.opener.clone()).or_default().push(popup.to_string());
        self.held = false;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.held {
            release(&mut lock(), &self.opener);
        }
    }
}

fn release(p: &mut Popups, opener: &str) {
    if let Some(n) = p.pending.get_mut(opener) {
        *n = n.saturating_sub(1);
        if *n == 0 {
            p.pending.remove(opener);
        }
    }
}

/// Popup panes open or being opened anywhere below `node`: its own, and
/// theirs, and so on. Records stay with each popup's direct opener, so when a
/// pane in the middle of a chain closes, the subtree under it still counts
/// for whichever pane is now its root.
fn count_tree(p: &mut Popups, node: &str, exists: &dyn Fn(&str) -> bool, depth: usize) -> usize {
    let kids = prune(p, node, exists);
    let mut n = kids.len() + p.pending.get(node).copied().unwrap_or(0);
    if depth < CHAIN_DEPTH_LIMIT {
        for k in kids {
            n += count_tree(p, &k, exists, depth + 1);
        }
    }
    n
}

/// How deep `count_tree` follows popups of popups.
const CHAIN_DEPTH_LIMIT: usize = 16;

/// Admit one more popup pane from `opener` if `admit` agrees, given how many
/// popup panes the chain it belongs to (rooted at `root`) has open or being
/// opened, and hold its slot. The count and the slot are taken under one lock.
pub(crate) fn reserve<E>(
    opener: &str,
    root: &str,
    exists: impl Fn(&str) -> bool,
    admit: impl FnOnce(usize) -> Result<(), E>,
) -> Result<Reservation, E> {
    let mut p = lock();
    let taken = count_tree(&mut p, root, &exists, 0);
    admit(taken)?;
    *p.pending.entry(opener.to_string()).or_default() += 1;
    Ok(Reservation { opener: opener.to_string(), held: true })
}

/// Remember that `opener` opened `popup` (outside `reserve`; used by tests).
#[cfg(test)]
pub(crate) fn remember(opener: &str, popup: &str) {
    lock().open.entry(opener.to_string()).or_default().push(popup.to_string());
}

/// The popups `opener` has open, oldest first. `exists` says whether a block
/// is still there; closed ones are dropped from the record as they're found.
pub(crate) fn open_popups(opener: &str, exists: impl Fn(&str) -> bool) -> Vec<String> {
    prune(&mut lock(), opener, exists)
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
        // Not the same site because their last two octets match.
        assert!(!same_site("http://192.0.2.5/", "http://198.51.2.5/"));
        assert!(!same_site("http://[2001:db8::1]/", "http://[2001:db8::2]/"));
        assert!(same_site("http://[2001:db8::1]:80/a", "http://[2001:db8::1]/b"));
        // A name and an address are never one site.
        assert!(!same_site("http://example.com/", "http://192.0.2.5/"));
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
    fn a_held_slot_counts_against_the_cap_until_it_opens_or_is_dropped() {
        let opener = "test-reserve-opener";
        let cap = |taken: usize| if taken < 2 { Ok(()) } else { Err(taken) };
        // Two admitted at once: both slots count, so a third doesn't fit,
        // although nothing is open yet.
        let a = reserve(opener, opener, |_| true, cap).unwrap();
        let b = reserve(opener, opener, |_| true, cap).unwrap();
        assert_eq!(reserve(opener, opener, |_| true, cap).err(), Some(2));
        // One opens, the other fails: the failed one's slot comes back.
        a.commit("test-reserve-p1");
        drop(b);
        assert_eq!(open_popups(opener, |_| true), vec!["test-reserve-p1"]);
        let c = reserve(opener, opener, |_| true, cap).unwrap();
        assert_eq!(reserve(opener, opener, |_| true, cap).err(), Some(2));
        drop(c);
        // A closed popup frees its slot too.
        assert!(reserve(opener, opener, |id| id != "test-reserve-p1", cap).is_ok());
        assert!(open_popups(opener, |_| true).is_empty());
    }

    #[test]
    fn a_chain_counts_together_and_survives_its_root_closing() {
        // R opened A; A opened B.
        remember("test-tree-r", "test-tree-a");
        remember("test-tree-a", "test-tree-b");
        let count = |root: &str, exists: &dyn Fn(&str) -> bool| {
            let mut taken = None;
            let _ = reserve("test-tree-probe", root, |id| exists(id), |n| {
                taken = Some(n);
                Err::<(), ()>(())
            });
            taken.unwrap()
        };
        assert_eq!(count("test-tree-r", &|_| true), 2);
        // R closed: A is the root now, and B still counts for it.
        assert_eq!(count("test-tree-a", &|id| id != "test-tree-r"), 1);
        assert_eq!(open_popups("test-tree-a", |_| true), vec!["test-tree-b"]);
        open_popups("test-tree-r", |_| false);
        open_popups("test-tree-a", |_| false);
    }

    #[test]
    fn open_popups_drops_closed_ones() {
        remember("test-popup-opener", "p1");
        remember("test-popup-opener", "p2");
        assert_eq!(open_popups("test-popup-opener", |_| true), vec!["p1", "p2"]);
        assert_eq!(open_popups("test-popup-opener", |p| p == "p2"), vec!["p2"]);
        assert!(open_popups("test-popup-opener", |_| false).is_empty());
        assert!(open_popups("test-popup-opener", |_| true).is_empty());
        assert!(open_popups("test-popup-unknown", |_| true).is_empty());
    }
}
