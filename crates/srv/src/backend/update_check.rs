// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether a newer AgentMux release exists
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §3).
//!
//! srv asks GitHub Releases for the latest release only when asked: the
//! user's **Check for update** in the version panel, or an agent's `App`
//! `version` or `upgrade` ([`check_now`]), at most every 30 seconds between
//! them. Nothing checks on its own, at startup or on a timer: AgentMux makes
//! no request the user didn't ask for. Offline, a rate limit or any error
//! keeps what the last check found.
//!
//! The answer is kept here, sent to every window as the `app:updatestatus`
//! event when it changes, and read on demand with `app.update-status`. It
//! names the release file for this install (`update_install`) with the
//! SHA-256 GitHub lists for it, and that check's answer is what the rest
//! installs: `update_download` fetches exactly that file, and
//! `update_restart` verifies it against that checksum. A new check replaces
//! the release on offer, except while a download or a restart into it is
//! under way, which finishes first.
//!
//! Any portable or installed build upgrades to a newer release, a local
//! `task package` build (`local-…` channel) included: it compares versions like
//! a release. Only a `task dev` build (`dev-…` channel), which runs from source,
//! never upgrades; it reports which release is latest.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::backend::update_install::InstallKind;

/// The event every window gets when the status changes.
pub const EVENT_APP_UPDATE_STATUS: &str = "app:updatestatus";

/// GitHub's latest-release endpoint for the public repo. Unauthenticated:
/// 60 requests an hour per address.
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/agentmuxai/agentmux/releases/latest";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// The shortest gap between two checks the user or an agent asks for: GitHub
/// allows 60 unauthenticated requests an hour per address.
const CHECK_NOW_EVERY: Duration = Duration::from_secs(30);
/// Release notes kept for the panel, in bytes.
const MAX_NOTES_BYTES: usize = 16 * 1024;

/// What srv knows about releases, as every window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AppUpdateStatus {
    /// `up-to-date`, `available` (a newer release exists), `local-build`
    /// (this build isn't a release; `latest` says which release is) or
    /// `unknown` (not checked yet).
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
    /// When the last check answered (unix ms), 0 before any. The status is
    /// that check's answer until the next one.
    #[ts(type = "number")]
    pub checked_ms: i64,
    /// How this AgentMux was installed (`update_install`): `windows-portable`,
    /// `windows-setup`, `windows-store`, `macos-app`, `linux-appimage`,
    /// `linux-portable`, `linux-package` or `unknown`.
    #[serde(default)]
    pub install: String,
    /// The latest release's file for this install, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub asset: Option<ReleaseAsset>,
    /// Whether AgentMux can download and stage that file itself; otherwise
    /// the release page (or the Store) is the way.
    #[serde(default)]
    pub can_download: bool,
    /// A download of the latest release, once one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub download: Option<DownloadStatus>,
}

/// A release file, as GitHub lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReleaseAsset {
    pub name: String,
    #[ts(type = "number")]
    pub size: u64,
    pub url: String,
    /// Lowercase hex.
    pub sha256: String,
}

/// Where a download of a release stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DownloadStatus {
    /// The release being downloaded.
    pub version: String,
    /// `downloading`, `unpacking`, `ready` or `failed`; then, after "Upgrade
    /// & restart", `waiting` (for agents to finish their turn) and
    /// `restarting` (the launcher was asked).
    pub state: String,
    #[ts(type = "number")]
    pub received: u64,
    #[ts(type = "number")]
    pub total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// When ready, on a portable: the folder the new version is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub folder: Option<String>,
    /// When ready, on a `setup.exe` install: the new version's installer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub installer: Option<String>,
    /// While `waiting`: how many agents are still in a turn.
    #[serde(default)]
    pub waiting_for: u32,
}

impl AppUpdateStatus {
    fn initial(current: &str) -> Self {
        Self {
            state: "unknown".to_string(),
            current: current.to_string(),
            latest: None,
            url: None,
            notes: None,
            published_at: None,
            checked_ms: 0,
            install: install_kind().label().to_string(),
            asset: None,
            can_download: false,
            download: None,
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
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

/// A file attached to a GitHub release. `digest` is `sha256:<hex>`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GithubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

/// The release file `name`, if the release lists it with a SHA-256 and it is
/// served from AgentMux's own releases.
fn find_asset(release: &Release, name: &str) -> Option<ReleaseAsset> {
    let a = release.assets.iter().find(|a| a.name == name)?;
    let sha256 = a.digest.as_deref()?.strip_prefix("sha256:")?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if !a.browser_download_url.starts_with(crate::backend::update_download::RELEASE_DOWNLOAD_PREFIX) {
        return None;
    }
    Some(ReleaseAsset { name: a.name.clone(), size: a.size, url: a.browser_download_url.clone(), sha256 })
}

/// `major.minor.patch` out of a version or tag (`v0.59.27`, `0.59.27+g1a2b`),
/// or `None` when it doesn't start with three integers.
pub(crate) fn version_core(v: &str) -> Option<(u64, u64, u64)> {
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

/// Whether `channel` is a `task dev` build, which runs from source and so
/// never upgrades itself. A local `task package` build (`local-…`) is a
/// portable like a release's, and upgrades like one.
fn is_dev_build(channel: &str) -> bool {
    channel.starts_with("dev-") || channel == "dev"
}

/// The status for a release GitHub reported, seen from `current` on `channel`
/// for an install of `kind`.
pub(crate) fn status_for(current: &str, channel: &str, release: &Release, now_ms: i64, kind: &InstallKind) -> AppUpdateStatus {
    let latest = release.tag_name.trim().trim_start_matches('v').to_string();
    let state = if release.draft || release.prerelease {
        // `releases/latest` never returns these; treat one as no news.
        "up-to-date"
    } else if is_dev_build(channel) {
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
    let asset = kind.asset_name(&latest, std::env::consts::ARCH).and_then(|n| find_asset(release, &n));
    AppUpdateStatus {
        state: state.to_string(),
        current: current.to_string(),
        can_download: state == "available" && kind.can_download() && asset.is_some(),
        latest: Some(latest),
        url: Some(release.html_url.clone()),
        notes,
        published_at: release.published_at.clone(),
        checked_ms: now_ms,
        install: kind.label().to_string(),
        asset,
        download: None,
    }
}

static STATUS: OnceLock<RwLock<AppUpdateStatus>> = OnceLock::new();
static EVENT_BUS: OnceLock<Arc<crate::backend::eventbus::EventBus>> = OnceLock::new();
static INSTALL_KIND: OnceLock<InstallKind> = OnceLock::new();
/// The client every check uses, made by [`init`].
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
/// When [`check_now`] last asked GitHub, and why it failed if it did; held
/// while it asks, so clicks that arrive meanwhile wait for its answer.
static LAST_CHECK_NOW: tokio::sync::Mutex<Option<(std::time::Instant, Option<String>)>> = tokio::sync::Mutex::const_new(None);

/// How this AgentMux was installed, read once.
pub(crate) fn install_kind() -> InstallKind {
    INSTALL_KIND.get_or_init(crate::backend::update_install::detect).clone()
}

/// Record where a download stands and tell every window (`update_download`).
pub(crate) fn set_download(download: Option<DownloadStatus>) {
    let Some(bus) = EVENT_BUS.get() else { return };
    let mut next = current_status();
    next.download = download;
    publish(bus, next);
}

/// What srv knows now (`app.update-status`).
pub fn current_status() -> AppUpdateStatus {
    STATUS
        .get()
        .map(|s| s.read().clone())
        .unwrap_or_else(|| AppUpdateStatus::initial(env!("CARGO_PKG_VERSION")))
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

/// Get ready to check when asked (`bootstrap::background`). Asks nothing.
pub fn init(event_bus: Arc<crate::backend::eventbus::EventBus>) {
    let current = env!("CARGO_PKG_VERSION");
    let _ = STATUS.set(RwLock::new(AppUpdateStatus::initial(current)));
    let _ = EVENT_BUS.set(event_bus);
    match reqwest::Client::builder()
        .user_agent(format!("AgentMux/{current}"))
        .timeout(REQUEST_TIMEOUT)
        .build()
    {
        Ok(c) => {
            let _ = CLIENT.set(c);
        }
        Err(e) => tracing::warn!(error = %e, "update check: no HTTP client, can't check"),
    }
}

/// Ask GitHub now, for the user's **Check for update** or an agent's `App`
/// `version` or `upgrade`, and return what srv knows after. Asked again within
/// [`CHECK_NOW_EVERY`], it answers what the last check found, its failure
/// included; concurrent callers share one request.
pub async fn check_now() -> Result<AppUpdateStatus, String> {
    let (Some(client), Some(bus)) = (CLIENT.get(), EVENT_BUS.get()) else {
        return Err("AgentMux isn't checking for updates yet; try again in a moment".into());
    };
    let mut last = LAST_CHECK_NOW.lock().await;
    if let Some((_, failed)) = last.as_ref().filter(|(at, _)| at.elapsed() < CHECK_NOW_EVERY) {
        return failed.clone().map_or_else(|| Ok(current_status()), Err);
    }
    let current = env!("CARGO_PKG_VERSION");
    let channel = crate::backend::reactive::registry::local_channel_id();
    let outcome = match fetch_latest(client).await {
        Ok(release) => {
            let next = status_for(current, &channel, &release, agentmux_common::time::now_ms(), &install_kind());
            match pinned_by_upgrade(&current_status(), &next) {
                Some(version) => tracing::info!(
                    latest = ?next.latest,
                    version,
                    "update check: a newer release is out; the upgrade under way finishes first"
                ),
                None => publish(bus, next),
            }
            Ok(current_status())
        }
        Err(e) => {
            tracing::info!(error = %e, "update check: asked now, no answer");
            Err(format!("couldn't reach GitHub: {e}"))
        }
    };
    *last = Some((std::time::Instant::now(), outcome.as_ref().err().cloned()));
    outcome
}

/// The release a download or restart under way is installing, when `next`
/// would replace it with another: that one finishes first, and a check after
/// it offers the newer release.
fn pinned_by_upgrade(cur: &AppUpdateStatus, next: &AppUpdateStatus) -> Option<String> {
    let d = cur.download.as_ref()?;
    let under_way = matches!(d.state.as_str(), "downloading" | "unpacking" | "waiting" | "restarting");
    (under_way && next.latest.as_deref() != Some(d.version.as_str())).then(|| d.version.clone())
}

/// Keep `next` and tell every window when it changed.
fn publish(event_bus: &crate::backend::eventbus::EventBus, mut next: AppUpdateStatus) {
    let Some(lock) = STATUS.get() else { return };
    let changed = {
        let mut cur = lock.write();
        // A new check keeps a download of the release it still reports.
        if next.download.is_none() && next.latest.is_some() && next.latest == cur.latest {
            next.download = cur.download.clone();
        }
        // The check time alone changing is not news to a window.
        let same = AppUpdateStatus { checked_ms: cur.checked_ms, ..next.clone() } == *cur;
        // Download progress is news to a window, not to the log.
        let step = |s: &AppUpdateStatus| (s.state.clone(), s.latest.clone(), s.download.as_ref().map(|d| d.state.clone()));
        if step(&cur) != step(&next) {
            tracing::info!(
                state = %next.state,
                latest = ?next.latest,
                download = ?next.download.as_ref().map(|d| d.state.as_str()),
                "update check: status changed"
            );
        }
        *cur = next.clone();
        !same
    };
    if changed {
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
            assets: Vec::new(),
        }
    }

    #[test]
    fn a_newer_release_is_available_on_a_release_build() {
        let s = status_for("0.59.26", "stable", &release("v0.59.27"), 5, &InstallKind::Unknown);
        assert_eq!(s.state, "available");
        assert_eq!(s.latest.as_deref(), Some("0.59.27"));
        assert_eq!(s.current, "0.59.26");
        assert!(s.url.unwrap().ends_with("/v0.59.27"));
        assert_eq!(s.checked_ms, 5);
        assert_eq!(s.notes.as_deref(), Some("- Sysinfo opens on a new Overview"));
    }

    #[test]
    fn the_same_or_an_older_release_is_up_to_date() {
        assert_eq!(status_for("0.59.27", "stable", &release("v0.59.27"), 0, &InstallKind::Unknown).state, "up-to-date");
        assert_eq!(status_for("0.60.0", "stable", &release("v0.59.27"), 0, &InstallKind::Unknown).state, "up-to-date");
        // Numbers, not text: 0.59.100 is newer than 0.59.27.
        assert_eq!(status_for("0.59.27", "stable", &release("v0.59.100"), 0, &InstallKind::Unknown).state, "available");
        assert_eq!(status_for("0.59.100", "stable", &release("v0.59.27"), 0, &InstallKind::Unknown).state, "up-to-date");
    }

    #[test]
    fn a_dev_build_only_reports_the_latest_release() {
        for channel in ["dev-qtest-x-38b2af", "dev"] {
            let s = status_for("0.59.26", channel, &release("v0.59.27"), 0, &InstallKind::Unknown);
            assert_eq!(s.state, "local-build", "{channel}");
            assert_eq!(s.latest.as_deref(), Some("0.59.27"));
        }
    }

    #[test]
    fn a_local_package_build_upgrades_like_a_release() {
        let local = "local-main-b28b7a-f62415cd";
        assert_eq!(status_for("0.59.26", local, &release("v0.59.27"), 0, &InstallKind::Unknown).state, "available");
        // Built from main, ahead of the latest release: nothing to upgrade to.
        assert_eq!(status_for("0.59.28", local, &release("v0.59.27"), 0, &InstallKind::Unknown).state, "up-to-date");
    }

    #[test]
    fn versions_that_dont_parse_are_never_called_newer() {
        assert_eq!(status_for("0.59.26", "stable", &release("nightly"), 0, &InstallKind::Unknown).state, "unknown");
        assert_eq!(status_for("weird", "stable", &release("v0.59.27"), 0, &InstallKind::Unknown).state, "unknown");
        let mut pre = release("v0.60.0-rc1");
        pre.prerelease = true;
        assert_eq!(status_for("0.59.26", "stable", &pre, 0, &InstallKind::Unknown).state, "up-to-date");
        assert_eq!(version_core("v0.59.27+g1a2b"), Some((0, 59, 27)));
        assert_eq!(version_core("0.59"), None);
        assert_eq!(version_core("0.59.27.1"), None);
    }

    #[test]
    fn long_notes_are_cut_on_a_character_boundary() {
        let mut r = release("v0.59.27");
        r.body = Some("é".repeat(MAX_NOTES_BYTES));
        let notes = status_for("0.59.26", "stable", &r, 0, &InstallKind::Unknown).notes.unwrap();
        assert!(notes.len() <= MAX_NOTES_BYTES);
        assert!(notes.chars().all(|c| c == 'é'));
    }

    #[test]
    fn the_status_reads_as_the_ui_expects() {
        let s = status_for("0.59.26", "stable", &release("v0.59.27"), 7, &InstallKind::Unknown);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "available");
        assert_eq!(v["checked_ms"], 7);
        let mut unchecked = serde_json::to_value(AppUpdateStatus::initial("0.59.26")).unwrap();
        unchecked.as_object_mut().unwrap().remove("install");
        assert_eq!(unchecked, serde_json::json!({ "state": "unknown", "current": "0.59.26", "checked_ms": 0, "can_download": false }));
    }

    #[test]
    fn a_check_doesnt_replace_the_release_an_upgrade_is_installing() {
        let download = |state: &str| DownloadStatus {
            version: "0.59.27".into(),
            state: state.into(),
            received: 1,
            total: 2,
            error: None,
            folder: None,
            installer: None,
            waiting_for: 0,
        };
        let newer = status_for("0.59.26", "stable", &release("v0.59.28"), 9, &InstallKind::Unknown);
        let same = status_for("0.59.26", "stable", &release("v0.59.27"), 9, &InstallKind::Unknown);
        let mut cur = status_for("0.59.26", "stable", &release("v0.59.27"), 1, &InstallKind::Unknown);
        for state in ["downloading", "unpacking", "waiting", "restarting"] {
            cur.download = Some(download(state));
            assert_eq!(pinned_by_upgrade(&cur, &newer).as_deref(), Some("0.59.27"), "{state}");
            assert_eq!(pinned_by_upgrade(&cur, &same), None, "{state}: the same release is no change");
        }
        // Downloaded but not restarting, failed, or nothing asked: the newer one is offered.
        for state in ["ready", "failed"] {
            cur.download = Some(download(state));
            assert_eq!(pinned_by_upgrade(&cur, &newer), None, "{state}");
        }
        cur.download = None;
        assert_eq!(pinned_by_upgrade(&cur, &newer), None);
    }

    fn with_asset(mut r: Release, name: &str, digest: Option<&str>, url: &str) -> Release {
        r.assets.push(GithubAsset {
            name: name.into(),
            size: 124_705_866,
            browser_download_url: url.into(),
            digest: digest.map(Into::into),
        });
        r
    }

    #[test]
    fn a_portable_gets_its_release_file_with_github_s_checksum() {
        let name = "agentmux-0.59.27-x64-portable.7z";
        let url = format!("https://github.com/agentmuxai/agentmux/releases/download/v0.59.27/{name}");
        let sha = "DB750E1689752519AD21B730436F783493C63B59AD734677A433E413C9B47F2A";
        let kind = InstallKind::WindowsPortable { root: "C:/apps/agentmux".into() };
        let r = with_asset(release("v0.59.27"), name, Some(&format!("sha256:{sha}")), &url);
        let s = status_for("0.59.26", "stable", &r, 0, &kind);
        assert_eq!(s.install, "windows-portable");
        if std::env::consts::ARCH == "x86_64" {
            let asset = s.asset.clone().unwrap();
            assert_eq!(asset.sha256, sha.to_ascii_lowercase());
            assert_eq!((asset.name.as_str(), asset.size), (name, 124_705_866));
            assert!(s.can_download);
        }
        // Not newer: nothing to download.
        assert!(!status_for("0.59.27", "stable", &r, 0, &kind).can_download);
        // No checksum, or served from anywhere else: not offered.
        let unsigned = with_asset(release("v0.59.27"), name, None, &url);
        assert!(status_for("0.59.26", "stable", &unsigned, 0, &kind).asset.is_none());
        let elsewhere = with_asset(release("v0.59.27"), name, Some(&format!("sha256:{sha}")), "https://example.com/x.7z");
        assert!(status_for("0.59.26", "stable", &elsewhere, 0, &kind).asset.is_none());
        // A kind AgentMux can't install itself still learns of the release.
        let store = status_for("0.59.26", "stable", &r, 0, &InstallKind::WindowsStore);
        assert_eq!((store.state.as_str(), store.can_download), ("available", false));
    }
}
