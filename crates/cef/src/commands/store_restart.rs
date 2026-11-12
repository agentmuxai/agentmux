// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Microsoft Store update has to leave behind for AgentMux to say how it ended
//! (task #23915). The Store closes a packaged app to swap its package and doesn't start it
//! again by itself, so a Store install:
//!
//! - is registered for an application restart by the launcher (`agentmux.exe`, the package's
//!   entry point: `crates/launcher/src/store_restart.rs`); on the first real run (Area54, 2026-11-12) that
//!   did not fire after the Store's servicing, so `store_reopen` arranges the reopen as well;
//! - writes a small marker just before asking the Store to install, so the next start can say
//!   whether it came back updated or on the same version (the install failed after closing it), and
//!   whether the reopen could be arranged.
//!
//! NOT run against a packaged install by whoever wrote it: the decision about the marker is
//! unit-tested.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const FILE: &str = "agentmux-store-update.json";
/// A marker this old is not about an install the person just started.
const STALE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// An update was downloaded by this process and is waiting for Restart now. Not kept across starts: a normal quit
/// installs nothing, so the next start has nothing to finish (it downloads again, which is quick).
static DOWNLOADED: AtomicBool = AtomicBool::new(false);

/// The Store has the update downloaded (set by "Update in the Store").
pub fn set_downloaded(yes: bool) {
    DOWNLOADED.store(yes, Ordering::SeqCst);
}

/// The version this build is.
fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// An install that was asked of the Store: from which version, to which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub from: String,
    pub target: String,
    pub at_ms: u64,
    /// The reopen could not be arranged (the Scheduled Task was refused): the next start says AgentMux did not
    /// open itself after the update.
    #[serde(default)]
    pub reopen_failed: bool,
}

/// What the page is told at start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameBack {
    /// `none` (no install was in flight), `updated` (the app came back on a new version) or
    /// `stayed` (the Store closed it and it is still on the version it was).
    pub state: &'static str,
    pub from: String,
    pub target: String,
    /// The reopen could not be arranged before the update, so whoever sees this opened AgentMux themselves.
    pub reopen_failed: bool,
}

impl CameBack {
    fn new(state: &'static str, from: &str, target: &str) -> Self {
        Self { state, from: from.into(), target: target.into(), reopen_failed: false }
    }

    fn reopen_failed(mut self, failed: bool) -> Self {
        self.reopen_failed = failed;
        self
    }
}

/// What a marker says about this start, and whether it is kept for the next one:
/// a changed version is an update (and settled); the same version within a week is a
/// stay (kept, so the panel keeps saying so until another install is tried); older, or none, is nothing.
pub fn judge(marker: Option<&Marker>, running: &str, now_ms: u64) -> (CameBack, bool) {
    let Some(m) = marker else { return (CameBack::new("none", "", ""), false) };
    if m.from != running {
        return (CameBack::new("updated", &m.from, running).reopen_failed(m.reopen_failed), false);
    }
    if now_ms.saturating_sub(m.at_ms) > STALE_MS {
        return (CameBack::new("none", "", ""), false);
    }
    (CameBack::new("stayed", &m.from, &m.target).reopen_failed(m.reopen_failed), true)
}

fn read(dir: &Path) -> Option<Marker> {
    serde_json::from_slice(&std::fs::read(dir.join(FILE)).ok()?).ok()
}

/// Called once at start with the data paths (the marker lives in the instance folder): remembers it.
pub fn at_startup(paths: Option<&agentmux_common::DataPaths>) {
    if let Some(p) = paths {
        let _ = DIR.set(p.instance_dir.clone());
    }
}

/// What this start makes of the marker; settles it (removes it) unless the app stayed.
pub fn came_back() -> CameBack {
    let Some(dir) = DIR.get() else { return CameBack::new("none", "", "") };
    let (verdict, keep) = judge(read(dir).as_ref(), current(), agentmux_common::time::now_ms_u64());
    if !keep {
        let _ = std::fs::remove_file(dir.join(FILE));
    }
    verdict
}

/// Just before the Store is asked to install: the marker. `reopen_failed`: the reopen could not be arranged.
pub fn before_install(reopen_failed: bool) {
    if let Some(dir) = DIR.get() {
        let marker = Marker { from: current().into(), target: String::new(), at_ms: agentmux_common::time::now_ms_u64(), reopen_failed };
        match serde_json::to_vec(&marker) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(dir.join(FILE), bytes) {
                    tracing::warn!(error = %e, "store update: could not write the marker");
                }
            }
            Err(e) => tracing::warn!(error = %e, "store update: could not encode the marker"),
        }
    }
}

/// The install call came back. `completed`: the Store installed the update, so it closes this app
/// at its own pace (it may do so a moment after the call returns): the marker stays, and the next
/// start says `updated`, or `stayed` if the package did not change. Otherwise (cancelled, refused,
/// an error) nothing closes the app and there is nothing to come back from: the marker goes.
/// Whether it went, which is when the session record saved before the install is to be saved
/// again as for no quit.
pub fn install_returned(completed: bool) -> bool {
    DIR.get().is_some_and(|dir| after_return(dir, completed))
}

fn after_return(dir: &Path, completed: bool) -> bool {
    !completed && std::fs::remove_file(dir.join(FILE)).is_ok()
}

/// What `store_update_restart` tells the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Restart {
    /// Whether AgentMux is closing now (the Store installed it, or Windows is closing the app for servicing).
    pub ok: bool,
    /// The person said no in the Store's own window: the update stays downloaded and Restart now stays available.
    pub canceled: bool,
    /// A sentence for the person; when `ok` is false, why nothing was closed.
    pub message: String,
}

impl Restart {
    fn refused(message: impl Into<String>) -> Self {
        Self { ok: false, canceled: false, message: message.into() }
    }
}

/// Whether a restart may go ahead: the Store has the update downloaded by this process.
fn restart_wanted(downloaded: bool) -> Result<(), &'static str> {
    if downloaded {
        Ok(())
    } else {
        Err("No Store update is downloaded. Use Update in the Store first.")
    }
}

/// "Restart now" on a downloaded Store update, after the row asked and named what a restart interrupts. In order:
/// our window is raised (the Store's own prompt is owned by it); the reopen is arranged, best effort (if it can't be,
/// the update goes ahead and the next start says it could not reopen by itself); then the Store is asked to install,
/// silently when it allows that, else through its own window while this call stays open. Windows closes the running
/// app for servicing at the person's answer and swaps the package; an install that came back finished closes it here,
/// the way the tray's Quit does. It comes only from a click in this app's own window: no MuxAPI tool or route
/// reaches it, so an agent can't restart the person's AgentMux.
pub fn restart_now(state: &std::sync::Arc<crate::state::AppState>) -> Restart {
    if let Err(why) = restart_wanted(DOWNLOADED.load(Ordering::SeqCst)) {
        return Restart::refused(why);
    }
    super::store_update::raise_own_window();
    let reopen = super::store_reopen::schedule(current());
    if let Err(why) = &reopen {
        tracing::warn!(error = %why, "[store-restart] could not arrange the reopen; going on with the update");
    }
    before_install(reopen.is_err());
    let (web_endpoint, auth_key) = (state.backend_endpoints.lock().web_endpoint.clone(), state.auth_key.lock().clone());
    crate::client::save_session_snapshot_with(&web_endpoint, &auth_key, true, false);
    let outcome = super::store_update::install_downloaded(super::store_update::emit_for(state));
    let installed = outcome.outcome == "installed";
    // Back here: Windows did not close this app. Finished means the package is staged for a close we make ourselves;
    // anything else closes nothing: the marker goes and the windows' record is saved as for no quit.
    if install_returned(installed) {
        crate::client::backend_save_session_snapshot(&web_endpoint, &auth_key, false);
    }
    if !installed {
        return Restart { ok: false, canceled: outcome.outcome == "canceled", message: outcome.message };
    }
    tracing::info!("[store-restart] installed; quitting for the Store's swap");
    match super::window::quit_app(state) {
        Ok(_) => Restart { ok: true, canceled: false, message: "Closing AgentMux; it opens again on the new version.".into() },
        Err(why) => Restart::refused(format!("Couldn't close AgentMux ({why}).")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_goes_ahead_only_for_an_update_this_process_downloaded() {
        assert!(restart_wanted(true).is_ok());
        assert!(restart_wanted(false).is_err(), "nothing was downloaded: nothing is closed");
    }

    fn marker(at_ms: u64) -> Marker {
        Marker { from: "0.60.14".into(), target: "0.60.15".into(), at_ms, reopen_failed: false }
    }

    #[test]
    fn a_reopen_that_could_not_be_arranged_is_told_at_the_next_start() {
        let failed = Marker { reopen_failed: true, ..marker(1_000) };
        assert!(judge(Some(&failed), "0.60.15", 2_000).0.reopen_failed, "came back updated, opened by hand");
        assert!(judge(Some(&failed), "0.60.14", 2_000).0.reopen_failed);
        assert!(!judge(Some(&marker(1_000)), "0.60.15", 2_000).0.reopen_failed);
        // A marker written before the field existed reads as arranged.
        let old: Marker = serde_json::from_str(r#"{"from":"0.60.14","target":"","at_ms":5}"#).unwrap();
        assert!(!old.reopen_failed);
    }

    #[test]
    fn no_marker_means_no_install_was_in_flight() {
        let (v, keep) = judge(None, "0.60.14", 1_000);
        assert_eq!(v.state, "none");
        assert!(!keep);
    }

    #[test]
    fn a_new_version_after_a_marker_is_an_update_and_settles_it() {
        let (v, keep) = judge(Some(&marker(1_000)), "0.60.15", 2_000);
        assert_eq!((v.state, v.from.as_str(), v.target.as_str()), ("updated", "0.60.14", "0.60.15"));
        assert!(!keep);
    }

    #[test]
    fn the_same_version_after_a_marker_is_a_stay_that_is_kept() {
        let (v, keep) = judge(Some(&marker(1_000)), "0.60.14", 2_000);
        assert_eq!((v.state, v.from.as_str(), v.target.as_str()), ("stayed", "0.60.14", "0.60.15"));
        assert!(keep);
    }

    #[test]
    fn an_old_stay_is_forgotten() {
        let (v, keep) = judge(Some(&marker(1_000)), "0.60.14", 1_000 + STALE_MS + 1);
        assert_eq!(v.state, "none");
        assert!(!keep);
    }

    #[test]
    fn a_completed_install_keeps_its_marker_and_any_other_result_removes_it() {
        let dir = std::env::temp_dir().join(format!("agentmux-store-marker-after-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = || std::fs::write(dir.join(FILE), serde_json::to_vec(&marker(5)).unwrap()).unwrap();
        write();
        assert!(!after_return(&dir, true), "the Store is closing the app: nothing to save again");
        assert!(dir.join(FILE).exists(), "the next start says how it ended");
        assert!(after_return(&dir, false), "cancelled or refused: the marker was there and went");
        assert!(!dir.join(FILE).exists());
        assert!(!after_return(&dir, false), "no marker, nothing to remove");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_marker_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("agentmux-store-marker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = marker(5);
        std::fs::write(dir.join(FILE), serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(read(&dir), Some(m));
        std::fs::remove_dir_all(&dir).ok();
    }
}
