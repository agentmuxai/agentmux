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

/// Bytes first (they are the figure the page quotes), else the Store's own 0-to-1 fraction; never past 100.
fn percent_of(done: u64, total: u64, fraction: f64) -> Option<u8> {
    if total > 0 {
        return Some((done.min(total).saturating_mul(100) / total) as u8);
    }
    (fraction.is_finite() && fraction > 0.0).then(|| (fraction.min(1.0) * 100.0).round() as u8)
}

/// A report from the Store's progress handler.
pub fn from_status(state: UpdateState, done_bytes: u64, total_bytes: u64, fraction: f64) -> Progress {
    let (phase, percent) = match state {
        UpdateState::Pending => ("pending", None),
        UpdateState::Downloading => ("downloading", percent_of(done_bytes, total_bytes, fraction)),
        UpdateState::Deploying | UpdateState::Finished => ("installing", None),
    };
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
    fn a_download_reports_its_percent_from_the_bytes() {
        let p = from_status(UpdateState::Downloading, 88 * MB, 210 * MB, 0.0);
        assert_eq!((p.phase, p.percent, p.done_bytes, p.total_bytes), ("downloading", Some(41), 88 * MB, 210 * MB));
    }

    #[test]
    fn without_a_size_the_stores_fraction_is_used_and_without_either_there_is_no_percent() {
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.42).percent, Some(42));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 0.0).percent, None);
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, f64::NAN).percent, None);
    }

    #[test]
    fn a_percent_never_passes_100() {
        assert_eq!(from_status(UpdateState::Downloading, 300 * MB, 210 * MB, 0.0).percent, Some(100));
        assert_eq!(from_status(UpdateState::Downloading, 0, 0, 7.0).percent, Some(100));
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
        let at = |pct| from_status(UpdateState::Downloading, pct, 100, 0.0);
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
        assert_eq!(v["percent"], 50);
        assert_eq!(v["doneBytes"], 5);
        assert_eq!(v["totalBytes"], 10);
    }

    #[test]
    fn asking_and_installing_are_phases_without_numbers() {
        assert_eq!((asking().phase, asking().percent), ("asking", None));
        assert_eq!((installing().phase, installing().percent), ("installing", None));
    }
}
