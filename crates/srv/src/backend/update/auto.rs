// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The background update: check for a newer release on a timer, and download,
//! verify and stage it, with no click
//! (`docs/specs/SPEC_BACKGROUND_UPDATES_ONE_CLICK_RESTART_2026_11_03.md` §4.1, §4.2).
//!
//! It only calls what a click calls: [`check::check_now`] and
//! [`download::start`], so what is checked, downloaded, signed and staged is
//! the same either way. Nothing is installed here: a staged release waits for
//! the user's restart (the status bar's **Restart to update**, the tray, or
//! the version panel).
//!
//! The first check is a minute after start (never during startup), then every
//! six hours give or take a fifth, so a fleet doesn't ask GitHub together. A
//! failed check backs off: 15 minutes, doubling, at most a day, and the next
//! good one is back to six hours. `update:auto` off asks GitHub nothing;
//! `update:autodownload` off checks but leaves the download to the click. A
//! `task dev` build (it runs from source), a headless srv or test harness
//! (`AGENTMUX_DISABLE_CLOUD_SUBSCRIBER`) and `AGENTMUX_NO_AUTO_UPDATE` never
//! check by themselves.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::update::check::{self, AppUpdateStatus};
use crate::backend::update::download;
use crate::backend::wconfig::ConfigState;

/// The first check, after start.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(60);
/// Up to this much more, so installs started together don't check together.
const FIRST_CHECK_JITTER: Duration = Duration::from_secs(30);
/// Between two good checks.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The first wait after a failed check; it doubles up to [`BACKOFF_MAX`].
const BACKOFF_START: Duration = Duration::from_secs(15 * 60);
const BACKOFF_MAX: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the switch is looked at while it is off.
const OFF_RECHECK: Duration = Duration::from_secs(5 * 60);
/// A release whose download failed is tried again at the next checks, this
/// many times, then left to the user's click.
const MAX_DOWNLOAD_TRIES: u32 = 3;

/// The two switches, read from settings each round (absent = on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Switches {
    pub check: bool,
    pub download: bool,
}

pub(crate) fn switches(config: &ConfigState) -> Switches {
    let s = config.get_settings();
    Switches { check: s.update_auto.unwrap_or(true), download: s.update_auto_download.unwrap_or(true) }
}

/// Start the timer. Call once, after [`check::init`].
pub fn spawn(config: Arc<ConfigState>) {
    if std::env::var_os("AGENTMUX_NO_AUTO_UPDATE").is_some_and(|v| !v.is_empty()) {
        tracing::info!("update: background check off (AGENTMUX_NO_AUTO_UPDATE)");
        return;
    }
    // A test harness, or a headless srv (it sets this itself): neither updates an app.
    if std::env::var_os("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER").is_some() {
        return;
    }
    let channel = crate::backend::reactive::registry::local_channel_id();
    if check::is_dev_build(&channel) {
        return;
    }
    tokio::spawn(async move { run(config).await });
}

async fn run(config: Arc<ConfigState>) {
    tokio::time::sleep(FIRST_CHECK_AFTER + FIRST_CHECK_JITTER.mul_f64(fraction())).await;
    let mut failures = 0u32;
    let mut tries = DownloadTries::default();
    loop {
        let on = switches(&config);
        if !on.check {
            tokio::time::sleep(OFF_RECHECK).await;
            continue;
        }
        let wait = match check::check_now().await {
            Ok(status) => {
                failures = 0;
                if on.download {
                    maybe_download(&status, &mut tries);
                }
                after_good_check(fraction())
            }
            Err(e) => {
                failures = failures.saturating_add(1);
                tracing::info!(error = %e, failures, "update: background check failed");
                after_failure(failures)
            }
        };
        tokio::time::sleep(wait).await;
    }
}

/// Start the download of the release on offer when it is wanted and not tried
/// out.
fn maybe_download(status: &AppUpdateStatus, tries: &mut DownloadTries) {
    if !wants_download(status, tries) {
        return;
    }
    match download::start() {
        Ok(_) => tracing::info!(latest = ?status.latest, "update: downloading in the background"),
        Err(e) => tracing::info!(error = %e, "update: background download not started"),
    }
}

/// How often a release's download was started here, so a download that keeps
/// failing is left alone.
#[derive(Debug, Default)]
pub(crate) struct DownloadTries {
    version: Option<String>,
    started: u32,
}

/// Whether the background should start the download for `status` now, and
/// count the start in `tries`: a newer release is on offer, this install can
/// download it, nothing of it is under way or staged, and a failed download
/// has not failed [`MAX_DOWNLOAD_TRIES`] times.
pub(crate) fn wants_download(status: &AppUpdateStatus, tries: &mut DownloadTries) -> bool {
    let Some(version) = status.latest.as_deref() else { return false };
    if status.state != "available" || !status.can_download {
        return false;
    }
    if let Some(d) = &status.download {
        // Under way, staged, or asked to restart: not ours to start again.
        if d.version == version && d.state != "failed" {
            return false;
        }
    }
    if tries.version.as_deref() != Some(version) {
        *tries = DownloadTries { version: Some(version.to_string()), started: 0 };
    }
    if tries.started >= MAX_DOWNLOAD_TRIES {
        return false;
    }
    tries.started += 1;
    true
}

/// The wait after a good check: [`CHECK_EVERY`], 80%..120% of it for `r` in 0..1.
pub(crate) fn after_good_check(r: f64) -> Duration {
    CHECK_EVERY.mul_f64(0.8 + 0.4 * r.clamp(0.0, 1.0))
}

/// The wait after the `failures`th failed check in a row: 15 minutes, doubling,
/// at most a day.
pub(crate) fn after_failure(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    BACKOFF_START.saturating_mul(1u32 << doublings).min(BACKOFF_MAX)
}

/// A random number in 0..1, from the clock: nothing here needs better.
fn fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    f64::from(nanos % 1_000_000) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::update::check::DownloadStatus;

    fn offered(latest: &str) -> AppUpdateStatus {
        AppUpdateStatus {
            state: "available".into(),
            current: "0.60.1".into(),
            latest: Some(latest.into()),
            url: None,
            notes: None,
            published_at: None,
            checked_ms: 1,
            install: "windows-portable".into(),
            asset: None,
            can_download: true,
            download: None,
            install_first: None,
        }
    }

    fn download_in(version: &str, state: &str) -> DownloadStatus {
        DownloadStatus {
            version: version.into(),
            state: state.into(),
            received: 0,
            total: 0,
            error: None,
            folder: None,
            installer: None,
            waiting_for: 0,
            stopping: Vec::new(),
        }
    }

    #[test]
    fn a_good_check_waits_about_six_hours() {
        assert_eq!(after_good_check(0.5), CHECK_EVERY);
        assert_eq!(after_good_check(0.0), CHECK_EVERY.mul_f64(0.8));
        assert_eq!(after_good_check(1.0), CHECK_EVERY.mul_f64(1.2));
        // Out of range is held to the range.
        assert_eq!(after_good_check(7.0), CHECK_EVERY.mul_f64(1.2));
    }

    #[test]
    fn a_failed_check_backs_off_to_a_day() {
        assert_eq!(after_failure(1), Duration::from_secs(15 * 60));
        assert_eq!(after_failure(2), Duration::from_secs(30 * 60));
        assert_eq!(after_failure(3), Duration::from_secs(60 * 60));
        assert_eq!(after_failure(8), BACKOFF_MAX);
        assert_eq!(after_failure(u32::MAX), BACKOFF_MAX);
    }

    #[test]
    fn a_newer_release_is_downloaded_once_it_can_be() {
        let mut tries = DownloadTries::default();
        assert!(wants_download(&offered("0.60.2"), &mut tries));
    }

    #[test]
    fn nothing_is_downloaded_that_the_install_cannot_download() {
        let mut tries = DownloadTries::default();
        let mut store = offered("0.60.2");
        store.can_download = false;
        assert!(!wants_download(&store, &mut tries), "a Store install updates through the Store");
        let mut current = offered("0.60.1");
        current.state = "up-to-date".into();
        assert!(!wants_download(&current, &mut tries));
        let mut unchecked = offered("0.60.2");
        unchecked.latest = None;
        assert!(!wants_download(&unchecked, &mut tries));
    }

    #[test]
    fn a_download_under_way_or_staged_is_left_alone() {
        for state in ["downloading", "unpacking", "ready", "waiting", "restarting"] {
            let mut tries = DownloadTries::default();
            let mut s = offered("0.60.2");
            s.download = Some(download_in("0.60.2", state));
            assert!(!wants_download(&s, &mut tries), "{state}");
        }
    }

    #[test]
    fn a_download_of_an_older_release_does_not_block_the_newer_one() {
        let mut tries = DownloadTries::default();
        let mut s = offered("0.60.3");
        s.download = Some(download_in("0.60.2", "ready"));
        assert!(wants_download(&s, &mut tries));
    }

    #[test]
    fn a_failing_download_is_tried_three_times_then_left_to_the_click() {
        let mut tries = DownloadTries::default();
        let mut s = offered("0.60.2");
        s.download = Some(download_in("0.60.2", "failed"));
        for _ in 0..MAX_DOWNLOAD_TRIES {
            assert!(wants_download(&s, &mut tries));
        }
        assert!(!wants_download(&s, &mut tries));
        // A newer release starts over.
        let mut newer = offered("0.60.3");
        newer.download = Some(download_in("0.60.2", "failed"));
        assert!(wants_download(&newer, &mut tries));
    }
}
