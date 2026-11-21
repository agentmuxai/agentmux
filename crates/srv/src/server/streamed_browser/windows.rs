// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The windows of one streamed pane: the page it started with, and the windows that page (or a window of it) opened. Only
//! one is shown at a time. A window a page opens (a sign-in pop-up, a link to a new tab) becomes the shown one, so the person sees
//! it and its address; it can return its result to the page that opened it; it ends when its page closes it, and the pane goes
//! back to the window that opened it. A page cannot open windows without limit.
//!
//! Plain data and functions, so every rule is tested without a browser.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// At most this many windows in one pane, the first included.
pub const MAX_WINDOWS: usize = 4;
/// After the shown window changes without the person choosing it (a page opened one, or one closed itself), keys the person
/// types are dropped for this long: a key meant for one window must not land in another (a password typed into a page the
/// person had not yet seen). Pointer input is not held.
pub const KEY_GAP: Duration = Duration::from_millis(500);
/// The window id of the page a pane starts with. It is never closed by the person or by a page.
pub const FIRST: WindowId = 0;

pub type WindowId = u32;

/// One window: where the browser keeps it and what the person sees of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: WindowId,
    /// The browser's id of the page and the session this pane talks to it through.
    pub target: String,
    pub session: String,
    /// The window that opened this one.
    pub opener: Option<WindowId>,
    pub url: String,
    pub title: String,
    /// While the window was not shown it went from one site to another: (the site the person last saw, the site it is on now).
    pub moved: Option<(String, String)>,
}

/// What `remove` changed.
#[derive(Debug, PartialEq, Eq)]
pub struct Removed {
    pub was_active: bool,
    pub now_active: WindowId,
}

#[derive(Debug)]
pub struct Windows {
    list: Vec<Entry>,
    active: WindowId,
    next: WindowId,
}

impl Windows {
    /// The pane's first window, which is the shown one.
    pub fn starting(target: String, session: String) -> Windows {
        let first = Entry { id: FIRST, target, session, opener: None, url: String::new(), title: String::new(), moved: None };
        Windows { list: vec![first], active: FIRST, next: FIRST + 1 }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn has_room(&self) -> bool {
        self.list.len() < MAX_WINDOWS
    }

    pub fn active(&self) -> WindowId {
        self.active
    }

    pub fn entries(&self) -> &[Entry] {
        &self.list
    }

    pub fn get(&self, id: WindowId) -> Option<&Entry> {
        self.list.iter().find(|w| w.id == id)
    }

    pub fn by_session(&self, session: &str) -> Option<WindowId> {
        self.list.iter().find(|w| w.session == session).map(|w| w.id)
    }

    pub fn by_target(&self, target: &str) -> Option<WindowId> {
        self.list.iter().find(|w| w.target == target).map(|w| w.id)
    }

    /// A new window, opened by `opener`. It becomes the shown one. `None` when the pane is at its limit.
    pub fn add(&mut self, target: String, session: String, opener: Option<WindowId>) -> Option<WindowId> {
        if !self.has_room() {
            return None;
        }
        let id = self.next;
        self.next += 1;
        self.list.push(Entry { id, target, session, opener: opener.filter(|o| self.get(*o).is_some()), url: String::new(), title: String::new(), moved: None });
        self.active = id;
        Some(id)
    }

    /// The window ends. When it was the shown one, the one that opened it is shown again (else the one before it in the list). The first
    /// window is never removed here: when it ends the pane does.
    pub fn remove(&mut self, id: WindowId) -> Option<Removed> {
        if id == FIRST {
            return None;
        }
        let at = self.list.iter().position(|w| w.id == id)?;
        let gone = self.list.remove(at);
        // Windows this one opened now have no opener left to go back to.
        for w in &mut self.list {
            if w.opener == Some(id) {
                w.opener = gone.opener;
            }
        }
        let was_active = self.active == id;
        if was_active {
            let back = gone.opener.filter(|o| self.get(*o).is_some());
            self.active = back.or_else(|| self.list.get(at.saturating_sub(1)).map(|w| w.id)).unwrap_or(FIRST);
        }
        Some(Removed { was_active, now_active: self.active })
    }

    /// Show `id`. False when there is no such window.
    pub fn focus(&mut self, id: WindowId) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        self.active = id;
        true
    }

    pub fn set_location(&mut self, id: WindowId, url: String, title: String) {
        let shown = self.active == id;
        if let Some(w) = self.list.iter_mut().find(|w| w.id == id) {
            if !shown {
                // A window the person is not looking at went to another site: remember where it was when they last saw it.
                let (was, now) = (site_of(&w.url), site_of(&url));
                if was != now && was != NO_SITE {
                    let from = w.moved.take().map(|(from, _)| from).unwrap_or(was);
                    w.moved = if from == now { None } else { Some((from, now)) };
                }
            }
            w.url = url;
            w.title = title;
        }
    }

    /// What the person has not yet been told about `id`: it changed site while it was not shown. Said once.
    pub fn take_moved(&mut self, id: WindowId) -> Option<(String, String)> {
        self.list.iter_mut().find(|w| w.id == id).and_then(|w| w.moved.take())
    }

    /// The `windows` message: every window with its address, which is shown, and which the person may close.
    pub fn message(&self) -> Value {
        let list: Vec<Value> = self
            .list
            .iter()
            .map(|w| json!({ "id": w.id, "url": w.url, "title": w.title, "opener": w.opener, "closable": w.id != FIRST }))
            .collect();
        json!({ "t": "windows", "active": self.active, "windows": list })
    }
}

/// What a line says for an address that has no host.
const NO_SITE: &str = "a page";

/// The site an address belongs to, for a line the person reads: the host, or "a page" when there is none.
pub fn site_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).filter(|h| !h.is_empty()).unwrap_or_else(|| NO_SITE.to_string())
}

/// A character a person cannot see or that changes how the text around it reads: controls, the marks that reorder text
/// (bidi), and the ones that have no width (zero-width, joiners, the byte-order mark, soft hyphen, tag characters).
fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(c as u32, 0x061C | 0x00AD | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF | 0xE0000..=0xE007F)
}

/// Words a page wrote, made safe to show inside a line of ours: one line, nothing hidden, at most `max` characters.
fn shorten(text: &str, max: usize) -> String {
    // Line breaks and tabs become one space first, then every hidden character goes.
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|c| !is_hidden(*c)).collect();
    let one_line = one_line.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    format!("{}…", one_line.chars().take(max).collect::<String>())
}

/// The line for a page that opened more windows than a pane shows.
pub fn too_many_windows(opener_url: &str) -> String {
    format!("{} tried to open another window. A pane shows at most {MAX_WINDOWS} windows, so it was closed.", site_of(opener_url))
}

/// The line for a message box a page raised (alert, confirm, prompt, leaving the page), which the pane dismisses.
pub fn dialog_dismissed(page_url: &str, kind: &str, message: &str) -> String {
    let what = match kind {
        "confirm" => "a confirmation question",
        "prompt" => "a question",
        "beforeunload" => "a question about leaving the page",
        _ => "a message box",
    };
    let said = shorten(message, 100);
    if said.is_empty() {
        format!("{} opened {what}. It was dismissed.", site_of(page_url))
    } else {
        format!("{} opened {what} (“{said}”). It was dismissed.", site_of(page_url))
    }
}

/// The line for a window the pane could not show because the page opened it before the browser would let the pane see it.
pub fn window_not_shown(opener_url: &str) -> String {
    format!("{} opened a window that could not be shown, so it was closed.", site_of(opener_url))
}

/// The line for a window that went to another site while the person was not looking at it, said when they come back to it.
pub fn window_changed_site(from: &str, to: &str) -> String {
    format!("This window went from {from} to {to} while it was not shown.")
}

/// Google's page that says it will not sign in this browser. Matched on its address (host and path), never on what the page says.
const GOOGLE_REFUSAL: (&str, &str) = ("accounts.google.com", "/v3/signin/rejected");

/// A line for an address the pane knows a provider answers with a refusal to this browser, else none.
pub fn provider_refusal(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if u.scheme() == "https" && u.host_str() == Some(GOOGLE_REFUSAL.0) && u.path() == GOOGLE_REFUSAL.1 {
        return Some("Google refused to sign in with this browser. Other sign-in methods on the site, or a Microsoft or GitHub account, may work.".to_string());
    }
    None
}

/// Notices come at most this often: a page that raises a hundred dialogs is one line, not a hundred.
const NOTICE_EVERY: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub struct NoticeLimiter {
    last: Option<Instant>,
}

impl NoticeLimiter {
    /// May a notice be shown at `now`?
    pub fn allow(&mut self, now: Instant) -> bool {
        if self.last.is_some_and(|t| now.saturating_duration_since(t) < NOTICE_EVERY) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> Windows {
        Windows::starting("t0".into(), "s0".into())
    }

    fn open(w: &mut Windows, n: u32, opener: Option<WindowId>) -> WindowId {
        w.add(format!("t{n}"), format!("s{n}"), opener).expect("room")
    }

    #[test]
    fn a_pane_starts_with_one_window_which_is_shown_and_cannot_be_closed() {
        let w = pane();
        assert_eq!((w.len(), w.active()), (1, FIRST));
        let m = w.message();
        assert_eq!(m["t"], "windows");
        assert_eq!(m["windows"][0]["closable"], false);
        let mut w = pane();
        assert_eq!(w.remove(FIRST), None);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn a_window_a_page_opens_is_shown_and_knows_who_opened_it() {
        let mut w = pane();
        let popup = open(&mut w, 1, Some(FIRST));
        assert_eq!(w.active(), popup);
        assert_eq!(w.get(popup).unwrap().opener, Some(FIRST));
        assert_eq!(w.message()["windows"][1]["closable"], true);
        assert_eq!(w.message()["active"], popup);
    }

    #[test]
    fn a_page_cannot_open_windows_without_limit() {
        let mut w = pane();
        for n in 1..MAX_WINDOWS as u32 {
            assert!(w.has_room());
            open(&mut w, n, Some(FIRST));
        }
        assert_eq!(w.len(), MAX_WINDOWS);
        assert!(!w.has_room());
        assert_eq!(w.add("tx".into(), "sx".into(), Some(FIRST)), None);
        assert_eq!(w.len(), MAX_WINDOWS);
    }

    #[test]
    fn when_the_shown_window_ends_the_one_that_opened_it_is_shown_again() {
        let mut w = pane();
        let a = open(&mut w, 1, Some(FIRST));
        let b = open(&mut w, 2, Some(a));
        assert_eq!(w.remove(b), Some(Removed { was_active: true, now_active: a }));
        assert_eq!(w.remove(a), Some(Removed { was_active: true, now_active: FIRST }));
        assert_eq!(w.active(), FIRST);
    }

    #[test]
    fn a_window_that_ends_while_not_shown_changes_nothing_on_screen() {
        let mut w = pane();
        let a = open(&mut w, 1, Some(FIRST));
        let b = open(&mut w, 2, Some(FIRST));
        assert_eq!(w.active(), b);
        assert_eq!(w.remove(a), Some(Removed { was_active: false, now_active: b }));
        assert_eq!(w.remove(a), None, "once");
    }

    #[test]
    fn windows_a_closed_window_opened_go_back_to_its_opener() {
        let mut w = pane();
        let a = open(&mut w, 1, Some(FIRST));
        let b = open(&mut w, 2, Some(a));
        w.remove(a);
        assert_eq!(w.get(b).unwrap().opener, Some(FIRST));
        assert_eq!(w.remove(b), Some(Removed { was_active: true, now_active: FIRST }));
    }

    #[test]
    fn the_person_can_show_any_window_and_a_window_that_is_not_there_is_refused() {
        let mut w = pane();
        let a = open(&mut w, 1, Some(FIRST));
        assert!(w.focus(FIRST));
        assert_eq!(w.active(), FIRST);
        assert!(w.focus(a));
        assert!(!w.focus(99));
        assert_eq!(w.active(), a);
    }

    #[test]
    fn windows_are_found_by_the_browsers_ids_and_a_closed_ids_number_is_not_reused() {
        let mut w = pane();
        let a = open(&mut w, 1, Some(FIRST));
        assert_eq!(w.by_target("t1"), Some(a));
        assert_eq!(w.by_session("s1"), Some(a));
        assert_eq!(w.by_target("nope"), None);
        w.remove(a);
        let b = open(&mut w, 2, Some(FIRST));
        assert_ne!(a, b, "a late message about the old window cannot land on the new one");
    }

    #[test]
    fn an_address_is_kept_per_window_and_shown_in_the_message() {
        let mut w = pane();
        let a = open(&mut w, 1, Some(FIRST));
        w.set_location(a, "https://accounts.example.com/login".into(), "Sign in".into());
        let m = w.message();
        assert_eq!(m["windows"][1]["url"], "https://accounts.example.com/login");
        assert_eq!(m["windows"][1]["title"], "Sign in");
        assert_eq!(m["windows"][0]["url"], "");
    }

    #[test]
    fn a_line_names_the_site_and_says_what_was_done() {
        assert_eq!(site_of("https://login.example.com/a?b=1"), "login.example.com");
        assert_eq!(site_of("about:blank"), "a page");
        assert_eq!(site_of(""), "a page");
        let line = too_many_windows("https://spam.example/x");
        assert!(line.starts_with("spam.example tried to open another window"), "{line}");
        assert!(line.contains(&MAX_WINDOWS.to_string()));
        let d = dialog_dismissed("https://a.example/", "confirm", "Leave   the\npage?");
        assert_eq!(d, "a.example opened a confirmation question (“Leave the page?”). It was dismissed.");
        assert_eq!(dialog_dismissed("https://a.example/", "alert", ""), "a.example opened a message box. It was dismissed.");
        assert!(dialog_dismissed("https://a.example/", "alert", &"x".repeat(500)).contains('…'));
        assert!(window_not_shown("https://a.example/").starts_with("a.example opened a window"));
    }

    #[test]
    fn google_s_refusal_is_recognised_by_its_address_and_nothing_else_is() {
        assert!(provider_refusal("https://accounts.google.com/v3/signin/rejected?dsh=x&flowName=GlifWebSignIn").is_some());
        for other in [
            "https://accounts.google.com/v3/signin/identifier",
            "https://accounts.google.com/signin/rejected",
            "http://accounts.google.com/v3/signin/rejected",
            "https://evil.example/v3/signin/rejected",
            "https://accounts.google.com.evil.example/v3/signin/rejected",
            "about:blank",
            "",
        ] {
            assert!(provider_refusal(other).is_none(), "{other}");
        }
        let line = provider_refusal("https://accounts.google.com/v3/signin/rejected").unwrap();
        assert!(line.starts_with("Google refused"), "{line}");
    }

    #[test]
    fn what_a_page_wrote_is_shown_with_nothing_hidden_in_it_and_after_the_site() {
        // Control, bidi (right-to-left override and isolates), zero-width and tag characters are removed; the words stay.
        let nasty = "Sign\u{202E}in\u{200B} to\u{2066} pay\u{FEFF}\u{7}\u{E0041} now\u{061C}\u{00AD}";
        let line = dialog_dismissed("https://a.example/", "alert", nasty);
        assert_eq!(line, "a.example opened a message box (“Signin to pay now”). It was dismissed.");
        for c in line.chars() {
            assert!(!is_hidden(c), "{:?} is still in the line", c);
        }
        // The site comes first, so the person reads who is speaking before what they say.
        assert!(line.starts_with("a.example opened"), "{line}");
        // A message of nothing but hidden characters says nothing, as an empty one does.
        assert_eq!(dialog_dismissed("https://a.example/", "alert", "\u{200B}\u{202E}\n\t"), "a.example opened a message box. It was dismissed.");
        // Text that looks like an address stays text: the line is words, with no markup for a link.
        let link = dialog_dismissed("https://a.example/", "alert", "go to https://evil.example/login now");
        assert!(link.contains("https://evil.example/login") && !link.contains('<') && !link.contains("]("), "{link}");
    }

    #[test]
    fn a_window_that_changed_site_while_not_shown_is_told_once_on_return() {
        let mut w = pane();
        w.set_location(FIRST, "https://app.example/".into(), "App".into());
        let a = open(&mut w, 1, Some(FIRST));
        // The pop-up is shown, so the first window is the one not shown now.
        w.set_location(a, "https://idp.example/auth".into(), "Sign in".into());
        assert_eq!(w.take_moved(a), None, "the shown window changing site is not news");
        w.set_location(FIRST, "https://app.example/other".into(), "App".into());
        assert_eq!(w.take_moved(FIRST), None, "same site, nothing to say");
        w.set_location(FIRST, "https://evil.example/".into(), "x".into());
        w.set_location(FIRST, "https://evil.example/again".into(), "x".into());
        assert_eq!(w.take_moved(FIRST), Some(("app.example".to_string(), "evil.example".to_string())));
        assert_eq!(w.take_moved(FIRST), None, "said once");
        // Back to the site the person last saw: nothing to say.
        w.set_location(FIRST, "https://other.example/".into(), "x".into());
        w.set_location(FIRST, "https://evil.example/".into(), "x".into());
        w.take_moved(FIRST);
        w.set_location(FIRST, "https://a2.example/".into(), "x".into());
        w.set_location(FIRST, "https://evil.example/".into(), "x".into());
        assert_eq!(w.take_moved(FIRST), None);
        assert_eq!(window_changed_site("a.example", "b.example"), "This window went from a.example to b.example while it was not shown.");
    }

    #[test]
    fn keys_are_held_for_half_a_second() {
        assert_eq!(KEY_GAP, Duration::from_millis(500));
    }

    #[test]
    fn notices_are_not_a_flood() {
        let mut l = NoticeLimiter::default();
        let t = Instant::now();
        assert!(l.allow(t));
        assert!(!l.allow(t + Duration::from_millis(500)));
        assert!(!l.allow(t + Duration::from_millis(2900)));
        assert!(l.allow(t + Duration::from_secs(3)));
    }
}
