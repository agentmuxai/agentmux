// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Upgrade & restart", srv's half
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.1, §8.2,
//! phase 3b).
//!
//! Once a release is staged (`update::download`), a restart that is decided (the user's click or own turn, or
//! an approved request whose in-app countdown ran out with no veto) waits for
//! every agent to finish its turn (for as long as that takes, with who is
//! waited for read again every pass; "Now" skips the wait, and Cancel stops
//! it), then srv asks the launcher to restart into the new
//! version with an `AGENTMUXSRV-UPGRADE ... confirmed:1` line on its stderr. The launcher
//! checks the folder and the release's digest again and, the restart being decided, restarts
//! without asking again (`crates/launcher/src/upgrade_restart.rs`); `restarting` can't be
//! cancelled from here. If nothing has happened three
//! minutes later (the launcher refused the file, or an AgentMux that can't restart itself, or a
//! prompt it still had to show was declined), the
//! status goes back to ready and says so: when and why the restart wasn't
//! confirmed (`not_confirmed_at`, `not_confirmed`), and the agents that asked
//! are told. The launcher says why in its answer file and its own log.
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
/// How long the launcher has to quit and restart (or, for a prompt it still has to show, to ask the user). Longer
/// than the launcher counts a Yes (`ANSWER_WITHIN`, 150 s in `crates/launcher/src/upgrade_restart.rs`), so once
/// this says the restart didn't happen, no open prompt can still start one.
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
    // A new wait, restart or ready state is not the last attempt's "not confirmed": it goes with it.
    DownloadStatus {
        state: state.to_string(),
        waiting_for: waiting_on.len() as u32,
        waiting_on,
        error,
        stopping,
        not_confirmed_at: None,
        not_confirmed: None,
        ..d.clone()
    }
}

/// The reason word a restart that nothing answered carries (no launcher answer in time).
const UNANSWERED: &str = "unanswered";

/// What happened, in words, for `why` (a `not_confirmed` word).
fn why_words(why: &str) -> &'static str {
    match why {
        "no" => "the user answered No",
        "timed_out" => "nobody answered the prompt before it closed itself",
        "too_late" => "an answer came after the prompt had lapsed",
        UNANSWERED => "the launcher didn't restart AgentMux in time (it may have refused the file, or this AgentMux can't restart itself)",
        _ => "the restart was declined",
    }
}

/// `ms` (Unix) as "16:33 UTC".
fn clock(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).map_or_else(|| "an unknown time".into(), |t| t.format("%H:%M UTC").to_string())
}

/// What the version panel and the status say after a restart that was not confirmed.
fn not_confirmed_line(why: &str, at_ms: u64) -> String {
    format!(
        "The restart didn't go ahead at {}: {}. The release is downloaded; start the upgrade again from the version panel.",
        clock(at_ms),
        why_words(why)
    )
}

/// What an agent that asked for the upgrade is told.
fn not_confirmed_note(version: &str, why: &str, at_ms: u64) -> String {
    format!(
        "[AgentMux] The upgrade to {version} did not happen: the restart didn't go ahead at {}: {}. The release is downloaded and waiting. Ask again if it's still wanted.",
        clock(at_ms),
        why_words(why)
    )
}

/// The restart the launcher was asked for did not happen: back to `ready`, with why and when, and the agents that
/// asked told. Only if `run` is still the current run.
fn not_confirmed(run: u64, d: &DownloadStatus, why: &str, askers: &[String]) {
    let at = agentmux_common::time::now_ms_u64();
    let line = not_confirmed_line(why, at);
    let marked = if_current(run, || {
        crate::backend::background_work::write_for_launcher(&[]);
        check::set_download(Some(DownloadStatus {
            not_confirmed_at: Some(at),
            not_confirmed: Some(why.to_string()),
            ..with_wait(d, "ready", Vec::new(), Some(line.clone()), Vec::new())
        }));
    });
    if !marked {
        return;
    }
    tracing::info!(version = %d.version, why, "upgrade: the restart wasn't confirmed");
    let note = not_confirmed_note(&d.version, why, at);
    for block_id in askers {
        crate::backend::upgrade_request::tell(block_id, &note);
    }
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

/// Start "Upgrade & restart". `askers` are the panes of the agents that asked for it: they are told if the restart
/// is not confirmed (the user's own click has none).
pub fn start(now: bool, askers: Vec<String>) -> Result<AppUpdateStatus, String> {
    let mut current = run_lock();
    let (d, target) = staged(&check::current_status())?;
    *current += 1;
    let run_id = *current;
    // With nobody to wait for (or "Now") this says `restarting` at once: the launcher is asked a moment later.
    // What the restart stops follows from `run`, off the async threads.
    set(&d, "waiting", if now { Vec::new() } else { waited_for(&busy_now()) }, None, Vec::new());
    drop(current);
    tokio::spawn(run(run_id, d, target, now, askers));
    Ok(check::current_status())
}

/// Stop waiting (the user's Cancel, through the host). Once the launcher is asked, the restart has started and
/// can't be cancelled here.
pub fn cancel() -> Result<AppUpdateStatus, String> {
    let mut current = run_lock();
    let status = check::current_status();
    match status.download.as_ref() {
        Some(d) if d.state == "restarting" => {
            Err("AgentMux has started to restart into the new version; it can't be cancelled now".into())
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

/// Why the launcher answered No (or the question lapsed) for `version`, as a `not_confirmed` word, if `file` names
/// it. Each request clears the file before it asks and each answer is taken out as it is read, so one left by an
/// earlier attempt can't answer a new question, however soon the user tries again. A file from a launcher that
/// gives no reason says `declined`.
fn take_declined(file: &Path, version: &str) -> Option<&'static str> {
    let text = std::fs::read_to_string(file).ok()?;
    let (named, why) = agentmux_common::srv_stderr::parse_declined(&text);
    if named != version {
        return None;
    }
    let _ = std::fs::remove_file(file);
    Some(why.map_or("declined", |w| w.as_str()))
}

async fn run(run_id: u64, d: DownloadStatus, target: UpgradeTarget, now: bool, askers: Vec<String>) {
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
        // Asked only once the restart is decided: the user's own click or turn, or an approved request's in-app
        // countdown that ran out with no veto. The launcher asks nothing more.
        eprintln!("{}", agentmux_common::srv_stderr::upgrade_line(&d.version, &target, true));
    });
    if !asked {
        return;
    }
    // The launcher is told to go ahead. It asks only about a file it refuses or a
    // package install's own permission prompt; a No to that (or a question that
    // lapsed) comes back as a file, so the panel stops saying "Restarting" at
    // once; with no answer at all the grace runs out.
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
        if let Some(why) = declined.as_deref().and_then(|f| take_declined(f, &d.version)) {
            tracing::info!(version = %d.version, why, "upgrade: the restart was declined");
            not_confirmed(run_id, &d, why, &askers);
            return;
        }
    }
    // Nothing came back: say so the same way, with its own reason.
    not_confirmed(run_id, &d, UNANSWERED, &askers);
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::srv_stderr::{declined_text, Declined};

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
            download_blocked: None,
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
            not_confirmed_at: None,
            not_confirmed: None,
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
        assert_eq!(take_declined(&file, "0.59.37"), None, "no answer yet");
        std::fs::write(&file, "0.59.36\n").unwrap();
        assert_eq!(take_declined(&file, "0.59.37"), None, "an answer about another version");
        assert!(file.exists(), "...is left alone");
        // From a launcher that gives no reason.
        std::fs::write(&file, "0.59.37\n").unwrap();
        assert_eq!(take_declined(&file, "0.59.37"), Some("declined"));
        assert!(!file.exists(), "taken out as it is read");
        assert_eq!(take_declined(&file, "0.59.37"), None, "so it can't answer a second question");
        // With the reason the launcher left.
        for why in [Declined::No, Declined::TimedOut, Declined::TooLate] {
            std::fs::write(&file, declined_text("0.59.37", why)).unwrap();
            assert_eq!(take_declined(&file, "0.59.37"), Some(why.as_str()));
        }
    }

    #[test]
    fn a_restart_nobody_confirmed_says_so_with_why_and_when_and_clears_with_the_next_attempt() {
        // 2026-11-19 16:33:41 UTC.
        let at = 1_795_106_021_000u64;
        for (why, words) in [
            ("no", "answered No"),
            ("timed_out", "nobody answered"),
            ("too_late", "had lapsed"),
            (UNANSWERED, "didn't restart AgentMux in time"),
            ("declined", "was declined"),
        ] {
            let line = not_confirmed_line(why, at);
            assert!(line.contains("16:33 UTC") && line.contains(words) && !line.contains("150"), "{why}: {line}");
            let note = not_confirmed_note("0.59.35", why, at);
            assert!(note.starts_with("[AgentMux] The upgrade to 0.59.35 did not happen") && note.contains("16:33 UTC") && note.contains(words) && !note.contains("150"), "{why}: {note}");
        }
        // It is on the status as a word and a time (what a remote answer carries), and goes with the next wait or restart.
        let d = download("ready", None);
        assert_eq!((d.not_confirmed_at, d.not_confirmed.clone()), (None, None));
        let marked = DownloadStatus { not_confirmed_at: Some(at), not_confirmed: Some("timed_out".into()), ..d.clone() };
        for state in ["waiting", "restarting", "ready"] {
            let next = with_wait(&marked, state, vec![on("Vmer", "turn")], None, Vec::new());
            assert_eq!((next.not_confirmed_at, next.not_confirmed), (None, None), "{state}");
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
            agentmux_common::srv_stderr::upgrade_line(&d.version, &target, true),
            format!("AGENTMUXSRV-UPGRADE version:0.59.27 confirmed:1 sha256:{} installer:{}", "cd".repeat(32), dmg.display())
        );
        // A download of an older release than the latest doesn't count.
        let mut newer = status(Some(download("ready", Some(&folder))));
        newer.latest = Some("0.59.28".into());
        assert!(staged(&newer).is_err());
    }
}
