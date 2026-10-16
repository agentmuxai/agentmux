// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Upgrade & restart" (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`
//! §4.1, phases 3b and 3d): restart a Windows portable into the newer
//! release srv downloaded, verified and unpacked next to it, or run the newer
//! release's installer on a `setup.exe` install (`backend/update_download.rs`).
//!
//! srv asks with an `AGENTMUXSRV-UPGRADE` line on its stderr
//! (`agentmux_common::srv_stderr`); the reader in `srv_spawner` hands it here.
//! The supervisor then:
//! 1. checks what srv names again ([`staged`]): an AgentMux portable of
//!    exactly that version, newer than this one, beside a portable; or that
//!    version's installer, in this version's own `updates/<v>/`, on a
//!    `setup.exe` install;
//! 2. asks the user with a native dialog ([`confirm`]), which no page or agent
//!    tool in AgentMux can click: a restart stops every agent, so it takes the
//!    user's own answer here, whoever asked srv;
//! 3. quits the host the way the tray's Quit does, which closes the windows and
//!    saves the session;
//! 4. after srv has stopped (so the new version copies complete data,
//!    `srv/src/bootstrap/carry_forward.rs`), starts the new launcher, or a
//!    hidden `cmd` that waits for this launcher to be gone, runs the
//!    installer silently and starts AgentMux again ([`relaunch`]), and exits.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::srv_stderr::{UpgradeRequest, UpgradeTarget};
use tokio::sync::mpsc;

/// A confirmed restart that hasn't happened within this long (the host
/// never quit) no longer restarts into the new version on a later quit.
pub const CONFIRMED_FOR: Duration = Duration::from_secs(5 * 60);

static REQUESTS: Mutex<Option<mpsc::UnboundedSender<UpgradeRequest>>> = Mutex::new(None);
/// A Yes counts only this long after its dialog opened (the dialog itself
/// can't time out). Shorter than srv's wait (`RESTART_GRACE`, 3 minutes in
/// `srv/src/backend/update_restart.rs`), so by the time the panel says the
/// restart didn't happen, no open dialog can still start one.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(150);

/// The latest request asked about: only its dialog's Yes counts.
static LATEST_ASK: AtomicU64 = AtomicU64::new(0);
/// The thread showing the open dialog, 0 when none (Windows).
static DIALOG_THREAD: AtomicU32 = AtomicU32::new(0);

const DIALOG_TITLE: &str = "Upgrade AgentMux";

/// The supervisor's end of srv's upgrade requests. One per process: it lasts
/// across srv respawns, since the reader of each new srv sends here too.
pub fn requests() -> mpsc::UnboundedReceiver<UpgradeRequest> {
    let (tx, rx) = mpsc::unbounded_channel();
    *REQUESTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    rx
}

/// From the srv stderr reader.
pub fn forward(req: UpgradeRequest) {
    if let Some(tx) = REQUESTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(req);
    }
}

/// `major.minor.patch`, or `None`.
fn version_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['+', '-']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether `dir` holds Inno Setup's uninstaller (`unins000.exe`, …): a
/// `setup.exe` install.
fn has_uninstaller(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(|e| e.ok()).any(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("unins") && name.ends_with(".exe")
        })
    })
}

/// A portable folder. `setup.exe` installs the same `runtime/`, marker and
/// all, so a folder with Inno Setup's uninstaller isn't one.
fn is_portable(folder: &Path) -> bool {
    folder.join("runtime").join("agentmux-portable.marker").is_file() && !has_uninstaller(folder)
}

/// What a confirmed upgrade restarts into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staged {
    /// A portable's new folder: start its launcher.
    Launcher(PathBuf),
    /// A `setup.exe` install: run this installer for all users (an install
    /// under Program Files) or the current one, then start `own_dir` again.
    Installer { installer: PathBuf, own_dir: PathBuf, all_users: bool },
}

/// Whether `dir` is under Program Files, where a `setup.exe` install was made
/// for all users.
fn under_program_files(dir: &Path) -> bool {
    let dir = dir.to_string_lossy().to_ascii_lowercase();
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().any(|var| {
        std::env::var(var).is_ok_and(|root| !root.is_empty() && dir.starts_with(&root.to_ascii_lowercase()))
    })
}

/// What to restart into, if `req` names a newer release than `own_version`
/// that fits this install (in `own_dir`, with its data in `data_dir`): a
/// portable of exactly that version beside a portable, or that version's
/// installer in this version's own `updates/<v>/` on a `setup.exe` install.
pub fn staged(req: &UpgradeRequest, own_dir: &Path, data_dir: &Path, own_version: &str) -> Result<Staged, String> {
    match (version_core(&req.version), version_core(own_version)) {
        (Some(new), Some(own)) if new > own => {}
        _ => return Err(format!("{} isn't newer than {own_version}", req.version)),
    }
    match &req.target {
        UpgradeTarget::Launcher(launcher) => {
            let named_right = launcher.file_name().is_some_and(|n| n.eq_ignore_ascii_case("agentmux.exe"));
            if !launcher.is_absolute() || !named_right || !launcher.is_file() {
                return Err(format!("{} isn't an AgentMux launcher", launcher.display()));
            }
            let folder = launcher.parent().ok_or("the launcher has no folder")?;
            if !is_portable(own_dir) {
                return Err("this AgentMux isn't a portable install".into());
            }
            if !is_portable(folder) || !folder.join("runtime").join(format!("agentmux-{}.exe", req.version)).is_file() {
                return Err(format!("{} isn't an AgentMux {} portable", folder.display(), req.version));
            }
            if folder == own_dir {
                return Err("that is the running AgentMux".into());
            }
            Ok(Staged::Launcher(launcher.clone()))
        }
        UpgradeTarget::Installer(installer) => {
            let name = format!("AgentMux-{}-x64-setup.exe", req.version);
            let named_right = installer.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name.as_str()));
            let in_updates = installer.parent() == Some(data_dir.join("updates").join(&req.version).as_path());
            if !installer.is_absolute() || !named_right || !in_updates || !installer.is_file() {
                return Err(format!("{} isn't the AgentMux {} installer AgentMux downloaded", installer.display(), req.version));
            }
            if !has_uninstaller(own_dir) || !own_dir.join("agentmux.exe").is_file() {
                return Err("this AgentMux isn't a setup.exe install".into());
            }
            Ok(Staged::Installer {
                installer: installer.clone(),
                own_dir: own_dir.to_path_buf(),
                all_users: under_program_files(own_dir),
            })
        }
    }
}

/// Ask the user on a thread of its own, and call `on_yes` only for an
/// explicit Yes that still counts ([`answer_counts`]): one given within
/// [`ANSWER_WITHIN`], to the latest request. One dialog at a time: a newer
/// request closes the open one, and a dialog closes itself (as No) once its
/// time is up. A Yes that slips past either is told it expired.
pub fn ask_then(version: String, staged: Staged, on_yes: impl FnOnce(Staged) + Send + 'static) {
    let ask = LATEST_ASK.fetch_add(1, Ordering::SeqCst) + 1;
    dismiss(DIALOG_THREAD.load(Ordering::SeqCst));
    std::thread::spawn(move || {
        let opened = Instant::now();
        let thread = current_thread();
        DIALOG_THREAD.store(thread, Ordering::SeqCst);
        std::thread::spawn(move || {
            std::thread::sleep(ANSWER_WITHIN);
            if DIALOG_THREAD.load(Ordering::SeqCst) == thread {
                dismiss(thread);
            }
        });
        let yes = confirm(&version, &staged);
        let _ = DIALOG_THREAD.compare_exchange(thread, 0, Ordering::SeqCst, Ordering::SeqCst);
        if !yes {
            return;
        }
        if answer_counts(ask, LATEST_ASK.load(Ordering::SeqCst), opened.elapsed()) {
            on_yes(staged);
        } else {
            crate::log(&format!("upgrade: a Yes {}s after asking, or to an older request, was ignored", opened.elapsed().as_secs()));
            crate::show_fatal_dialog(
                "AgentMux upgrade",
                "This upgrade request expired, so AgentMux didn't restart.\n\nStart it again from the version panel.",
            );
        }
    });
}

/// Whether a Yes to request `ask`, given `waited` after its dialog opened,
/// still counts when `latest` is the newest request.
fn answer_counts(ask: u64, latest: u64, waited: Duration) -> bool {
    ask == latest && waited < ANSWER_WITHIN
}

#[cfg(target_os = "windows")]
fn current_thread() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(not(target_os = "windows"))]
fn current_thread() -> u32 {
    0
}

/// Close the dialog shown on `thread` by clicking its No button.
#[cfg(target_os = "windows")]
fn dismiss(thread: u32) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetDlgItem, GetWindowTextW, SendMessageW, BM_CLICK, IDNO,
    };
    unsafe extern "system" fn click_no(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut title = [0u16; 64];
        let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        if String::from_utf16_lossy(&title[..len]) == DIALOG_TITLE {
            let no = GetDlgItem(hwnd, IDNO);
            if !no.is_null() {
                SendMessageW(no, BM_CLICK, 0, 0);
            }
            return 0;
        }
        1
    }
    if thread != 0 {
        unsafe { EnumThreadWindows(thread, Some(click_no), 0) };
    }
}

#[cfg(not(target_os = "windows"))]
fn dismiss(_thread: u32) {}

#[cfg(target_os = "windows")]
fn confirm(version: &str, staged: &Staged) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    };
    let body = match staged {
        Staged::Launcher(launcher) => format!(
            "Restart AgentMux to upgrade to {version}?\n\n\
             Every window closes and every agent stops, then AgentMux {version} opens \
             from:\n{}\n\nYour tabs come back and agents resume their conversations.",
            launcher.parent().unwrap_or(launcher).display()
        ),
        Staged::Installer { all_users, .. } => format!(
            "Restart AgentMux to upgrade to {version}?\n\n\
             Every window closes and every agent stops, then the AgentMux {version} installer \
             runs{} and AgentMux opens again.\n\nYour tabs come back and agents resume their conversations.",
            if *all_users { " (Windows asks for permission: it is installed for all users)" } else { "" }
        ),
    };
    let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (title, body) = (wide(DIALOG_TITLE), wide(&body));
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
    answer == IDYES
}

#[cfg(not(target_os = "windows"))]
fn confirm(_version: &str, _staged: &Staged) -> bool {
    // Only the Windows portable restarts itself so far (phase 3b).
    false
}

/// Whether a restart confirmed at `at` still stands.
pub fn still_confirmed(at: Instant) -> bool {
    at.elapsed() < CONFIRMED_FOR
}

/// The hidden `cmd` line that upgrades a `setup.exe` install: wait a few
/// seconds for this launcher to be gone (its files are replaced), run the
/// installer silently, then start AgentMux again whatever the installer
/// did, so a failed install leaves the old version running, not none.
fn installer_script(installer: &Path, own_dir: &Path, all_users: bool) -> String {
    format!(
        "ping -n 4 127.0.0.1 >nul & start \"\" /wait \"{}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- {} & start \"\" \"{}\"",
        installer.display(),
        if all_users { "/ALLUSERS" } else { "/CURRENTUSER" },
        own_dir.join("agentmux.exe").display()
    )
}

/// Restart into `staged`, detached, without this AgentMux's own environment:
/// `AGENTMUX` would make the next launcher think it was started from a pane,
/// and the `AGENTMUX_*` this launcher set on itself (background service,
/// tray) would override the next one's settings.
pub fn relaunch(staged: &Staged) -> std::io::Result<()> {
    use agentmux_common::win32::NoWindow;
    let mut cmd = match staged {
        Staged::Launcher(launcher) => {
            let mut c = std::process::Command::new(launcher);
            if let Some(folder) = launcher.parent() {
                c.current_dir(folder);
            }
            c
        }
        Staged::Installer { installer, own_dir, all_users } => {
            let mut c = std::process::Command::new("cmd");
            c.args(["/d", "/c"]);
            #[cfg(target_os = "windows")]
            {
                // cmd parses its own line: passed as is, not re-quoted.
                use std::os::windows::process::CommandExt;
                c.raw_arg(installer_script(installer, own_dir, *all_users));
            }
            #[cfg(not(target_os = "windows"))]
            c.arg(installer_script(installer, own_dir, *all_users));
            c
        }
    };
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k == "AGENTMUX" || k.starts_with("AGENTMUX_") {
            cmd.env_remove(&key);
        }
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .no_window()
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portable(dir: &Path, version: &str) -> PathBuf {
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        std::fs::write(dir.join("runtime/agentmux-portable.marker"), b"").unwrap();
        std::fs::write(dir.join(format!("runtime/agentmux-{version}.exe")), b"").unwrap();
        std::fs::write(dir.join("agentmux.exe"), b"").unwrap();
        dir.join("agentmux.exe")
    }

    fn req(version: &str, launcher: &Path) -> UpgradeRequest {
        UpgradeRequest { version: version.into(), target: UpgradeTarget::Launcher(launcher.to_path_buf()) }
    }

    fn setup_req(version: &str, installer: &Path) -> UpgradeRequest {
        UpgradeRequest { version: version.into(), target: UpgradeTarget::Installer(installer.to_path_buf()) }
    }

    #[test]
    fn only_a_newer_portable_beside_a_portable_is_restarted_into() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let own = root.path().join("agentmux-0.59.26-x64-portable");
        portable(&own, "0.59.26");
        let new = portable(&root.path().join("agentmux-0.59.27-x64-portable"), "0.59.27");
        assert_eq!(staged(&req("0.59.27", &new), &own, &data, "0.59.26").unwrap(), Staged::Launcher(new.clone()));

        // Not newer, or not the version it claims.
        assert!(staged(&req("0.59.27", &new), &own, &data, "0.59.27").is_err());
        assert!(staged(&req("0.59.28", &new), &own, &data, "0.59.26").is_err());
        // Not a launcher, or not absolute.
        let other = root.path().join("agentmux-0.59.27-x64-portable/runtime/agentmux-0.59.27.exe");
        assert!(staged(&req("0.59.27", &other), &own, &data, "0.59.26").is_err());
        assert!(staged(&req("0.59.27", Path::new("agentmux.exe")), &own, &data, "0.59.26").is_err());
        // This AgentMux isn't a portable, including a setup.exe install,
        // which has the portable's runtime/ and marker too.
        let installed = root.path().join("Programs/AgentMux");
        portable(&installed, "0.59.26");
        std::fs::write(installed.join("unins000.exe"), b"").unwrap();
        assert!(staged(&req("0.59.27", &new), &installed, &data, "0.59.26").is_err());
    }

    #[test]
    fn only_that_versions_installer_in_this_versions_updates_runs_on_a_setup_install() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let own = root.path().join("Programs/AgentMux");
        portable(&own, "0.59.26");
        std::fs::write(own.join("unins000.exe"), b"").unwrap();
        let updates = data.join("updates/0.59.27");
        std::fs::create_dir_all(&updates).unwrap();
        let installer = updates.join("AgentMux-0.59.27-x64-setup.exe");
        std::fs::write(&installer, b"").unwrap();
        let Staged::Installer { installer: got, own_dir, .. } =
            staged(&setup_req("0.59.27", &installer), &own, &data, "0.59.26").unwrap()
        else {
            panic!("expected the installer")
        };
        assert_eq!((got, own_dir), (installer.clone(), own.clone()));

        // Not that version's name, not in this version's updates, or older.
        let renamed = updates.join("setup.exe");
        std::fs::write(&renamed, b"").unwrap();
        assert!(staged(&setup_req("0.59.27", &renamed), &own, &data, "0.59.26").is_err());
        let elsewhere = root.path().join("AgentMux-0.59.27-x64-setup.exe");
        std::fs::write(&elsewhere, b"").unwrap();
        assert!(staged(&setup_req("0.59.27", &elsewhere), &own, &data, "0.59.26").is_err());
        assert!(staged(&setup_req("0.59.27", &installer), &own, &data, "0.59.27").is_err());
        // A portable never runs an installer.
        let portable_dir = root.path().join("agentmux-0.59.26-x64-portable");
        portable(&portable_dir, "0.59.26");
        assert!(staged(&setup_req("0.59.27", &installer), &portable_dir, &data, "0.59.26").is_err());
    }

    #[test]
    fn the_installer_line_waits_runs_silently_and_always_starts_agentmux_again() {
        let line = installer_script(
            Path::new(r"C:\Users\A User\updates\0.59.27\AgentMux-0.59.27-x64-setup.exe"),
            Path::new(r"C:\Users\A User\AppData\Local\Programs\AgentMux"),
            false,
        );
        assert_eq!(
            line,
            r#"ping -n 4 127.0.0.1 >nul & start "" /wait "C:\Users\A User\updates\0.59.27\AgentMux-0.59.27-x64-setup.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /CURRENTUSER & start "" "C:\Users\A User\AppData\Local\Programs\AgentMux\agentmux.exe""#
        );
        assert!(installer_script(Path::new("i.exe"), Path::new("d"), true).contains("/ALLUSERS"));
    }

    #[test]
    fn a_late_or_superseded_yes_does_not_count() {
        assert!(answer_counts(3, 3, Duration::from_secs(10)));
        assert!(!answer_counts(3, 3, ANSWER_WITHIN));
        assert!(!answer_counts(2, 3, Duration::from_secs(10)));
        // The dialog must stop counting before srv gives up (3 minutes).
        assert!(ANSWER_WITHIN < Duration::from_secs(3 * 60));
    }

    #[test]
    fn a_confirmation_lapses() {
        assert!(still_confirmed(Instant::now()));
        if let Some(long_ago) = Instant::now().checked_sub(CONFIRMED_FOR + Duration::from_secs(1)) {
            assert!(!still_confirmed(long_ago));
        }
    }
}
