// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Microsoft Store update has to leave behind for AgentMux to come back
//! (task #23915). The Store closes a packaged app to swap its package and
//! doesn't start it again by itself, so a Store install:
//!
//! - registers for an application restart (`RegisterApplicationRestart`, for an
//!   update only, never after a crash, a hang or a reboot), at start and again
//!   just before it asks the Store to install, so Windows opens it after the swap;
//! - writes a small marker just before asking the Store to install, so the next
//!   start can say whether it came back updated or on the same version (the
//!   install failed after closing it).
//!
//! NOT run against a packaged install by whoever wrote it: the decision about the
//! marker is unit-tested, the Windows call is type-checked only.

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

/// Called once at start with the data paths (the marker lives in the instance folder): remembers
/// it and, for a Store install, registers for a restart after an update.
pub fn at_startup(paths: Option<&agentmux_common::DataPaths>) {
    if let Some(p) = paths {
        let _ = DIR.set(p.instance_dir.clone());
    }
    if is_packaged() {
        register_restart();
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

/// Just before the Store is asked to install `target`: the marker, and the restart registration again.
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
    register_restart();
}

/// The install call came back, so nothing closed this app: there is nothing to come back from.
/// Whether an install had been started (a marker was there).
pub fn install_returned() -> bool {
    DIR.get().is_some_and(|dir| std::fs::remove_file(dir.join(FILE)).is_ok())
}

/// A Store install runs from the package's folder.
fn is_packaged() -> bool {
    std::env::current_exe().map(|p| p.to_string_lossy().contains("WindowsApps")).unwrap_or(false)
}

/// Windows opens this app again after it closed it for an update. No command line, and not
/// after a crash, a hang or a reboot.
#[cfg(target_os = "windows")]
fn register_restart() {
    use windows_sys::Win32::System::Recovery::{RegisterApplicationRestart, RESTART_NO_CRASH, RESTART_NO_HANG, RESTART_NO_REBOOT};
    let hr = unsafe { RegisterApplicationRestart(std::ptr::null(), RESTART_NO_CRASH | RESTART_NO_HANG | RESTART_NO_REBOOT) };
    if hr != 0 {
        tracing::warn!(hresult = format!("{hr:#x}"), "store update: could not register for a restart after the update");
    }
}

#[cfg(not(target_os = "windows"))]
fn register_restart() {}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_marker_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("agentmux-store-marker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = marker(5);
        std::fs::write(dir.join(FILE), serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(read(&dir), Some(m));
        std::fs::remove_dir_all(&dir).ok();
    }
}
