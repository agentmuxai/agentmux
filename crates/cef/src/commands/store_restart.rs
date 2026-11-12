// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Microsoft Store update has to leave behind for AgentMux to say how it ended
//! (task #23915). The Store closes a packaged app to swap its package and doesn't start it
//! again by itself, so a Store install:
//!
//! - is registered for an application restart by the launcher (`agentmux.exe`, the package's
//!   entry point: `crates/launcher/src/store_restart.rs`), so Windows opens the launcher and with
//!   it the tray, srv and this host again after the swap. It is not registered here: Windows
//!   would reopen this host alone;
//! - writes a small marker just before asking the Store to install, so the next start can say
//!   whether it came back updated or on the same version (the install failed after closing it).
//!
//! NOT run against a packaged install by whoever wrote it: the decision about the marker is
//! unit-tested.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const FILE: &str = "agentmux-store-update.json";
/// A marker this old is not about an install the person just started.
const STALE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

static DIR: OnceLock<PathBuf> = OnceLock::new();

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
}

/// What the page is told at start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameBack {
    /// `none` (no install was in flight), `updated` (the app came back on a new version) or
    /// `stayed` (the Store closed it and it is still on the version it was).
    pub state: &'static str,
    pub from: String,
    pub target: String,
}

impl CameBack {
    fn new(state: &'static str, from: &str, target: &str) -> Self {
        Self { state, from: from.into(), target: target.into() }
    }
}

/// What a marker says about this start, and whether it is kept for the next one:
/// a changed version is an update (and settled); the same version within a week is a
/// stay (kept, so the panel keeps saying so until another install is tried); older, or none, is nothing.
pub fn judge(marker: Option<&Marker>, running: &str, now_ms: u64) -> (CameBack, bool) {
    let Some(m) = marker else { return (CameBack::new("none", "", ""), false) };
    if m.from != running {
        return (CameBack::new("updated", &m.from, running), false);
    }
    if now_ms.saturating_sub(m.at_ms) > STALE_MS {
        return (CameBack::new("none", "", ""), false);
    }
    (CameBack::new("stayed", &m.from, &m.target), true)
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

/// Just before the Store is asked to install `target`: the marker.
pub fn before_install(target: &str) {
    if let Some(dir) = DIR.get() {
        let marker = Marker { from: current().into(), target: target.into(), at_ms: agentmux_common::time::now_ms_u64() };
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
    /// Whether AgentMux is closing now.
    pub ok: bool,
    /// A sentence for the person; when `ok` is false, why nothing was closed.
    pub message: String,
}

/// Whether a restart may go ahead: an install that ended `staged` is waiting for this build to close. Its marker
/// (written just before the install, kept when the install is staged) is the proof; with none, or one for another
/// build, there is nothing to finish and nothing is closed.
fn restart_wanted(marker: Option<&Marker>, running: &str) -> Result<(), &'static str> {
    match marker {
        Some(m) if m.from == running => Ok(()),
        _ => Err("No Store update is waiting for AgentMux to close. Use Update in the Store first."),
    }
}

/// "Restart now" on a staged Store update: arrange for AgentMux to be opened again once the Store has swapped the
/// package, then quit the way the tray's Quit does (the windows are saved, every agent is told, srv and the launcher
/// stop). Nothing is closed unless the reopen was arranged. It comes only from a click in this app's own window: no
/// MuxAPI tool or route reaches it, so an agent can't restart the person's AgentMux.
pub fn restart_now(state: &std::sync::Arc<crate::state::AppState>) -> Restart {
    let refuse = |why: String| Restart { ok: false, message: why };
    if let Err(why) = restart_wanted(DIR.get().and_then(|d| read(d)).as_ref(), current()) {
        return refuse(why.into());
    }
    if let Err(why) = super::store_reopen::schedule(current()) {
        tracing::warn!(error = %why, "[store-restart] could not arrange the reopen; nothing was closed");
        return refuse(format!("Couldn't arrange for AgentMux to open again ({why}). Nothing was closed. Quit AgentMux and open it again to finish the update."));
    }
    tracing::info!("[store-restart] reopen arranged; quitting for the Store's swap");
    match super::window::quit_app(state) {
        Ok(_) => Restart { ok: true, message: "Closing AgentMux; it opens again on the new version.".into() },
        Err(why) => refuse(format!("Couldn't close AgentMux ({why}).")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_goes_ahead_only_for_an_install_waiting_on_this_build() {
        assert!(restart_wanted(Some(&marker(5)), "0.60.14").is_ok());
        assert!(restart_wanted(None, "0.60.14").is_err(), "no install was under way");
        assert!(restart_wanted(Some(&marker(5)), "0.60.15").is_err(), "the marker is for another build: that one already came back");
    }

    fn marker(at_ms: u64) -> Marker {
        Marker { from: "0.60.14".into(), target: "0.60.15".into(), at_ms }
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
