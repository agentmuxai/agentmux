// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Upgrade & restart", srv's half
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.1, §8.2,
//! phase 3b).
//!
//! Once a release is staged (`update::download`), the user's click waits for
//! every agent to finish its turn (for as long as that takes, with who is
//! waited for read again every pass; "Now" skips the wait, and Cancel stops
//! it), then srv asks the launcher to restart into the new
//! version with an `AGENTMUXSRV-UPGRADE` line on its stderr. The launcher
//! checks the folder again and asks the user itself
//! (`crates/launcher/src/upgrade_restart.rs`). From then on that dialog is
//! the only place to decide: srv can't reach it, so `restarting` can't be
//! cancelled here (No in the dialog does). If nothing has happened three
//! minutes later (declined, or an AgentMux that can't restart itself), the
//! status goes back to ready and says so.
//!
//! What the wait is for, and nothing else: an agent whose process is alive
//! and which is in the middle of a turn ([`busy_controllers`]). A turn held
//! up by a request that is being retried is still a turn, and the status says
//! so, since such a wait can be long. What agents left running in the
//! background is not waited for: it is listed, and the restart stops it.
//! With nobody to wait for the state is `restarting` at once: `waiting`
//! always names at least one agent.
//!
//! Every change of state happens under [`RUN`], checked against the run it
//! belongs to, so a Cancel can never be overtaken by a run it stopped.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use agentmux_common::srv_stderr::{UpgradeTarget, INSTALL_RESULT_FILE, UPGRADE_DECLINED_FILE};

use crate::backend::blockcontroller::{BlockControllerRuntimeStatus, STATUS_DONE};
use crate::backend::update::check::{self, AppUpdateStatus, DownloadStatus, WaitingOn};

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

/// While the restart waits or is asked for, `stopping` (what agents left
/// running that it stops, [`stopping`]) is listed with it; the launcher's
/// dialog lists it too. Gathered by the caller, outside [`RUN`]: it scans
/// every agent pane's processes.
fn set(d: &DownloadStatus, state: &str, waiting_on: Vec<WaitingOn>, error: Option<String>, stopping: Vec<String>) {
    let stopping = if matches!(state, "waiting" | "restarting") { stopping } else { Vec::new() };
    crate::backend::background_work::write_for_launcher(&stopping);
    check::set_download(Some(with_wait(d, state, waiting_on, error, stopping)));
}

/// `d` in `state`. Only `waiting` says who is waited for, and it always names someone: a wait for nobody is not a
/// wait, so it is said as `restarting`, which is what comes next.
fn with_wait(d: &DownloadStatus, state: &str, waiting_on: Vec<WaitingOn>, error: Option<String>, stopping: Vec<String>) -> DownloadStatus {
    let waiting_on = if state == "waiting" { waiting_on } else { Vec::new() };
    let state = if state == "waiting" && waiting_on.is_empty() { "restarting" } else { state };
    DownloadStatus { state: state.to_string(), waiting_for: waiting_on.len() as u32, waiting_on, error, stopping, ..d.clone() }
}

/// [`stopping`] off the async threads.
pub async fn stopping_now() -> Vec<String> {
    tokio::task::spawn_blocking(stopping).await.unwrap_or_default()
}

/// What agents left running now, a line each.
pub(crate) fn stopping() -> Vec<String> {
    let now = agentmux_common::time::now_ms_u64();
    crate::backend::background_work::running().iter().map(|w| crate::backend::background_work::describe(w, now)).collect()
}

/// A pane's controller, as the wait reads it.
#[derive(Debug, Clone, Default)]
struct Seen {
    block: String,
    kind: String,
    status: BlockControllerRuntimeStatus,
    /// The agent's name; empty for a pane that has none.
    agent: String,
    /// Its CLI is trying a request to the model again now: `Some(None)` when the request got no answer (the
    /// network), `Some(Some(status))` when the API answered with that HTTP status.
    retry: Option<Option<u16>>,
}

/// One agent the restart waits for: what a window is told, and its line for the log (block id, kind, process state).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Busy {
    on: WaitingOn,
    log: String,
}

/// The controllers in the middle of a turn.
///
/// A controller whose process has ended can't be in a turn: a `turn_active` left behind by an agent that stopped
/// or errored mid-turn is not counted, or the restart would wait for something that will never finish. An agent
/// whose request is being retried is still in its turn and is waited for; it says why, since the retries of a
/// request that gets no answer can go on for minutes.
fn busy_controllers(controllers: &[Seen]) -> Vec<Busy> {
    controllers
        .iter()
        .filter(|c| c.status.turn_active && c.status.shellprocstatus != STATUS_DONE)
        .map(|c| {
            let (why, held_up) = match c.retry {
                None => ("turn", ""),
                Some(None) => ("network_retry", ", retrying after no answer from the API"),
                Some(Some(_)) => ("api_retry", ", retrying after an API error"),
            };
            Busy {
                on: WaitingOn { agent: c.agent.clone(), why: why.to_string() },
                log: format!("{} ({}, {}{held_up})", c.block, c.kind, c.status.shellprocstatus),
            }
        })
        .collect()
}

/// Agents in the middle of a turn, read now each time it is asked and never kept.
fn busy_now() -> Vec<Busy> {
    let names: std::collections::HashMap<String, String> =
        crate::backend::reactive::get_global_handler().list_agents().into_iter().map(|a| (a.block_id, a.agent_id)).collect();
    let seen: Vec<Seen> = crate::backend::blockcontroller::get_all_controllers()
        .iter()
        .map(|(block, c)| Seen {
            block: block.clone(),
            kind: c.controller_type().to_string(),
            status: c.get_runtime_status(),
            agent: names.get(block).cloned().unwrap_or_default(),
            retry: crate::backend::agent_api_retry::current_api_retry(block).retry.map(|r| r.error_status),
        })
        .collect();
    busy_controllers(&seen)
}

fn waited_for(busy: &[Busy]) -> Vec<WaitingOn> {
    busy.iter().map(|b| b.on.clone()).collect()
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
    // A portable's new folder, a setup.exe install's installer, or an
    // AppImage's next one.
    if let Some(folder) = d.folder.as_deref().map(PathBuf::from) {
        if !crate::backend::update::download::is_staged_portable(&folder, &d.version) {
            return Err(format!("{} is gone; download the new version again", folder.display()));
        }
        // On Linux the tarball's script, which sets the library path for the
        // new folder's binaries before it starts the launcher.
        let launcher = folder.join(if cfg!(windows) { "agentmux.exe" } else { "agentmux.sh" });
        return Ok((d, UpgradeTarget::Launcher(launcher)));
    }
    let installer = PathBuf::from(d.installer.as_deref().ok_or("the new version isn't ready yet")?);
    if !installer.is_file() {
        return Err(format!("{} is gone; download the new version again", installer.display()));
    }
    // An AppImage is started as it is, like a portable's launcher.
    if status.install == "linux-appimage" {
        return Ok((d, UpgradeTarget::Launcher(installer)));
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
    let (d, target) = staged(&check::current_status())?;
    *current += 1;
    let run_id = *current;
    // With nobody to wait for (or "Now") this says `restarting` at once: the launcher is asked a moment later.
    // What the restart stops follows from `run`, off the async threads.
    set(&d, "waiting", if now { Vec::new() } else { waited_for(&busy_now()) }, None, Vec::new());
    drop(current);
    tokio::spawn(run(run_id, d, target, now));
    Ok(check::current_status())
}

/// Stop waiting (the user's Cancel, through the host). Once the launcher is asking the user,
/// only its dialog can say no.
pub fn cancel() -> Result<AppUpdateStatus, String> {
    let mut current = run_lock();
    let status = check::current_status();
    match status.download.as_ref() {
        Some(d) if d.state == "restarting" => {
            Err("AgentMux is asking you to confirm the restart: answer No in its dialog to cancel".into())
        }
        Some(d) if d.state == "waiting" => {
            *current += 1;
            set(d, "ready", Vec::new(), None, Vec::new());
            Ok(check::current_status())
        }
        _ => Ok(status),
    }
}

/// Where the launcher leaves a No (`updates/` of this version's data folder).
fn declined_file() -> Option<PathBuf> {
    std::env::var_os("AGENTMUX_DATA_DIR")
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("updates").join(UPGRADE_DECLINED_FILE))
}

/// Why a `.deb`/`.rpm` upgrade to `version` didn't happen, as the launcher
/// left it in `updates/<version>/` ([`INSTALL_RESULT_FILE`]): the password was
/// refused or couldn't be asked for, the file failed its checks, or the install
/// failed. Taken out as it is read, and ignored when it is for another version
/// or a day old, so an old attempt can't speak for a new one.
pub(crate) fn take_install_failure(updates: &Path, version: &str) -> Option<String> {
    let file = updates.join(version).join(INSTALL_RESULT_FILE);
    let fresh = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(24 * 60 * 60));
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    // The file is the user's to write: one line, so a stray process can't put a paragraph in the panel.
    let error: String = json.get("error")?.as_str()?.lines().next().unwrap_or("").trim().chars().take(300).collect();
    (fresh && json.get("version")?.as_str()? == version && !error.is_empty()).then(|| format!("The install didn't finish: {error}"))
}

/// Whether the launcher answered No (or the question lapsed) for `version`:
/// `file` names it. Each request clears the file before it asks and each answer
/// is taken out as it is read, so one left by an earlier attempt can't answer a
/// new question, however soon the user tries again.
fn take_declined(file: &Path, version: &str) -> bool {
    if std::fs::read_to_string(file).map(|v| v.trim() == version).unwrap_or(false) {
        let _ = std::fs::remove_file(file);
        return true;
    }
    false
}

async fn run(run_id: u64, d: DownloadStatus, target: UpgradeTarget, now: bool) {
    // Waits for every agent to finish its turn for as long as that takes. It ends when nobody is in one, with
    // "Now", or when the user cancels (which supersedes this run). Who is waited for is read again each pass,
    // and logged whenever that changes, so a wait that doesn't end can be explained.
    let mut last_busy: Option<Vec<String>> = None;
    loop {
        let busy = if now { Vec::new() } else { busy_now() };
        if busy.is_empty() {
            break;
        }
        let lines: Vec<String> = busy.iter().map(|b| b.log.clone()).collect();
        if last_busy.as_ref() != Some(&lines) {
            tracing::info!(version = %d.version, busy = ?lines, "upgrade: waiting for agents to finish their turn");
            last_busy = Some(lines);
        }
        let listed = stopping_now().await;
        if !if_current(run_id, || set(&d, "waiting", waited_for(&busy), None, listed)) {
            return;
        }
        tokio::time::sleep(POLL).await;
    }
    let listed = stopping_now().await;
    let declined = declined_file();
    let asked = if_current(run_id, || {
        // In the same guarded step that asks, so a run that was superseded while it
        // listed what it stops can't remove a newer run's answer. Before the
        // launcher is asked: an answer from an earlier attempt isn't this one's.
        if let Some(file) = declined.as_deref() {
            let _ = std::fs::remove_file(file);
        }
        set(&d, "restarting", Vec::new(), None, listed);
        tracing::info!(version = %d.version, target = ?target, "upgrade: asking the launcher to restart");
        eprintln!("{}", agentmux_common::srv_stderr::upgrade_line(&d.version, &target));
    });
    if !asked {
        return;
    }
    // The launcher is asking the user. A No (or a question that lapsed) comes
    // back as a file, so the panel stops saying "Restarting" at once; with no
    // answer at all the grace runs out.
    let grace_ends = tokio::time::Instant::now() + RESTART_GRACE;
    while tokio::time::Instant::now() < grace_ends {
        tokio::time::sleep(POLL).await;
        if !if_current(run_id, || {}) {
            return;
        }
        // A `.deb`/`.rpm` upgrade the launcher couldn't start: it says why.
        if let Some(why) = take_install_failure(&crate::backend::update::download::updates_root(), &d.version) {
            tracing::info!(version = %d.version, %why, "upgrade: the package install didn't start");
            if_current(run_id, || set(&d, "ready", Vec::new(), Some(why), Vec::new()));
            return;
        }
        if declined.as_deref().is_some_and(|f| take_declined(f, &d.version)) {
            tracing::info!(version = %d.version, "upgrade: the restart was declined");
            if_current(run_id, || set(&d, "ready", Vec::new(), None, Vec::new()));
            return;
        }
    }
    if_current(run_id, || {
        set(
            &d,
            "ready",
            Vec::new(),
            Some("AgentMux didn't restart: the upgrade was declined or not answered in time, or this AgentMux can't restart itself. Try again, or install the new version from the release page.".into()),
            Vec::new(),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller(turn_active: bool, process: &str) -> BlockControllerRuntimeStatus {
        BlockControllerRuntimeStatus { turn_active, shellprocstatus: process.to_string(), ..Default::default() }
    }

    fn seen(list: &[(&str, &str, BlockControllerRuntimeStatus)]) -> Vec<Seen> {
        list.iter()
            .map(|(block, kind, status)| Seen { block: block.to_string(), kind: kind.to_string(), status: status.clone(), ..Default::default() })
            .collect()
    }

    /// The log's lines for who is waited for.
    fn busy(list: &[(&str, &str, BlockControllerRuntimeStatus)]) -> Vec<String> {
        busy_controllers(&seen(list)).into_iter().map(|b| b.log).collect()
    }

    fn agent(name: &str, turn_active: bool, retry: Option<Option<u16>>) -> Seen {
        Seen { block: format!("b-{name}"), kind: "persistent".into(), status: controller(turn_active, "running"), agent: name.into(), retry }
    }

    fn on(agent: &str, why: &str) -> WaitingOn {
        WaitingOn { agent: agent.into(), why: why.into() }
    }

    #[test]
    fn an_agent_retrying_a_request_is_waited_for_and_says_what_holds_it_up() {
        let list = [
            agent("Vmer", true, Some(None)),
            agent("Deke", true, Some(Some(529))),
            agent("Posa", true, None),
            // A retry left from a turn that has ended holds nothing up.
            agent("Idle", false, Some(None)),
        ];
        let busy = busy_controllers(&list);
        assert_eq!(waited_for(&busy), [on("Vmer", "network_retry"), on("Deke", "api_retry"), on("Posa", "turn")]);
        assert_eq!(busy[0].log, "b-Vmer (persistent, running, retrying after no answer from the API)");
        assert_eq!(busy[1].log, "b-Deke (persistent, running, retrying after an API error)");
        assert_eq!(busy[2].log, "b-Posa (persistent, running)");
        // Its process ended mid-retry: nothing is left to finish.
        let mut gone = agent("Vmer", true, Some(None));
        gone.status.shellprocstatus = "done".into();
        assert!(busy_controllers(&[gone]).is_empty());
    }

    #[test]
    fn a_wait_always_names_someone_and_a_wait_for_nobody_is_the_restart() {
        let d = download("ready", None);
        let waiting = with_wait(&d, "waiting", vec![on("Vmer", "turn"), on("Deke", "network_retry")], None, vec!["Vmer: `task dev`".into()]);
        assert_eq!((waiting.state.as_str(), waiting.waiting_for), ("waiting", 2));
        assert_eq!(waiting.waiting_on, [on("Vmer", "turn"), on("Deke", "network_retry")]);
        assert_eq!(waiting.stopping, ["Vmer: `task dev`"]);
        // Nobody in a turn, or "Now": never `waiting` with a count of 0.
        let nobody = with_wait(&d, "waiting", Vec::new(), None, Vec::new());
        assert_eq!((nobody.state.as_str(), nobody.waiting_for, nobody.waiting_on.len()), ("restarting", 0, 0));
        // Who was waited for is not carried into the states after the wait.
        for state in ["restarting", "ready"] {
            let after = with_wait(&waiting, state, vec![on("Vmer", "turn")], None, Vec::new());
            assert_eq!((after.state.as_str(), after.waiting_for, after.waiting_on.len()), (state, 0, 0));
        }
    }

    #[test]
    fn only_a_controller_with_a_live_process_in_a_turn_is_waited_for() {
        let list = [
            ("b-working", "persistent", controller(true, "running")),
            ("b-starting", "acp", controller(true, "init")),
            ("b-idle", "persistent", controller(false, "running")),
            // A turn flag left behind by an agent that stopped or errored mid-turn.
            ("b-stuck", "persistent", controller(true, "done")),
        ];
        assert_eq!(busy(&list), ["b-working (persistent, running)", "b-starting (acp, init)"]);
    }

    #[test]
    fn nobody_in_a_turn_means_nothing_to_wait_for() {
        let list = [("b-idle", "persistent", controller(false, "running")), ("b-done", "subprocess", controller(false, "done"))];
        assert!(busy(&list).is_empty());
        assert!(busy(&[]).is_empty());
    }

    #[test]
    fn the_count_follows_the_controllers_each_time_it_is_asked() {
        let mut list = vec![("b1", "persistent", controller(true, "running")), ("b2", "persistent", controller(true, "running"))];
        assert_eq!(busy(&list).len(), 2);
        list[0].2.turn_active = false;
        assert_eq!(busy(&list), ["b2 (persistent, running)"]);
        list[1].2.shellprocstatus = "done".into();
        assert!(busy(&list).is_empty(), "an agent whose process ended is no longer waited for");
    }

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
            install_first: None,
            rev: 0,
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
            waiting_on: Vec::new(),
            stopping: Vec::new(),
        }
    }

    #[test]
    fn an_install_failure_is_read_once_for_its_version_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("0.60.2");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join(INSTALL_RESULT_FILE);
        assert_eq!(take_install_failure(dir.path(), "0.60.2"), None, "nothing left");
        std::fs::write(&file, r#"{"version": "0.60.2", "error": "dnf couldn't install the package"}"#).unwrap();
        assert_eq!(take_install_failure(dir.path(), "0.60.1"), None, "another version has no folder");
        assert_eq!(
            take_install_failure(dir.path(), "0.60.2").as_deref(),
            Some("The install didn't finish: dnf couldn't install the package")
        );
        assert!(!file.exists(), "taken out as it is read");
        // For another version than its folder's, or without a reason: nothing to say.
        std::fs::write(&file, r#"{"version": "0.60.9", "error": "x"}"#).unwrap();
        assert_eq!(take_install_failure(dir.path(), "0.60.2"), None);
        std::fs::write(&file, r#"{"version": "0.60.2", "error": " "}"#).unwrap();
        assert_eq!(take_install_failure(dir.path(), "0.60.2"), None);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(take_install_failure(dir.path(), "0.60.2"), None);
        // The file is the user's to write: the panel gets one short line of it.
        let long = format!("{}\nsecond line", "x".repeat(500));
        std::fs::write(&file, serde_json::json!({ "version": "0.60.2", "error": long }).to_string()).unwrap();
        let said = take_install_failure(dir.path(), "0.60.2").unwrap();
        assert_eq!(said, format!("The install didn't finish: {}", "x".repeat(300)));
    }

    #[test]
    fn a_no_is_taken_once_and_only_for_the_version_asked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(UPGRADE_DECLINED_FILE);
        assert!(!take_declined(&file, "0.59.37"), "no answer yet");
        std::fs::write(&file, "0.59.36
").unwrap();
        assert!(!take_declined(&file, "0.59.37"), "an answer about another version");
        assert!(file.exists(), "...is left alone");
        std::fs::write(&file, "0.59.37
").unwrap();
        assert!(take_declined(&file, "0.59.37"));
        assert!(!file.exists(), "taken out as it is read");
        assert!(!take_declined(&file, "0.59.37"), "so it can't answer a second question");
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

        crate::backend::update::download::fake_portable(&folder, "0.59.27");
        let (d, t) = staged(&status(Some(download("ready", Some(&folder))))).unwrap();
        let launcher = folder.join(if cfg!(windows) { "agentmux.exe" } else { "agentmux.sh" });
        assert_eq!((d.version.as_str(), t), ("0.59.27", UpgradeTarget::Launcher(launcher)));

        // A setup.exe install restarts through its downloaded installer.
        let installer = dir.path().join("AgentMux-0.59.27-x64-setup.exe");
        let mut setup = download("ready", None);
        setup.installer = Some(installer.display().to_string());
        assert!(staged(&status(Some(setup.clone()))).unwrap_err().contains("download the new version again"));
        std::fs::write(&installer, b"x").unwrap();
        // Not without the release's digest to check it against.
        assert!(staged(&status(Some(setup.clone()))).unwrap_err().contains("checksum"));
        let mut with_asset = status(Some(setup));
        with_asset.asset = Some(crate::backend::update::check::ReleaseAsset {
            name: "AgentMux-0.59.27-x64-setup.exe".into(),
            size: 1,
            url: String::new(),
            sha256: "AB".repeat(32),
        });
        assert_eq!(staged(&with_asset).unwrap().1, UpgradeTarget::Installer { installer: installer.clone(), sha256: "ab".repeat(32) });
        // An AppImage's next one is started as it is.
        let mut appimage = with_asset.clone();
        appimage.install = "linux-appimage".into();
        assert_eq!(staged(&appimage).unwrap().1, UpgradeTarget::Launcher(installer));
        // A macOS app restarts through its staged .dmg, with the release's digest.
        let dmg = dir.path().join("AgentMux_0.59.27_arm64.dmg");
        std::fs::write(&dmg, b"x").unwrap();
        let mut mac = download("ready", None);
        mac.installer = Some(dmg.display().to_string());
        let mut mac = status(Some(mac));
        mac.install = "macos-app".into();
        assert!(staged(&mac).unwrap_err().contains("checksum"));
        mac.asset = Some(crate::backend::update::check::ReleaseAsset {
            name: "AgentMux_0.59.27_arm64.dmg".into(),
            size: 1,
            url: String::new(),
            sha256: "CD".repeat(32),
        });
        let (d, target) = staged(&mac).unwrap();
        assert_eq!(target, UpgradeTarget::Installer { installer: dmg.clone(), sha256: "cd".repeat(32) });
        assert_eq!(
            agentmux_common::srv_stderr::upgrade_line(&d.version, &target),
            format!("AGENTMUXSRV-UPGRADE version:0.59.27 sha256:{} installer:{}", "cd".repeat(32), dmg.display())
        );
        // A download of an older release than the latest doesn't count.
        let mut newer = status(Some(download("ready", Some(&folder))));
        newer.latest = Some("0.59.28".into());
        assert!(staged(&newer).is_err());
    }
}
