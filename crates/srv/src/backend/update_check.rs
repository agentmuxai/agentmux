// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether a newer AgentMux release exists
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §3).
//!
//! srv asks GitHub Releases for the latest release a minute after it starts,
//! then every 6 hours, while the `app:updatecheck` setting is on (the
//! default). Never during startup and never blocking anything: offline, a
//! rate limit or any error just means "no news" until the next check
//! (`SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md` §5).
//!
//! The answer is kept here, sent to every window as the `app:updatestatus`
//! event when it changes, and read on demand with `app.update-status`.
//! Nothing is downloaded or installed: the status bar offers the release
//! page. A local or dev build (`local-…`, `dev-…` channel) is not a
//! release, so it is never "behind"; it only reports which release is latest.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Settings key: check for new releases. On unless set to `false`.
pub const SETTING_UPDATE_CHECK: &str = "app:updatecheck";
/// The event every window gets when the status changes.
pub const EVENT_APP_UPDATE_STATUS: &str = "app:updatestatus";

/// GitHub's latest-release endpoint for the public repo. Unauthenticated:
/// 60 requests an hour per address, against one request every 6 hours.
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/agentmuxai/agentmux/releases/latest";
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Release notes kept for the panel, in bytes.
const MAX_NOTES_BYTES: usize = 16 * 1024;

/// What srv knows about releases, as every window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AppUpdateStatus {
    /// `up-to-date`, `available` (a newer release exists), `local-build`
    /// (this build isn't a release; `latest` says which release is),
    /// `off` (the check is turned off) or `unknown` (not checked yet, or the
    /// last check failed).
    pub state: String,
    /// This build's version.
    pub current: String,
    /// The latest release's version, without the `v`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub latest: Option<String>,
    /// The latest release's page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url: Option<String>,
    /// Its notes (the release body), at most 16 KB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub published_at: Option<String>,
    /// When the last check answered (unix ms), 0 before any.
    #[ts(type = "number")]
    pub checked_ms: i64,
}

impl AppUpdateStatus {
    fn initial(current: &str, enabled: bool) -> Self {
        Self {
            state: if enabled { "unknown" } else { "off" }.to_string(),
            current: current.to_string(),
            latest: None,
            url: None,
            notes: None,
            published_at: None,
            checked_ms: 0,
        }
    }
}

/// The part of GitHub's release JSON this reads.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// `major.minor.patch` out of a version or tag (`v0.59.27`, `0.59.27+g1a2b`),
/// or `None` when it doesn't start with three integers.
fn version_core(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['+', '-']).next().unwrap_or(v);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `channel` is a local or dev build rather than a release.
fn is_local_build(channel: &str) -> bool {
    channel.starts_with("local-") || channel.starts_with("dev-") || channel == "dev"
}

/// The status for a release GitHub reported, seen from `current` on `channel`.
pub(crate) fn status_for(current: &str, channel: &str, release: &Release, now_ms: i64) -> AppUpdateStatus {
    let latest = release.tag_name.trim().trim_start_matches('v').to_string();
    let state = if release.draft || release.prerelease {
        // `releases/latest` never returns these; treat one as no news.
        "up-to-date"
    } else if is_local_build(channel) {
        "local-build"
    } else {
        match (version_core(&latest), version_core(current)) {
            (Some(l), Some(c)) if l > c => "available",
            (Some(_), Some(_)) => "up-to-date",
            _ => "unknown",
        }
    };
    let notes = release.body.as_deref().map(|b| {
        let mut end = b.len().min(MAX_NOTES_BYTES);
        while !b.is_char_boundary(end) {
            end -= 1;
        }
        b[..end].to_string()
    });
    AppUpdateStatus {
        state: state.to_string(),
        current: current.to_string(),
        latest: Some(latest),
        url: Some(release.html_url.clone()),
        notes,
        published_at: release.published_at.clone(),
        checked_ms: now_ms,
    }
}

static STATUS: OnceLock<RwLock<AppUpdateStatus>> = OnceLock::new();

/// What srv knows now (`app.update-status`).
pub fn current_status() -> AppUpdateStatus {
    STATUS
        .get()
        .map(|s| s.read().clone())
        .unwrap_or_else(|| AppUpdateStatus::initial(env!("CARGO_PKG_VERSION"), true))
}

/// Whether the user lets AgentMux check: on unless set to `false`.
fn check_enabled(config: &crate::backend::wconfig::ConfigState) -> bool {
    config.get_settings().extra.get(SETTING_UPDATE_CHECK).and_then(|v| v.as_bool()).unwrap_or(true)
}

async fn fetch_latest(client: &reqwest::Client) -> Result<Release, String> {
    let resp = client
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub answered {}", resp.status()));
    }
    resp.json::<Release>().await.map_err(|e| e.to_string())
}

/// Start the periodic check (`bootstrap::background`).
pub fn spawn(config: Arc<crate::backend::wconfig::ConfigState>, event_bus: Arc<crate::backend::eventbus::EventBus>) {
    let current = env!("CARGO_PKG_VERSION");
    let _ = STATUS.set(RwLock::new(AppUpdateStatus::initial(current, check_enabled(&config))));
    let client = match reqwest::Client::builder()
        .user_agent(format!("AgentMux/{current}"))
        .timeout(REQUEST_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "update check: no HTTP client, not checking");
            return;
        }
    };
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            let next = if check_enabled(&config) {
                let channel = crate::backend::reactive::registry::local_channel_id();
                match fetch_latest(&client).await {
                    Ok(release) => Some(status_for(current, &channel, &release, agentmux_common::time::now_ms())),
                    Err(e) => {
                        tracing::info!(error = %e, "update check: no answer, trying again later");
                        None
                    }
                }
            } else {
                Some(AppUpdateStatus::initial(current, false))
            };
            if let Some(next) = next {
                publish(&event_bus, next);
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Keep `next` and tell every window when it changed.
fn publish(event_bus: &crate::backend::eventbus::EventBus, next: AppUpdateStatus) {
    let Some(lock) = STATUS.get() else { return };
    let changed = {
        let mut cur = lock.write();
        // The check time alone changing is not news to a window.
        let same = AppUpdateStatus { checked_ms: cur.checked_ms, ..next.clone() } == *cur;
        *cur = next.clone();
        !same
    };
    if changed {
        tracing::info!(state = %next.state, latest = ?next.latest, "update check: status changed");
        event_bus.broadcast_event(&crate::backend::eventbus::EventBus::typed_ws_event::<
            crate::backend::mps_events::AppUpdateStatusChanged,
        >(&next, String::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("https://github.com/agentmuxai/agentmux/releases/tag/{tag}"),
            body: Some("- Sysinfo opens on a new Overview".into()),
            published_at: Some("2026-10-15T10:28:48Z".into()),
            draft: false,
            prerelease: false,
        }
    }

    #[test]
    fn a_newer_release_is_available_on_a_release_build() {
        let s = status_for("0.59.26", "stable", &release("v0.59.27"), 5);
        assert_eq!(s.state, "available");
        assert_eq!(s.latest.as_deref(), Some("0.59.27"));
        assert_eq!(s.current, "0.59.26");
        assert!(s.url.unwrap().ends_with("/v0.59.27"));
        assert_eq!(s.checked_ms, 5);
        assert_eq!(s.notes.as_deref(), Some("- Sysinfo opens on a new Overview"));
    }

    #[test]
    fn the_same_or_an_older_release_is_up_to_date() {
        assert_eq!(status_for("0.59.27", "stable", &release("v0.59.27"), 0).state, "up-to-date");
        assert_eq!(status_for("0.60.0", "stable", &release("v0.59.27"), 0).state, "up-to-date");
        // Numbers, not text: 0.59.100 is newer than 0.59.27.
        assert_eq!(status_for("0.59.27", "stable", &release("v0.59.100"), 0).state, "available");
        assert_eq!(status_for("0.59.100", "stable", &release("v0.59.27"), 0).state, "up-to-date");
    }

    #[test]
    fn a_local_or_dev_build_only_reports_the_latest_release() {
        for channel in ["local-main-b28b7a-f62415cd", "dev-qtest-x-38b2af", "dev"] {
            let s = status_for("0.59.26", channel, &release("v0.59.27"), 0);
            assert_eq!(s.state, "local-build", "{channel}");
            assert_eq!(s.latest.as_deref(), Some("0.59.27"));
        }
    }

    #[test]
    fn versions_that_dont_parse_are_never_called_newer() {
        assert_eq!(status_for("0.59.26", "stable", &release("nightly"), 0).state, "unknown");
        assert_eq!(status_for("weird", "stable", &release("v0.59.27"), 0).state, "unknown");
        let mut pre = release("v0.60.0-rc1");
        pre.prerelease = true;
        assert_eq!(status_for("0.59.26", "stable", &pre, 0).state, "up-to-date");
        assert_eq!(version_core("v0.59.27+g1a2b"), Some((0, 59, 27)));
        assert_eq!(version_core("0.59"), None);
        assert_eq!(version_core("0.59.27.1"), None);
    }

    #[test]
    fn long_notes_are_cut_on_a_character_boundary() {
        let mut r = release("v0.59.27");
        r.body = Some("é".repeat(MAX_NOTES_BYTES));
        let notes = status_for("0.59.26", "stable", &r, 0).notes.unwrap();
        assert!(notes.len() <= MAX_NOTES_BYTES);
        assert!(notes.chars().all(|c| c == 'é'));
    }

    #[test]
    fn the_status_reads_as_the_ui_expects() {
        let s = status_for("0.59.26", "stable", &release("v0.59.27"), 7);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "available");
        assert_eq!(v["checked_ms"], 7);
        let off = serde_json::to_value(AppUpdateStatus::initial("0.59.26", false)).unwrap();
        assert_eq!(off, serde_json::json!({ "state": "off", "current": "0.59.26", "checked_ms": 0 }));
    }
}
