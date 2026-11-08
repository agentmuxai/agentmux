// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the host refuses in a browser pane an agent drives, told to the agent
//! (docs/reports/REPORT_AGENTS_AND_OS_PROMPTS_2026_11_07.md §3, tier 1): a page's link to a program on the
//! computer (`mailto:`, `zoommtg:`), and a site whose certificate is not trusted. Both were already
//! refused; the agent only saw a page that did nothing or an error page. Now its next `Browser` call says
//! what was refused and what to do. Only the scheme or the host goes up, never a path or a query, which
//! can carry a token. The person's own panes are not touched.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cef::*;

/// A page that keeps trying (a redirect loop to `mailto:`) is told once, then again after it has been quiet this long.
const EVERY: Duration = Duration::from_secs(5);

static LAST: Mutex<Option<HashMap<(String, &'static str), Instant>>> = Mutex::new(None);

/// What of `url` is safe to tell: for a link to a program its scheme, for a certificate error its host.
fn shown(kind: &str, url: &str) -> String {
    let u = url.trim();
    if kind == "external_protocol" {
        return u.split(':').next().unwrap_or("").to_ascii_lowercase();
    }
    let rest = u.split("://").nth(1).unwrap_or(u);
    rest.split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("").to_string()
}

/// Whether `kind` for `target` may be told now; records that it was.
fn due(target: &str, kind: &'static str, now: Instant) -> bool {
    let mut guard = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, at| now.saturating_duration_since(*at) < EVERY);
    map.insert((target.to_string(), kind), now).is_none()
}

/// Tell srv that `kind` (`external_protocol` or `certificate_error`) was refused for `url` in the pane
/// `browser` belongs to, if an agent drives it.
pub(super) fn tell(state: &Arc<crate::state::AppState>, browser: &Browser, kind: &'static str, url: &str) {
    let Some((_pane, target)) = super::jsdialog::driven(state, browser) else { return };
    if !due(&target, kind, Instant::now()) {
        return;
    }
    tracing::info!(target = %target, kind, "browser pane: refused something in an agent-driven pane and told the agent");
    super::jsdialog::notify(state, serde_json::json!({ "target": target, "notice": kind, "message": shown(kind, url) }));
}

/// What to run when a sign-in prompt for `host` in the pane `block_id` times out unanswered: if an agent drives the
/// pane, it is told, since it would otherwise see only a page that failed to load. Only the host goes up.
pub(super) fn on_auth_expired(state: &Arc<crate::state::AppState>, block_id: &str, host: &str) -> Box<dyn FnOnce() + Send> {
    let (state, block_id, host) = (state.clone(), block_id.to_string(), host.to_string());
    Box::new(move || {
        if super::jsdialog::driven_pane(&state, &block_id) {
            tracing::info!(target = %block_id, "browser pane: a sign-in prompt timed out in an agent-driven pane and the agent was told");
            super::jsdialog::notify(&state, serde_json::json!({ "target": block_id, "notice": "auth_expired", "message": host }));
        }
    })
}

/// [`tell`] for a navigation the handler cancels: returns 1, `on_before_browse`'s "cancel".
pub(super) fn refused(
    state: &Arc<crate::state::AppState>,
    browser: Option<&Browser>,
    kind: &'static str,
    url: &str,
) -> ::std::os::raw::c_int {
    if let Some(b) = browser {
        tell(state, b, kind, url);
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_scheme_or_the_host_is_told() {
        assert_eq!(shown("external_protocol", "MAILTO:someone@example.com?subject=secret"), "mailto");
        assert_eq!(shown("external_protocol", "zoommtg://zoom.us/join?confno=123&pwd=abc"), "zoommtg");
        assert_eq!(shown("certificate_error", "https://user:pw@expired.example:8443/path?token=abc#x"), "expired.example:8443");
        assert_eq!(shown("certificate_error", "https://self-signed.example/"), "self-signed.example");
    }

    #[test]
    fn a_page_that_keeps_trying_is_told_once_in_a_while() {
        let t0 = Instant::now();
        assert!(due("pane-notice-test", "external_protocol", t0));
        assert!(!due("pane-notice-test", "external_protocol", t0 + Duration::from_secs(1)), "again, too soon");
        assert!(due("pane-notice-test", "certificate_error", t0 + Duration::from_secs(1)), "another kind is its own");
        assert!(due("pane-notice-test-2", "external_protocol", t0 + Duration::from_secs(1)), "another pane is its own");
        assert!(due("pane-notice-test", "external_protocol", t0 + EVERY + Duration::from_secs(1)), "after the wait");
    }
}
