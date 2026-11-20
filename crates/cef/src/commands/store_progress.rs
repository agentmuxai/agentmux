// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Microsoft Store says while it downloads and installs an update, in the shape the
//! Maintenance row shows (task #2514). `RequestDownloadStorePackageUpdatesAsync` and
//! `RequestDownloadAndInstallStorePackageUpdatesAsync` are operations with progress: each report is a
//! `StorePackageUpdateStatus` (`PackageBytesDownloaded`, `PackageDownloadSizeInBytes`,
//! `PackageDownloadProgress` and `TotalDownloadProgress` from 0 to 1, and a `PackageUpdateState`).
//! They are turned into a [`Progress`] sent to every window as the `store-update-progress` event.
//!
//! The WinRT handler that feeds this is in `store_update`; the rules here are plain and tested.

use serde::Serialize;

/// The event every window listens for.
pub const EVENT: &str = "store-update-progress";

/// The Store's `StorePackageUpdateState`, apart from the WinRT type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateState {
    Pending,
    Downloading,
    Deploying,
    /// Completed, cancelled or any error: the install call's own result says which.
    Finished,
}

/// One report for the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// `pending` (the Store has not started), `downloading`, `installing` (the Store is deploying the
    /// package, or waiting to) or `asking` (the Store is asking the person to confirm, in its own window).
    pub phase: &'static str,
    /// 0 to 99 while downloading, or `None` when the Store has given no progress of the whole update (the page then
    /// keeps its plain line).
    pub percent: Option<u8>,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

/// How far the whole update is, from the Store's own 0-to-1 fraction over every package it downloads
/// (`TotalDownloadProgress`), and nothing else. The bytes a report carries (`PackageBytesDownloaded` over
/// `PackageDownloadSizeInBytes`) are the current package's only: an update that is several packages, a small one
/// first, read 99% when the small one was done and then 1% again when the large one began, so they are not what
/// the percent is made from, and not shown beside it (they would be a second measure of one download).
/// Floored, and never 100 while downloading: that is the Store's next state, installing. `None` while the Store has
/// given no progress of the whole yet (not a number, or not above 0): the page keeps its plain line and shows a figure
/// from the first real one.
fn percent_of(fraction: f64) -> Option<u8> {
    if !(fraction.is_finite() && fraction > 0.0) {
        return None;
    }
    // The small epsilon keeps 0.29 (which times 100 is 28.999...) at 29.
    Some(((fraction.min(1.0) * 100.0 + 1e-9).floor() as u8).min(99))
}

/// A report from the Store's progress handler: its state and the fraction of the whole update.
pub fn from_status(state: UpdateState, fraction: f64) -> Progress {
    let (phase, percent) = match state {
        UpdateState::Pending => ("pending", None),
        UpdateState::Downloading => ("downloading", percent_of(fraction)),
        UpdateState::Deploying | UpdateState::Finished => ("installing", None),
    };
    // The bytes fields stay for the page's line, which quotes them when it has them; no report of the whole has any.
    Progress { phase, percent, done_bytes: 0, total_bytes: 0 }
}

/// Before the Store has said anything.
pub fn starting() -> Progress {
    Progress { phase: "pending", percent: None, done_bytes: 0, total_bytes: 0 }
}

/// The Store has been asked to install and is asking the person through its own window (the install call stays open
/// until they answer it): the row says so for as long as the call is open.
pub fn asking() -> Progress {
    Progress { phase: "asking", percent: None, done_bytes: 0, total_bytes: 0 }
}

/// The install is under way without a question (the Store may install silently): AgentMux will close and open again.
pub fn installing() -> Progress {
    Progress { phase: "installing", percent: None, done_bytes: 0, total_bytes: 0 }
}

/// Lets a report through only when what the page shows would change: the Store reports many times a second.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<(&'static str, Option<u8>)>,
}

impl Throttle {
    pub fn admits(&mut self, p: &Progress) -> bool {
        let key = (p.phase, p.percent);
        if self.last == Some(key) {
            return false;
        }
        self.last = Some(key);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downloading(fraction: f64) -> Progress {
        from_status(UpdateState::Downloading, fraction)
    }

    #[test]
    fn a_download_reports_the_percent_of_the_whole_update() {
        let p = downloading(0.1);
        assert_eq!(p.phase, "downloading");
        assert_eq!(p.percent, Some(10));
    }

    /// The flash: the update is a small package then a large one. The small package's own bytes read 99% when it was
    /// done while the whole update was at 1 to 2%, and the large package's own bytes then read 1% again. The package's
    /// bytes are not an input any more, so the figure follows the whole update only.
    #[test]
    fn a_small_package_finishing_first_does_not_read_99_when_the_update_is_at_one_percent() {
        let reports = [0.0, 0.0001, 0.0002, 0.0002, 0.0102, 0.0197].map(downloading);
        let seen: Vec<_> = reports.iter().map(|p| p.percent).collect();
        assert_eq!(seen, [None, Some(0), Some(0), Some(0), Some(1), Some(1)]);
        assert!(reports.iter().all(|p| p.percent.unwrap_or(0) < 2), "never the 99 a package's own bytes read");
    }

    #[test]
    fn the_percent_is_the_whole_numbers_floor_and_a_download_never_reads_100() {
        assert_eq!(downloading(0.42).percent, Some(42));
        assert_eq!(downloading(0.429).percent, Some(42));
        assert_eq!(downloading(0.29).percent, Some(29));
        assert_eq!(downloading(0.58).percent, Some(58));
        // 100 is the Store's next state (installing), not a download that is still reporting.
        assert_eq!(downloading(0.999).percent, Some(99));
        assert_eq!(downloading(1.0).percent, Some(99));
        assert_eq!(downloading(7.0).percent, Some(99));
    }

    #[test]
    fn with_no_progress_of_the_whole_there_is_no_percent_not_a_fake_zero() {
        assert_eq!(downloading(0.0).percent, None);
        assert_eq!(downloading(-0.5).percent, None);
        assert_eq!(downloading(f64::NAN).percent, None);
        assert_eq!(downloading(f64::INFINITY).percent, None);
        assert_eq!(downloading(f64::NEG_INFINITY).percent, None);
    }

    #[test]
    fn a_report_carries_no_bytes_beside_a_percent_of_the_whole() {
        let p = downloading(0.1);
        assert_eq!((p.done_bytes, p.total_bytes), (0, 0));
    }

    #[test]
    fn the_other_states_name_their_phase_and_carry_no_percent() {
        assert_eq!(from_status(UpdateState::Pending, 0.0).phase, "pending");
        assert_eq!(from_status(UpdateState::Deploying, 1.0).phase, "installing");
        assert_eq!(from_status(UpdateState::Finished, 1.0).percent, None);
        assert_eq!(from_status(UpdateState::Pending, 0.5).percent, None);
    }

    #[test]
    fn the_throttle_lets_through_only_a_change_of_what_the_page_shows() {
        let mut t = Throttle::default();
        let at = |pct: u64| downloading(pct as f64 / 100.0);
        assert!(t.admits(&at(1)));
        assert!(!t.admits(&at(1)), "the same percent again");
        assert!(t.admits(&at(2)));
        assert!(t.admits(&from_status(UpdateState::Deploying, 1.0)), "a new phase");
        assert!(!t.admits(&from_status(UpdateState::Deploying, 1.0)));
    }

    #[test]
    fn a_report_that_never_gets_a_figure_is_let_through_once_and_a_figure_after_it_at_once() {
        let mut t = Throttle::default();
        assert!(t.admits(&downloading(0.0)));
        assert!(!t.admits(&downloading(0.0)), "still no figure");
        assert!(t.admits(&downloading(0.03)), "the first figure");
        assert!(t.admits(&downloading(0.04)));
    }

    #[test]
    fn the_event_carries_camel_case_fields_for_the_page() {
        let v = serde_json::to_value(downloading(0.5)).unwrap();
        assert_eq!(v["phase"], "downloading");
        assert_eq!(v["percent"], 50);
        assert_eq!(v["doneBytes"], 0);
        assert_eq!(v["totalBytes"], 0);
    }

    #[test]
    fn asking_and_installing_are_phases_without_numbers() {
        assert_eq!((asking().phase, asking().percent), ("asking", None));
        assert_eq!((installing().phase, installing().percent), ("installing", None));
    }
}
