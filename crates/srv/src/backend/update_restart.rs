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
//! (`crates/launcher/src/upgrade_restart.rs`); if nothing has happened
//! three minutes later (declined, or an AgentMux that can't restart itself),
//! the status goes back to ready and says so.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::update_check::{self, AppUpdateStatus, DownloadStatus};

const WAIT_FOR_AGENTS: Duration = Duration::from_secs(10 * 60);
const POLL: Duration = Duration::from_secs(2);
/// How long the launcher has to ask the user and quit.
const RESTART_GRACE: Duration = Duration::from_secs(3 * 60);

/// Bumped by every start and cancel: a waiting run that sees it change stops.
static GENERATION: AtomicU64 = AtomicU64::new(0);

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
fn staged(status: &AppUpdateStatus) -> Result<(DownloadStatus, PathBuf), String> {
    let d = status
        .download
        .clone()
        .filter(|d| status.latest.as_deref() == Some(d.version.as_str()))
        .ok_or("download the new version first")?;
    if !matches!(d.state.as_str(), "ready" | "waiting") {
        return Err("the new version isn't ready yet".into());
    }
    let folder = PathBuf::from(d.folder.as_deref().ok_or("the new version isn't ready yet")?);
    if !crate::backend::update_download::is_staged_portable(&folder, &d.version) {
        return Err(format!("{} is gone; download the new version again", folder.display()));
    }
    Ok((d, folder))
}

/// Start "Upgrade & restart".
pub fn start(now: bool) -> Result<AppUpdateStatus, String> {
    let (d, folder) = staged(&update_check::current_status())?;
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    set(&d, "waiting", if now { 0 } else { working_agents() }, None);
    tokio::spawn(run(generation, d, folder, now));
    Ok(update_check::current_status())
}

/// Stop waiting (`app.upgrade-cancel`).
pub fn cancel() -> AppUpdateStatus {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    let status = update_check::current_status();
    if let Some(d) = status.download.as_ref().filter(|d| d.state == "waiting" || d.state == "restarting") {
        set(d, "ready", 0, None);
    }
    update_check::current_status()
}

async fn run(generation: u64, d: DownloadStatus, folder: PathBuf, now: bool) {
    let current = || GENERATION.load(Ordering::SeqCst) == generation;
    let deadline = tokio::time::Instant::now() + WAIT_FOR_AGENTS;
    loop {
        if !current() {
            return;
        }
        let busy = if now { 0 } else { working_agents() };
        if busy == 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let error = format!("{busy} agent(s) were still working after 10 minutes. Restart now, or try again later.");
            set(&d, "ready", 0, Some(error));
            return;
        }
        set(&d, "waiting", busy, None);
        tokio::time::sleep(POLL).await;
    }
    set(&d, "restarting", 0, None);
    tracing::info!(version = %d.version, folder = %folder.display(), "upgrade: asking the launcher to restart");
    eprintln!("{}", agentmux_common::srv_stderr::upgrade_line(&d.version, &folder.join("agentmux.exe")));
    tokio::time::sleep(RESTART_GRACE).await;
    if current() {
        set(
            &d,
            "ready",
            0,
            Some("AgentMux didn't restart: the upgrade was declined, or this AgentMux can't restart itself. You can start the new version from its folder.".into()),
        );
    }
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
            waiting_for: 0,
        }
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
        let (d, f) = staged(&status(Some(download("ready", Some(&folder))))).unwrap();
        assert_eq!((d.version.as_str(), f), ("0.59.27", folder.clone()));
        // A download of an older release than the latest doesn't count.
        let mut newer = status(Some(download("ready", Some(&folder))));
        newer.latest = Some("0.59.28".into());
        assert!(staged(&newer).is_err());
    }
}
