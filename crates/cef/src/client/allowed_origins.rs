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
    /// The address the host last stopped in each pane or popup window, so the
    /// ERR_ABORTED that follows can be told from any other abort
    /// (`show_stopped_page`).
    pub stopped: Mutex<std::collections::HashMap<String, String>>,
    /// When srv last answered a report for each pane with "load it after
    /// all": the loop guard (`note_settled`).
    pub settled: Mutex<std::collections::HashMap<String, Vec<std::time::Instant>>>,
    /// Panes whose reports are muted until the instant: the loop guard tripped.
    pub muted: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// The panes srv's last push confirmed are not limited, to tell a pane newly
    /// confirmed from one a push repeats.
    pub confirmed: Mutex<std::collections::HashSet<String>>,
}

const EARLY_LIST_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// Panes and popup windows `SiteLimits::stopped` remembers at once; past it the
/// notes start over (a stopped page is then not shown for the ones forgotten).
const MAX_STOPPED: usize = 64;

impl SiteLimits {
    /// `pane`'s list, if it has one. An early copy srv never confirmed ends
    /// after `EARLY_LIST_GRACE` here, without waiting for srv's next push: a
    /// pane taken over (or never created) must not stay limited until then.
    pub fn list(&self, pane: &str) -> Option<Vec<String>> {
        // `lists` before `early`, the order `replace_lists` takes them in:
        // the two run on different threads.
        let mut lists = self.lists.lock();
        let mut early = self.early.lock();
        if early.get(pane).is_some_and(|at| at.elapsed() >= EARLY_LIST_GRACE) {
            early.remove(pane);
            lists.remove(pane);
            return None;
        }
        lists.get(pane).cloned()
    }
}

/// Replace the lists with srv's (`allowed`), keeping each early copy srv's
/// push doesn't cover yet.
#[cfg(test)]
pub(crate) fn replace_lists(limits: &SiteLimits, allowed: std::collections::HashMap<String, Vec<String>>) {
    replace_lists_confirmed(limits, allowed, &[]);
}

/// `replace_lists`, with the panes srv has confirmed are not limited: their
/// early copy (a list the block still carried, from before srv lost its own
/// record, say at a restart) goes at once instead of lasting out
/// `EARLY_LIST_GRACE`, which stopped every navigation of such a pane again and
/// again (srv answering each report with "load it").
pub(crate) fn replace_lists_confirmed(limits: &SiteLimits, allowed: std::collections::HashMap<String, Vec<String>>, unlimited: &[String]) {
    // `lists` before `early`, as everywhere both are held.
    let mut lists = limits.lists.lock();
    let mut early = limits.early.lock();
    let mut next = allowed;
    // The loop guard belongs to one stale state: when srv's answer for a pane
    // changes (a list that differs from the host's, or the pane newly confirmed
    // unlimited), what the guard counted no longer applies and the person must be
    // asked again. A push that repeats what the host holds changes nothing: srv
    // pushes before every answer, and that must not reset the count.
    {
        let mut settled = limits.settled.lock();
        let mut muted = limits.muted.lock();
        let mut confirmed = limits.confirmed.lock();
        let changed = next
            .iter()
            .filter(|(pane, list)| lists.get(*pane) != Some(*list))
            .map(|(pane, _)| pane)
            .chain(unlimited.iter().filter(|pane| !confirmed.contains(*pane)));
        for pane in changed {
            settled.remove(pane);
            muted.remove(pane);
        }
        *confirmed = unlimited.iter().cloned().collect();
        let now = std::time::Instant::now();
        muted.retain(|_, until| *until > now);
    }
    early.retain(|pane, at| {
        if next.contains_key(pane) || at.elapsed() >= EARLY_LIST_GRACE || unlimited.contains(pane) {
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
        let mut lists = limits.lists.lock();
        if !list.is_empty() && !lists.contains_key(pane) {
            lists.insert(pane.to_string(), list);
            limits.early.lock().insert(pane.to_string(), std::time::Instant::now());
        }
    }
}

/// The pane `browser` belongs to, and the id srv knows it by: a pane is both;
/// a popup window belongs to its opener and is known by its label.
pub(crate) fn pane_and_target(state: &std::sync::Arc<crate::state::AppState>, browser: &Browser) -> Option<(String, String)> {
    if let Some(block) = crate::browser_pane::callbacks::resolve_pane_block_id(state, browser) {
        return Some((block.clone(), block));
    }
    if state.popup_openers.lock().is_empty() {
        return None;
    }
    let label = state
        .list_browsers()
        .into_iter()
        .find(|(_, b)| {
            let b = b.clone();
            let mut other: cef::Browser = browser.clone();
            b.is_same(Some(&mut other)) != 0
        })
        .map(|(label, _)| label)?;
    let opener = state.popup_openers.lock().get(&label).cloned()?;
    Some((opener, label))
}

impl AgentMuxHandler {
    /// The pane whose site list governs `browser`, and the id srv loads in on
    /// Allow: a pane is both; a popup window is governed by its opener and
    /// loaded by its label.
    pub(super) fn limited_pane_of(&self, browser: &Browser) -> Option<(String, String)> {
        pane_and_target(&self.state, browser)
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
        if self.state.site_limits.lists.lock().is_empty() || is_our_page(url) {
            return false;
        }
        let Some((pane, target)) = self.limited_pane_of(browser) else { return false };
        if !self.off_list(&pane, url) {
            return false;
        }
        tracing::info!(pane = %pane, target = %target, url = %url, "browser pane: stopped a navigation off its allowed sites");
        {
            let mut stopped = self.state.site_limits.stopped.lock();
            if stopped.len() >= MAX_STOPPED {
                stopped.clear();
            }
            stopped.insert(target.clone(), url.to_string());
        }
        report_off_list(&self.state, &pane, &target, url, "navigate");
        true
    }

    /// The abort of a navigation `stop_off_list_navigation` cancelled just
    /// reached `on_load_error`. When the frame has no document to leave in
    /// place (the first load of a popup pane, or of a pane, was the one
    /// stopped), put the stopped page there, so a pane whose sign-in went off
    /// its list never stays grey (#18229). The banner still asks the person.
    pub(super) fn on_aborted(&self, browser: Option<&Browser>, frame: Option<&mut Frame>, is_main_frame: bool, failed_url: &str) {
        if let (true, true, Some(browser), Some(frame)) = (self.is_browser_pane, is_main_frame, browser, frame) {
            self.show_stopped_page(browser, frame, failed_url);
        }
    }

    /// A popup asked for by one of our own pages (the stopped page's link) goes
    /// to the system browser, whatever the pane's list says.
    pub(super) fn popup_from_our_page(&self, browser: &Browser, url: &str) -> bool {
        let ours = super::navigation::on_marked_page(browser);
        if ours {
            super::lifecycle::open_popup_in_system_browser(url);
        }
        ours
    }

    fn show_stopped_page(&self, browser: &Browser, frame: &mut Frame, failed_url: &str) {
        let Some((pane, target)) = self.limited_pane_of(browser) else { return };
        if self.state.site_limits.stopped.lock().remove(&target).as_deref() != Some(failed_url) {
            return;
        }
        let current = crate::browser_pane::callbacks::main_frame_url(browser);
        if !super::stopped_page::has_no_document(&current, failed_url) {
            return;
        }
        let sites = self.state.site_limits.list(&pane).unwrap_or_default();
        let html = super::stopped_page::render_stopped_html(failed_url, &sites);
        tracing::info!(pane = %pane, url = %failed_url, "browser pane: nothing to show where a navigation was stopped, showing the stopped page");
        super::navigation::load_html_page(frame, &html, failed_url);
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

/// Is `url` one of the pages the host loads itself (the load-error page, the
/// stopped page)? A `data:` address carrying the failed-address marker; it has
/// no web origin, so the list would stop it, and the pane would never show why.
/// A page can't navigate its own top frame to a `data:` address, so only the
/// host puts one there.
fn is_our_page(url: &str) -> bool {
    super::navigation::error_page_failed_url(url).is_some()
}

/// How many times srv may answer a pane's reports with "load it" inside
/// `SETTLED_WINDOW` before the host stops reporting it.
const MAX_SETTLED: usize = 3;
const SETTLED_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
/// How long a pane's reports stay muted once the guard trips.
const MUTE_FOR: std::time::Duration = std::time::Duration::from_secs(60);

/// Note that srv just answered a report for `pane` with "load it after all".
/// True when that is the `MAX_SETTLED`th inside `SETTLED_WINDOW`: srv loads the
/// address, the list stops it, and the two go round (a stale list on one side).
/// The list is never dropped here: only srv says a pane isn't limited.
fn note_settled(limits: &SiteLimits, pane: &str, now: std::time::Instant) -> bool {
    let mut settled = limits.settled.lock();
    let times = settled.entry(pane.to_string()).or_default();
    times.retain(|t| now.saturating_duration_since(*t) < SETTLED_WINDOW);
    times.push(now);
    let looping = times.len() >= MAX_SETTLED;
    if looping {
        settled.remove(pane);
        limits.muted.lock().insert(pane.to_string(), now + MUTE_FOR);
    }
    looping
}

/// Is `pane`'s reporting muted at `now`? An expired mute is forgotten.
fn is_muted(limits: &SiteLimits, pane: &str, now: std::time::Instant) -> bool {
    let mut muted = limits.muted.lock();
    match muted.get(pane) {
        Some(until) if *until > now => true,
        Some(_) => {
            muted.remove(pane);
            false
        }
        None => false,
    }
}

/// What the host does once srv has answered a report, or failed to.
#[derive(Debug, PartialEq, Eq)]
enum AfterReport {
    /// srv is asking the person: further attempts wait for its push.
    Asking,
    /// srv loaded it again, or the pane already says why it stopped.
    Settled,
    /// Nothing in the pane says why: the host shows its own notice, so a
    /// stopped navigation never ends in a blank pane with no word
    /// (REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md §6.2).
    Notice,
}

/// Did srv answer with "load it after all"? The only answer the loop guard counts:
/// a question, a busy pane or a notice is srv speaking in the pane, not a replay.
fn was_replayed(answer: &Result<serde_json::Value, String>) -> bool {
    answer.as_ref().is_ok_and(|d| d.get("settled").and_then(|v| v.as_str()) == Some("replayed"))
}

fn after_report(answer: &Result<serde_json::Value, String>) -> AfterReport {
    let Ok(data) = answer else { return AfterReport::Notice };
    if data.get("asked").and_then(|v| v.as_bool()) == Some(true) {
        return AfterReport::Asking;
    }
    match data.get("settled").and_then(|v| v.as_str()) {
        Some("replayed" | "asking" | "busy" | "notice") => AfterReport::Settled,
        _ => AfterReport::Notice,
    }
}

/// The pane's own notice that a navigation was stopped, for when srv
/// couldn't show one: a line in the pane's banner area, from the host alone.
fn local_notice(state: &crate::state::AppState, pane: &str, url: &str, why: &str) {
    let sites = state.site_limits.lists.lock().get(pane).cloned().unwrap_or_default();
    let payload = serde_json::json!({ "block_id": pane, "url": url, "sites": sites, "why": why });
    if !crate::events::emit_browser_pane_event(state, pane, "browser-pane-notice", &payload) {
        tracing::warn!(pane = %pane, "couldn't show the pane why a navigation stopped");
    }
}

/// Tell srv that `target` (a pane, or a popup window by its label) tried to
/// go to `url`, off the site list of `pane`, and was stopped; srv asks the
/// person (SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §4). On its own
/// thread, since the caller is a CEF callback on the UI thread, and one at a
/// time per pane: srv asks one question at a time anyway. When srv can't be
/// told, or doesn't settle it, the pane shows the host's own notice.
fn report_off_list(state: &std::sync::Arc<crate::state::AppState>, pane: &str, target: &str, url: &str, kind: &str) {
    // The guard tripped: the navigation stays stopped (the pane shows why) and
    // srv is not asked again for a minute.
    if is_muted(&state.site_limits, pane, std::time::Instant::now()) {
        return;
    }
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
        let (pane, url) = (pane.clone(), url.to_string());
        let state = state.clone();
        move || {
            // While srv asks, the marker stays: the page's further attempts
            // are cancelled without another report, until a push from srv
            // shows the question is over (browser_api::routes::owned_panes).
            let answer = crate::client::backend_browser_navigation(&web_endpoint, &auth_key, &ipc_token, &body);
            match after_report(&answer) {
                AfterReport::Asking => return,
                AfterReport::Settled if was_replayed(&answer) && note_settled(&state.site_limits, &pane, std::time::Instant::now()) => {
                    // srv keeps saying "load it" and the list keeps stopping it: stop
                    // asking for a while, and show why.
                    tracing::warn!(pane = %pane, url = %url, "srv keeps loading an address the host's list keeps stopping; no longer reporting it");
                    local_notice(&state, &pane, &url, "AgentMux stopped this address again and again. Reload to try it once more.");
                }
                AfterReport::Settled => {}
                AfterReport::Notice => {
                    match &answer {
                        Err(e) => tracing::warn!(pane = %pane, error = %e, "couldn't tell srv about a navigation off the allowed sites"),
                        Ok(data) => tracing::info!(pane = %pane, answer = %data, "srv didn't settle a stopped navigation"),
                    }
                    local_notice(&state, &pane, &url, "AgentMux couldn't ask you about it.");
                }
            }
            state.site_limits.reports.lock().remove(&pane);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start the browser-off-list thread");
        state.site_limits.reports.lock().remove(&pane);
        local_notice(&state, &pane, url, "AgentMux couldn't ask you about it.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosts_own_pages_are_not_stopped_by_the_list_but_other_data_urls_are() {
        let ours = format!("data:text/html;base64,AAAA{}", super::super::navigation::failed_url_fragment("https://admin.microsoft.com/"));
        assert!(is_our_page(&ours));
        assert!(!is_our_page("data:text/html;base64,AAAA"));
        assert!(!is_our_page("https://admin.microsoft.com/"));
        // The list itself still refuses any data: address, ours included.
        assert!(!agentmux_common::allowed_origins::allows(&["https://mail.example.com".to_string()], &ours));
    }

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
    fn a_pane_srv_confirms_is_not_limited_loses_its_early_copy_at_once() {
        let limits = SiteLimits::default();
        limit_before_create(&limits, "stale", Some(&serde_json::json!(["example.com"])));
        limit_before_create(&limits, "fresh", Some(&serde_json::json!(["example.com"])));
        // srv's push doesn't cover either: both early copies stay (a new pane's list
        // may still be on its way), except the one srv has confirmed is not limited.
        replace_lists_confirmed(&limits, Default::default(), &["stale".to_string()]);
        assert!(limits.list("stale").is_none());
        assert!(limits.list("fresh").is_some());
        // A list srv itself sent is never dropped by the confirmation of another pane.
        replace_lists_confirmed(
            &limits,
            std::collections::HashMap::from([("owned".to_string(), vec!["https://o.example".to_string()])]),
            &["stale".to_string()],
        );
        assert!(limits.list("owned").is_some());
    }

    #[test]
    fn the_loop_guard_trips_on_the_third_load_it_inside_the_window_and_mutes_the_pane() {
        use std::time::{Duration, Instant};
        let limits = SiteLimits::default();
        let t0 = Instant::now();
        assert!(!note_settled(&limits, "p", t0));
        assert!(!note_settled(&limits, "p", t0 + Duration::from_millis(14)));
        assert!(note_settled(&limits, "p", t0 + Duration::from_millis(28)));
        // Muted for a while, not for good.
        assert!(is_muted(&limits, "p", t0 + Duration::from_secs(30)));
        assert!(!is_muted(&limits, "p", t0 + MUTE_FOR + Duration::from_secs(1)));
        assert!(!is_muted(&limits, "other", t0));
        // Answers spread over time never trip it: a person allowing sites one by one.
        let limits = SiteLimits::default();
        for i in 0..6 {
            assert!(!note_settled(&limits, "q", t0 + Duration::from_secs(i * 11)));
        }
    }

    #[test]
    fn only_a_replay_counts_for_the_guard() {
        use serde_json::json;
        assert!(was_replayed(&Ok(json!({ "asked": false, "settled": "replayed" }))));
        for settled in ["asking", "busy", "notice", "unshown"] {
            assert!(!was_replayed(&Ok(json!({ "settled": settled }))), "{settled}");
        }
        assert!(!was_replayed(&Err("connection refused".into())));
    }

    #[test]
    fn srv_limiting_or_clearing_a_pane_ends_its_mute() {
        use std::time::{Duration, Instant};
        let limits = SiteLimits::default();
        let t0 = Instant::now();
        for i in 0..3 {
            note_settled(&limits, "a", t0 + Duration::from_millis(i));
            note_settled(&limits, "b", t0 + Duration::from_millis(i));
        }
        assert!(is_muted(&limits, "a", t0 + Duration::from_secs(1)));
        assert!(is_muted(&limits, "b", t0 + Duration::from_secs(1)));
        // srv limits `a` again; `b` is newly confirmed unlimited: both may be asked about again.
        let a_list = std::collections::HashMap::from([("a".to_string(), vec!["https://a.example".to_string()])]);
        replace_lists_confirmed(&limits, a_list.clone(), &["b".to_string()]);
        assert!(!is_muted(&limits, "a", t0 + Duration::from_secs(1)));
        assert!(!is_muted(&limits, "b", t0 + Duration::from_secs(1)));
    }

    #[test]
    fn a_push_that_repeats_what_the_host_holds_does_not_reset_the_guard() {
        use std::time::{Duration, Instant};
        let limits = SiteLimits::default();
        let t0 = Instant::now();
        let a_list = std::collections::HashMap::from([("a".to_string(), vec!["https://a.example".to_string()])]);
        let unlimited = ["b".to_string()];
        replace_lists_confirmed(&limits, a_list.clone(), &unlimited);
        // srv pushes before each answer: the count builds up across the pushes and trips.
        assert!(!note_settled(&limits, "b", t0));
        replace_lists_confirmed(&limits, a_list.clone(), &unlimited);
        assert!(!note_settled(&limits, "b", t0 + Duration::from_millis(14)));
        replace_lists_confirmed(&limits, a_list.clone(), &unlimited);
        assert!(note_settled(&limits, "b", t0 + Duration::from_millis(28)));
        // A later repeat doesn't end the mute either.
        replace_lists_confirmed(&limits, a_list, &unlimited);
        assert!(is_muted(&limits, "b", t0 + Duration::from_secs(30)));
    }

    #[test]
    fn a_report_srv_didnt_settle_gets_the_hosts_own_notice() {
        use serde_json::json;
        // srv unreachable, or not JSON.
        assert_eq!(after_report(&Err("connection refused".into())), AfterReport::Notice);
        assert_eq!(after_report(&Ok(json!({ "asked": true, "settled": "asking" }))), AfterReport::Asking);
        for settled in ["replayed", "busy", "notice"] {
            assert_eq!(after_report(&Ok(json!({ "asked": false, "settled": settled }))), AfterReport::Settled, "{settled}");
        }
        // srv couldn't show its notice, or answered something unknown.
        assert_eq!(after_report(&Ok(json!({ "asked": false, "settled": "unshown" }))), AfterReport::Notice);
        assert_eq!(after_report(&Ok(json!({ "asked": false }))), AfterReport::Notice);
        assert_eq!(after_report(&Ok(serde_json::Value::Null)), AfterReport::Notice);
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
