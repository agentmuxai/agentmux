// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The "Updating to <version>" window of an in-app upgrade that has to stop the running launcher first: a
//! `setup.exe` install on Windows, a `.dmg` install on macOS.
//!
//! The old launcher has to exit so the installer can replace its files, and the installer runs hidden
//! (`upgrade_restart::installer_script`): from the moment the person confirms the restart until the new
//! version's own splash appears, nothing of AgentMux is on screen. This window fills that gap. It looks like
//! the splash (same logo, same card) and its footer says which version is coming.
//!
//! It is a copy of this launcher, started from a folder in the temp directory (the install folder's files are
//! about to be replaced, and a running one cannot be), with `--updating-window <version> <script pid> <folder>`.
//!
//! On macOS the copy is of the whole `.app`, made with `clonefile(2)` (instant, and no disk space: the clone shares
//! the original's blocks). A launcher copied out of its bundle on its own is killed by the kernel on current macOS
//! (its signature covers the bundle's `Info.plist`, and `AppleSystemPolicy` refuses it); a clone of the bundle keeps
//! the signature valid and notarized, so Gatekeeper accepts it, and it never carries the quarantine flag. Where the
//! bundle can't be cloned (a volume without `clonefile`), there is no window and the upgrade goes on without one.
//! The new launcher's splash tells the window it is up by creating [`new_splash_marker`] in the window's folder, and
//! the hand-off script (the macOS one) leaves [`failed_marker`] when the swap failed.
//! It shows nothing but that text and ends by itself, never by waiting on anything that may not come:
//!
//! - the new launcher's splash is on screen (it sets the event [`event_name`]): the normal end;
//! - the hand-off script has ended and the new splash has not come within [`QUIET_END`]: gone quietly;
//! - the script ended after an install that did not succeed (it leaves [`failed_marker`]): it says so for
//!   [`FAILED_HOLD`], while the script starts the version the person had, then ends;
//! - [`SLOW_AFTER`]: the text says it is taking longer; [`HARD_CAP`]: it ends whatever happens.
//!
//! The upgrade does not depend on it: if the copy cannot be made or started, the upgrade goes on without it.
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

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

/// A hand-off folder left this long ago is removed by the next start (the window removes its own on macOS).
const STALE_AFTER: Duration = Duration::from_secs(3600);

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

/// The file the new launcher's splash leaves in the window's folder when it is on screen (macOS: the Windows window
/// uses [`event_name`] instead).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn new_splash_marker(dir: &Path) -> PathBuf {
    dir.join("new-splash")
}

/// Whether `name` is the folder name of a hand-off's window ([`dir`]).
fn is_hand_off_dir_name(name: &str) -> bool {
    name.strip_prefix(DIR_PREFIX).is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
}

/// Tell every hand-off window with a folder in `root` that the new launcher's splash is on screen. Nothing happens
/// when none is up (the usual case: it is one directory listing). A marker is created new, never over a link.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn signal_new_splash_in(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if is_dir && is_hand_off_dir_name(&e.file_name().to_string_lossy()) {
            let _ = std::fs::OpenOptions::new().write(true).create_new(true).open(new_splash_marker(&e.path()));
        }
    }
}

/// Remove the folders of hand-offs long over from `root`: those last changed more than `older_than` ago. A running
/// copy can't delete itself on Windows, so each start tidies the earlier ones. Returns how many it removed.
pub fn remove_stale_in(root: &Path, older_than: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else { return 0 };
    let mut removed = 0;
    for e in entries.flatten() {
        let old = is_hand_off_dir_name(&e.file_name().to_string_lossy())
            && e.file_type().is_ok_and(|t| t.is_dir())
            && e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > older_than);
        if old && std::fs::remove_dir_all(e.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// What `--updating-window` was given: the version to show, the hand-off script's pid, the folder. `None` when the
/// arguments are not a request for the window; `Some(Err(()))` when they are but are not usable.
fn parse_request(args: &[String]) -> Option<Result<(String, u32, PathBuf), ()>> {
    if args.get(1).map(String::as_str) != Some(ARG) {
        return None;
    }
    let (Some(version), Some(pid), Some(dir)) = (args.get(2), args.get(3).and_then(|p| p.parse::<u32>().ok()), args.get(4)) else {
        return Some(Err(()));
    };
    Some(if valid_version(version) { Ok((version.clone(), pid, PathBuf::from(dir))) } else { Err(()) })
}

/// The name of the event the new launcher's splash sets when it is on screen (the session's own namespace).
#[cfg_attr(not(windows), allow(dead_code))]
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
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn footer(version: &str, p: Phase) -> Vec<String> {
    match p {
        Phase::Failed => vec![format!("Could not update to v{version}"), "Opening the version you had".into()],
        Phase::Slow => vec![format!("Updating to v{version}"), "Taking longer than usual".into()],
        _ => vec![format!("Updating to v{version}"), "AgentMux opens again by itself".into()],
    }
}

/// [`footer`] for the macOS card: its footer is 13 pt fixed-pitch text on a 220 pt card, 26 characters at most, so
/// the lines are shorter. Same meaning, ASCII only.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn footer_mac(version: &str, p: Phase) -> Vec<String> {
    match p {
        Phase::Failed => vec![format!("Update to v{version} failed"), "Opening your old version".into()],
        Phase::Slow => vec![format!("Updating to v{version}"), "Taking longer than usual".into()],
        _ => vec![format!("Updating to v{version}"), "Opens again by itself".into()],
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

    /// This launcher is the window's copy: `Some(exit code)` when it was.
    fn run_if_requested(args: &[String]) -> Option<i32> {
        match parse_request(args)? {
            Ok((version, pid, dir)) => {
                crate::splash::run_updating_window(&version, pid, &dir);
                Some(0)
            }
            Err(()) => Some(2),
        }
    }

    /// `main`'s first look at the arguments: `Some(exit code)` when this launcher is the window's copy, which shows
    /// the window and nothing else; otherwise the folders of earlier upgrades' windows are tidied (a running copy
    /// can't delete its own folder) and the start goes on.
    pub fn early(args: &[String]) -> Option<i32> {
        if let Some(code) = run_if_requested(args) {
            return Some(code);
        }
        remove_stale_in(&std::env::temp_dir(), STALE_AFTER);
        None
    }
}

#[cfg(windows)]
pub use win::{early, signal_new_splash, start};

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::CommandExt;

    /// Called by the new launcher once its splash (or, with the splash off, its menu-bar item) is up: ends the
    /// window, if one is up.
    pub fn signal_new_splash() {
        signal_new_splash_in(&std::env::temp_dir());
    }

    /// `to` as a clone of the directory tree `from`: `clonefile(2)`, which shares the blocks (nothing is read or
    /// written, so it takes well under a second for the whole app and uses no disk) and keeps every link and
    /// attribute as it is, so the bundle's signature still verifies. It fails, and changes nothing, where the two
    /// are not on one volume that can clone.
    pub(super) fn clone_tree(from: &Path, to: &Path) -> std::io::Result<()> {
        let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e));
        let (src, dst) = (c(from)?, c(to)?);
        // CLONE_NOFOLLOW (1): a link at `from` itself is not followed.
        // SAFETY: both are NUL-terminated paths that outlive the call; `clonefile` only reads them.
        if unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), 1) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Where this launcher's copy lives inside a clone of `app` made at `clone`: the same path under it.
    pub(super) fn exe_in_clone(exe: &Path, app: &Path, clone: &Path) -> Option<PathBuf> {
        exe.strip_prefix(app).ok().map(|rel| clone.join(rel))
    }

    /// Clone the `.app` this launcher is in to `dir` and start the window from the clone. `script_pid` is the swap
    /// script, whose end the window watches. Not needed by the upgrade: an error only means no window.
    pub fn start(version: &str, script_pid: u32, app: &Path, dir: &Path) -> std::io::Result<()> {
        let invalid = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidInput, m.to_string());
        if !valid_version(version) {
            return Err(invalid("not a release version"));
        }
        let exe = std::env::current_exe()?;
        let clone = dir.join(app.file_name().ok_or_else(|| invalid("the app has no name"))?);
        let copy = exe_in_clone(&exe, app, &clone).ok_or_else(|| invalid("this launcher is not in the app"))?;
        // Only this user: the folder holds the failed marker the swap script writes.
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        if let Err(e) = clone_tree(app, &clone) {
            let _ = std::fs::remove_dir_all(dir);
            return Err(e);
        }
        let mut cmd = std::process::Command::new(&copy);
        cmd.arg(ARG).arg(version).arg(script_pid.to_string()).arg(dir).current_dir(dir);
        // Like the script: what the next version must not inherit of this one's AgentMux environment stays out.
        crate::upgrade_restart::restart_env(&mut cmd, std::env::vars_os());
        // Its own process group, like the script: it outlives this launcher.
        cmd.process_group(0).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if let Err(e) = cmd.spawn() {
            let _ = std::fs::remove_dir_all(dir);
            return Err(e);
        }
        crate::log(&format!("upgrade: updating window started for v{version} (swap script pid {script_pid}, from {})", copy.display()));
        Ok(())
    }

    /// `dir` is a hand-off window's folder in this user's temp directory, and so safe for the window to delete when
    /// it is done (it is given as an argument, so it is checked, not trusted).
    fn owns(dir: &Path) -> bool {
        let named = dir.file_name().is_some_and(|n| is_hand_off_dir_name(&n.to_string_lossy()));
        let parent = dir.parent().and_then(|p| p.canonicalize().ok());
        named && parent.is_some() && parent == std::env::temp_dir().canonicalize().ok()
    }

    /// `main`'s first look at the arguments: `Some(exit code)` when this launcher is the window's clone, which shows
    /// the window, removes its folder (the clone, the markers) and nothing else; otherwise the folders of earlier
    /// upgrades' windows are tidied and the start goes on.
    pub fn early(args: &[String]) -> Option<i32> {
        match parse_request(args) {
            Some(Ok((version, pid, dir))) => {
                crate::splash_mac::run_updating_window(&version, pid, &dir);
                if owns(&dir) {
                    let _ = std::fs::remove_dir_all(&dir);
                }
                Some(0)
            }
            Some(Err(())) => Some(2),
            None => {
                remove_stale_in(&std::env::temp_dir(), STALE_AFTER);
                None
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{early, signal_new_splash, start};

/// Nothing to do on Linux.
#[cfg(not(any(windows, target_os = "macos")))]
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
    fn the_mac_footer_says_the_same_and_fits_the_card() {
        for p in [Phase::Updating, Phase::Slow, Phase::Failed] {
            let lines = footer_mac("0.60.35", p);
            assert_eq!(lines.len(), 2);
            assert!(lines.iter().all(|l| l.is_ascii() && l.len() <= 26), "{p:?}: {lines:?}");
            assert!(lines[0].contains("v0.60.35"), "{p:?}");
        }
        assert_eq!(footer_mac("0.60.35", Phase::Updating)[0], footer("0.60.35", Phase::Updating)[0]);
        assert!(footer_mac("0.60.35", Phase::Failed)[0].contains("failed"));
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

    #[test]
    fn the_new_splash_tells_every_window_folder_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("agentmux-updating-4242");
        let b = root.path().join("agentmux-updating-77");
        let not_ours = root.path().join("agentmux-updating-extra");
        let file = root.path().join("agentmux-updating-9");
        for d in [&a, &b, &not_ours] {
            std::fs::create_dir(d).unwrap();
        }
        std::fs::write(&file, b"").unwrap();
        signal_new_splash_in(root.path());
        assert!(new_splash_marker(&a).exists() && new_splash_marker(&b).exists());
        assert!(!new_splash_marker(&not_ours).exists(), "only <prefix><pid> folders");
        assert!(file.is_file(), "a file of that name is left alone");
        // Again, and with no window up: nothing breaks.
        signal_new_splash_in(root.path());
        signal_new_splash_in(&root.path().join("missing"));
    }

    #[cfg(unix)]
    #[test]
    fn a_marker_is_never_written_through_a_link() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("agentmux-updating-5");
        std::fs::create_dir(&dir).unwrap();
        let target = root.path().join("precious");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, new_splash_marker(&dir)).unwrap();
        signal_new_splash_in(root.path());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn old_window_folders_are_removed_and_new_ones_kept() {
        let root = tempfile::tempdir().unwrap();
        let hand_off = root.path().join("agentmux-updating-100");
        let other = root.path().join("something-else");
        for d in [&hand_off, &other] {
            std::fs::create_dir(d).unwrap();
            std::fs::write(d.join("f"), b"x").unwrap();
        }
        assert_eq!(remove_stale_in(root.path(), Duration::from_secs(3600)), 0, "a fresh folder stays");
        assert!(hand_off.exists());
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(remove_stale_in(root.path(), Duration::ZERO), 1);
        assert!(!hand_off.exists());
        assert!(other.exists(), "only a hand-off's folder is touched");
        assert_eq!(remove_stale_in(&root.path().join("missing"), Duration::ZERO), 0);
    }

    #[test]
    fn the_window_arguments_are_checked_before_anything_is_drawn() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_request(&a(&["agentmux"])).is_none());
        assert!(parse_request(&a(&["agentmux", "--version"])).is_none());
        let ok = parse_request(&a(&["agentmux", ARG, "0.60.36", "4242", "/tmp/agentmux-updating-1"])).unwrap().unwrap();
        assert_eq!(ok, ("0.60.36".to_string(), 4242, PathBuf::from("/tmp/agentmux-updating-1")));
        for bad in [
            a(&["agentmux", ARG]),
            a(&["agentmux", ARG, "0.60.36", "4242"]),
            a(&["agentmux", ARG, "0.60.36", "not-a-pid", "/tmp/x"]),
            a(&["agentmux", ARG, "0.60.36; rm", "4242", "/tmp/x"]),
        ] {
            assert!(parse_request(&bad).unwrap().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_a_pid_named_folder_is_a_hand_off_folder() {
        assert!(is_hand_off_dir_name("agentmux-updating-4242"));
        for bad in ["agentmux-updating-", "agentmux-updating-x", "agentmux-updating-12a", "agentmux-updating", "other-4242", ""] {
            assert!(!is_hand_off_dir_name(bad), "{bad}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_clones_launcher_is_at_the_same_place_in_the_bundle() {
        let exe = Path::new("/Applications/AgentMux.app/Contents/MacOS/agentmux-launcher");
        let clone = Path::new("/var/folders/x/T/agentmux-updating-7/AgentMux.app");
        assert_eq!(
            mac::exe_in_clone(exe, Path::new("/Applications/AgentMux.app"), clone),
            Some(clone.join("Contents/MacOS/agentmux-launcher"))
        );
        assert_eq!(mac::exe_in_clone(exe, Path::new("/Users/u/Other.app"), clone), None);
    }

    /// The clone the window runs from is a whole bundle made without copying a byte: files, links and modes as they
    /// were, and a second one refused rather than merged over the first.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_app_is_cloned_whole_and_never_over_an_existing_folder() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("AgentMux.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), b"plist").unwrap();
        std::fs::write(app.join("Contents/MacOS/agentmux-launcher"), b"exe").unwrap();
        std::fs::set_permissions(app.join("Contents/MacOS/agentmux-launcher"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("Contents/MacOS", app.join("link")).unwrap();
        let clone = root.path().join("clone/AgentMux.app");
        std::fs::create_dir(root.path().join("clone")).unwrap();
        match mac::clone_tree(&app, &clone) {
            Ok(()) => {}
            // A volume that can't clone: the real start then makes no window, which is the designed fallback.
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTSUP) | Some(libc::EXDEV)) => return,
            Err(e) => panic!("{e}"),
        }
        assert_eq!(std::fs::read(clone.join("Contents/Info.plist")).unwrap(), b"plist");
        assert_eq!(std::fs::metadata(clone.join("Contents/MacOS/agentmux-launcher")).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(std::fs::read_link(clone.join("link")).unwrap(), Path::new("Contents/MacOS"));
        assert!(mac::clone_tree(&app, &clone).is_err(), "an existing folder is never merged into");
    }
}
