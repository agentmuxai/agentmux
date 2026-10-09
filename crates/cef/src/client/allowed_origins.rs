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
            .lists
            .lock()
            .get(pane)
            .is_some_and(|list| !agentmux_common::allowed_origins::allows(list, url))
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
