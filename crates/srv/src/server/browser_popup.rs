// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! New windows a browser pane's page opens
//! (docs/specs/SPEC_BROWSER_PANE_POPUPS_ADOPTED_2026_10_08.md,
//! docs/specs/SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md).
//!
//! Two kinds, by what the page asked for:
//! - **popup panes** (a link or window asking for a tab or window): srv opens
//!   a browser pane beside the opener when the host asks
//!   (`/api/v1/host/browser_popup`); remembered here per opener.
//! - **popup windows** (`window.open` with popup features): the host creates
//!   a native CEF popup itself and reports it (`/api/v1/host/browser_popup_window`);
//!   srv keeps who opened it and which agent, if any, may drive it.
//!
//! Which requests are honored is `agentmux_common::popup_rules`, shared with
//! the host. Everything here is in memory, like ownership: after srv
//! restarts, popups are ordinary panes and windows nobody drives.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub(crate) use agentmux_common::popup_rules::{decide, origin_of, MAX_POPUPS_PER_PANE};

/// Block meta on a popup pane: the pane that opened it. Written only by srv.
pub(crate) const POPUP_OF_META_KEY: &str = "browser:popup_of";
/// Block meta on a popup pane: the opener's origin, shown as "Popup from …".
/// Written only by srv.
pub(crate) const POPUP_FROM_META_KEY: &str = "browser:popup_from";
/// Block meta on an opener: its open popup windows, `[{id, url}]`, for the
/// strip with Show and Close. Written only by srv.
pub(crate) const POPUP_WINDOWS_META_KEY: &str = "browser:popup_windows";

/// A popup window's id is the host's label for it.
pub(crate) fn is_window_id(id: &str) -> bool {
    id.starts_with("popup-")
}

// ---- popup panes ----

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

/// Admit one more popup pane for `opener` if `admit` agrees, given how many
/// it has open or being opened, and hold its slot. The count and the slot are
/// taken under one lock.
pub(crate) fn reserve<E>(
    opener: &str,
    exists: impl Fn(&str) -> bool,
    admit: impl FnOnce(usize) -> Result<(), E>,
) -> Result<Reservation, E> {
    let mut p = lock();
    let taken = prune(&mut p, opener, exists).len() + p.pending.get(opener).copied().unwrap_or(0);
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

// ---- popup windows ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PopupWindow {
    /// The browser pane whose page opened it.
    pub opener: String,
    /// Its address, as last reported.
    pub url: String,
    /// The agent that owned the opener when it opened, if any. Driving it
    /// also needs that agent to still own the opener (`ui_handlers`).
    pub owner: Option<String>,
    seq: u64,
}

fn windows() -> &'static Mutex<(u64, HashMap<String, PopupWindow>)> {
    static WINDOWS: OnceLock<Mutex<(u64, HashMap<String, PopupWindow>)>> = OnceLock::new();
    WINDOWS.get_or_init(Default::default)
}

/// Record a popup window the host created. `owner` is the opener's owner
/// from srv's own record, never anything the host or the page said.
pub(crate) fn window_opened(id: &str, opener: &str, url: &str, owner: Option<String>) {
    let mut g = windows().lock().unwrap_or_else(|p| p.into_inner());
    g.0 += 1;
    let seq = g.0;
    g.1.insert(
        id.to_string(),
        PopupWindow { opener: opener.to_string(), url: url.to_string(), owner, seq },
    );
}

/// The popup window navigated. Returns its opener, if it is known.
pub(crate) fn window_navigated(id: &str, url: &str) -> Option<String> {
    let mut g = windows().lock().unwrap_or_else(|p| p.into_inner());
    let w = g.1.get_mut(id)?;
    w.url = url.to_string();
    Some(w.opener.clone())
}

/// The popup window closed. Returns its opener, if it was known.
pub(crate) fn window_closed(id: &str) -> Option<String> {
    windows()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .1
        .remove(id)
        .map(|w| w.opener)
}

/// Forget the popup windows whose opener `ours` recognises (the host
/// registered anew; see `ui_handlers::reset_popup_windows`).
pub(crate) fn clear_windows(ours: impl Fn(&str) -> bool) {
    windows().lock().unwrap_or_else(|p| p.into_inner()).1.retain(|_, w| !ours(&w.opener));
}

pub(crate) fn window(id: &str) -> Option<PopupWindow> {
    windows().lock().unwrap_or_else(|p| p.into_inner()).1.get(id).cloned()
}

/// `opener`'s open popup windows, oldest first.
pub(crate) fn windows_of(opener: &str) -> Vec<(String, PopupWindow)> {
    let g = windows().lock().unwrap_or_else(|p| p.into_inner());
    let mut out: Vec<_> = g
        .1
        .iter()
        .filter(|(_, w)| w.opener == opener)
        .map(|(id, w)| (id.clone(), w.clone()))
        .collect();
    out.sort_by_key(|(_, w)| w.seq);
    out
}

/// The opener's popup-windows strip, as written to its meta.
pub(crate) fn strip_value(opener: &str) -> serde_json::Value {
    serde_json::Value::Array(
        windows_of(opener)
            .into_iter()
            .map(|(id, w)| serde_json::json!({ "id": id, "url": w.url }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_slot_counts_against_the_cap_until_it_opens_or_is_dropped() {
        let opener = "test-reserve-opener";
        let cap = |taken: usize| if taken < 2 { Ok(()) } else { Err(taken) };
        // Two admitted at once: both slots count, so a third doesn't fit,
        // although nothing is open yet.
        let a = reserve(opener, |_| true, cap).unwrap();
        let b = reserve(opener, |_| true, cap).unwrap();
        assert_eq!(reserve(opener, |_| true, cap).err(), Some(2));
        // One opens, the other fails: the failed one's slot comes back.
        a.commit("test-reserve-p1");
        drop(b);
        assert_eq!(open_popups(opener, |_| true), vec!["test-reserve-p1"]);
        let c = reserve(opener, |_| true, cap).unwrap();
        assert_eq!(reserve(opener, |_| true, cap).err(), Some(2));
        drop(c);
        // A closed popup frees its slot too.
        assert!(reserve(opener, |id| id != "test-reserve-p1", cap).is_ok());
        assert!(open_popups(opener, |_| true).is_empty());
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

    #[test]
    fn a_window_is_recorded_listed_in_order_followed_and_forgotten() {
        let opener = "test-window-opener";
        window_opened("popup-test-a", opener, "https://signin.example.com/a", Some("lark".into()));
        window_opened("popup-test-b", opener, "https://signin.example.com/b", None);
        window_opened("popup-test-other", "test-window-other-opener", "https://x.example.com/", None);
        let ids: Vec<_> = windows_of(opener).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["popup-test-a", "popup-test-b"]);
        assert_eq!(window("popup-test-a").unwrap().owner.as_deref(), Some("lark"));

        assert_eq!(window_navigated("popup-test-a", "https://signin.example.com/done").as_deref(), Some(opener));
        assert_eq!(
            strip_value(opener),
            serde_json::json!([
                { "id": "popup-test-a", "url": "https://signin.example.com/done" },
                { "id": "popup-test-b", "url": "https://signin.example.com/b" },
            ])
        );

        assert_eq!(window_closed("popup-test-a").as_deref(), Some(opener));
        assert_eq!(window_closed("popup-test-a"), None);
        assert_eq!(window_navigated("popup-test-a", "https://x/"), None);
        assert_eq!(windows_of(opener).len(), 1);
        window_closed("popup-test-b");
        window_closed("popup-test-other");
        assert_eq!(strip_value(opener), serde_json::json!([]));
    }

    #[test]
    fn window_ids_are_the_hosts_popup_labels() {
        assert!(is_window_id("popup-3f2c"));
        assert!(!is_window_id("3f2c-block-id"));
    }
}
