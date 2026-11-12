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
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::store_progress::{self, Progress};

/// How long the Store gets to list the updates before it counts as not answering.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a download may take at most: it legitimately takes minutes, and the Store reports as it goes,
/// so this is only the end of a download that never finishes.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(60 * 60);

/// How long the install call gets to finish by itself once everything is downloaded. A packaged desktop app
/// that is running is not closed by the Store: it stages the package, marks the install "blocked on the
/// user" and waits for the app to be closed, so the call stays open. After this long that is the answer
/// (`staged`), and the page stops saying it is asking the Store.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const INSTALL_GRACE: Duration = Duration::from_secs(20);

/// Sends a progress report to every window.
pub type Emit = Arc<dyn Fn(&Progress) + Send + Sync>;

/// The reason a call gives when the Store never answered; `refused` and `decide_check` turn it into
/// their own outcome rather than quoting it.
pub const NO_ANSWER: &str = "the Microsoft Store did not answer";

/// What the install call did in the time it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// It finished, with this overall state.
    Finished(StoreState),
    /// It is still open: the Store has the package and is waiting for AgentMux to be closed.
    WaitingForClose,
}

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
    /// `installed`, `staged` (downloaded and waiting for AgentMux to be closed), `canceled`, `none` (the Store
    /// has no update yet), or `fallback` (open the Store page).
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
/// missing, a call failed, or it never answered): the page opens the Store page instead, and says so.
pub fn refused(why: &str) -> StoreOutcome {
    if why == NO_ANSWER {
        return StoreOutcome::new("fallback", "The Microsoft Store didn't answer in time.");
    }
    StoreOutcome::new("fallback", format!("Couldn't ask the Store to install it ({why})."))
}

/// The decision, once the Store answered: how many updates it listed for this
/// app, and, when it was asked to install them, the overall state.
pub fn decide(updates_listed: usize, installed: Option<StoreState>) -> StoreOutcome {
    if updates_listed == 0 {
        return StoreOutcome::new("none", "The Microsoft Store has no newer version.");
    }
    match installed {
        None => refused("the Store listed an update but wasn't asked to install it"),
        Some(StoreState::Completed) => StoreOutcome::new("installed", "The Store installed the update. Open AgentMux again to use it."),
        Some(StoreState::Canceled) => StoreOutcome::new("canceled", "The update was cancelled."),
        Some(StoreState::LowBattery) => StoreOutcome::new("canceled", "The Store won't install it on a low battery. Plug in and try again."),
        Some(StoreState::WifiRequired) => StoreOutcome::new("canceled", "The Store needs Wi-Fi for this update. Connect and try again."),
        Some(StoreState::WifiRecommended) => StoreOutcome::new("canceled", "The Store recommends Wi-Fi for this update. Connect and try again."),
        Some(StoreState::Other) => refused("the Store reported an error"),
    }
}

/// The decision for the install call, once it finished or was given up on waiting for.
pub fn decide_install(updates_listed: usize, install: Install) -> StoreOutcome {
    match install {
        Install::Finished(state) => decide(updates_listed, Some(state)),
        Install::WaitingForClose => StoreOutcome::new("staged", "Downloaded. Close AgentMux to finish the update, or use Restart now."),
    }
}

/// What the Store itself says about an update, without installing anything: the
/// answer the page needs to decide whether to offer "Update in the Store".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoreCheck {
    /// `available` (the Store lists an update), `none` (it lists none yet), `no_answer` (it was asked
    /// and never answered), or `unknown` (it could not be asked: not a packaged install, offline, or
    /// the call failed).
    pub state: &'static str,
    /// Why, when the state is `unknown`; empty otherwise.
    pub message: String,
    /// When `available`: the version the listed item's package carries (`0.60.17`); empty when it had
    /// none readable. Windows documents that package as "the package that has an update available", so
    /// it can be the installed version and not the update's: the page names a version only when it is
    /// newer than this build, and offers the update either way.
    pub version: String,
}

impl StoreCheck {
    fn new(state: &'static str, message: String, version: String) -> Self {
        Self { state, message, version }
    }
}

/// `major.minor.build` of a package version, as the release's version reads (the fourth part is
/// reserved by the Store and always 0).
fn version_text((major, minor, build): (u16, u16, u16)) -> String {
    format!("{major}.{minor}.{build}")
}

/// The decision for a check: the versions of the updates the Store listed, or why it
/// could not be asked. An unreachable Store is `unknown`, never guessed from GitHub.
/// The offered version is the highest one listed (`None`: listed, but unreadable).
pub fn decide_check(listed: Result<Vec<Option<(u16, u16, u16)>>, String>) -> StoreCheck {
    match listed {
        Ok(updates) if updates.is_empty() => StoreCheck::new("none", String::new(), String::new()),
        Ok(updates) => {
            let version = updates.into_iter().flatten().max().map(version_text).unwrap_or_default();
            StoreCheck::new("available", String::new(), version)
        }
        Err(why) if why == NO_ANSWER => StoreCheck::new("no_answer", String::new(), String::new()),
        Err(why) => StoreCheck::new("unknown", why, String::new()),
    }
}

/// The one log line per check: what the Store answered and how long it took, so a report of "the Store
/// says no newer version" can be checked against the log afterwards.
pub fn check_log_line(check: &StoreCheck, took: Duration) -> String {
    let version = if check.version.is_empty() { "-" } else { &check.version };
    let mut line = format!("[store-check] state={} version={version} took_ms={}", check.state, took.as_millis());
    if !check.message.is_empty() {
        line.push_str(&format!(" why={}", check.message));
    }
    line
}

/// Ask the Store whether it has an update for AgentMux, without installing it.
/// Blocks on the Store's answer, so call it off the async workers.
#[cfg(target_os = "windows")]
pub fn check_update() -> StoreCheck {
    decide_check(on_winrt_thread("store-check", || list_updates().map(|(_, versions)| versions)).and_then(|r| r))
}

/// Only Windows has a Store.
#[cfg(not(target_os = "windows"))]
pub fn check_update() -> StoreCheck {
    decide_check(Err("the Microsoft Store is Windows only".into()))
}

/// Run `f` on a thread that initialised WinRT (as for the Windows Hello prompt
/// in vault.rs); a panic or a failed spawn comes back as an `Err` string.
#[cfg(target_os = "windows")]
pub(super) fn on_winrt_thread<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let initialised = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
        let out = f();
        if initialised {
            unsafe { RoUninitialize() };
        }
        out
    });
    match spawned.map(|h| h.join()) {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(_)) => Err("the Store call stopped unexpectedly".into()),
        Err(e) => Err(format!("couldn't start it: {e}")),
    }
}

/// Poll `done` every `step` until it is true (`true`) or `limit` has passed (`false`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn wait_until(limit: Duration, step: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(step);
    }
    true
}

/// The Store's answer to `op`, or `NO_ANSWER` (and the call cancelled) if it has not come within `limit`.
#[cfg(target_os = "windows")]
fn answer_within<T: windows::core::RuntimeType + 'static>(op: windows_future::IAsyncOperation<T>, limit: Duration) -> Result<T, String> {
    use windows_future::AsyncStatus;
    let started = || op.Status().map_or(false, |s| s == AsyncStatus::Started);
    if wait_until(limit, Duration::from_millis(100), || !started()) {
        return op.get().map_err(|e| e.to_string());
    }
    let _ = op.Cancel();
    Err(NO_ANSWER.to_string())
}

/// A failed or unanswered listing, as the reason a call gives: the no-answer marker stays itself.
#[cfg(target_os = "windows")]
fn list_error(e: String) -> String {
    if e == NO_ANSWER { e } else { format!("couldn't list updates: {e}") }
}

/// The version one listed update's package carries, when the Store gives one (see `StoreCheck::version`:
/// it can be the installed package's).
#[cfg(target_os = "windows")]
fn update_version(update: &windows::Services::Store::StorePackageUpdate) -> Option<(u16, u16, u16)> {
    let v = update.Package().ok()?.Id().ok()?.Version().ok()?;
    Some((v.Major, v.Minor, v.Build))
}

/// The Store's context (our window attached) and the version of each update it lists for this app.
#[cfg(target_os = "windows")]
fn list_updates() -> Result<(windows::Services::Store::StoreContext, Vec<Option<(u16, u16, u16)>>), String> {
    use windows::core::Interface;
    use windows::Services::Store::StoreContext;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::IInitializeWithWindow;

    let context = StoreContext::GetDefault().map_err(|e| format!("no Store context: {e}"))?;
    // A desktop app has to say which window the Store's own prompts belong to.
    if let Some(hwnd) = super::vault::foreground_own_window() {
        let init: IInitializeWithWindow = context.cast().map_err(|e| format!("no window interop: {e}"))?;
        unsafe { init.Initialize(HWND(hwnd as *mut _)) }.map_err(|e| format!("couldn't attach the window: {e}"))?;
    }
    let updates = context.GetAppAndOptionalStorePackageUpdatesAsync().map_err(|e| list_error(e.to_string()))?;
    let updates = answer_within(updates, LIST_TIMEOUT).map_err(list_error)?;
    let listed = updates.Size().map_err(|e| format!("couldn't count updates: {e}"))?;
    let versions = (0..listed).map(|i| updates.GetAt(i).ok().and_then(|u| update_version(&u))).collect();
    Ok((context, versions))
}

/// The IPC commands `store_update_install` (args `{ version }`: the release being installed) and
/// `store_update_check`, `store_update_came_back` (what the Store's last install left, see
/// `store_restart`) and `store_update_restart` (close AgentMux now and open it again on the new version,
/// after an install that ended `staged`). The Store calls block, so they run on the blocking pool.
pub async fn run(state: &std::sync::Arc<crate::state::AppState>, cmd: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    tracing::info!("[ipc] {cmd}");
    let gone = |e: tokio::task::JoinError| format!("store_update: {e}");
    match cmd {
        "store_update_check" => {
            let started = Instant::now();
            let check = tokio::task::spawn_blocking(check_update).await.map_err(gone)?;
            tracing::info!("{}", check_log_line(&check, started.elapsed()));
            json(check)
        }
        "store_update_came_back" => json(super::store_restart::came_back()),
        "store_update_restart" => {
            let state = state.clone();
            json(tokio::task::spawn_blocking(move || super::store_restart::restart_now(&state)).await.map_err(gone)?)
        }
        _ => {
            let target = args.get("version").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let (web_endpoint, auth_key) = (state.backend_endpoints.lock().web_endpoint.clone(), state.auth_key.lock().clone());
            let sender = state.clone();
            let emit: Emit = Arc::new(move |p| {
                if let Ok(payload) = serde_json::to_value(p) {
                    crate::events::emit_event_all_windows(&sender, store_progress::EVENT, &payload);
                }
            });
            json(tokio::task::spawn_blocking(move || install_update(&target, &web_endpoint, &auth_key, emit)).await.map_err(gone)?)
        }
    }
}

fn json<T: Serialize>(v: T) -> Result<serde_json::Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// Ask the Store to install AgentMux's update. Blocks until the Store is done
/// (minutes, for a download), so call it off the async workers.
#[cfg(target_os = "windows")]
pub fn install_update(target: &str, web_endpoint: &str, auth_key: &str, emit: Emit) -> StoreOutcome {
    let (target, web_endpoint, auth_key) = (target.to_string(), web_endpoint.to_string(), auth_key.to_string());
    let (endpoint, key) = (web_endpoint.clone(), auth_key.clone());
    let outcome = match on_winrt_thread("store-update", move || install_update_here(&target, &endpoint, &key, &emit).unwrap_or_else(|e| refused(&e))) {
        Ok(outcome) => outcome,
        Err(why) => refused(&why),
    };
    tracing::info!("[store-install] outcome={} message={}", outcome.outcome, outcome.message);
    // Back here. If the Store installed it, it closes this app at its own pace and the next start says how it
    // ended; if it staged it, it is waiting for this app to be closed, and the marker says how that ended.
    // Any other result closes nothing: there is nothing to come back from, and the record saved as a
    // quit's is saved again as for no quit (a later crash restores by the usual rules).
    if super::store_restart::install_returned(matches!(outcome.outcome, "installed" | "staged")) {
        crate::client::backend_save_session_snapshot(&web_endpoint, &auth_key, false);
    }
    outcome
}

#[cfg(target_os = "windows")]
fn store_state(s: windows::Services::Store::StorePackageUpdateState) -> StoreState {
    use windows::Services::Store::StorePackageUpdateState as S;
    match s {
        S::Completed => StoreState::Completed,
        S::Canceled => StoreState::Canceled,
        S::ErrorLowBattery => StoreState::LowBattery,
        S::ErrorWiFiRecommended => StoreState::WifiRecommended,
        S::ErrorWiFiRequired => StoreState::WifiRequired,
        _ => StoreState::Other,
    }
}

#[cfg(target_os = "windows")]
#[cfg(target_os = "windows")]
fn update_state(s: windows::Services::Store::StorePackageUpdateState) -> store_progress::UpdateState {
    use store_progress::UpdateState as U;
    use windows::Services::Store::StorePackageUpdateState as S;
    match s {
        S::Pending => U::Pending,
        S::Downloading => U::Downloading,
        S::Deploying => U::Deploying,
        _ => U::Finished,
    }
}

/// An operation of the Store that reports progress.
#[cfg(target_os = "windows")]
type StoreOp = windows_future::IAsyncOperationWithProgress<windows::Services::Store::StorePackageUpdateResult, windows::Services::Store::StorePackageUpdateStatus>;

/// Listens to an operation's progress and sends each change of what the page shows to `emit`.
#[cfg(target_os = "windows")]
fn report_progress(op: &StoreOp, emit: &Emit) {
    use windows::Services::Store::{StorePackageUpdateResult, StorePackageUpdateStatus};
    use windows_future::AsyncOperationProgressHandler;
    let emit = emit.clone();
    let mut throttle = store_progress::Throttle::default();
    let handler = AsyncOperationProgressHandler::<StorePackageUpdateResult, StorePackageUpdateStatus>::new(move |_, status| {
        if let Ok(s) = status.ok() {
            let p = store_progress::from_status(update_state(s.PackageUpdateState), s.PackageBytesDownloaded, s.PackageDownloadSizeInBytes, s.TotalDownloadProgress);
            if throttle.admits(&p) {
                emit(&p);
            }
        }
        Ok(())
    });
    if let Err(e) = op.SetProgress(&handler) {
        tracing::warn!(error = %e, "[store-install] could not listen to the Store's progress");
    }
}

/// The result of `op` once it has finished, or `None` when it is still open after `limit` (left open, not
/// cancelled: an install the Store holds is the Store's to finish).
#[cfg(target_os = "windows")]
fn finished_within(op: &StoreOp, limit: Duration) -> Result<Option<windows::Services::Store::StorePackageUpdateResult>, String> {
    use windows_future::AsyncStatus;
    let started = || op.Status().map_or(false, |s| s == AsyncStatus::Started);
    if wait_until(limit, Duration::from_millis(100), || !started()) {
        return op.get().map(Some).map_err(|e| e.to_string());
    }
    Ok(None)
}

#[cfg(target_os = "windows")]
fn install_update_here(target: &str, web_endpoint: &str, auth_key: &str, emit: &Emit) -> Result<StoreOutcome, String> {
    let (context, versions) = list_updates()?;
    let listed = versions.len();
    if listed == 0 {
        return Ok(decide(0, None));
    }
    // The calls below take the list itself, so ask for it again.
    let updates = context.GetAppAndOptionalStorePackageUpdatesAsync().map_err(|e| list_error(e.to_string()))?;
    let updates = answer_within(updates, LIST_TIMEOUT).map_err(list_error)?;
    emit(&store_progress::starting());
    // Downloaded first (it takes minutes and the app is not touched): only the install closes this app.
    let op = context.RequestDownloadStorePackageUpdatesAsync(&updates).map_err(|e| format!("the download call failed: {e}"))?;
    report_progress(&op, emit);
    let Some(downloaded) = finished_within(&op, DOWNLOAD_LIMIT).map_err(|e| format!("the download call failed: {e}"))? else {
        let _ = op.Cancel();
        return Err("the Store stopped downloading".into());
    };
    let state = store_state(downloaded.OverallState().map_err(|e| format!("no result state: {e}"))?);
    if state != StoreState::Completed {
        return Ok(decide(listed, Some(state)));
    }
    // The package is here and staged. Register the install for a reopen after, and save every window as a
    // quit saves them (the closes that follow keep the record), without the quit's background-work record.
    super::store_restart::before_install(target);
    crate::client::save_session_snapshot_with(web_endpoint, auth_key, true, false);
    emit(&store_progress::downloaded(0));
    let op = context.RequestDownloadAndInstallStorePackageUpdatesAsync(&updates).map_err(|e| format!("the install call failed: {e}"))?;
    report_progress(&op, emit);
    let install = match finished_within(&op, INSTALL_GRACE).map_err(|e| format!("the install call failed: {e}"))? {
        Some(result) => Install::Finished(store_state(result.OverallState().map_err(|e| format!("no result state: {e}"))?)),
        None => Install::WaitingForClose,
    };
    Ok(decide_install(listed, install))
}

/// Only Windows has a Store: elsewhere the page opens the Store page (it never offers this).
#[cfg(not(target_os = "windows"))]
pub fn install_update(_target: &str, _web_endpoint: &str, _auth_key: &str, _emit: Emit) -> StoreOutcome {
    refused("the Microsoft Store is Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_check_leaves_one_line_with_the_answer_and_the_time() {
        let took = Duration::from_millis(5400);
        let none = decide_check(Ok(vec![]));
        assert_eq!(check_log_line(&none, took), "[store-check] state=none version=- took_ms=5400");
        let available = decide_check(Ok(vec![Some((0, 60, 22))]));
        assert_eq!(check_log_line(&available, took), "[store-check] state=available version=0.60.22 took_ms=5400");
        let unknown = decide_check(Err("the Microsoft Store is Windows only".into()));
        assert_eq!(check_log_line(&unknown, took), "[store-check] state=unknown version=- took_ms=5400 why=the Microsoft Store is Windows only");
        let silent = decide_check(Err(NO_ANSWER.into()));
        assert_eq!(check_log_line(&silent, Duration::from_secs(60)), "[store-check] state=no_answer version=- took_ms=60000");
    }

    #[test]
    fn nothing_listed_means_the_store_has_no_newer_version() {
        let o = decide(0, None);
        assert_eq!(o.outcome, "none");
        assert_eq!(o.message, "The Microsoft Store has no newer version.");
        // Even if a state came with it, an empty list is "none".
        assert_eq!(decide(0, Some(StoreState::Completed)).outcome, "none");
    }

    #[test]
    fn a_completed_install_is_installed() {
        assert_eq!(decide(1, Some(StoreState::Completed)).outcome, "installed");
        assert_eq!(decide(3, Some(StoreState::Completed)).outcome, "installed");
    }

    /// An install call that stays open is the Store waiting for AgentMux to be closed (the 2026-11-12 run on
    /// Area54: the item went BlockedOnUser): told as that, not left as "asking the Store".
    #[test]
    fn an_install_that_waits_for_a_close_is_staged_and_a_finished_one_is_decided_as_before() {
        let staged = decide_install(1, Install::WaitingForClose);
        assert_eq!(staged.outcome, "staged");
        assert!(staged.message.contains("Close AgentMux"), "{}", staged.message);
        assert_eq!(decide_install(1, Install::Finished(StoreState::Completed)).outcome, "installed");
        assert_eq!(decide_install(1, Install::Finished(StoreState::Canceled)).outcome, "canceled");
        assert_eq!(decide_install(1, Install::Finished(StoreState::Other)).outcome, "fallback");
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
        assert!(o.message.contains("0x80070490"), "{}", o.message);
    }

    /// A Store that never answers is its own outcome, in the check and in the install: told plainly,
    /// not as a call that failed.
    #[test]
    fn a_store_that_never_answers_is_told_as_that() {
        let check = decide_check(Err(NO_ANSWER.into()));
        assert_eq!((check.state, check.message.as_str(), check.version.as_str()), ("no_answer", "", ""));
        let install = refused(NO_ANSWER);
        assert_eq!(install.outcome, "fallback");
        assert!(install.message.contains("didn't answer"), "{}", install.message);
    }

    /// The wait for the Store's listing, with a stand-in that never completes and one that does.
    #[test]
    fn the_wait_gives_up_at_the_limit_and_returns_at_once_when_done() {
        let started = Instant::now();
        let mut polls = 0;
        assert!(!wait_until(Duration::from_millis(60), Duration::from_millis(5), || {
            polls += 1;
            false
        }));
        assert!(polls >= 2 && started.elapsed() >= Duration::from_millis(60) && started.elapsed() < Duration::from_secs(5));
        let mut left = 3;
        assert!(wait_until(Duration::from_secs(30), Duration::from_millis(1), || {
            left -= 1;
            left == 0
        }));
        let quick = Instant::now();
        assert!(wait_until(Duration::from_secs(30), Duration::from_secs(10), || true));
        assert!(quick.elapsed() < Duration::from_secs(1), "an answer that is already there is not waited for");
    }

    #[test]
    fn a_check_says_what_the_store_listed_and_never_guesses() {
        let one = decide_check(Ok(vec![Some((0, 60, 17))]));
        assert_eq!((one.state, one.version.as_str()), ("available", "0.60.17"));
        let none = decide_check(Ok(vec![]));
        assert_eq!((none.state, none.message.as_str(), none.version.as_str()), ("none", "", ""));
        let unknown = decide_check(Err("no Store context: 0x80070490".into()));
        assert_eq!((unknown.state, unknown.version.as_str()), ("unknown", ""));
        assert!(unknown.message.contains("0x80070490"));
    }

    /// The offer is the package version the Store lists, not whatever GitHub's latest tag is.
    #[test]
    fn the_offered_version_is_the_highest_one_the_store_lists() {
        let many = decide_check(Ok(vec![Some((0, 60, 9)), Some((0, 60, 17)), None, Some((0, 59, 40))]));
        assert_eq!((many.state, many.version.as_str()), ("available", "0.60.17"));
        // Numbers, not text: 0.60.100 is newer than 0.60.17.
        assert_eq!(decide_check(Ok(vec![Some((0, 60, 17)), Some((0, 60, 100))])).version, "0.60.100");
        // Listed, but no readable version: still an update the Store has, with nothing to name.
        let unreadable = decide_check(Ok(vec![None]));
        assert_eq!((unreadable.state, unreadable.version.as_str()), ("available", ""));
    }

    #[test]
    fn the_check_serialises_its_version_for_the_page() {
        let v = serde_json::to_value(decide_check(Ok(vec![Some((0, 60, 17))]))).unwrap();
        assert_eq!((v["state"].as_str(), v["version"].as_str()), (Some("available"), Some("0.60.17")));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn off_windows_a_check_is_unknown_not_none() {
        assert_eq!(check_update().state, "unknown");
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
        assert_eq!(install_update("0.60.15", "", "", Arc::new(|_| {})).outcome, "fallback");
    }
}
