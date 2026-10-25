// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Update in the Store" for a Microsoft Store install: ask the Store itself to
//! download and install AgentMux's update (task #88, phase 3f of
//! `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`), instead of only opening
//! the Store page.
//!
//! The Store API (`Windows.Services.Store`) works only for a packaged app, and
//! the Store certifies a release after GitHub publishes it, so this has three
//! ways not to install: nothing waiting yet, the person cancelled, or the API
//! refused. The first is told to the person; the last tells the page to open
//! the Store page as before, so nothing is worse than it was.
//!
//! NOT run against a packaged install by whoever wrote it: the decision below is
//! unit-tested, the WinRT calls are type-checked only.

use serde::Serialize;

/// What the Store's overall result for the update was, apart from the WinRT type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreState {
    Completed,
    Canceled,
    LowBattery,
    WifiRecommended,
    WifiRequired,
    /// Anything else the Store reports (pending, downloading, deploying, other errors).
    Other,
}

/// What the page does next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoreOutcome {
    /// `installed`, `canceled`, `none` (the Store has no update yet), or `fallback` (open the Store page).
    pub outcome: &'static str,
    /// A sentence for the person; empty when there is nothing to say.
    pub message: String,
}

impl StoreOutcome {
    fn new(outcome: &'static str, message: impl Into<String>) -> Self {
        Self { outcome, message: message.into() }
    }
}

/// The API refused or isn't there (not a packaged install, the Store app is
/// missing, a call failed): the page opens the Store page instead.
pub fn refused(why: &str) -> StoreOutcome {
    StoreOutcome::new("fallback", format!("Couldn't ask the Store to install it ({why}); opening the Store page instead."))
}

/// The decision, once the Store answered: how many updates it listed for this
/// app, and, when it was asked to install them, the overall state.
pub fn decide(updates_listed: usize, installed: Option<StoreState>) -> StoreOutcome {
    if updates_listed == 0 {
        return StoreOutcome::new("none", "The Microsoft Store doesn't have this update yet. It gets each release after GitHub does; try again later.");
    }
    match installed {
        None => refused("the Store listed an update but wasn't asked to install it"),
        Some(StoreState::Completed) => StoreOutcome::new("installed", "The Store installed the update. AgentMux restarts to finish it."),
        Some(StoreState::Canceled) => StoreOutcome::new("canceled", "The update was cancelled."),
        Some(StoreState::LowBattery) => StoreOutcome::new("canceled", "The Store won't install it on a low battery. Plug in and try again."),
        Some(StoreState::WifiRequired) => StoreOutcome::new("canceled", "The Store needs Wi-Fi for this update. Connect and try again."),
        Some(StoreState::WifiRecommended) => StoreOutcome::new("canceled", "The Store recommends Wi-Fi for this update. Connect and try again."),
        Some(StoreState::Other) => refused("the Store reported an error"),
    }
}

/// Ask the Store to install AgentMux's update. Blocks until the Store is done
/// (minutes, for a download), so call it off the async workers.
#[cfg(target_os = "windows")]
pub fn install_update() -> StoreOutcome {
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
    let spawned = std::thread::Builder::new().name("store-update".into()).spawn(|| {
        // As for the Windows Hello prompt (vault.rs): a WinRT call needs a thread that initialised WinRT.
        let initialised = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
        let outcome = install_update_here().unwrap_or_else(|e| refused(&e));
        if initialised {
            unsafe { RoUninitialize() };
        }
        outcome
    });
    match spawned.map(|h| h.join()) {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) => refused("the Store call stopped unexpectedly"),
        Err(e) => refused(&format!("couldn't start it: {e}")),
    }
}

#[cfg(target_os = "windows")]
fn install_update_here() -> Result<StoreOutcome, String> {
    use windows::core::Interface;
    use windows::Services::Store::{StoreContext, StorePackageUpdateState as S};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::IInitializeWithWindow;

    let context = StoreContext::GetDefault().map_err(|e| format!("no Store context: {e}"))?;
    // A desktop app has to say which window the Store's own prompts belong to.
    if let Some(hwnd) = super::vault::foreground_own_window() {
        let init: IInitializeWithWindow = context.cast().map_err(|e| format!("no window interop: {e}"))?;
        unsafe { init.Initialize(HWND(hwnd as *mut _)) }.map_err(|e| format!("couldn't attach the window: {e}"))?;
    }
    let updates = context
        .GetAppAndOptionalStorePackageUpdatesAsync()
        .and_then(|op| op.get())
        .map_err(|e| format!("couldn't list updates: {e}"))?;
    let listed = updates.Size().map_err(|e| format!("couldn't count updates: {e}"))? as usize;
    if listed == 0 {
        return Ok(decide(0, None));
    }
    let result = context
        .RequestDownloadAndInstallStorePackageUpdatesAsync(&updates)
        .and_then(|op| op.get())
        .map_err(|e| format!("the install call failed: {e}"))?;
    let state = match result.OverallState().map_err(|e| format!("no result state: {e}"))? {
        S::Completed => StoreState::Completed,
        S::Canceled => StoreState::Canceled,
        S::ErrorLowBattery => StoreState::LowBattery,
        S::ErrorWiFiRecommended => StoreState::WifiRecommended,
        S::ErrorWiFiRequired => StoreState::WifiRequired,
        _ => StoreState::Other,
    };
    Ok(decide(listed, Some(state)))
}

/// Only Windows has a Store: elsewhere the page opens the Store page (it never offers this).
#[cfg(not(target_os = "windows"))]
pub fn install_update() -> StoreOutcome {
    refused("the Microsoft Store is Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_listed_means_the_store_doesnt_have_it_yet() {
        let o = decide(0, None);
        assert_eq!(o.outcome, "none");
        assert!(o.message.contains("gets each release after GitHub"));
        // Even if a state came with it, an empty list is "none".
        assert_eq!(decide(0, Some(StoreState::Completed)).outcome, "none");
    }

    #[test]
    fn a_completed_install_is_installed() {
        assert_eq!(decide(1, Some(StoreState::Completed)).outcome, "installed");
        assert_eq!(decide(3, Some(StoreState::Completed)).outcome, "installed");
    }

    #[test]
    fn the_people_s_own_choices_and_conditions_are_told_not_worked_around() {
        for state in [StoreState::Canceled, StoreState::LowBattery, StoreState::WifiRequired, StoreState::WifiRecommended] {
            let o = decide(1, Some(state));
            assert_eq!(o.outcome, "canceled", "{state:?}");
            assert!(!o.message.is_empty(), "{state:?}");
        }
        assert!(decide(1, Some(StoreState::LowBattery)).message.contains("battery"));
        assert!(decide(1, Some(StoreState::WifiRequired)).message.contains("Wi-Fi"));
    }

    #[test]
    fn an_error_or_an_unasked_install_falls_back_to_the_store_page() {
        assert_eq!(decide(1, Some(StoreState::Other)).outcome, "fallback");
        assert_eq!(decide(1, None).outcome, "fallback");
        let o = refused("no Store context: 0x80070490");
        assert_eq!(o.outcome, "fallback");
        assert!(o.message.contains("0x80070490") && o.message.contains("Store page"));
    }

    #[test]
    fn the_outcome_serialises_for_the_page() {
        let v = serde_json::to_value(decide(0, None)).unwrap();
        assert_eq!(v["outcome"], "none");
        assert!(v["message"].is_string());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn off_windows_it_falls_back_without_trying() {
        assert_eq!(install_update().outcome, "fallback");
    }
}
