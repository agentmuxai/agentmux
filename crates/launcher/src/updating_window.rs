// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows: the "Updating to <version>" window of the in-app upgrade of a `setup.exe` install.
//!
//! The old launcher has to exit so the installer can replace its files, and the installer runs hidden
//! (`upgrade_restart::installer_script`): from the moment the person confirms the restart until the new
//! version's own splash appears, nothing of AgentMux is on screen. This window fills that gap. It looks like
//! the splash (same logo, same card) and its footer says which version is coming.
//!
//! It is a copy of this launcher, started from a folder in the temp directory (the install folder's files are
//! about to be replaced, and a running one cannot be), with `--updating-window <version> <script pid> <folder>`.
//! It shows nothing but that text and ends by itself, never by waiting on anything that may not come:
//!
//! - the new launcher's splash is on screen (it sets the event [`event_name`]): the normal end;
//! - the hand-off script has ended and the new splash has not come within [`QUIET_END`]: gone quietly;
//! - the script ended after an install that did not succeed (it leaves [`failed_marker`]): it says so for
//!   [`FAILED_HOLD`], while the script starts the version the person had, then ends;
//! - [`SLOW_AFTER`]: the text says it is taking longer; [`HARD_CAP`]: it ends whatever happens.
//!
//! The upgrade does not depend on it: if the copy cannot be made or started, the upgrade goes on without it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The argument that makes a launcher run only this window.
pub const ARG: &str = "--updating-window";

/// After this long with no end, the text says the update is taking longer than usual.
pub const SLOW_AFTER: Duration = Duration::from_secs(90);
/// The window never lives longer than this.
pub const HARD_CAP: Duration = Duration::from_secs(240);
/// How long after the hand-off script ended the window waits for the new splash before it leaves on its own.
pub const QUIET_END: Duration = Duration::from_secs(8);
/// How long a failed install's message stays after the script ended.
pub const FAILED_HOLD: Duration = Duration::from_secs(5);

/// The folder (in the temp directory) a hand-off's window runs from: `agentmux-updating-<pid of the old launcher>`.
const DIR_PREFIX: &str = "agentmux-updating-";

/// The folder this launcher's hand-off window runs from; the script's marker is in it too.
pub fn dir() -> PathBuf {
    std::env::temp_dir().join(format!("{DIR_PREFIX}{}", std::process::id()))
}

/// The file the hand-off script leaves when the install did not succeed (the installer's exit code was not 0, or the
/// installer was not the one checked): the window then says so.
pub fn failed_marker(dir: &Path) -> PathBuf {
    dir.join("failed")
}

/// The name of the event the new launcher's splash sets when it is on screen (the session's own namespace).
pub fn event_name() -> &'static str {
    "Local\\AgentMuxUpdatingWindow"
}

/// A version the window will show: what a release is called (digits, dots, and a `-`/`+` suffix), and no longer than
/// that. Anything else is not drawn: this process is started with it as an argument.
pub fn valid_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 32 && v.starts_with(|c: char| c.is_ascii_digit()) && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

/// What the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Updating,
    Slow,
    Failed,
    Close,
}

/// What the window knows each moment.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    /// Since the window started.
    pub elapsed: Duration,
    /// Since the hand-off script ended, once it has.
    pub script_ended_for: Option<Duration>,
    /// The new launcher's splash is on screen.
    pub new_splash: bool,
    /// The script ended and left [`failed_marker`].
    pub failed: bool,
}

/// What to show now, or that the window is done.
pub fn phase(i: &Inputs) -> Phase {
    if i.elapsed >= HARD_CAP {
        return Phase::Close;
    }
    if i.failed {
        return match i.script_ended_for {
            Some(after) if after >= FAILED_HOLD => Phase::Close,
            _ => Phase::Failed,
        };
    }
    if i.new_splash || i.script_ended_for.is_some_and(|after| after >= QUIET_END) {
        return Phase::Close;
    }
    if i.elapsed >= SLOW_AFTER {
        Phase::Slow
    } else {
        Phase::Updating
    }
}

/// The line in the splash's stage area. ASCII only: the splash font has no more.
pub fn headline(p: Phase) -> &'static str {
    match p {
        Phase::Slow => "Still updating...",
        Phase::Failed => "The update was not installed",
        _ => "Updating AgentMux...",
    }
}

/// The footer's two lines, in place of `user@host` and the version: the version shown is the one coming, never the
/// one that is leaving.
pub fn footer(version: &str, p: Phase) -> Vec<String> {
    match p {
        Phase::Failed => vec![format!("Could not update to v{version}"), "Opening the version you had".into()],
        Phase::Slow => vec![format!("Updating to v{version}"), "Taking longer than usual".into()],
        _ => vec![format!("Updating to v{version}"), "AgentMux opens again by itself".into()],
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use agentmux_common::win32::NoWindow;

    /// Called by the new launcher once its splash is on screen: ends the window, if one is up.
    pub fn signal_new_splash() {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
        let name: Vec<u16> = format!("{}\0", event_name()).encode_utf16().collect();
        unsafe {
            let ev = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
            if !ev.is_null() {
                SetEvent(ev);
                CloseHandle(ev);
            }
        }
    }

    /// Copy this launcher into `dir` and start the window from the copy. `script_pid` is the hand-off script, whose
    /// end the window watches. Not needed by the upgrade: an error only means no window.
    pub fn start(version: &str, script_pid: u32, dir: &Path) -> std::io::Result<()> {
        if !valid_version(version) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a release version"));
        }
        std::fs::create_dir_all(dir)?;
        let copy = dir.join("agentmux-updating.exe");
        std::fs::copy(std::env::current_exe()?, &copy)?;
        let mut cmd = std::process::Command::new(&copy);
        cmd.arg(ARG).arg(version).arg(script_pid.to_string()).arg(dir).current_dir(dir);
        // Like the script: what the next version must not inherit of this one's AgentMux environment stays out.
        crate::upgrade_restart::restart_env(&mut cmd, std::env::vars_os());
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).no_window().spawn()?;
        crate::log(&format!("upgrade: updating window started for v{version} (hand-off script pid {script_pid}, from {})", copy.display()));
        Ok(())
    }

    /// At start: remove the folders of hand-offs long over (a running copy can't delete itself).
    fn clean_stale() {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
        for e in entries.flatten() {
            let stale = e.file_name().to_string_lossy().starts_with(DIR_PREFIX)
                && e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(3600));
            if stale {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    /// This launcher is the window's copy: `Some(exit code)` when it was.
    fn run_if_requested(args: &[String]) -> Option<i32> {
        if args.get(1).map(String::as_str) != Some(ARG) {
            return None;
        }
        let (Some(version), Some(pid), Some(dir)) = (args.get(2), args.get(3).and_then(|p| p.parse::<u32>().ok()), args.get(4)) else {
            return Some(2);
        };
        if !valid_version(version) {
            return Some(2);
        }
        crate::splash::run_updating_window(version, pid, Path::new(dir));
        Some(0)
    }

    /// `main`'s first look at the arguments: `Some(exit code)` when this launcher is the window's copy, which shows
    /// the window and nothing else; otherwise the folders of earlier upgrades' windows are tidied (a running copy
    /// can't delete its own folder) and the start goes on.
    pub fn early(args: &[String]) -> Option<i32> {
        if let Some(code) = run_if_requested(args) {
            return Some(code);
        }
        clean_stale();
        None
    }
}

#[cfg(windows)]
pub use win::{early, signal_new_splash, start};

/// Nothing to do away from Windows.
#[cfg(not(windows))]
pub fn early(_args: &[String]) -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn at(elapsed: u64, ended: Option<u64>, new_splash: bool, failed: bool) -> Phase {
        phase(&Inputs { elapsed: S(elapsed), script_ended_for: ended.map(S), new_splash, failed })
    }

    #[test]
    fn it_shows_the_update_until_the_new_splash_is_up() {
        assert_eq!(at(0, None, false, false), Phase::Updating);
        assert_eq!(at(30, None, false, false), Phase::Updating);
        assert_eq!(at(30, None, true, false), Phase::Close);
    }

    #[test]
    fn a_long_install_says_it_is_taking_longer_and_never_outlives_the_cap() {
        assert_eq!(at(89, None, false, false), Phase::Updating);
        assert_eq!(at(90, None, false, false), Phase::Slow);
        assert_eq!(at(239, None, false, false), Phase::Slow);
        assert_eq!(at(240, None, false, false), Phase::Close);
        // The cap holds against everything else.
        assert_eq!(at(240, None, false, true), Phase::Close);
        assert_eq!(at(400, Some(1), false, true), Phase::Close);
    }

    #[test]
    fn it_leaves_quietly_when_the_script_ended_and_no_new_splash_came() {
        assert_eq!(at(20, Some(0), false, false), Phase::Updating);
        assert_eq!(at(25, Some(7), false, false), Phase::Updating);
        assert_eq!(at(26, Some(8), false, false), Phase::Close);
        // A splash that came after the script ended closes it at once.
        assert_eq!(at(21, Some(1), true, false), Phase::Close);
    }

    #[test]
    fn a_failed_install_is_said_for_a_few_seconds_even_when_the_version_it_had_is_already_up() {
        assert_eq!(at(20, Some(0), false, true), Phase::Failed);
        // The old version's own splash is up (it sets the event too): the message stays its time.
        assert_eq!(at(22, Some(2), true, true), Phase::Failed);
        assert_eq!(at(25, Some(5), true, true), Phase::Close);
        assert_eq!(at(25, Some(5), false, true), Phase::Close);
    }

    #[test]
    fn the_footer_names_the_version_coming_and_never_the_one_leaving() {
        assert_eq!(footer("0.60.35", Phase::Updating), vec!["Updating to v0.60.35", "AgentMux opens again by itself"]);
        assert_eq!(footer("0.60.35", Phase::Slow)[0], "Updating to v0.60.35");
        assert_eq!(footer("0.60.35", Phase::Failed), vec!["Could not update to v0.60.35", "Opening the version you had"]);
        for p in [Phase::Updating, Phase::Slow, Phase::Failed] {
            assert!(headline(p).is_ascii() && footer("0.60.35", p).iter().all(|l| l.is_ascii()), "{p:?}");
        }
    }

    #[test]
    fn only_a_release_version_is_drawn() {
        for ok in ["0.60.35", "1.2.3-rc.1", "0.60.35+build5"] {
            assert!(valid_version(ok), "{ok}");
        }
        for bad in ["", "v0.60.35", "0.60.35 DEV", "0.60.35\"", "0.60.35; calc", "..\\x", "1234567890123456789012345678901234567890"] {
            assert!(!valid_version(bad), "{bad}");
        }
    }

    #[test]
    fn its_folder_and_marker_are_in_the_temp_directory_and_named_for_the_old_launcher() {
        let d = dir();
        assert!(d.starts_with(std::env::temp_dir()));
        assert_eq!(d.file_name().unwrap().to_string_lossy(), format!("agentmux-updating-{}", std::process::id()));
        assert_eq!(failed_marker(&d), d.join("failed"));
    }
}
