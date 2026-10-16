// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Upgrade & restart", srv's half
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.1, §8.2,
//! phase 3b).
//!
//! Once a release is staged (`update_download`), the user's click waits for
//! every agent to finish its turn (up to 10 minutes; "Now" skips the wait,
//! and Cancel stops it), then srv asks the launcher to restart into the new
//! version with an `AGENTMUXSRV-UPGRADE` line on its stderr. The launcher
//! checks the folder again and asks the user itself
//! (`crates/launcher/src/upgrade_restart.rs`). From then on that dialog is
//! the only place to decide: srv can't reach it, so `restarting` can't be
//! cancelled here (No in the dialog does). If nothing has happened three
//! minutes later (declined, or an AgentMux that can't restart itself), the
//! status goes back to ready and says so.
//!
//! Every change of state happens under [`RUN`], checked against the run it
//! belongs to, so a Cancel can never be overtaken by a run it stopped.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use agentmux_common::srv_stderr::UpgradeTarget;

use crate::backend::update_check::{self, AppUpdateStatus, DownloadStatus};

const WAIT_FOR_AGENTS: Duration = Duration::from_secs(10 * 60);
const POLL: Duration = Duration::from_secs(2);
/// How long the launcher has to ask the user and quit. Longer than the
/// launcher counts a Yes (`ANSWER_WITHIN`, 150 s in
/// `crates/launcher/src/upgrade_restart.rs`), so once this says the restart
/// didn't happen, no open dialog can still start one.
const RESTART_GRACE: Duration = Duration::from_secs(3 * 60);

/// The current run, bumped by every start and cancel. Held across each
/// change of state, so a run that was cancelled changes nothing.
static RUN: Mutex<u64> = Mutex::new(0);

fn run_lock() -> std::sync::MutexGuard<'static, u64> {
    RUN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Do `f` only if `run` is still the current run.
fn if_current(run: u64, f: impl FnOnce()) -> bool {
    let current = run_lock();
    if *current != run {
        return false;
    }
    f();
    true
}

/// `app.upgrade-restart`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAppUpgradeRestartData {
    /// Restart without waiting for agents to finish their turn.
    #[serde(default)]
    pub now: bool,
}

fn set(d: &DownloadStatus, state: &str, waiting_for: u32, error: Option<String>) {
    update_check::set_download(Some(DownloadStatus { state: state.to_string(), waiting_for, error, ..d.clone() }));
}

/// Agents in the middle of a turn.
fn working_agents() -> u32 {
    crate::backend::blockcontroller::get_all_controllers()
        .values()
        .filter(|c| c.get_runtime_status().turn_active)
        .count() as u32
}

/// The staged release of the latest version, if it is still on disk.
fn staged(status: &AppUpdateStatus) -> Result<(DownloadStatus, UpgradeTarget), String> {
    let d = status
        .download
        .clone()
        .filter(|d| status.latest.as_deref() == Some(d.version.as_str()))
        .ok_or("download the new version first")?;
    if !matches!(d.state.as_str(), "ready" | "waiting") {
        return Err("the new version isn't ready yet".into());
    }
    // A portable's new folder, or a setup.exe install's installer.
    if let Some(folder) = d.folder.as_deref().map(PathBuf::from) {
        if !crate::backend::update_download::is_staged_portable(&folder, &d.version) {
            return Err(format!("{} is gone; download the new version again", folder.display()));
        }
        let launcher = folder.join("agentmux.exe");
        return Ok((d, UpgradeTarget::Launcher(launcher)));
    }
    let installer = PathBuf::from(d.installer.as_deref().ok_or("the new version isn't ready yet")?);
    if !installer.is_file() {
        return Err(format!("{} is gone; download the new version again", installer.display()));
    }
    // The release's digest goes with it: the launcher checks the file it runs.
    let sha256 = status
        .asset
        .as_ref()
        .filter(|a| installer.file_name().is_some_and(|n| n == a.name.as_str()))
        .map(|a| a.sha256.to_ascii_lowercase())
        .ok_or("the new version's checksum is unknown; check for updates again")?;
    Ok((d, UpgradeTarget::Installer { installer, sha256 }))
}

/// Start "Upgrade & restart".
pub fn start(now: bool) -> Result<AppUpdateStatus, String> {
    let mut current = run_lock();
    let (d, target) = staged(&update_check::current_status())?;
    *current += 1;
    let run_id = *current;
    set(&d, "waiting", if now { 0 } else { working_agents() }, None);
    drop(current);
    tokio::spawn(run(run_id, d, target, now));
    Ok(update_check::current_status())
}

/// Stop waiting (`app.upgrade-cancel`). Once the launcher is asking the user,
/// only its dialog can say no.
pub fn cancel() -> Result<AppUpdateStatus, String> {
    let mut current = run_lock();
    let status = update_check::current_status();
    match status.download.as_ref() {
        Some(d) if d.state == "restarting" => {
            Err("AgentMux is asking you to confirm the restart: answer No in its dialog to cancel".into())
        }
        Some(d) if d.state == "waiting" => {
            *current += 1;
            set(d, "ready", 0, None);
            Ok(update_check::current_status())
        }
        _ => Ok(status),
    }
}

async fn run(run_id: u64, d: DownloadStatus, target: UpgradeTarget, now: bool) {
    let deadline = tokio::time::Instant::now() + WAIT_FOR_AGENTS;
    loop {
        let busy = if now { 0 } else { working_agents() };
        if busy == 0 {
            break;
        }
        let still = if tokio::time::Instant::now() >= deadline {
            let error = format!("{busy} agent(s) were still working after 10 minutes. Restart now, or try again later.");
            if_current(run_id, || set(&d, "ready", 0, Some(error)));
            false
        } else {
            if_current(run_id, || set(&d, "waiting", busy, None))
        };
        if !still {
            return;
        }
        tokio::time::sleep(POLL).await;
    }
    let asked = if_current(run_id, || {
        set(&d, "restarting", 0, None);
        tracing::info!(version = %d.version, target = ?target, "upgrade: asking the launcher to restart");
        eprintln!("{}", agentmux_common::srv_stderr::upgrade_line(&d.version, &target));
    });
    if !asked {
        return;
    }
    tokio::time::sleep(RESTART_GRACE).await;
    if_current(run_id, || {
        set(
            &d,
            "ready",
            0,
            Some("AgentMux didn't restart: the upgrade was declined or not answered in time, or this AgentMux can't restart itself. Try again, or install the new version from the release page.".into()),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(download: Option<DownloadStatus>) -> AppUpdateStatus {
        AppUpdateStatus {
            state: "available".into(),
            current: "0.59.26".into(),
            latest: Some("0.59.27".into()),
            url: None,
            notes: None,
            published_at: None,
            checked_ms: 0,
            install: "windows-portable".into(),
            asset: None,
            can_download: true,
            download,
        }
    }

    fn download(state: &str, folder: Option<&std::path::Path>) -> DownloadStatus {
        DownloadStatus {
            version: "0.59.27".into(),
            state: state.into(),
            received: 1,
            total: 1,
            error: None,
            folder: folder.map(|f| f.display().to_string()),
            installer: None,
            waiting_for: 0,
        }
    }

    #[test]
    fn a_cancelled_run_changes_nothing() {
        let run_id = {
            let mut current = run_lock();
            *current += 1;
            *current
        };
        let mut ran = false;
        assert!(if_current(run_id, || ran = true));
        assert!(ran);
        *run_lock() += 1; // a Cancel, or a newer start
        assert!(!if_current(run_id, || panic!("a stale run changed the state")));
    }

    #[test]
    fn only_a_staged_release_that_is_still_there_can_be_restarted_into() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("agentmux-0.59.27-x64-portable");
        assert!(staged(&status(None)).is_err());
        assert!(staged(&status(Some(download("downloading", None)))).is_err());
        // Ready, but the folder was deleted since.
        let err = staged(&status(Some(download("ready", Some(&folder))))).unwrap_err();
        assert!(err.contains("download the new version again"), "{err}");

        std::fs::create_dir_all(folder.join("runtime")).unwrap();
        std::fs::write(folder.join("agentmux.exe"), b"").unwrap();
        std::fs::write(folder.join("runtime/agentmux-portable.marker"), b"").unwrap();
        std::fs::write(folder.join("runtime/agentmux-0.59.27.exe"), b"").unwrap();
        let (d, t) = staged(&status(Some(download("ready", Some(&folder))))).unwrap();
        assert_eq!((d.version.as_str(), t), ("0.59.27", UpgradeTarget::Launcher(folder.join("agentmux.exe"))));

        // A setup.exe install restarts through its downloaded installer.
        let installer = dir.path().join("AgentMux-0.59.27-x64-setup.exe");
        let mut setup = download("ready", None);
        setup.installer = Some(installer.display().to_string());
        assert!(staged(&status(Some(setup.clone()))).unwrap_err().contains("download the new version again"));
        std::fs::write(&installer, b"x").unwrap();
        // Not without the release's digest to check it against.
        assert!(staged(&status(Some(setup.clone()))).unwrap_err().contains("checksum"));
        let mut with_asset = status(Some(setup));
        with_asset.asset = Some(crate::backend::update_check::ReleaseAsset {
            name: "AgentMux-0.59.27-x64-setup.exe".into(),
            size: 1,
            url: String::new(),
            sha256: "AB".repeat(32),
        });
        assert_eq!(staged(&with_asset).unwrap().1, UpgradeTarget::Installer { installer, sha256: "ab".repeat(32) });
        // A download of an older release than the latest doesn't count.
        let mut newer = status(Some(download("ready", Some(&folder))));
        newer.latest = Some("0.59.28".into());
        assert!(staged(&newer).is_err());
    }
}
