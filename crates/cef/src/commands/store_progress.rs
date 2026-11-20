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
    /// 0 to 100, or `None` when the Store gave no numbers (the page then keeps its plain line).
    pub percent: Option<u8>,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

/// Whether the Store gave a fraction of the whole update to go by.
fn has_whole(fraction: f64) -> bool {
    fraction.is_finite() && fraction > 0.0
}

/// How far the whole update is, from the Store's own 0-to-1 fraction over every package it downloads
/// (`TotalDownloadProgress`). The bytes a report carries (`PackageBytesDownloaded` over
/// `PackageDownloadSizeInBytes`) are the current package's only: an update that is several packages, a small one
/// first, read 99% when the small one was done and then 1% again when the large one began, so they are not what
/// the percent is made from. Never 100 while downloading: that is the Store's next state, installing. `None` when the Store gave no
/// size and no progress (the page keeps its plain line); 0 when it gave a size and none has arrived yet.
fn percent_of(total_bytes: u64, fraction: f64) -> Option<u8> {
    if has_whole(fraction) {
        // The small epsilon keeps 0.29 (which times 100 is 28.999...) at 29.
        return Some(((fraction.min(1.0) * 100.0 + 1e-9).floor() as u8).min(99));
    }
    (total_bytes > 0).then_some(0)
}

/// A report from the Store's progress handler. The bytes it carries are the current package's, so they are passed on
/// (for the page's "88 of 210 MB") only when the Store gave no fraction of the whole: next to a percent of the whole
/// they would be two different measures of the download.
pub fn from_status(state: UpdateState, done_bytes: u64, total_bytes: u64, fraction: f64) -> Progress {
    let (phase, percent) = match state {
        UpdateState::Pending => ("pending", None),
        UpdateState::Downloading => ("downloading", percent_of(total_bytes, fraction)),
        UpdateState::Deploying | UpdateState::Finished => ("installing", None),
    };
    let (done_bytes, total_bytes) = if state == UpdateState::Downloading && !has_whole(fraction) { (done_bytes, total_bytes) } else { (0, 0) };
    Progress { phase, percent, done_bytes, total_bytes }
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

    const MB: u64 = 1024 * 1024;

    #[test]
    fn a_download_reports_the_percent_of_the_whole_update_not_of_the_current_package() {
        // 88 of this package's 210 MB is 41%, but the update as a whole is a tenth of the way.
        let p = from_status(UpdateState::Downloading, 88 * MB, 210 * MB, 0.1);
        assert_eq!(p.phase, "downloading");
        assert_eq!(p.percent, Some(10));
    }

    /// The flash: the update is a small package then a large one. The small package's own bytes read 99% when it was done,
    /// while the whole update was at 1 to 2%, and the large package's own bytes then read 1% again.
    #[test]
    fn a_small_package_finishing_first_does_not_read_99_when_the_update_is_at_one_percent() {
        let small = 3 * 1024;
        let large = 210 * MB;
        let reports = [
            from_status(UpdateState::Downloading, 0, small, 0.0),
            from_status(UpdateState::Downloading, small * 99 / 100, small, 0.0001),
            from_status(UpdateState::Downloading, small, small, 0.0002),
            from_status(UpdateState::Downloading, 0, large, 0.0002),
            from_status(UpdateState::Downloading, 2 * MB, large, 0.0102),
            from_status(UpdateState::Downloading, 4 * MB, large, 0.0197),
        ];
        let seen: Vec<_> = reports.iter().map(|p| p.percent).collect();
        assert_eq!(seen, [Some(0), Some(0), Some(0), Some(0), Some(1), Some(1)]);
        assert!(reports.iter().all(|p| p.percent.unwrap_or(0) < 2), "never the 99 a package's own bytes read");
    }

    #[test]
    fn the_percent_is_the_whole_numbers_floor_and_a_download_never_reads_100() {
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.42).percent, Some(42));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.429).percent, Some(42));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.29).percent, Some(29));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.58).percent, Some(58));
        // 100 is the Store's next state (installing), not a download that is still reporting.
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.999).percent, Some(99));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 1.0).percent, Some(99));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 7.0).percent, Some(99));
    }

    #[test]
    fn without_a_fraction_it_reads_zero_when_the_store_gave_a_size_and_has_no_percent_without_either() {
        assert_eq!(from_status(UpdateState::Downloading, 0, 210 * MB, 0.0).percent, Some(0));
        // A package's own bytes are never turned into a percent.
        assert_eq!(from_status(UpdateState::Downloading, 209 * MB, 210 * MB, 0.0).percent, Some(0));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.0).percent, None);
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, f64::NAN).percent, None);
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, -0.5).percent, None);
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, f64::INFINITY).percent, None);
    }

    #[test]
    fn the_packages_bytes_go_to_the_page_only_when_there_is_no_fraction_of_the_whole() {
        let with_whole = from_status(UpdateState::Downloading, 88 * MB, 210 * MB, 0.1);
        assert_eq!((with_whole.done_bytes, with_whole.total_bytes), (0, 0));
        let without = from_status(UpdateState::Downloading, 88 * MB, 210 * MB, 0.0);
        assert_eq!((without.done_bytes, without.total_bytes), (88 * MB, 210 * MB));
    }

    #[test]
    fn the_other_states_name_their_phase_and_carry_no_percent() {
        assert_eq!(from_status(UpdateState::Pending, 0, 0, 0.0).phase, "pending");
        assert_eq!(from_status(UpdateState::Deploying, 210 * MB, 210 * MB, 1.0).phase, "installing");
        assert_eq!(from_status(UpdateState::Finished, 210 * MB, 210 * MB, 1.0).percent, None);
    }

    #[test]
    fn the_throttle_lets_through_only_a_change_of_what_the_page_shows() {
        let mut t = Throttle::default();
        let at = |pct: u64| from_status(UpdateState::Downloading, 0, 100, pct as f64 / 100.0);
        assert!(t.admits(&at(1)));
        assert!(!t.admits(&at(1)), "the same percent again");
        assert!(t.admits(&at(2)));
        assert!(t.admits(&from_status(UpdateState::Deploying, 100, 100, 1.0)), "a new phase");
        assert!(!t.admits(&from_status(UpdateState::Deploying, 100, 100, 1.0)));
    }

    #[test]
    fn the_event_carries_camel_case_fields_for_the_page() {
        let v = serde_json::to_value(from_status(UpdateState::Downloading, 5, 10, 0.0)).unwrap();
        assert_eq!(v["phase"], "downloading");
        assert_eq!(v["percent"], 0);
        assert_eq!(v["doneBytes"], 5);
        assert_eq!(v["totalBytes"], 10);
    }

    #[test]
    fn asking_and_installing_are_phases_without_numbers() {
        assert_eq!((asking().phase, asking().percent), ("asking", None));
        assert_eq!((installing().phase, installing().percent), ("installing", None));
    }
}
