// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeping an agent's browser pane on the sites it was opened for
//! (SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §6). srv sends each
//! limited pane's list with the owned-pane set (`AppState::allowed_origins`);
//! `on_before_browse` and `on_before_popup` run synchronously on the UI thread
//! and can't wait for the person, so a navigation or popup off the list is
//! cancelled here and reported to srv, which asks in the pane. On Allow srv
//! sends the longer list and loads the address again.

use cef::*;
use parking_lot::Mutex;

use super::AgentMuxHandler;

/// The sites each limited pane may go to, as srv last sent them with the
/// owned-pane set, and the panes with an off-list report on its way to srv:
/// one at a time per pane, so a page retrying in a loop can't start a thread
/// per try.
#[derive(Default)]
pub struct SiteLimits {
    pub lists: Mutex<std::collections::HashMap<String, Vec<String>>>,
    pub reports: Mutex<std::collections::HashSet<String>>,
    /// Panes whose list came with `browser_pane_create`, and when. srv's push
    /// can be taken before it records the pane; such a push mustn't wipe the
    /// early copy, so it is kept until a push covers the pane, or for
    /// `EARLY_LIST_GRACE`.
    pub early: Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

const EARLY_LIST_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

impl SiteLimits {
    /// `pane`'s list, if it has one. An early copy srv never confirmed ends
    /// after `EARLY_LIST_GRACE` here, without waiting for srv's next push: a
    /// pane taken over (or never created) must not stay limited until then.
    pub fn list(&self, pane: &str) -> Option<Vec<String>> {
        let mut early = self.early.lock();
        if early.get(pane).is_some_and(|at| at.elapsed() >= EARLY_LIST_GRACE) {
            early.remove(pane);
            self.lists.lock().remove(pane);
            return None;
        }
        self.lists.lock().get(pane).cloned()
    }
}

/// Replace the lists with srv's (`allowed`), keeping each early copy srv's
/// push doesn't cover yet.
pub(crate) fn replace_lists(limits: &SiteLimits, allowed: std::collections::HashMap<String, Vec<String>>) {
    let mut lists = limits.lists.lock();
    let mut early = limits.early.lock();
    let mut next = allowed;
    early.retain(|pane, at| {
        if next.contains_key(pane) || at.elapsed() >= EARLY_LIST_GRACE {
            return false;
        }
        if let Some(list) = lists.get(pane) {
            next.insert(pane.clone(), list.clone());
        }
        true
    });
    *lists = next;
}

/// A pane's site list as its block carries it (`browser:allowed_origins`,
/// which only srv writes), passed with `browser_pane_create` so the host has
/// it before the pane's first load: srv's own push can come after it. Only
/// fills a gap: a list srv already sent wins, and its next push replaces this.
pub(crate) fn limit_before_create(limits: &SiteLimits, pane: &str, list: Option<&serde_json::Value>) {
    let Some(entries) = list.and_then(|v| v.as_array()) else { return };
    let entries: Vec<String> = entries.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    if let Ok(list) = agentmux_common::allowed_origins::parse_list(&entries) {
        if !list.is_empty() && !limits.lists.lock().contains_key(pane) {
            limits.lists.lock().insert(pane.to_string(), list);
            limits.early.lock().insert(pane.to_string(), std::time::Instant::now());
        }
    }
}

impl AgentMuxHandler {
    /// The pane whose site list governs `browser`, and the id srv loads in on
    /// Allow: a pane is both; a popup window is governed by its opener and
    /// loaded by its label.
    pub(super) fn limited_pane_of(&self, browser: &Browser) -> Option<(String, String)> {
        if let Some(block) = crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, browser) {
            return Some((block.clone(), block));
        }
        if self.state.popup_openers.lock().is_empty() {
            return None;
        }
        let label = self
            .state
            .list_browsers()
            .into_iter()
            .find(|(_, b)| {
                let b = b.clone();
                let mut other: cef::Browser = browser.clone();
                b.is_same(Some(&mut other)) != 0
            })
            .map(|(label, _)| label)?;
        let opener = self.state.popup_openers.lock().get(&label).cloned()?;
        Some((opener, label))
    }

    /// Is `url` off the site list srv sent for `pane`? False when the pane
    /// has no list.
    fn off_list(&self, pane: &str, url: &str) -> bool {
        self.state
            .site_limits
            .list(pane)
            .is_some_and(|list| !agentmux_common::allowed_origins::allows(&list, url))
    }

    /// `on_before_browse`, main frame: true (cancel) when `browser` is a
    /// limited pane, or a popup window of one, and `url` is off its list.
    pub(super) fn stop_off_list_navigation(&self, browser: &Browser, url: &str) -> bool {
        if self.state.site_limits.lists.lock().is_empty() {
            return false;
        }
        let Some((pane, target)) = self.limited_pane_of(browser) else { return false };
        if !self.off_list(&pane, url) {
            return false;
        }
        tracing::info!(pane = %pane, target = %target, url = %url, "browser pane: stopped a navigation off its allowed sites");
        report_off_list(&self.state, &pane, &target, url, "navigate");
        true
    }

    /// `on_before_popup`: true (cancel) when `opener` is limited and `url` is
    /// off its list. The person is asked; on Allow the click is repeated.
    pub(super) fn stop_off_list_popup(&self, opener: &str, url: &str) -> bool {
        if !self.off_list(opener, url) {
            return false;
        }
        report_off_list(&self.state, opener, opener, url, "popup");
        true
    }
}

/// Tell srv that `target` (a pane, or a popup window by its label) tried to
/// go to `url`, off the site list of `pane`, and was stopped; srv asks the
/// person (SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §4). On its own
/// thread, since the caller is a CEF callback on the UI thread, and one at a
/// time per pane: srv asks one question at a time anyway.
fn report_off_list(state: &std::sync::Arc<crate::state::AppState>, pane: &str, target: &str, url: &str, kind: &str) {
    if !state.site_limits.reports.lock().insert(pane.to_string()) {
        return;
    }
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let ipc_token = state.ipc_token.clone();
    let body = serde_json::json!({ "pane": pane, "target": target, "url": url, "kind": kind });
    let state = state.clone();
    let pane = pane.to_string();
    let spawned = std::thread::Builder::new().name("browser-off-list".into()).spawn({
        let pane = pane.clone();
        let state = state.clone();
        move || {
            // While srv asks, the marker stays: the page's further attempts
            // are cancelled without another report, until a push from srv
            // shows the question is over (browser_api::routes::owned_panes).
            match crate::client::backend_browser_navigation(&web_endpoint, &auth_key, &ipc_token, &body) {
                Ok(true) => return,
                Ok(false) => {}
                Err(e) => tracing::warn!(pane = %pane, error = %e, "couldn't tell srv about a navigation off the allowed sites"),
            }
            state.site_limits.reports.lock().remove(&pane);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start the browser-off-list thread");
        state.site_limits.reports.lock().remove(&pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_given_at_creation_fills_a_gap_and_never_replaces_srvs() {
        let limits = SiteLimits::default();
        limit_before_create(&limits, "p1", Some(&serde_json::json!(["example.com"])));
        assert_eq!(limits.lists.lock().get("p1").unwrap(), &vec!["https://example.com".to_string()]);
        // srv's list (here, after an Allow) is never replaced by it.
        limits.lists.lock().insert("p2".into(), vec!["https://a.example".into(), "https://b.example".into()]);
        limit_before_create(&limits, "p2", Some(&serde_json::json!(["a.example"])));
        assert_eq!(limits.lists.lock().get("p2").unwrap().len(), 2);
        // Nothing given, empty, or not a valid list: no entry.
        for v in [None, Some(serde_json::json!([])), Some(serde_json::json!(["ftp://x"])), Some(serde_json::json!("x"))] {
            limit_before_create(&limits, "p3", v.as_ref());
        }
        assert!(!limits.lists.lock().contains_key("p3"));
    }

    #[test]
    fn a_push_that_doesnt_cover_a_new_pane_keeps_its_early_list() {
        let limits = SiteLimits::default();
        limit_before_create(&limits, "new", Some(&serde_json::json!(["example.com"])));
        // srv's push, taken before it recorded the pane.
        replace_lists(&limits, std::collections::HashMap::from([("other".to_string(), vec!["https://o.example".to_string()])]));
        assert_eq!(limits.lists.lock().get("new").unwrap(), &vec!["https://example.com".to_string()]);
        // srv's next push covers it: srv's list wins, and the early copy is done.
        replace_lists(&limits, std::collections::HashMap::from([("new".to_string(), vec!["https://example.com".to_string(), "https://b.example".to_string()])]));
        assert_eq!(limits.lists.lock().get("new").unwrap().len(), 2);
        assert!(limits.early.lock().is_empty());
        // And a push without it after that drops it: the limit ended.
        replace_lists(&limits, std::collections::HashMap::new());
        assert!(!limits.lists.lock().contains_key("new"));
    }

    #[test]
    fn an_early_list_srv_never_confirms_ends_on_its_own() {
        let limits = SiteLimits::default();
        limit_before_create(&limits, "gone", Some(&serde_json::json!(["example.com"])));
        assert!(limits.list("gone").is_some());
        // Its grace ran out with no push from srv covering it.
        *limits.early.lock().get_mut("gone").unwrap() -= EARLY_LIST_GRACE;
        assert!(limits.list("gone").is_none());
        assert!(!limits.lists.lock().contains_key("gone"));
    }
}
